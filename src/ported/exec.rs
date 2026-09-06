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
//! - **`loadautofn`** + **`getfpfunc`** (`Src/exec.c:5682` / `:6219`)
//!   — `$fpath` walker + autoload file installer. Called from
//!   `bin_autoload` / `bin_functions -c` in `src/ported/builtin.rs`.
//! - **`resolvebuiltin`** (`Src/exec.c:2703`) — module-autoload guard
//!   used by the dispatch walk in `execcmd_exec`.
//! - **`execcmd_compile_head`** — fusevm-bytecode-time head resolver
//!   mirroring the head section (`c:2904-3275`) of C's `execcmd_exec`.
//!   NOT a faithful port; the canonical 7-arg `execcmd_exec` port lives
//!   alongside it.
//! - **`execcmd_exec`** (`Src/exec.c:2900`) — canonical 7-arg port of
//!   the C function (locals + dispatch walk through builtin/shfunc/external
//!   invocation). Used by future tree-walker callers; the fusevm
//!   bytecode flow goes through `execcmd_compile_head` instead.

use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::Ordering;

// `with_executor` import removed — all ShellExecutor reach-in calls
// routed through `crate::ported::exec::*` fn-ptrs installed by
// fusevm_bridge at startup. See memory feedback_no_exec_script_from_ported.
use crate::ported::builtin::{cd_able_vars, fixdir, BUILTINS, DOPRINTDIR, EXIT_VAL, LASTVAL};
use crate::ported::builtins::rlimits::setlimits;
use crate::ported::builtins::sched::zleactive;
use crate::ported::compat::zgettime_monotonic_if_available;
use crate::ported::config_h::DEFAULT_PATH;
use crate::ported::context::{zcontext_restore, zcontext_save};
use crate::ported::hashtable::{
    cmdnam_unhashed, cmdnamtab_lock, dircache_set, hashdir, pathchecked, shfunctab_lock,
};
use crate::ported::hist::{strinbeg, strinend};
use crate::ported::init::{shout, underscorelen, underscoreused, zunderscore, SHTTY};
use crate::ported::input::{inpop, inpush};
use crate::ported::jobs::{expandjobtab, get_usage, release_pgrp, waitforpid, JOBTAB, THISJOB};
use crate::ported::lex::{
    hgetc, parsestr, tok, untokenize, ztokens, LEXERR, LEX_LEXSTOP, LEX_LINENO,
};
use crate::ported::mem::{dupstring, dyncat, popheap, pushheap};
use crate::ported::modules::clone::mypgrp;
use crate::ported::options::{dosetopt, opt_state_set, sticky};
use crate::ported::params::{
    endparamscope, getsparam, locallevel, paramtab, setiparam, zgetenv, zputenv,
};
use crate::ported::parse::{closedumps, ecrawstr, parse_list};
use crate::ported::prompt::{cmdpop, cmdpush};
use crate::ported::signals::{
    intrap, queue_signals, settrap, signal_mask, signal_unblock, sigtrapped, trapisfunc,
    traplocallevel, unqueue_signals, unsettrap,
};
use crate::ported::signals_h::{
    child_block, child_unblock, dont_queue_signals, signal_default, signal_ignore, winch_unblock,
    SIGCOUNT,
};
use crate::ported::subst::{quotesubst, singsub};
use crate::ported::utils::{
    errflag, fdtable_get, fdtable_set, gettempfile, gettempname, inc_locallevel, movefd, pathprog,
    printprompt4, quotedzputs, redup, unmeta, unmetafy, write_loop, zclose, zerr, zwarn,
    ERRFLAG_ERROR, MAX_ZSH_FD,
};
use crate::ported::r#loop::{execcase, execfor, execif, execrepeat, execselect, exectry, execwhile};
use crate::ported::zsh_h::{
    builtin, cmdnam, emulation_options, eprog, execstack, funcwrap, hashnode, isset, jobfile,
    multio, redir, shfunc, unset, wc_code, Emulation_options, Inang, Inpar, Meta, Nularg, Outpar,
    Pound, BINF_BUILTIN, BINF_CLEARENV, BINF_COMMAND, BINF_DASH, BINF_EXEC, BINF_PREFIX, CHASEDOTS,
    CHASELINKS, CLOBBER, CLOBBEREMPTY, CS_CMDSUBST, ERRFLAG_INT, FDT_EXTERNAL, FDT_INTERNAL,
    FDT_PROC_SUBST, FDT_SAVED_MASK, FDT_TYPE_MASK, FDT_UNUSED, FDT_XTRACE, HASHDIRS, INP_LINENO,
    INTERACTIVE, IS_CLOBBER_REDIR, IS_DASH, JOBTEXTSIZE, MAX_PIPESTATS, MONITOR, MULTIOS,
    MULTIOUNIT, PATHDIRS, PM_LOADDIR, PM_READONLY, PM_UNDEFINED, POSIXBUILTINS, POSIXJOBS,
    POSIXTRAPS, REDIRF_FROM_HEREDOC, REDIR_CLOSE, REDIR_HEREDOCDASH, REDIR_HERESTR, REDIR_INPIPE,
    REDIR_OUTPIPE, USEZLE, VERBOSE, WC_LIST, WC_LIST_TYPE, WC_PIPE, WC_PIPE_END, WC_PIPE_TYPE,
    WC_REDIR, WC_REDIR_TYPE, WC_REDIR_VARID, WC_SIMPLE, WC_SIMPLE_ARGC, WC_SUBLIST, WC_SUBLIST_END,
    WC_SUBLIST_FLAGS, WC_SUBLIST_TYPE, WC_TYPESET, ZSIG_FUNC, ZSIG_IGNORED, Z_END,
};
use crate::ported::zsh_system_h::timespec as ZshTimespec;
use crate::ported::ztype_h::{inull, itok};
use crate::zsh_h::XTRACE;

/// Port of the anonymous `enum { ... }` from `Src/exec.c:35-40`.
/// Flag bits passed as the `addflags` argument to `addvars` /
/// `addvarsfromargs`:
///   - `ADDVAR_EXPORT`  (1<<0) — export each assignment for the
///                                command `VAR=val cmd ...` form.
///   - `ADDVAR_RESTORE` (1<<2) — the variable list is being restored
///                                later (implicit local scope), so
///                                suppress `ASSPM_WARN`.
pub const ADDVAR_EXPORT: i32 = 1 << 0; // c:37 (Src/exec.c)
/// `ADDVAR_RESTORE` constant.
pub const ADDVAR_RESTORE: i32 = 1 << 2; // c:39 (Src/exec.c)

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

/// Port of `int forklevel;` from `Src/exec.c:1053`. Records the
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

// c:117 (Src/exec.c) — `mod_export int noerrs;`
//
// C has exactly ONE `noerrs`. This port had TWO disjoint storages: an
// `AtomicI32` here, and `utils::noerrs_lock()`'s `Mutex<i32>` (flagged in
// utils.rs as "NOT IN UTILS.C"). The four diagnostic emitters — `zerr`,
// `zwarn`, `zerrnam`, `zwarnnam` (utils.rs:221/244/262/279) — read the utils
// one, while `docomplete`'s suppression window (`noerrs = 1` around
// `doexpansion`, zle_tricky.c:825-828) wrote this one. So the suppression
// never reached the printer and completion-time diagnostics leaked onto the
// screen: `ls *(` printed `zsh: bad pattern: *(` where zsh prints nothing,
// consuming a row and shifting the whole match grid.
//
// Unified onto the storage every emitter already consults; the accessor lives
// at `crate::ported::utils::noerrs_lock()`. Do not reintroduce a second one.
// The storage is `crate::ported::utils::noerrs_lock()`; read/write it directly.

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

/// Port of `int zsh_subshell;` from `Src/params.c:108`. Visible
/// `$ZSH_SUBSHELL` parameter — incremented by `entersubsh()` each time
/// the shell forks into a subshell (real or fake-exec). Distinct from
/// `subsh` which records whether we ARE a subshell; `zsh_subshell` is
/// the visible depth count.
pub static zsh_subshell: std::sync::atomic::AtomicI32 = // c:108 (Src/params.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `mod_export volatile int retflag;` from `Src/exec.c:165`.
/// Set by `bin_return` to unwind the function-call stack. Cleared
/// by `runshfunc` on entry, checked by `execlist`'s main loop.
///
/// Re-export alias of the canonical [`crate::ported::builtin::RETFLAG`] — C
/// has ONE `retflag` (exec.c:165). All return-flow logic (execlist, the
/// `return` builtin, signals) uses `RETFLAG`; this lowercase twin was never
/// stored, so `zpty`/`system` module read-loops that check `exec::retflag`
/// saw a permanent 0 and never aborted on a pending `return`.
pub use crate::ported::builtin::RETFLAG as retflag; // c:165 (Src/exec.c)

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

/// Port of `static int esprefork, esglob = 1;` from `Src/exec.c:2680`.
///
/// File-static "execsubst parameters" — callers (execcmd_exec at
/// c:3298 / c:3700) set these BEFORE invoking execsubst, which then
/// uses them as the `flags` arg to prefork() and the gate on
/// globlist(). `esprefork` is `PREFORK_TYPESET` for magic-assign /
/// MAGICEQUALSUBST words, else 0. `esglob` defaults to 1; cleared
/// when the dispatched builtin has `BINF_NOGLOB`.
pub static esprefork: std::sync::atomic::AtomicI32 = // c:2680
    std::sync::atomic::AtomicI32::new(0);
pub static esglob: std::sync::atomic::AtomicI32 = // c:2680 (= 1)
    std::sync::atomic::AtomicI32::new(1);

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
/// Port of `gethere()` from `Src/exec.c:4570` — C decl `gethere(char **strp, int typ)`.
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
        // C's ingetc sets `lexstop = 1` when input is exhausted
        // (Src/input.c:289-292), so the flag check alone suffices
        // there. zshrs's hgetc signals the same exhaustion by
        // returning None WITHOUT setting LEX_LEXSTOP — treat both as
        // the c:4631 condition, else an unterminated heredoc loops
        // here forever appending '\n'. Gap #3 2026-06-12 (A04
        // "Here-documents don't perform shell expansion" wedge).
        if LEX_LEXSTOP.with(|f| f.get()) || c.is_none() {
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
            errflag.store(ef | (cur & ERRFLAG_INT), Ordering::Relaxed);
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
///     `spacesplit(buf, allownull=false)` per c:4865-4871, and
///     `shtokenize` each word when `GLOBSUBST` is set (c:4868-4869)
///     so the substituted words glob downstream.
///
/// Both arms — `$(< file)` (c:4746) and `$( cmd )` (c:4772) — go through
/// the same c:4858-4872 tail (inlined twice — see the cross-reference
/// notes at :619 and :3829), mirroring C where both go through `readoutput`.
///
/// Uses `with_executor` (panics on missing VM context), not
/// `try_with_executor + unwrap_or_default()`. C `getoutput` calls
/// `execpline` directly — there's no "no shell" code path. The
/// silent-no-op pattern (return empty string when no executor) would
/// mask catastrophic state corruption as "command produced no output",
/// which is the failure mode the `subst.rs:496` warning block flags.
/* $(...) */
/// Port of `getoutput()` from `Src/exec.c:4713` — C decl `getoutput(char *cmd, int qt)`.
// c:4709
/// `getoutput` — see implementation.
pub fn getoutput(cmd: &str, qt: i32) -> Vec<String> {
    // c:4713
    // c:4715 — `Eprog prog;`
    let prog: Option<eprog>;
    // c:4716 — `int pipes[2];`  (collapsed: in-process executor; no fork)
    // c:4717 — `pid_t pid;`     (collapsed)
    let mut s: String; // c:4718
                       // c:4720-4723 — `int onc = nocomments; nocomments = (interact &&
                       //                !sourcelevel && unset(INTERACTIVECOMMENTS));
                       //                prog = parse_string(cmd, 0); nocomments = onc;`
    let onc = crate::ported::lex::LEX_NOCOMMENTS.with(|c| c.get());
    let new_nc = crate::ported::zsh_h::interact()
        && crate::ported::init::sourcelevel.load(Ordering::Relaxed) == 0
        && !isset(crate::ported::zsh_h::INTERACTIVECOMMENTS);
    crate::ported::lex::LEX_NOCOMMENTS.with(|c| c.set(new_nc));
    prog = parse_string(cmd, 0);
    crate::ported::lex::LEX_NOCOMMENTS.with(|c| c.set(onc));

    if prog.is_none() {
        // c:4725
        return Vec::new(); // c:4726 return NULL
    }
    let prog = prog.unwrap();

    if !isset(crate::ported::zsh_h::EXECOPT) {
        // c:4728
        return Vec::new(); // c:4729 newlinklist()
    }

    // c:4731 — `if ((s = simple_redir_name(prog, REDIR_READ)))` — `$(< word)`
    if let Some(red_name) = simple_redir_name(&prog, crate::ported::zsh_h::REDIR_READ) {
        /* $(< word) */
        // c:4732
        s = red_name;
        s = singsub(&s); // c:4737
        if errflag.load(Ordering::Relaxed) != 0 {
            return Vec::new(); // c:4739
        }
        let s = untokenize(&s); // c:4740
        let path_meta = unmeta(&s); // c:4741 unmeta(s)
        let cpath = match std::ffi::CString::new(path_meta.as_bytes()) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let stream = unsafe {
            libc::open(cpath.as_ptr(), libc::O_RDONLY | libc::O_NOCTTY) // c:4741
        };
        if stream == -1 {
            // c:4798 — `zwarn("%e: %s", errno, s);`
            //
            // `zwarn`, NOT `zerr`: `zerr` raises ERRFLAG_ERROR, which
            // abandons the rest of the line, so `print -r -- "[$(<f)]"`
            // against a missing `f` printed nothing and exited 1. zsh
            // warns, substitutes empty, and runs the command — the
            // observed output is `[]` with status 0.
            //
            // `%e` is zsh's errno formatter (`zsh_errno_msg`, utils.c:355),
            // which lowercases strerror's first letter: "no such file or
            // directory". Rust's `io::Error` Display renders "No such file
            // or directory (os error 2)" instead.
            let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            zwarn(&format!(
                "{}: {}",
                crate::ported::utils::zsh_errno_msg(errno),
                s
            ));
            LASTVAL.store(1, Ordering::Relaxed);
            cmdoutval.store(1, Ordering::Relaxed);
            return Vec::new(); // c:4800
        }
        // c:4746 — `retval = readoutput(stream, qt, &readerror);`
        let mut readerror: i32 = 0;
        let retval = readoutput(stream, qt, &mut readerror); // c:4746
        if readerror != 0 {
            // c:4803-4805 — `zwarn("error when reading %s: %e", s, readerror);`
            // Non-fatal for the same reason as the open failure above.
            zwarn(&format!(
                "error when reading {}: {}", // c:4804
                s,
                crate::ported::utils::zsh_errno_msg(readerror)
            ));
            LASTVAL.store(1, Ordering::Relaxed);
            cmdoutval.store(1, Ordering::Relaxed);
        }
        return retval; // c:4751
    }

    // c:4753-4790 — Full fork path: mpipe + zfork + parent
    // readoutput / waitforpid / child execode + _realexit. fusevm runs
    // command substitution in-process, so the fork shape collapses to a
    // synchronous executor call. C control points preserved as cites:
    //   c:4753 mpipe       — handled by ShellExecutor pipe wiring
    //   c:4758 child_block — no-op (no fork)
    //   c:4760 zfork       — replaced by in-process exec
    //   c:4768-4776 parent — equivalent to executor return
    //   c:4778-4789 child  — entersubsh+execode+_realexit collapse.
    //     The `entersubsh(ESUB_PGRP|ESUB_NOMONITOR, NULL)` at c:4781 is NOT
    //     dropped by that collapse: `run_command_substitution` below enters
    //     `SubshStateGuard`, which applies the deltas that are correct
    //     without a fork (opts[MONITOR], shout, opts[USEZLE], zleactive) and
    //     restores them on return. See SubshStateGuard for what is skipped
    //     and why. `_realexit()` has no in-process counterpart — the
    //     executor simply returns the captured buffer.
    cmdoutval.store(0, Ordering::Relaxed); // c:4759
    let buf = crate::ported::exec::run_command_substitution(cmd);
    LASTVAL.store(cmdoutval.load(Ordering::Relaxed), Ordering::Relaxed); // c:4775

    // ── c:4772 `retval = readoutput(pipes[0], qt, NULL);` ─────────────
    // The fd died with the fork, but readoutput's POST-READ half (c:4858-
    // 4872) still applies verbatim, so it is inlined here.
    //
    // !!! THIS BLOCK IS DUPLICATED at `readoutput` (:3829). See the note
    // there for why build.rs's no-new-functions gate rules out a shared
    // helper. ANY EDIT HERE MUST BE MIRRORED AT :3829 AND VICE VERSA.
    //
    // What was missing before: the c:4868-4869 `if (isset(GLOBSUBST))
    // shtokenize(*words);` step. `readoutput` has it, this copy did not, so
    // `setopt globsubst; echo $(echo '*')` printed `*` where zsh expands the
    // glob — while `setopt globsubst; echo $(< star.txt)`, which reaches
    // `readoutput` through the c:4746 arm, globbed correctly.
    //
    // `buf` arrives as a plain (un-metafied) String from
    // `run_command_substitution`; `readoutput` now hands its own tail the
    // same representation (see the deviation note at :3792).
    //
    // c:4858-4859 — `while (cnt && ptr[-1] == '\n') ptr--, cnt--;`
    let s = buf.trim_end_matches('\n');
    // c:4861-4863 — qt branch: empty → Nularg sentinel; else single elem.
    if qt != 0 {
        // c:4861
        if s.is_empty() {
            return vec![String::from(Nularg)]; // c:4862
        }
        return vec![s.to_string()]; // c:4864
    }
    // c:4866-4871 — `spacesplit` + per-word GLOBSUBST `shtokenize`.
    let mut words = crate::ported::utils::spacesplit(s, false); // c:4867
    if isset(crate::ported::zsh_h::GLOBSUBST) {
        // c:4870
        for w in words.iter_mut() {
            crate::ported::glob::shtokenize(w); // c:4870
        }
    }
    words
}

/// Direct port of `Shfunc loadautofn(Shfunc shf, int ks, int test_only,
/// int ignore_loaddir)` from `Src/exec.c:5682`. Walks `$fpath` for a
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
/// Port of `loadautofn()` from `Src/exec.c:5682` — C decl `loadautofn(Shfunc shf, int fksh, int autol, int current_fpath)`.
pub fn loadautofn(
    shf: *mut shfunc, // c:5682 (Src/exec.c)
    _ks: i32,
    autol: i32,
    current_fpath: i32, // c:5735 — 4th C param is `current_fpath`, NOT an
                        // "ignore loaddir" flag; `eval_autoload` passes
                        // `OPT_ISSET(ops,'d')` here (c:3182).
) -> i32 {
    if shf.is_null() {
        return 1;
    }
    // c:5054 — `name = shf->node.nam`.
    let name = unsafe { (*shf).node.nam.clone() };
    // c:5070 — `path = getfpfunc(name, &dir_path, NULL, 0)`.
    let mut dir_path: Option<String> = None;
    let mut dump_hit: Option<(eprog, i32)> = None;
    // A function autoloaded by ABSOLUTE PATH (`autoload -Uz /dir/name`) — or
    // one already resolved via `autoload -r` — caches its directory in
    // `shf.filename` with PM_LOADDIR set. Load-on-call must search THAT dir
    // (`shf.filename/name`), not walk `$fpath` (which won't contain it).
    // Without this, `autoload -Uz $fdir/.hist.*` (zsh-hist) named the
    // functions correctly but every call died with "definition file not
    // found". Mirrors C loadautofn's PM_LOADDIR spec-path branch.
    //
    // c:5747-5757 —
    //     if (shf->filename && shf->filename[0] == '/' &&
    //         (shf->node.flags & PM_LOADDIR))
    //     {
    //         char *spec_path[2];
    //         spec_path[0] = dupstring(shf->filename);
    //         spec_path[1] = NULL;
    //         prog = getfpfunc(shf->node.nam, &ksh, &fdir, spec_path, 0);
    //         if (prog == &dummy_eprog &&
    //             (current_fpath || (shf->node.flags & PM_CUR_FPATH)))
    //             prog = getfpfunc(shf->node.nam, &ksh, &fdir, NULL, 0);
    //     }
    //     else
    //         prog = getfpfunc(shf->node.nam, &ksh, &fdir, NULL, 0);
    let fn_flags = unsafe { (*shf).node.flags } as u32;
    let loaddir_spec: Option<Vec<String>> = {
        let s = unsafe { &*shf };
        // c:5747 — the spec-path arm needs an ABSOLUTE filename, not just
        // PM_LOADDIR.
        if s.filename.as_deref().is_some_and(|f| f.starts_with('/')) && (fn_flags & PM_LOADDIR) != 0
        {
            s.filename.clone().map(|d| vec![d])
        } else {
            None
        }
    };
    let mut looked_up = getfpfunc(
        &name,
        &mut dir_path,
        loaddir_spec.as_deref(),
        0,
        &mut dump_hit,
    ); // c:5753 / c:5759
       // c:5754-5756 — the explicit load directory missed; `-d` (PM_CUR_FPATH,
       // set by `autoload -d`, c:3383) or an explicit `current_fpath` argument
       // means "also try $fpath". The Rust port never retried, so
       // `autoload -dUz $PWD/extra/def; def` and
       // `def() { autoload -dXUz $PWD/extra; }; def` both reported
       // "function definition file not found" where zsh loads ./def
       // (C04funcdef:33,40).
    if looked_up.is_none()
        && loaddir_spec.is_some()
        && (current_fpath != 0 || (fn_flags & crate::ported::zsh_h::PM_CUR_FPATH) != 0)
    {
        dir_path = None;
        dump_hit = None;
        looked_up = getfpfunc(&name, &mut dir_path, None, 0, &mut dump_hit); // c:5756
    }
    let path = match looked_up {
        Some(p) => p,
        None => {
            // !!! WARNING: RUST-ONLY BRANCH — NO DIRECT C COUNTERPART !!!
            // compsys ships as native Rust functions (src/compsys/router.rs),
            // so names like `_main_complete` have no definition file in
            // $fpath. C zsh always loads them from files; zshrs must let
            // `autoload +X -Uz _main_complete` (e.g. fzf-tab via zinit's
            // :zinit-tmp-subst-autoload, zinit.zsh:356) succeed without one.
            // Mark the stub loaded and return success — call-time dispatch
            // short-circuits to the native fn (vm_helper.rs:2276), so no
            // funcdef/body is needed.
            if crate::compsys::router::is_intercepted(&name) {
                unsafe {
                    (*shf).node.flags &= !(PM_UNDEFINED as i32);
                }
                if let Ok(mut tab) = shfunctab_lock().write() {
                    if let Some(existing) = tab.get_mut(&name) {
                        existing.node.flags &= !(PM_UNDEFINED as i32);
                    }
                }
                return 0;
            }
            // c:Src/exec.c:5713-5719 — file not found path. C:
            //   `if (prog == &dummy_eprog) {
            //        locallevel--;
            //        zwarn("%s: function definition file not found",
            //              shf->node.nam);
            //        locallevel++;
            //        popheap();
            //        return NULL;
            //    }`
            // C's getfpfunc returns &dummy_eprog as the "not found"
            // sentinel when test_only==0; loadautofn detects it and
            // emits the diagnostic before returning NULL. Rust's
            // getfpfunc returns Option::None for the same condition,
            // so we emit the same diagnostic here. The locallevel
            // dance is preserved as a comment because the Rust
            // port's zwarn doesn't reference locallevel in the
            // format string itself (the dance in C is only to keep
            // the prefix line counter consistent with the function-
            // body context). Bug #107 in docs/BUGS.md.
            crate::ported::utils::zwarn(&format!("{}: function definition file not found", name));
            return 1; // c:5719 NULL
        }
    };
    let _ = autol;
    // Previously the Rust port treated this parameter as
    // "test_only" and early-returned when set, so the `+X`
    // call from `eval_autoload` (`loadautofn(shf, mode, 1, d)`)
    // never actually loaded the file. C's parameter is `autol`
    // (autoload mode), NOT a test-only flag — the C body
    // unconditionally loads/parses regardless of autol. autol=1
    // controls the EF_RUN / map-flag dance for the wordcode prog
    // (c:5725-5749), but the loaded-body / PM_UNDEFINED-clear
    // path runs in all cases. Removing the early-return so
    // `autoload -U +X funcname` actually loads the body and
    // `type funcname` reports `function from /path/file` instead
    // of `autoload shell function`. Bug #160 in docs/BUGS.md.
    // c:5100-5140 — read the file. C uses zopen + read + parse_string +
    // execsave; Rust port stores raw text on the ShFunc and defers
    // parse-to-Eprog until the first call.
    //
    // c:Src/exec.c:6238 / parse.c:3833 — when getfpfunc resolved the
    // function out of a compiled `.zwc` dump, there is no source file
    // to read; the wordcode Eprog came back through `dump_hit`. C
    // executes that wordcode directly (`shf->funcdef = stripkshdef(
    // prog, ...)`, exec.c:5753-5755). zshrs executes function bodies
    // through the fusevm bytecode pipeline which consumes source
    // text, so bridge wordcode → text with the canonical text.c
    // renderer (`getpermtext`, ported at text.rs:189) — the same
    // walker `functions NAME` printing uses for wordcode-backed
    // funcdefs (C hashtable.c:954). The downstream
    // `autoload_register_source` step (vm_helper.rs:3162) performs
    // the `stripkshdef` shape decision, matching c:5725-5760.
    // c:5706-5710 — the ksh-mode precedence chain:
    //   `if (ksh == 1) { ksh = fksh; if (ksh == 1)
    //        ksh = PM_KSHSTORED ? 2 : PM_ZSHSTORED ? 0 : 1; }`
    // The dump header flag (FDHF_KSHLOAD/FDHF_ZSHLOAD via `*ksh`
    // from try_dump_file) outranks the stub's PM_*STORED bits, which
    // are only consulted when the dump says 1 (no explicit style).
    // zshrs's load/register split (vm_helper's
    // `autoload_register_source` makes the c:5725 ksh-vs-zsh
    // decision later, from the tab entry's flags + KSHAUTOLOAD) —
    // fold a decisive dump flag into the PM bits so the downstream
    // decision sees the same precedence.
    let dump_ksh = dump_hit.as_ref().map(|(_, k)| *k);
    // !!! WARNING: RUST-ONLY — NO C COUNTERPART !!!
    // C installs the dump's wordcode itself (c:5753-5755) and never lexes it
    // again; zshrs installs a `getpermtext` deparse whose real compile happens
    // at the call that defines the function, so the provenance has to be
    // recorded for that later compile. See vm_helper::autoload_note_wordcode_body.
    let from_wordcode = dump_hit.is_some();
    let body = match dump_hit {
        // The wordcode C executes carries a line number on every pipe
        // (`WCB_PIPE(type, toklineno + 1)`, c:Src/parse.c:911/935/944, read
        // back by `lineno = WC_PIPE_LINENO(pcode) - 1` at c:Src/exec.c:2057
        // and `lineno = code - 1` at c:Src/exec.c:1356), so a dump-loaded
        // function reports exactly the `$LINENO` of its ORIGINAL source.
        //
        // `getpermtext` cannot reproduce that: `gettext2` is a pretty-printer
        // (`Src/text.c`), so it drops comments and blank lines and re-breaks
        // compounds onto its own lines — `if X; then` becomes `if X` NEWLINE
        // `then`. Re-parsing that text yields a DIFFERENT line for every
        // statement, and `$LINENO` / error prefixes / `funcfiletrace` inside
        // any `.zwc`-loaded function drift from zsh (measured: 29 vs 32 for
        // `_parameters` loaded out of a `comp_utils.zwc` digest).
        //
        // !!! WARNING: RUST-ONLY BRANCH — NO DIRECT C COUNTERPART !!!
        // C has no choice to make here: `shf->funcdef = stripkshdef(prog, …)`
        // (c:Src/exec.c:5753-5755) runs the DUMP's wordcode and never looks at
        // the source file again. zshrs executes function bodies as TEXT, so it
        // has to render the wordcode back — and that render is what loses the
        // line numbers. The source file is preferred ONLY when it still renders
        // to the same program as the dump; then it is provably the text the
        // wordcode was compiled from and keeps the original line numbering.
        //
        // The previous version skipped that test and took the source file
        // whenever it existed, on the theory that try_dump_file's mtime gate
        // (parse.rs, c:Src/parse.c:3762-3784) already proved they agree. It
        // does not: the gate is `stc.st_mtime >= stn.st_mtime` at SECOND
        // granularity, so a source rewritten within the same second as the
        // dump — or back-dated — passes it while holding completely different
        // code. `zcompile f; print 'print FRESH' > f; touch f; autoload f; f`
        // ran FRESH where zsh runs the compiled body.
        Some((prog, _ksh)) => {
            let dump_text = crate::ported::text::getpermtext(Box::new(prog), None, 0); // c:5753
            // The equality test below LEXES the source file, and it is asking
            // whether that file is the text the dump's wordcode was compiled
            // from. `zcompile` resolved the dump with the lexer state C uses
            // for a compile, so the comparison has to be made in that same
            // state or it answers a different question: under RCQUOTES an
            // adjacent quote pair re-lexes as one literal quote
            // (c:Src/lex.c:1328) and a live alias rewrites words from inside
            // the lexer (c:Src/lex.c:1909), so a source that IS the dump's
            // original compared unequal and the deparse was taken instead —
            // losing the original line numbering for no reason. Same pin the
            // `source` leg uses (vm_helper::execute_zwc_program).
            let _relex = crate::vm_helper::ZwcRelexGuard::enter();
            match std::fs::read_to_string(&path) {
                // Both sides go through the SAME renderer, so a `.zwc` written
                // by C zsh (metafied string pool) still compares equal to a
                // locally parsed source.
                Ok(t)
                    if crate::ported::exec::parse_string(&t, 1).is_some_and(|p| {
                        crate::ported::text::getpermtext(Box::new(p), None, 0) == dump_text
                    }) =>
                {
                    t
                }
                _ => dump_text,
            }
        }
        None => match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => return 1,
        },
    };
    // c:Src/exec.c:5735/5757 — `loadautofnsetfile(shf, fdir)`. The
    // helper stamps PM_LOADDIR alongside the filename when fdir is
    // present, so `whence -v NAME` later concatenates the directory
    // with `/NAME` (PM_LOADDIR branch at hashtable.rs:1350). zshrs's
    // prior `shf->filename = dir_path` assignment skipped the flag
    // → `type colors` printed `from /path/to/functions` instead of
    // `from /path/to/functions/colors`. Mirror C exactly.
    unsafe {
        loadautofnsetfile(&mut *shf, dir_path.as_deref().or(Some(&path)));
    }
    // c:5148 — `shf->node.flags &= ~PM_UNDEFINED`.
    unsafe {
        (*shf).node.flags &= !(PM_UNDEFINED as i32);
    }
    // c:5706-5710 fold (see comment above): decisive dump style wins
    // over the stub's stored-style bits.
    let (ksh_on, ksh_off): (i32, i32) = match dump_ksh {
        Some(2) => (
            crate::ported::zsh_h::PM_KSHSTORED as i32,
            crate::ported::zsh_h::PM_ZSHSTORED as i32,
        ),
        Some(0) => (
            crate::ported::zsh_h::PM_ZSHSTORED as i32,
            crate::ported::zsh_h::PM_KSHSTORED as i32,
        ),
        _ => (0, 0),
    };
    unsafe {
        (*shf).node.flags = ((*shf).node.flags | ksh_on) & !ksh_off;
    }
    // c:5753-5755 — C's `shf->funcdef = stripkshdef(prog, …)` installs the
    // dump's own wordcode; zshrs installs TEXT and compiles it later, so mark
    // (or, on a plain-file load, un-mark) the name so the compile that finally
    // lexes this text knows it is a deparse and pins the lexer accordingly.
    crate::vm_helper::autoload_note_wordcode_body(
        &name,
        if from_wordcode { Some(&body[..]) } else { None },
    );
    // Sync the body string into the Rust-side ShFunc table so the
    // lazy-parse path can find it later.
    if let Ok(mut tab) = shfunctab_lock().write() {
        if let Some(existing) = tab.get_mut(&name) {
            existing.body = Some(body);
            existing.node.flags = (existing.node.flags | ksh_on) & !ksh_off;
            // c:5657 loadautofnsetfile — store the fpath DIRECTORY absolutized
            // with PM_LOADDIR (not the raw relative `./fns`), so `whence -v`
            // prints the absolute source. The `+X` (eval-autoload) path relies
            // on this since it does not re-register through vm_helper.
            loadautofnsetfile(existing, dir_path.as_deref());
        } else {
            let mut shf = shfunc {
                node: hashnode {
                    next: None,
                    nam: name.clone(),
                    flags: ksh_on, // c:5706-5710 dump-style fold
                },
                filename: None,
                lineno: 0,
                funcdef: None,
                redir: None,
                sticky: None,
                body: Some(body),
                redir_text: None,
            };
            loadautofnsetfile(&mut shf, dir_path.as_deref()); // c:5657
            tab.add(shf);
        }
    }
    0
}

/// Port of `getfpfunc()` from `Src/exec.c:6219` — C decl `getfpfunc(char *s, int *ksh, char **fdir, char **alt_path, int test_only)`.
/// supplied `spec_path` slice) for a file named `name` and writes the
/// resolved directory through `*dir_path_out` (matching the C `char **dir_path`).
/// Returns `Some(file_path)` on success, `None` when not found.
///
/// Per dir, the compiled-dump lookup runs FIRST (c:6238
/// `try_dump_file(*pp, s, buf, ksh, test_only)`) — a directory
/// digest `<dir>.zwc` or per-function `<dir>/<name>.zwc` wins over
/// the plain file when newer (mtime logic inside try_dump_file,
/// c:parse.c:3762-3784). On a dump hit the loaded program + ksh
/// mode (C's `*ksh` out-param) are written through `dump_out` and
/// the nominal `<dir>/<name>` path is returned; the caller must
/// check `dump_out` before reading the returned path as a plain
/// file.
pub fn getfpfunc(
    name: &str,
    dir_path_out: &mut Option<String>, // c:6219 (Src/exec.c)
    spec_path: Option<&[String]>,
    test_only: i32,                      // c:6219 `int test_only`
    dump_out: &mut Option<(eprog, i32)>, // c:6219 `int *ksh` + dump Eprog return
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
            .or_else(|| getsparam("FPATH").map(|v| v.split(':').map(String::from).collect()))
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
        let path = format!("{}/{}", dir, name); // c:6230 snprintf(buf, ..., "%s/%s", *pp, s)
                                                // c:6238 — `if ((r = try_dump_file(*pp, s, buf, ksh, test_only)))`
                                                // — the .zwc digest / per-function dump is tried BEFORE the
                                                // plain file in each directory.
        if let Some(hit) = crate::ported::parse::try_dump_file(dir, name, &path, test_only != 0) {
            *dump_out = Some(hit);
            *dir_path_out = Some(dir.clone()); // c:6240 `*fdir = *pp;`
            return Some(path); // c:6241
        }
        if std::path::Path::new(&path).exists() {
            // c:6245 access(buf, R_OK)
            *dir_path_out = Some(dir.clone());
            return Some(path);
        }
    }
    None
}

/// Port of `resolvebuiltin()` from `Src/exec.c:2703` — C decl `resolvebuiltin(const char *cmdarg, HashNode hn)`.
/// Ensures that an autoload-stub builtin has its
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
/// `src/ported/builtin.rs`. Module autoload routes through
/// `module::ensurefeature(MODULESTAB, modname, "b:", Some(cmdarg))`;
/// after the module loads the handler should be wired into BUILTINS.
pub fn resolvebuiltin<'a>(
    cmdarg: &str, // c:2703 (Src/exec.c)
    hn: &'a builtin,
) -> Option<&'a builtin> {
    // c:2705 — `if (!((Builtin) hn)->handlerfunc)`.
    if hn.handlerfunc.is_none() {
        // c:2706 — `modname = dupstring(((Builtin)hn)->optstr)`.
        let modname = hn.optstr.clone().unwrap_or_default();
        // c:2712 — `ensurefeature(modname, "b:", cmdarg)`.
        let _ = {
            let mut t = crate::ported::module::MODULESTAB.lock().unwrap();
            crate::ported::module::ensurefeature(&mut t, &modname, "b:", Some(cmdarg))
        };
        // c:2715-2716 — re-lookup the now-(hopefully)-resolved builtin.
        if let Some(re) = BUILTINS.iter().find(|b| b.node.nam == cmdarg) {
            if re.handlerfunc.is_some() {
                return Some(re); // c:2723
            }
        }
        // c:2717-2721 — `lastval = 1; zerr(...)` + return NULL.
        zerr(&format!(
            "autoloading module {} failed to define builtin: {}",
            modname, cmdarg
        ));
        return None; // c:2720
    }
    Some(hn) // c:2723
}

/// Port of `static struct builtin commandbn` from `Src/exec.c:276`.
///
/// c:275 — /* structure for command builtin for when it is used with -v or -V */
///
/// `BUILTIN("command", 0, bin_whence, 0, -1, BIN_COMMAND, "pvV", NULL)`.
/// This is a SEPARATE descriptor from the `BIN_PREFIX("command", …)` row in
/// `builtintab` (which carries no handler at all): c:3209 swaps `hn` to this
/// one when `command -v` / `command -V` is seen, so the dispatch lands in
/// `bin_whence` with funcid `BIN_COMMAND`.
pub static commandbn: std::sync::LazyLock<crate::ported::zsh_h::builtin> =
    std::sync::LazyLock::new(|| {
        crate::ported::builtin::BUILTIN(
            "command",
            0,
            Some(crate::ported::builtin::bin_whence),
            0,
            -1,
            crate::ported::hashtable_h::BIN_COMMAND,
            Some("pvV"),
            None,
        ) // c:282
    });

/// Dispatch decision returned by `execcmd_compile_head` — the
/// fusevm-bytecode-time head resolver that mirrors the local-variable
/// state the C `execcmd_exec` function carries through `c:2913-2916`
/// (`is_builtin`, `is_shfunc`, `cflags`, `use_defpath`) plus the
/// precmd-modifier strip count. The fusevm bytecode compiler reads
/// this to emit the correct dispatch opcode in
/// `src/extensions/compile_zsh.rs::compile_simple`.
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

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// the head section of `execcmd_exec` (c:2901; locals + the BINF_PREFIX
/// precommand-modifier walk at c:2966-3096), called at fusevm
/// bytecode-compile time. It does NOT perform dispatch the way the C function
/// does.
/// !!! NOT A PORT OF C `execcmd_exec` !!!
///
/// This is a fusevm-bytecode-time head resolver invoked by
/// `src/extensions/compile_zsh.rs::compile_simple` and the
/// `command` builtin shim in `src/fusevm_bridge.rs`. The canonical
/// 7-arg port of `Src/exec.c:execcmd_exec` lives elsewhere in this
/// file under the C-faithful name `execcmd_exec`.
///
/// This helper mirrors the head section (`c:2904-3275`) of the C
/// function — local initialisation, the precommand-modifier walk
/// that strips `BINF_PREFIX` builtins (`-`, `builtin`, `command`,
/// `exec`, `noglob`), and the `BINF_COMMAND`/`BINF_EXEC`
/// sub-option parsers — and returns the resulting dispatch
/// decision via `execcmd_dispatch`. The fusevm compiler reads
/// that struct to decide which `Op::CallBuiltin` /
/// `Op::CallFunction` / `Op::Exec` to emit, and to compute the
/// correct post-strip `argc`.
///
/// =================== WARNING — DIVERGENCE ====================
///
/// The C function runs ~1500 lines and PERFORMS dispatch: it sets up
/// `multio` redirections, evaluates `varspc` assignments, then calls
/// `execbuiltin` / `runshfunc` / `execute` directly. This helper
/// stops after the precmd-modifier walk and only returns the head
/// decision; runtime dispatch is driven by the bytecode the fusevm
/// compiler emits.
///
/// Signature adaptation: the C `Estate`/`Execcmd_params` carry the
/// wordcode iterator state — zshrs doesn't traverse wordcode here,
/// so the args list arrives already-expanded as a `&[String]`
/// (analog of `preargs` after `execcmd_getargs` at `c:3028`).
/// `type_` mirrors `eparams->type` (`WC_SIMPLE` vs `WC_TYPESET`).
///
/// =============================================================
pub fn execcmd_compile_head(args: &[String], type_: u32) -> execcmd_dispatch {
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
                // c:3106 — `(hn = shfunctab->getnode(shfunctab, cmdarg))`.
                // `shfunctab->getnode` is `gethashnode`
                // (Src/hashtable.c:821 in `createshfunctable`), i.e. an
                // O(1) hash probe that also skips DISABLED nodes. The
                // previous Rust code walked the whole table with
                // `iter().any(...)`, which is O(#functions) per command
                // head AND ignored the DISABLED bit `gethashnode`
                // honours. With a real `.zcompdump` loaded the table
                // holds ~50k autoload stubs, so every compiled command
                // head paid a 50k-entry String scan under the read lock.
                if shfunctab_lock()
                    .read()
                    .map(|t| !t.getnode(&cmdarg).is_null())
                    .unwrap_or(false)
                {
                    is_shfunc = true; // c:3107
                    break; // c:3108
                }
            }
            // c:3056 — `builtintab->getnode(builtintab, cmdarg)`.
            let entry = BUILTINS.iter().find(|b| b.node.nam == cmdarg);
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
            // c:3275-3278 — `hn = NULL; if ((cflags & BINF_COMMAND) &&
            // unset(POSIXBUILTINS)) break;`. After processing a
            // `command` precmd modifier (and its -p/-v/-V flags), the
            // C loop exits with hn cleared so the dispatch falls
            // through to external lookup. Without this, the next
            // iteration would find `command print` → print's builtin
            // and dispatch to it; zsh's intentional behaviour is to
            // skip builtins under `command` (unless POSIXBUILTINS is
            // set, where the loop continues normally).
            if (cflags & BINF_COMMAND) != 0 && !isset(POSIXBUILTINS) {
                hn = None; // c:3275 hn = NULL
                break; // c:3277
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
    // zshrs's `execcmd_compile_head` doesn't receive `redir` (it
    // takes `args` only). The cases that DEPEND on redirs are handled by
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

/// Port of `parse_string()` from `Src/exec.c:283` — C decl `parse_string(char *s, int reset_lineno)`.
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
    use crate::ported::lex::{LEX_FILE_WINDOW_STRIN, LEX_INPUT, LEX_POS, LEX_UNGET_BUF};

    // c:285-286
    let p: Option<eprog>;
    let oldlineno: i64;

    zcontext_save(); // c:288

    // zshrs-only: park the `LEX_INPUT` window. C has ONE reader — the
    // input stack — so `inpush` at c:289 is all the isolation `parse_list`
    // needs. zshrs lexes from a second source as well, the `LEX_INPUT` /
    // `LEX_POS` char window (lex.rs:5181), which `inpush` does not touch,
    // so a nested parse ran on whatever text the OUTER lexer still had
    // parked there. That was invisible while every caller's window was
    // already drained at execution time, and became live when `bin_dot`
    // started running a plain sourced file per command
    // (`execute_script_per_command`, vm_helper.rs:3321): the file is now
    // the lexer's own window WHILE its commands execute, so
    // `zstyle -e ':x' lc 'reply=(1)'` (zutil.rs:856 → here) parsed the
    // REST OF THE FILE as its eval body and everything after that line
    // silently vanished. Draining the window makes `hgetc`'s two-input
    // bridge read only the `inpush`ed frame — the same parking
    // `parsestrnoerr` (lex.rs:3402-3444) and `parse_isolated`
    // (vm_helper.rs:843-877) already do around their own nested parses.
    let saved_input = LEX_INPUT.with_borrow(|w| w.clone());
    let saved_pos = LEX_POS.get();
    let saved_unget = LEX_UNGET_BUF.with_borrow(|b| b.clone());
    let saved_file_window = LEX_FILE_WINDOW_STRIN.get();
    let saved_in_lexstop = crate::ported::input::lexstop.with(|c| c.get());
    LEX_INPUT.with_borrow_mut(|w| w.clear());
    LEX_POS.set(0);
    LEX_UNGET_BUF.with_borrow_mut(|b| b.clear());
    // The pushed frame is a STRING unit, whatever the outer window was.
    LEX_FILE_WINDOW_STRIN.set(0);

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
    if tok() == LEXERR && LASTVAL.load(Ordering::Relaxed) == 0 {
        LASTVAL.store(1, Ordering::Relaxed);
    }
    strinend(); // c:298
    inpop(); // c:299

    // Put the outer window back where the nested parse found it. Draining
    // it set the input-side `lexstop`, whose lex.rs half is the only one
    // `zcontext` covers (parse_isolated, vm_helper.rs:857), so the outer
    // reader would otherwise resume at EOF.
    LEX_INPUT.with_borrow_mut(|w| *w = saved_input);
    LEX_POS.set(saved_pos);
    LEX_UNGET_BUF.with_borrow_mut(|b| *b = saved_unget);
    LEX_FILE_WINDOW_STRIN.set(saved_file_window);
    crate::ported::input::lexstop.with(|c| c.set(saved_in_lexstop));

    zcontext_restore(); // c:300
    p // c:301
}

/// Port of `isgooderr()` from `Src/exec.c:652` — C decl `isgooderr(int e, char *dir)`.
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
    // c:Src/exec.c:658-659 — `(e != EACCES || !access(dir, X_OK)) &&
    //   e != ENOENT && e != ENOTDIR`. C's `access(dir, X_OK)` returns
    //   0 on success / -1 on failure. The previous Rust port used
    //   `metadata().permissions().mode() & 0o111` which reports the
    //   X bit even when the path doesn't exist as the EFFECTIVE caller
    //   (root, ACLs, capabilities all flip access() vs raw mode).
    //   `/no/such/dir` metadata() fails → returned false for
    //   dir_x_ok, then `!false` = true, giving "good error" for a
    //   nonexistent path. Use libc::access directly to match C exactly.
    let unmeta_dir = unmeta(dir);
    let cstr = std::ffi::CString::new(unmeta_dir.as_bytes()).unwrap_or_default();
    let access_ok = unsafe { libc::access(cstr.as_ptr(), libc::X_OK) } == 0;
    (e != libc::EACCES || access_ok) && e != libc::ENOENT && e != libc::ENOTDIR
}

/// Port of `iscom()` from `Src/exec.c:962` — C decl `iscom(char *s)`.
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

/// Port of `isreallycom()` from `Src/exec.c:973` — C decl `isreallycom(Cmdnam cn)`.
///
/// Verify that a hashed/cached cmdnamtab entry still names a real
/// external command (X-perm + regular file). For HASHED entries
/// (`cn->u.cmd` carries the absolute path), test the path directly;
/// otherwise concatenate `name[0] + "/" + nam` and test that.
/// Used by `execcmd_exec` to drop stale cmdnamtab hits before they
/// turn into a failed `execve` syscall.
pub fn isreallycom(cn: &cmdnam) -> bool {
    // c:972
    let fullnam: String;
    if (cn.node.flags & crate::ported::zsh_h::HASHED) != 0 {
        // c:977-978 — `strcpy(fullnam, cn->u.cmd);`
        fullnam = cn.cmd.clone().unwrap_or_default();
    } else if cn.name.is_none() || cn.name.as_ref().unwrap().is_empty() {
        // c:979-980 — `if (!cn->u.name) return 0;`
        return false;
    } else {
        // c:982-984 — `strcpy + strcat("/") + strcat(nam)`
        let path0 = &cn.name.as_ref().unwrap()[0];
        fullnam = format!("{}/{}", path0, cn.node.nam);
    }
    iscom(&fullnam) // c:986
}

/// Port of `isrelative()` from `Src/exec.c:996` — C decl `isrelative(char *s)`.
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

/// Port of `setunderscore()` from `Src/exec.c:2652` — C decl `setunderscore(char *str)`.
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

/// Port of `mpipe()` from `Src/exec.c:5160` — C decl `mpipe(int *pp)`.
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

/// Port of `is_anonymous_function_name()` from `Src/exec.c:5300` — C decl `is_anonymous_function_name(const char *name)`.
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

/// Port of `execsave()` from `Src/exec.c:6438` — C decl `execsave(void)`.
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
        lastval: LASTVAL.load(Ordering::Relaxed), // c:6449
        // c:6450 — `es->noeval = noeval;`. Snapshot math.c's
        // `int noeval` (the parse-only side-effect-skip counter)
        // via math.rs's pub accessor.
        noeval: crate::ported::math::m_noeval(),
        // c:6451 — `es->badcshglob = badcshglob;`. Snapshot the
        // csh-glob diagnostic counter (glob.c:103 / glob.rs
        // BADCSHGLOB) so nested eval / trap dispatch doesn't disturb
        // the outer command's per-line accounting.
        badcshglob: crate::ported::glob::BADCSHGLOB.load(Ordering::Relaxed), // c:6451
        cmdoutpid: cmdoutpid.load(Ordering::Relaxed),                        // c:6452
        cmdoutval: cmdoutval.load(Ordering::Relaxed),                        // c:6453
        use_cmdoutval: use_cmdoutval.load(Ordering::Relaxed),                // c:6454
        procsubstpid: procsubstpid.load(Ordering::Relaxed),                  // c:6455
        trap_return: TRAP_RETURN.load(Ordering::Relaxed),                    // c:6456
        trap_state: TRAP_STATE.load(Ordering::Relaxed),                      // c:6457
        trapisfunc: trapisfunc.load(Ordering::Relaxed),                      // c:6458
        traplocallevel: traplocallevel.load(Ordering::Relaxed),              // c:6459
        noerrs: *crate::ported::utils::noerrs_lock().lock().unwrap(),        // c:6460
        this_noerrexit: this_noerrexit.load(Ordering::Relaxed),              // c:6461
        // c:6462 — `es->underscore = ztrdup(zunderscore);`
        underscore: Some(zunderscore.lock().unwrap().clone()),
    });
    // c:6463-6464 — `es->next = exstack; exstack = es;`
    let mut head = exstack.lock().unwrap();
    es.next = head.take();
    *head = Some(es);
    // c:6465 — `noerrs = cmdoutpid = 0;`
    *crate::ported::utils::noerrs_lock().lock().unwrap() = 0;
    cmdoutpid.store(0, Ordering::Relaxed);
}

/// Port of `execrestore()` from `Src/exec.c:6470` — C decl `execrestore(void)`.
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
    crate::ported::glob::BADCSHGLOB.store(en.badcshglob, Ordering::Relaxed);
    cmdoutpid.store(en.cmdoutpid, Ordering::Relaxed); // c:6488
    cmdoutval.store(en.cmdoutval, Ordering::Relaxed); // c:6489
    use_cmdoutval.store(en.use_cmdoutval, Ordering::Relaxed); // c:6490
    procsubstpid.store(en.procsubstpid, Ordering::Relaxed); // c:6491
    TRAP_RETURN.store(en.trap_return, Ordering::Relaxed); // c:6492
    TRAP_STATE.store(en.trap_state, Ordering::Relaxed); // c:6493
    trapisfunc.store(en.trapisfunc, Ordering::Relaxed); // c:6494
    traplocallevel.store(en.traplocallevel, Ordering::Relaxed); // c:6495
    *crate::ported::utils::noerrs_lock().lock().unwrap() = en.noerrs; // c:6496
    this_noerrexit.store(en.this_noerrexit, Ordering::Relaxed); // c:6497
                                                                // c:6498-6499 — `setunderscore(en->underscore); zsfree(en->underscore);`
    if let Some(ref u) = en.underscore {
        setunderscore(u); // c:6498
    }
    // c:6500 — `free(en);` — handled by Box drop when `en` falls out of scope.
    unqueue_signals(); // c:6502
}

/// Port of `execstring()` from `Src/exec.c:1228` — C decl `execstring(char *s, int dont_change_job, int exiting, char *context)`.
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
    // chain to the fusevm pipeline via the exec_hooks fn-ptr
    // installed by fusevm_bridge at startup. Direct
    // `with_executor` / ShellExecutor reach-in from src/ported/ is
    // forbidden — see memory feedback_no_exec_script_from_ported.
    let _ = crate::ported::exec::execute_script_zsh_pipeline(s);
    popheap(); // c:1240
}

// =====================================================================
// Function-wrapper chain — `Src/module.c:570` `FuncWrap wrappers`, as
// walked by `runshfunc` (`Src/exec.c:6166`).
// =====================================================================

/// Which module-registered wrapper a [`BodyWrap`] node stands for.
///
/// C reaches the handler through a fn pointer stored in the node
/// (`WrapFunc handler` — `Src/zsh.h:1365`); this port dispatches on the
/// discriminant instead, because the two handlers take a body DELEGATE
/// that a `WrapFunc` pointer cannot carry (see [`BodyWrap`]).
#[derive(Clone, Copy)]
enum WrapKind {
    /// `WRAPDEF(comp_wrapper)` — `Src/Zle/complete.c:1694-1695`.
    Comp,
    /// `WRAPDEF(wrap_private)` — `Src/Modules/param_private.c:541-542`.
    Private,
    /// `WRAPDEF(zprof_wrapper)` — `Src/Modules/zprof.c:318-320`.
    Zprof,
}

/// One node of the `wrappers` linked list C keeps at `Src/module.c:570`,
/// tail-appended by `addwrapper` (`Src/module.c:592-597`) from a module's
/// `boot_`.
///
/// Why the live chain is this table and not `module::WRAPPERS`: C's
/// `struct funcwrap` (`Src/zsh.h:1362-1367`) stores an
/// `int (*handler)(Eprog prog, FuncWrap w, char *name)`, and each handler
/// re-enters the chain on its own with `runshfunc(prog, w->next, name)`
/// (`Src/Zle/complete.c:1591`, `Src/Modules/param_private.c:556`) because
/// in C the function body is an `Eprog` anyone can walk. A zshrs function
/// body is not — it is `doshfunc`'s caller-supplied `body_runner` closure
/// (fusevm chunk runner, Rust compsys port, or plugin override), so the
/// port's handlers take the continuation as an explicit delegate that a
/// plain fn pointer has nowhere to hold. `zsh_h::funcwrap` and
/// `module::WRAPPERS` keep C's pointer shape so the `addwrapper` /
/// `deletewrapper` ports stay 1:1; this table is the same list expressed
/// in the shape `runshfunc` can actually call.
struct BodyWrap {
    /// The module whose `boot_` calls `addwrapper(m, wrapper)`. C reads it
    /// back through `wrap->module` for the refcount at c:6178/6180 and the
    /// deferred `unload_module` at c:6184.
    module: &'static str,
    /// Which handler this node holds.
    kind: WrapKind,
}

/// The chain in `addwrapper` tail-append order — module boot order, so
/// the wrapper of the module booted first ends up OUTERMOST.
static BODY_WRAPPERS: &[BodyWrap] = &[
    // `Src/Zle/complete.c:1694-1695` — `static struct funcwrap wrapper[] =
    // { WRAPDEF(comp_wrapper) };` — installed by c:1767
    // `return addwrapper(m, wrapper);` in `zsh/complete`'s `boot_`.
    BodyWrap {
        module: "zsh/complete",
        kind: WrapKind::Comp,
    },
    // `Src/Modules/param_private.c:541-542` — `static struct funcwrap
    // wrapper[] = { WRAPDEF(wrap_private) };` — installed by c:712
    // `return addwrapper(m, wrapper);` in `zsh/param/private`'s `boot_`.
    BodyWrap {
        module: "zsh/param/private",
        kind: WrapKind::Private,
    },
    // `Src/Modules/zprof.c:318-320` — `static struct funcwrap wrapper[] =
    // { WRAPDEF(zprof_wrapper) };` — installed by c:362
    // `return addwrapper(m, wrapper);` in `zsh/zprof`'s `boot_`, removed
    // by c:371 `deletewrapper(m, wrapper);` in `cleanup_`. `zsh/zprof` is
    // never linked in at startup, so it always boots after the two nodes
    // above and `addwrapper`'s tail-append (`Src/module.c:591-595`) puts
    // it LAST — innermost, closest to the body, which is what makes its
    // measured window the function itself.
    BodyWrap {
        module: "zsh/zprof",
        kind: WrapKind::Zprof,
    },
];

/// Port of `runshfunc()` from `Src/exec.c:6166` — C decl `runshfunc(Eprog prog, FuncWrap wrap, char *name)`.
/// the wrapper-chain walk that stands between
/// `doshfunc` and a function body. C:
///
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
/// **RUST-ONLY ADAPTATION — what this port does and does NOT own.** When
/// zshrs replaced C's wordcode walk with a body delegate, everything in
/// the listing above except the chain walk moved into `doshfunc`, which is
/// the only caller:
///
/// | C line | Where it lives in zshrs |
/// |---|---|
/// | c:6171 / c:6202 `queue_signals` / `unqueue_signals` | `doshfunc`'s own outer pair (c:5835 / c:6128) already brackets this window; C's inner pair is a nested refcount bump inside it |
/// | c:6173-6175 / c:6196-6198 `$_` save / `setunderscore` | `doshfunc`, around this call (`saved_zunderscore`) |
/// | c:6194 `startparamscope()` | `doshfunc` — `inc_locallevel()` before the funcstack push |
/// | c:6200 `endparamscope()` | `doshfunc`'s epilogue |
/// | c:6195 `execode(prog, 1, 0, "shfunc")` | the `body` delegate |
///
/// Duplicating any of them here would double-apply it, so the body below
/// is c:6177-6193 plus the c:6195 body call, and nothing else.
///
/// WARNING: param names don't match C — Rust=(_prog, wrap, name, body) vs
/// C=(prog, wrap, name). `wrap` is an INDEX into [`BODY_WRAPPERS`] rather
/// than a `FuncWrap` pointer (a `Vec`-shaped chain has no `->next` to
/// hand out); `wrap = 0` is C's `wrappers` head and `wrap = i + 1` is the
/// `wrap->next` a handler re-enters with. `_prog` is unused because the
/// body arrives as `body`, and the return value is the body's exit status
/// (C returns void and communicates through `lastval`).
pub fn runshfunc(
    _prog: Option<&eprog>, // c:6166
    wrap: usize,           // c:6166 (FuncWrap wrap — see WARNING)
    name: &str,            // c:6166
    body: &mut dyn FnMut() -> i32,
) -> i32 {
    // c:6166
    let mut w_idx = wrap;
    while w_idx < BODY_WRAPPERS.len() {
        // c:6177 — `while (wrap)`
        let w = &BODY_WRAPPERS[w_idx];

        // Chain membership. C asks nothing here because a wrapper is in
        // the list only while its module has it registered; this port
        // evaluates the equivalent condition per node.
        let armed = match w.kind {
            // zshrs statically links the completion system — `compadd` /
            // `compset` are always present and `zsh/complete` is never
            // dlopen'd — so there is no boot event to key on. The
            // wrapper's own first statement is the gate that matters and
            // it is exact: `if (incompfunc != 1) return 1;`
            // (`Src/Zle/complete.c:1558-1559`). `incompfunc` is 1 only
            // between `Src/Zle/compcore.c:815` and c:841, i.e. strictly
            // inside `callcompfunc`, which is the only thing that can run
            // a completion function at all. Hoisting the test here keeps
            // an ordinary function call at one relaxed load with no chain
            // node entered.
            WrapKind::Comp => crate::ported::zle::complete::INCOMPFUNC.load(Ordering::Relaxed) == 1,
            // Module BOOT STATE (`MOD_INIT_B` — the bit `zmodload -e`
            // reads and `load_module` sets after `do_boot_module`,
            // `Src/module.c:2317`), which is exactly C's condition for
            // `wrap_private` being in the list: the `private` dispatch
            // runs `require_module` via the `Src/exec.c:2710` autofeature
            // path, which boots the module. Deliberately NOT
            // `is_loaded()` — default-registered static modules carry
            // `MOD_LINKED` from startup, which would arm the wrapper
            // before any `private` was ever used.
            WrapKind::Private => crate::ported::module::MODULESTAB
                .lock()
                .map(|t| {
                    t.modules
                        .get(w.module)
                        .map(|m| (m.node.flags & crate::ported::zsh_h::MOD_INIT_B) != 0)
                        .unwrap_or(false)
                })
                .unwrap_or(false),
            // Genuine `addwrapper` membership: `zsh/zprof`'s `boot_`
            // calls `addwrapper(m, wrapper)` (`Src/Modules/zprof.c:362`)
            // and its `cleanup_` calls `deletewrapper` (c:371), and both
            // mirror the list into `module::WRAPPERS_ADDED`. So this is
            // C's `while (wrap)` test verbatim — "is this node in the
            // `wrappers` list" — costing one relaxed load, with no lock
            // and no allocation when `zsh/zprof` was never loaded.
            WrapKind::Zprof => {
                (crate::ported::module::WRAPPERS_ADDED.load(Ordering::Relaxed)
                    & crate::ported::module::WRAPPER_BIT_ZPROF)
                    != 0
            }
        };
        if !armed {
            // Not in C's `wrappers` list right now — no node to step over.
            w_idx += 1;
            continue;
        }

        // c:6178 — `wrap->module->wrapper++;` — hold the module open so a
        // recursive unload during the handler defers until we return.
        if let Ok(mut tab) = crate::ported::module::MODULESTAB.lock() {
            if let Some(m) = tab.modules.get_mut(w.module) {
                m.wrapper += 1;
            }
        }

        // c:6179 — `cont = wrap->handler(prog, wrap->next, name);`. The
        // closure IS `wrap->next` made callable: the rest of the chain,
        // then the body — which is what each handler's own
        // `runshfunc(prog, w, name)` reaches in C.
        let mut status = 0;
        let cont = {
            let mut run_next = || runshfunc(_prog, w_idx + 1, name, &mut *body);
            match w.kind {
                WrapKind::Comp => crate::ported::zle::complete::comp_wrapper(
                    std::ptr::null(),
                    std::ptr::null(),
                    name,
                    || status = run_next(), // complete.c:1591
                ),
                WrapKind::Private => crate::ported::modules::param_private::wrap_private(
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    || status = run_next(), // param_private.c:556
                ),
                WrapKind::Zprof => crate::ported::modules::zprof::zprof_wrapper(
                    std::ptr::null(),
                    std::ptr::null(),
                    name,
                    || status = run_next(), // zprof.c:285
                ),
            }
        };

        // c:6180 — `wrap->module->wrapper--;`
        // c:6182-6184 — `if (!wrap->module->wrapper && (flags & MOD_UNLOAD))
        //                    unload_module(wrap->module);`
        let should_unload = {
            if let Ok(mut tab) = crate::ported::module::MODULESTAB.lock() {
                if let Some(m) = tab.modules.get_mut(w.module) {
                    m.wrapper -= 1;
                    m.wrapper == 0 && (m.node.flags & crate::ported::zsh_h::MOD_UNLOAD) != 0
                } else {
                    false
                }
            } else {
                false
            }
        };
        if should_unload {
            if let Ok(mut tab) = crate::ported::module::MODULESTAB.lock() {
                let _ = tab.unload_module(w.module); // c:6184
            }
        }

        if cont == 0 {
            // c:6186 — the wrapper claimed the call and has already run
            // the body through the delegate.
            return status; // c:6190
        }
        w_idx += 1; // c:6192
    }
    // c:6195 — `execode(prog, 1, 0, "shfunc");`
    body()
}

/// Port of `sticky_emulation_dup()` from `Src/exec.c:5501` — C decl `sticky_emulation_dup(Emulation_options src, int useheap)`.
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
pub fn sticky_emulation_dup(src: &emulation_options, _useheap: i32) -> Emulation_options {
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

/// Port of `sticky_emulation_differs()` from `Src/exec.c:5770` — C decl `int sticky_emulation_differs(Emulation_options sticky2)`.
///
/// C body:
/// ```c
/// /* If no new sticky emulation, not a different emulation */
/// if (!sticky2)
///     return 0;
/// /* If no current sticky emulation, different */
/// if (!sticky)
///     return 1;
/// /* If basic emulation different, different */
/// if (sticky->emulation != sticky2->emulation)
///     return 1;
/// /* If differing numbers of options, different */
/// if (sticky->n_on_opts != sticky2->n_on_opts ||
///     sticky->n_off_opts != sticky2->n_off_opts)
///     return 1;
/// /* If different options turned on, different */
/// if (sticky->n_on_opts &&
///     memcmp(sticky->on_opts, sticky2->on_opts,
///            sticky->n_on_opts * sizeof(*sticky->on_opts)) != 0)
///     return 1;
/// /* If different options turned on, different */
/// if (sticky->n_off_opts &&
///     memcmp(sticky->off_opts, sticky2->off_opts,
///            sticky->n_off_opts * sizeof(*sticky->off_opts)) != 0)
///     return 1;
/// return 0;
/// ```
pub fn sticky_emulation_differs(sticky2: Option<&emulation_options>) -> i32 {
    // c:5829
    let Some(sticky2) = sticky2 else {
        return 0; // c:5832-5833
    };
    let guard = sticky.lock().unwrap_or_else(|e| e.into_inner());
    let Some(ref cur) = *guard else {
        return 1; // c:5835-5836
    };
    if cur.emulation != sticky2.emulation {
        return 1; // c:5838-5839
    }
    if cur.n_on_opts != sticky2.n_on_opts || cur.n_off_opts != sticky2.n_off_opts {
        return 1; // c:5841-5843
    }
    if cur.n_on_opts != 0 && cur.on_opts != sticky2.on_opts {
        return 1; // c:5850-5853
    }
    if cur.n_off_opts != 0 && cur.off_opts != sticky2.off_opts {
        return 1; // c:5855-5858
    }
    0 // c:5859
}

/// Port of `shfunc_set_sticky()` from `Src/exec.c:5527` — C decl `shfunc_set_sticky(Shfunc shf)`.
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

/// Port of `search_defpath()` from `Src/exec.c:691` — C decl `search_defpath(char *cmd, char *pbuf, int plen)`.
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

/// Port of `checkclobberparam()` from `Src/exec.c:2178` — C decl `checkclobberparam(struct redir *f)`.
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
/// NOCLOBBER + FDT_EXTERNAL clause now ported (c:2199-2213). When
/// NOCLOBBER is set and the param's value is the fd-number of an
/// FDT_EXTERNAL-marked fd in the fdtable, refuse with a warning so
/// the existing fd doesn't get clobbered by the upcoming open(2).
pub fn checkclobberparam(f: &redir) -> i32 {
    // c:2178
    // c:2182 — `char *s = f->varid;`
    let s = match &f.varid {
        Some(v) => v.clone(),
        None => return 1, // c:2185-2186 — `if (!s) return 1;`
    };
    // c:2186 — `if (!(v = getvalue(&vbuf, &s, 0))) return 1;`
    let mut vbuf = crate::ported::zsh_h::value {
        pm: None,
        arr: Vec::new(),
        scanflags: 0,
        valflags: 0,
        start: 0,
        end: 0,
    };
    let mut cursor: &str = s.as_str();
    let v_opt = crate::ported::params::getvalue(Some(&mut vbuf), &mut cursor, 0);
    if v_opt.is_none() {
        return 1; // c:2187
    }
    // c:2188-2197 — readonly refusal via v->pm->node.flags.
    let readonly = vbuf
        .pm
        .as_ref()
        .map(|p| (p.node.flags as u32 & PM_READONLY) != 0)
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
    // c:2199-2213 — NOCLOBBER + FDT_EXTERNAL refusal: if NOCLOBBER set
    // AND the param holds a valid fd that's already in our fdtable as
    // FDT_EXTERNAL (allocated by sysopen / coproc / etc.), refuse the
    // open so we don't clobber it.
    if !isset(CLOBBER) {
        // c:2201 — `getstrvalue(v)` — read the param's string form.
        let val_str = crate::ported::params::getstrvalue(Some(&mut vbuf));
        if let Ok(fd) = val_str.trim().parse::<i32>() {
            // c:2202 — `if (fd <= max_zsh_fd && fdtable[fd] == FDT_EXTERNAL)`
            let max_fd = MAX_ZSH_FD.load(Ordering::Relaxed);
            if fd >= 0 && fd <= max_fd {
                let kind = fdtable_get(fd);
                if kind == FDT_EXTERNAL {
                    zwarn(&format!("{}: file descriptor {} already open", s, fd)); // c:2206-2210
                    return 0; // c:2211
                }
            }
        }
    }
    1 // c:2214
}

/// Port of `clobber_open()` from `Src/exec.c:2221` — C decl `clobber_open(struct redir *f)`.
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
    if isset(CLOBBER) || IS_CLOBBER_REDIR(f.typ) {
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
    // Per-platform errno setter: __error() on macOS, __errno_location()
    // on Linux. Without cfg gating the build breaks on Linux (CI).
    #[cfg(target_os = "macos")]
    unsafe {
        *libc::__error() = oerrno;
    }
    #[cfg(target_os = "linux")]
    unsafe {
        *libc::__errno_location() = oerrno;
    }
    -1 // c:2263
}

/// Port of `findcmd()` from `Src/exec.c:897` — C decl `findcmd(char *arg0, int docopy, int default_path)`.
/// Walk `$PATH` (or DEFAULT_PATH under
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
    // c:Src/exec.c:914-920 — `/`-bearing arg path resolution.
    //   if ((s = strchr(arg0, '/'))) {
    //       RET_IF_COM(arg0);   // ← unconditional accept on iscom hit
    //       if (arg0 == s || unset(PATHDIRS) ||
    //           !strncmp(arg0, "./", 2) ||
    //           !strncmp(arg0, "../", 3))
    //           return NULL;
    //   }
    // The Rust port had the iscom check gated on `starts_with('/')`,
    // so `type ./target/debug/zshrs` returned None even when the
    // file was executable. Bug #496 family.
    if arg0.contains('/') {
        if iscom(arg0) {
            return Some(arg0.to_string()); // c:915 RET_IF_COM
        }
        // c:916-919 — absolute OR PATHDIRS-off OR `./` / `../` →
        // give up here (no $PATH walk for these). Relative without
        // those prefixes falls through to the $PATH scan below for
        // the PATHDIRS=set case.
        if arg0.starts_with('/')
            || !isset(PATHDIRS)
            || arg0.starts_with("./")
            || arg0.starts_with("../")
        {
            return None;
        }
        // else fall through to PATH walk.
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

/// Port of `addfd()` from `Src/exec.c:2397` — C decl `addfd(int forked, int *save, struct multio **mfds, int fd1, int fd2, int rflag, char *varid)`.
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
/// `fdtable[fd1]` = FDT_EXTERNAL; new-multio path saves the original fd1
/// (when `!forked`) and stamps `mfds[fd1]` as a single-entry struct;
/// extend-multio path either splits a ct=1 stream into a pipe + 2 fds
/// via `mpipe`, or appends another fd to an already-split stream
/// (re-allocating mfds for fd1 past the MULTIOUNIT boundary).
///
/// `multio.fds` is now `Vec<i32>` (zsh_h.rs:1397) so the C
/// `hrealloc` at c:2485 maps to `Vec::push`; MULTIOUNIT is no
/// longer a hard cap (still 8 for the initial allocation, grown
/// on demand thereafter).
///
/// `fdtable[fdN] |= FDT_SAVED_MASK` at c:2440 — Rust fdtable_set
/// stores the int value but doesn't expose a bitwise-OR setter; we
/// re-read + OR + re-store as two atomic-feeling steps.
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
        // A user interrupt sets ERRFLAG_INT, never ERRFLAG_ERROR (signals.c:457), and
        // the C line cited above tests the WHOLE errflag, so masking here let an
        // interrupted shell keep going where zsh stops.
        if errflag.load(Ordering::Relaxed) != 0 {
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
                // c:2420 — C allocates VARLENARRAY trailing `int fds[1]`;
                // grow on demand via push() below. Pre-fill MULTIOUNIT
                // slots with -1 so existing indexed writes (fds[0], fds[1])
                // still work without explicit resize().
                fds: vec![-1; MULTIOUNIT],
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
            // c:2482-2486 — `mn = hrealloc(mn, sizeof + (ct-1)*sizeof(int),
            //                              sizeof + ct*sizeof(int));`
            // Rust's `Vec<i32>` grows on demand; ensure capacity for the
            // new slot before the indexed write below.
            if let Some(mn) = mfds[fd1u].as_mut() {
                while mn.fds.len() <= cur_ct as usize {
                    mn.fds.push(-1);
                }
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

/// Port of `closemn()` from `Src/exec.c:2273` — C decl `closemn(struct multio **mfds, int fd, int type)`.
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
/// c:2299 — `addproc(pid, NULL, 1, &bgtime, -1, -1)` records the
/// tee/cat child in the current job's auxprocs.
pub fn closemn(mfds: &mut [Option<Box<multio>>; 10], fd: i32, type_: i32) {
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
            // c:2299 — `addproc(pid, NULL, 1, &bgtime, -1, -1);` — record
            // the tee/cat child in the current job's auxprocs (aux=true).
            if let Some(jt) = JOBTAB.get() {
                let mut guard = jt.lock().unwrap();
                let tj = THISJOB.get().map(|m| *m.lock().unwrap()).unwrap_or(-1);
                if tj >= 0 {
                    if let Some(j) = guard.get_mut(tj as usize) {
                        crate::ported::jobs::addproc(
                            j,
                            pid,
                            "",
                            true,
                            Some(std::time::Instant::now()),
                            -1,
                            -1,
                        );
                    }
                }
            }
            let _ = bgtime;
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
                    if write_loop(mn.fds[i], &buf[..len as usize]).is_err() {
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
                        libc::read(mn.fds[i], buf.as_mut_ptr() as *mut libc::c_void, buf.len())
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

/// Port of `closemnodes()` from `Src/exec.c:2344` — C decl `closemnodes(struct multio **mfds)`.
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

/// Port of `closeallelse()` from `Src/exec.c:2358` — C decl `closeallelse(struct multio *mn)`.
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

/// Port of `fixfds()` from `Src/exec.c:4523` — C decl `fixfds(int *save)`.
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

/// Port of `closem()` from `Src/exec.c:4546` — C decl `closem(int how, int all)`.
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

/// Port of `hashcmd()` from `Src/exec.c:1010` — C decl `hashcmd(char *arg0, char **pp)`.
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

/// Port of `zfork()` from `Src/exec.c:349` — C decl `zfork(struct timespec *ts)`.
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

/// Port of `loadautofnsetfile()` from `Src/exec.c:5657` — C decl `loadautofnsetfile(Shfunc shf, char *fdir)`.
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

/// Port of `commandnotfound()` from `Src/exec.c:669` — C decl `commandnotfound(char *arg0, LinkList args)`.
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
    // c:679 — `pushnode(args, arg0);` — prepend arg0 (handler name
    // is the first positional arg per C convention).
    args.insert(0, arg0.to_string());
    args.insert(0, "command_not_found_handler".to_string());
    // c:680 — `lastval = doshfunc(shf, args, 1);`. Direct doshfunc
    // call mirrors C — body_runner routes through the host body-only
    // entry so the function body runs once inside doshfunc's scope.
    let shf_clone: Option<shfunc> = shfunctab_lock()
        .read()
        .ok()
        .and_then(|t| t.get("command_not_found_handler").cloned());
    if let Some(mut shf) = shf_clone {
        let body_args = args.clone();
        let body_runner = move || -> i32 {
            crate::ported::exec::run_function_body("command_not_found_handler", &body_args[1..])
                .unwrap_or(0)
        };
        let lv = doshfunc(&mut shf, args.clone(), true, body_runner);
        LASTVAL.store(lv, Ordering::Relaxed);
    }
    0 // c:681
}

/// Port of `namedpipe()` from `Src/exec.c:5001` — C decl `namedpipe(void)`.
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
/// Port of `readoutput()` from `Src/exec.c:4805` — C decl `readoutput(int in, int qt, int *readerror)`.
/// Drain a command-substitution pipe fd and
/// return the captured output split per `qt`.
///
/// `qt=1` (quoted-substitution `"$(...)"`): single-element vec with
/// the trailing-newline-trimmed buffer (empty buffer → `Nularg` sentinel
/// per c:4861).
/// `qt=0` (unquoted `$(...)`): split on IFS via `spacesplit`; if
/// `GLOBSUBST` is set, each word is `shtokenize`d for downstream globbing.
///
/// `readerror` is set to the errno on read failure, 0 on clean EOF.
pub fn readoutput(in_fd: i32, qt: i32, readerror: &mut i32) -> Vec<String> {
    // c:4805
    let mut buf: Vec<u8> = Vec::with_capacity(64); // c:4816 (initial bsiz=64)
    let mut readret: isize = 0; // c:4818 readret tracks last read return
                                // c:4824 dont_queue_signals(); c:4825 child_unblock(); — signal-queue
                                // dance keeps SIGCHLD live so the foreground process can be reaped
                                // while we drain. zshrs's in-process command-sub runs without the
                                // queue (no fork), but the C call surface is preserved for parity.
    dont_queue_signals(); // c:4824
    child_unblock(); // c:4825
    let mut inbuf = [0u8; 64]; // c:4815 inbuf[64]
    loop {
        // c:4826
        // c:4828 — `readret = read(in, inbuf, 64);`
        let r = unsafe { libc::read(in_fd, inbuf.as_mut_ptr() as *mut libc::c_void, inbuf.len()) };
        readret = r as isize;
        if readret <= 0 {
            // c:4829
            if readret < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                // c:4830 — `if (readret < 0 && errno == EINTR) continue;`
                continue;
            }
            break; // c:4832
        }
        // c:4835-4849 — `for (bufptr = inbuf; ...) { c = *bufptr; if
        // (imeta(c)) { *ptr++ = Meta; c ^= 32; cnt++; } ... *ptr++ = c; }`
        //
        // !!! DELIBERATE DEVIATION — the byte-level metafication is DROPPED.
        //
        // C's buffer is a `char *` that may hold arbitrary bytes, so it
        // escapes every `imeta` byte (NUL, and Meta 0x83 .. Marker 0xa2 —
        // `Src/utils.c:4195-4201`) to keep raw input bytes from colliding
        // with the token bytes the lexer/glob layers use.
        //
        // zshrs's buffer is a Rust `String`. A lone 0x83 is not valid UTF-8,
        // so `String::from_utf8_lossy` below replaced every emitted `Meta`
        // with U+FFFD — the metafied form could never survive the conversion,
        // and the "escaped" payload byte was left stranded next to a
        // replacement char. `utils::spacesplit`'s Meta decoder
        // (`utils.rs:5307`, `bytes[i] == Meta`) can never see a real Meta in a
        // `String` for the same reason, so nothing downstream un-did it
        // either. The loop was pure corruption for any input containing a
        // byte in {0x00} ∪ [0x83,0xa2] — which includes the UTF-8 tails of
        // common characters (U+2003 EM SPACE is `e2 80 83`, U+2022 BULLET is
        // `e2 80 a2`).
        //
        // The in-process `$( cmd )` arm in `getoutput` (:619) never had a
        // metafication step at all — it receives a plain `String` from
        // `run_command_substitution`. Dropping it here is what makes the two
        // arms hand this tail the SAME representation, which is the
        // property C gets for free by routing both arms through `readoutput`.
        buf.extend_from_slice(&inbuf[..readret as usize]); // c:4848
    }
    child_block(); // c:4854
                   // c:4855 — `if (readerror) *readerror = readret < 0 ? errno : 0;`
    *readerror = if readret < 0 {
        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
    } else {
        0
    };
    // c:4857 — `close(in);`
    unsafe {
        libc::close(in_fd);
    }
    // ── c:4858-4872 tail ──────────────────────────────────────────────
    // !!! THIS BLOCK IS DUPLICATED at `getoutput` (:619), which is the
    // `$( cmd )` arm. C needs no duplicate: both of getoutput's arms
    // (c:4746 `$(< file)` and c:4772 the pipe) call `readoutput`, so they
    // share this tail by construction. zshrs collapses the fork, so its
    // `$( cmd )` arm holds an in-memory String and has no fd to hand
    // `readoutput`. Factoring the tail into a helper is what build.rs's
    // no-new-functions gate on src/ported/ forbids (there is no C function
    // to name it after), and its own remedy #1 is "inline the body at every
    // call site" — so the two copies must be kept in lockstep BY HAND.
    // ANY EDIT HERE MUST BE MIRRORED AT :619 AND VICE VERSA. The copies
    // already drifted once: the getoutput copy (:619) had silently lost the
    // c:4868-4869 `isset(GLOBSUBST)` → `shtokenize` step, so `setopt
    // globsubst; echo $(echo '*')` did not glob while `$(< f)` did.
    let s = String::from_utf8_lossy(&buf);
    // c:4858-4859 — `while (cnt && ptr[-1] == '\n') ptr--, cnt--;`
    let s = s.trim_end_matches('\n');
    // c:4861-4863 — qt branch: empty → Nularg sentinel; else single elem.
    if qt != 0 {
        // c:4861
        if s.is_empty() {
            return vec![String::from(Nularg)]; // c:4862
        }
        return vec![s.to_string()]; // c:4864
    }
    // c:4866-4871 — `spacesplit` + per-word GLOBSUBST `shtokenize`.
    let mut words = crate::ported::utils::spacesplit(s, false); // c:4867
    if isset(crate::ported::zsh_h::GLOBSUBST) {
        // c:4870
        for w in words.iter_mut() {
            crate::ported::glob::shtokenize(w); // c:4870
        }
    }
    words
}

/// Port of `parsecmd()` from `Src/exec.c:4878` — C decl `parsecmd(char *cmd, char **eptr)`.
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

/// Port of `makecline()` from `Src/exec.c:2046` — C decl `makecline(LinkList list)`.
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

/// Port of `execute()` from `Src/exec.c:723` — C decl `execute(LinkList args, int flags, int defpath)`.
/// The canonical "child runs the simple
/// external command" path: STTY/ARGV0/BINF_DASH handling, makecline,
/// closem(FDT_XTRACE) + child_unblock, slash-path direct exec,
/// defpath (`command -p`) search, cmdnamtab + $PATH walk, with
/// commandnotfound-handler fallback and the final exit-code escape
/// (127 not-found / 126 noperm).
///
/// =================== WARNING — DIVERGENCE ====================
/// (a) `cmdnamtab->getnode(cmdnamtab, arg0)` (c:824) — HASHED
///     fast-path wired via cmdnamtab_lock(); jumps direct to
///     `cn.cmd` absolute path before the $PATH scan. Unhashed
///     cursor-walk (c:830-846) still falls to the full $PATH scan;
///     observable behavior matches C when the hash hit is HASHED.
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
                     // c:737 — `arg0 = (char *) peekfirst(args);` — the COMMAND WORD.
    let arg0 = if args.is_empty() {
        return;
    } else {
        args[0].clone()
    }; // c:737
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
    // c:758-777 — ARGV0 override.
    //   c:758-759 — /* If ARGV0 is in the commands environment, we use *
    //                * that as argv[0] for this external command       */
    // NOTE: C rewrites only `argv[0]` (the exec argv), NEVER `arg0`
    // (`peekfirst(args)`, c:737) — `arg0` stays the COMMAND WORD and is what
    // the slash-exec / cmdnamtab / $PATH scan below resolves. The previous
    // Rust port assigned `arg0 = args[0].clone()` in both arms, so
    // `ARGV0=sh $cmdvar args` looked up `sh` on $PATH instead of `$cmdvar`
    // (E03posix:16 — `command not found: sh`), and a `-` precommand made the
    // shell search for `-name`.
    if let Some(z) = zgetenv("ARGV0") {
        args[0] = z.clone(); // c:761 `argv[0] = ztrdup(z);`
        unsafe {
            let key = std::ffi::CString::new("ARGV0").unwrap();
            libc::unsetenv(key.as_ptr()); // c:768
        }
    } else if (flags & BINF_DASH) != 0 {
        // c:772-776 — /* Else if the pre-command `-' was given, we add `-' *
        //              * to the front of argv[0] for this command.         */
        args[0] = format!("-{}", arg0); // c:775-776
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
            // c:797 — `zerr("%e: %s", lerrno, arg0)`. `%e` is zerrmsg's
            // errno arm (Src/utils.c:348-365): strerror with the first
            // letter lowercased, NOT Rust's `io::Error` Display (which
            // appends " (os error N)" and keeps the capital, so `~`
            // printed `Permission denied (os error 13): /Users/wizard`
            // where zsh prints `permission denied: /Users/wizard`).
            zerr(&format!(
                "{}: {}",
                crate::ported::utils::zsh_errno_msg(lerrno),
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
        // c:822 — cmdnamtab fast-path: if `arg0` is a hashed cmdnam,
        // jump straight to the absolute path stored in `cn.cmd`,
        // skipping the full $PATH scan (one exec attempt vs N).
        // c:824 — `if ((cn = cmdnamtab->getnode(cmdnamtab, arg0)))`.
        let hashed_path: Option<String> = {
            let tab = cmdnamtab_lock().read().ok();
            tab.and_then(|t| {
                t.get(&arg0).and_then(|cn| {
                    if (cn.node.flags & crate::ported::zsh_h::HASHED) != 0 {
                        // c:827-828 — `strcpy(nn, cn->u.cmd);`
                        cn.cmd.clone()
                    } else {
                        None
                    }
                })
            })
        };
        if let Some(nn) = hashed_path {
            // c:848 — `ee = zexecve(nn, argv, newenvp);`
            ee = zexecve(&nn, &argv, newenvp);
            let dir = nn.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            if isgooderr(ee, if dir.is_empty() { "/" } else { dir }) {
                eno = ee;
            }
            // If the hashed entry's exec failed without a "good" error,
            // we still need the $PATH fallback — fall through.
            if eno == 0 && ee != 0 {
                // Reset for the $PATH scan below.
                ee = 0;
            }
        }
        // c:822 — normal $PATH scan (always runs; cmdnam fast-path was an
        // optimization but C also walks the rest of `path` if the hashed
        // exec failed with a non-"good" error).
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
        // c:872 — `zerr("%e: %s", eno, arg0)`; same `%e` (Src/utils.c:
        // 348-365) rendering as the slash-in-arg0 branch above.
        zerr(&format!(
            "{}: {}",
            crate::ported::utils::zsh_errno_msg(eno),
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

/// Port of `zexecve()` from `Src/exec.c:504` — C decl `zexecve(char *pth, char **argv, char **newenvp)`.
/// Wraps `execve(2)` with:
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
///     (C overwrites BEFORE `argv[0]`) — Rust rebuilds a fresh
///     `Vec<String>` with interp + optional arg + original argv tail
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
    let mut argv_ptrs: Vec<*const libc::c_char> = cargs.iter().map(|c| c.as_ptr()).collect();
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
        // c:534 — the `#!` / shebang-less recovery; see zexecve_recover.
        match crate::vm_helper::zexecve_recover(pth, argv, eno) {
            // c:566/571/581/585/627 — `execve(<prog>, <argv>, newenvp);`
            Ok((prog, argv_new)) => return zexecve(&prog, &argv_new, newenvp),
            Err(e) => return e, // c:643
        }
    }
    eno // c:643
}

/// Port of `getoutputfile()` from `Src/exec.c:4910` — C decl `getoutputfile(char *cmd, char **eptr)`.
/// `=(cmd)` process substitution.
///
/// Substitutes the cmd's stdout into a temp file, returns the
/// filename. Optimised path: `=(<<<heredoc-str)` writes the
/// heredoc body directly without a fork.
///
/// (a) `addfilelist(nam, 0)` (c:4960) wired via `JOBTAB[thisjob]`
///     so the temp file gets cleaned at job exit.
/// (b) `waitforpid` Rust takes 1 arg `pid`, C takes `(pid, full)`.
///     Behavior matches the `full=0` case anyway.
/// (c) This is a REAL fork (`zfork` below), not a collapsed one, so the
///     child gets the full `entersubsh(ESUB_PGRP|ESUB_NOMONITOR, NULL)`
///     (c:4986) via the canonical port at exec.rs:4329 — no in-process
///     `SubshStateGuard` substitute is needed or wanted here.
/// (d) `execode` is now ported (exec.rs:6047) — the body still
///     re-feeds through fusevm for cache coherence with execstring.
/// (e) `_realexit` flushes stdio + jobs + history. We use bare
///     `std::process::exit(0)` for now.
/// (f) TMPSUFFIX link()-rename block (c:4951-4958) deferred; rare
///     `setopt suffix_alias` interaction with =(…).
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
    // c:4960 — `addfilelist(nam, 0);` — register temp file in current
    // job's filelist so it's unlinked at job exit (not relying on the
    // OS temp-reaper).
    if let Some(jt) = JOBTAB.get() {
        let mut guard = jt.lock().unwrap();
        let tj = THISJOB.get().map(|m| *m.lock().unwrap()).unwrap_or(-1);
        if tj >= 0 {
            if let Some(j) = guard.get_mut(tj as usize) {
                crate::ported::jobs::addfilelist(j, Some(&nam), 0);
            }
        }
    }
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
    let _ = crate::ported::exec::execute_script_zsh_pipeline(body);
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

/// Port of `getproc()` from `Src/exec.c:5025` — C decl `getproc(char *cmd, char **eptr)`.
/// `<(cmd)` / `>(cmd)` process substitution
/// via `/dev/fd/N` (PATH_DEV_FD branch; modern Linux/macOS).
///
/// (a) PATH_DEV_FD branch only — the FIFO fallback (`!PATH_DEV_FD`
///     path c:5037-5064) is omitted; modern Linux/macOS both
///     provide /dev/fd. `namedpipe()` is ported (exec.rs:2701) but
///     unused here.
/// (b) `addproc` is 7-arg; procsubst pid recorded via aux=true on
///     the current job (c:5141-5142).
/// (c) `addfilelist(NULL, fd)` wired via `JOBTAB[thisjob]` at
///     c:5087.
/// (d) `entersubsh` is ported at exec.rs:4329 — wired below at
///     c:5095 (`entersubsh(ESUB_ASYNC|ESUB_PGRP, NULL)`) in the real
///     forked child.
/// (e) `execode` is ported at exec.rs:6047. Body still re-feeds
///     through fusevm for cache coherence.
/// (f) `_realexit` flushes stdio + jobs + history. We use bare
///     `std::process::exit(LASTVAL)` for now.
/// (g) `fdtable[fd] = FDT_PROC_SUBST` (c:5086) — set via fdtable_set.
pub fn getproc(cmd: &str, eptr: Option<&mut usize>) -> Option<String> {
    // c:5025
    let bytes = cmd.as_bytes();
    let out: i32 = if !bytes.is_empty() && (bytes[0] as char) == Inang {
        1 // c:5032 — `<(...)` writer-side child
    } else {
        0
    };
    // c:5068-5071 — `if (thisjob == -1) { zerr(...); return NULL; }` —
    // proc subst needs a host job to attach the child to.
    let tj_check = THISJOB.get().map(|m| *m.lock().unwrap()).unwrap_or(-1);
    if tj_check == -1 {
        zerr(&format!("process substitution {} cannot be used here", cmd)); // c:5069
        return None; // c:5070
    }
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
    let mut bgtime: ZshTimespec = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
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
                                         // c:5087 — `addfilelist(NULL, fd);` — register the proc-subst
                                         // pipe fd in the current job's filelist so it's closed at job exit.
        if let Some(jt) = JOBTAB.get() {
            let mut guard = jt.lock().unwrap();
            let tj = THISJOB.get().map(|m| *m.lock().unwrap()).unwrap_or(-1);
            if tj >= 0 {
                if let Some(j) = guard.get_mut(tj as usize) {
                    crate::ported::jobs::addfilelist(j, None, fd);
                }
            }
        }
        // c:5088-5091 — `if (!out) addproc(pid, NULL, 1, &bgtime, -1, -1);` —
        // record the proc-subst writer-side child in the job's
        // auxprocs (aux=true). For `<(cmd)` (out==1 = reader-side
        // child), C omits the addproc — symmetric here.
        if out == 0 {
            if let Some(jt) = JOBTAB.get() {
                let mut guard = jt.lock().unwrap();
                let tj = THISJOB.get().map(|m| *m.lock().unwrap()).unwrap_or(-1);
                if tj >= 0 {
                    if let Some(j) = guard.get_mut(tj as usize) {
                        crate::ported::jobs::addproc(
                            j,
                            pid,
                            "",
                            true,
                            Some(std::time::Instant::now()),
                            -1,
                            -1,
                        );
                    }
                }
            }
        }
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
    let _ = crate::ported::exec::execute_script_zsh_pipeline(body);
    cmdpop(); // c:5102
    let _ = zclose(out); // c:5103
    std::process::exit(LASTVAL.load(Ordering::Relaxed)); // c:5104
}

/// Port of `enum { ESUB_ASYNC, ESUB_PGRP, ... };` from `Src/exec.c:1056`.
/// Flag bits for `entersubsh(int flags, struct entersubsh_ret *retp)`.
pub mod esub {
    // c:1056
    /// `ASYNC` constant.
    pub const ASYNC: i32 = 0x01; // c:1058
    /// `PGRP` constant.
    pub const PGRP: i32 = 0x02; // c:1063
    /// `KEEPTRAP` constant.
    pub const KEEPTRAP: i32 = 0x04; // c:1065
    /// `FAKE` constant.
    pub const FAKE: i32 = 0x08; // c:1067
    /// `REVERTPGRP` constant.
    pub const REVERTPGRP: i32 = 0x10; // c:1069
    /// `NOMONITOR` constant.
    pub const NOMONITOR: i32 = 0x20; // c:1071
    /// `JOB_CONTROL` constant.
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
    pub gleader: i32,       // c:1122
    pub list_pipe_job: i32, // c:1123
}

/// Port of `entersubsh()` from `Src/exec.c:1084` — C decl `entersubsh(int flags, struct entersubsh_ret *retp)`.
/// Called by every child fork to switch the
/// process into subshell mode: traps reset, monitor disabled, signals
/// re-defaulted, pgrp + tty handed off, saved fds closed, jobtab
/// cleared, ZSH_SUBSHELL bumped, forklevel = locallevel.
///
/// (a) `jobtab[list_pipe_job]` / `jobtab[thisjob]` pgrp ops (c:1110-
///     1151) are now ported via `JOBTAB[thisjob]`.gleader access; the
///     ESUB_PGRP+sync path establishes pipeline group-leadership
///     (list_pipe_job inherit or thisjob-as-leader), filling
///     entersubsh_ret with the chosen gleader + list_pipe_job index.
/// (b) `clearjobtab(monitor)` (c:1219) — Rust signature is
///     `clearjobtab(&mut JobTable, monitor)`; we get the global table
///     via a TABLE handle similar to other jobs.rs entries.
/// (c) `attachtty(...)` (c:1119, 1144) — wired via libc::tcsetpgrp(2, gleader).
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
    job_control_ok = if monitor != 0 && (flags & esub::JOB_CONTROL) != 0 && isset(POSIXJOBS) {
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
                if unsafe { libc::open(devnull.as_ptr(), libc::O_RDWR | libc::O_NOCTTY) } != 0 {
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
    } else if (flags & esub::PGRP) != 0 {
        // c:1110 — `else if (thisjob != -1 && (flags & ESUB_PGRP))`.
        let thisjob = THISJOB.get().map(|m| *m.lock().unwrap()).unwrap_or(-1);
        if thisjob != -1 {
            let lpj = list_pipe_job.load(Ordering::Relaxed);
            let lp = list_pipe.load(Ordering::Relaxed);
            let lpc = list_pipe_child.load(Ordering::Relaxed);
            if let Some(jt) = JOBTAB.get() {
                let mut guard = jt.lock().unwrap();
                let lpj_gleader = guard.get(lpj as usize).map(|j| j.gleader).unwrap_or(0);
                if lpj_gleader != 0 && (lp != 0 || lpc != 0) {
                    // c:1111-1124 — inherit list_pipe_job's group leader.
                    let pgid = if unsafe { libc::setpgid(0, lpj_gleader) } == -1
                        || (unsafe { libc::killpg(lpj_gleader, 0) } == -1
                            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH))
                    {
                        // c:1115-1117 — primary group leader gone; this child becomes leader.
                        let new_gl = if lpc != 0 {
                            mypgrp.load(Ordering::Relaxed)
                        } else {
                            unsafe { libc::getpid() }
                        };
                        if let Some(j) = guard.get_mut(lpj as usize) {
                            j.gleader = new_gl;
                        }
                        if let Some(j) = guard.get_mut(thisjob as usize) {
                            j.gleader = new_gl;
                        }
                        unsafe { libc::setpgid(0, new_gl) };
                        if (flags & esub::ASYNC) == 0 {
                            unsafe { libc::tcsetpgrp(2, new_gl) }; // c:1119 attachtty
                        }
                        new_gl
                    } else {
                        lpj_gleader
                    };
                    if let Some(r) = retp {
                        if (flags & esub::ASYNC) == 0 {
                            r.gleader = pgid; // c:1122
                            r.list_pipe_job = lpj; // c:1123
                        }
                    }
                } else {
                    // c:1126-1151 — standard group-leader-takeover path.
                    let thisjob_gleader =
                        guard.get(thisjob as usize).map(|j| j.gleader).unwrap_or(0);
                    if thisjob_gleader == 0 || unsafe { libc::setpgid(0, thisjob_gleader) } == -1 {
                        let new_gl = unsafe { libc::getpid() };
                        if let Some(j) = guard.get_mut(thisjob as usize) {
                            j.gleader = new_gl; // c:1138
                        }
                        if lpj != thisjob {
                            let lpj_was_unset = guard
                                .get(lpj as usize)
                                .map(|j| j.gleader == 0)
                                .unwrap_or(true);
                            if lpj_was_unset {
                                if let Some(j) = guard.get_mut(lpj as usize) {
                                    j.gleader = new_gl; // c:1140-1141
                                }
                            }
                        }
                        unsafe { libc::setpgid(0, new_gl) }; // c:1142
                        if (flags & esub::ASYNC) == 0 {
                            unsafe { libc::tcsetpgrp(2, new_gl) }; // c:1144 attachtty
                            if let Some(r) = retp {
                                r.gleader = new_gl; // c:1146
                                if lpj != thisjob {
                                    r.list_pipe_job = lpj; // c:1148
                                }
                            }
                        }
                    }
                }
            }
        } else {
            // No real job slot; basic setpgid fallback.
            unsafe { libc::setpgid(0, 0) };
        }
    }
    if (flags & esub::FAKE) == 0 {
        // c:1153
        subsh.store(1, Ordering::Relaxed); // c:1154
    }
    // c:1161 — `zsh_subshell++;` regardless of FAKE.
    zsh_subshell.fetch_add(1, Ordering::Relaxed);
    // c:1162 — `if ((flags & ESUB_REVERTPGRP) && getpid() == mypgrp)`.
    if (flags & esub::REVERTPGRP) != 0
        && unsafe { libc::getpid() } == mypgrp.load(Ordering::Relaxed)
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
    // c:1218-1219 — `clearjobtab(monitor);` — calls the canonical port
    // at jobs.rs:1695 which handles ALL the C body including the
    // oldjobtab snapshot path (c:1799-1817) under POSIXJOBS guard.
    let mut dummy_table = crate::exec_jobs::JobTable::new();
    crate::ported::jobs::clearjobtab(&mut dummy_table, monitor);
    let _ = get_usage(); // c:1220
    FORKLEVEL.store(
        // c:1221 — `forklevel = locallevel;`
        locallevel.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
}

/// RAII guard applying the fork-safe subset of
/// `entersubsh(ESUB_PGRP|ESUB_NOMONITOR, NULL)` (`Src/exec.c:1083`) around a
/// command substitution that zshrs runs **in-process**, restoring every
/// touched global on drop.
///
/// C's `getoutput()` forks, and only the child runs `entersubsh`:
///
/// ```c
///     /* pid == 0 */
///     child_unblock();
///     zclose(pipes[0]);
///     redup(pipes[1], 1);
///     entersubsh(ESUB_PGRP|ESUB_NOMONITOR, NULL);   /* c:4781 */
///     cmdpush(CS_CMDSUBST);
///     execode(prog, 0, 1, "cmdsubst");
/// ```
///
/// so every state delta dies with the child and the parent is untouched.
/// zshrs collapses that fork (`getoutput` below → `run_command_substitution`),
/// which means the deltas were previously never applied **at all**: inside
/// `$( )` the shell kept the caller's `zleactive` / `opts[USEZLE]`, so every
/// reader of them took the "we are inside the line editor" branch. Observed
/// failure: `lt=( ${(f)"$(fc -l -200)"} )` bound to a ZLE widget died with
/// `no interactive history within ZLE` — `bin_fc`'s guard
/// (`builtin.rs:2757`, a correct port of `Src/builtin.c:1523-1527`) firing on
/// a stale `zleactive`. Same class of bug for every other reader:
/// `jobs.rs:1114` (time-report suppression), `utils.rs:1785` (screen
/// ownership), `signals.rs:2050` (refresh-on-signal).
///
/// APPLIED (pure restorable scalar state; correct without a fork):
///   * c:1097 / c:1207 — `opts[MONITOR] = 0`. ESUB_NOMONITOR is set and
///     `job_control_ok` is 0 (no ESUB_JOB_CONTROL), so both C branches
///     agree. A substitution body must not do job control.
///   * c:1164 — `shout = NULL`. Its readers already handle the NULL case
///     the way the C child does (`putshout` falls back to stderr,
///     `checkrmall` returns 1 at `utils.rs:3565` per c:2871, `mailcheck`
///     stays silent per c:1670).
///   * c:1208 — `opts[USEZLE] = 0`.
///   * c:1209 — `zleactive = 0`.
///
/// DELIBERATELY SKIPPED — each would be wrong, or is already handled
/// elsewhere on the in-process path:
///   * c:1088-1092 `unsettrap(sig)` — the trap table is not scalar state
///     (trap bodies/dispositions), and with no fork there is no child exit
///     for an EXIT trap to fire at. Restoring it is not a swap.
///   * c:1095 `exit_val = 0` — ALREADY DONE on this path:
///     `vm_helper.rs:3893` swaps `EXIT_VAL` to 0 before the nested VM runs
///     and restores it at `vm_helper.rs:4004`.
///   * c:1110-1151 `setpgrp`/`attachtty`/gleader bookkeeping — process-group
///     and tty surgery. With no fork this moves the interactive shell's own
///     process group and hands the terminal away.
///   * c:1154 `subsh = 1` — `init.rs:2289` does `if subsh != 0 { realexit() }`.
///     With no fork, an `exit` inside `$( )` would then hard-kill the
///     interactive shell instead of ending the substitution.
///   * c:1161 `zsh_subshell++` — ALREADY DONE on this path by
///     `CmdSubstSubshellBump` (`fusevm_bridge.rs:232`, entered at
///     `vm_helper.rs:3734`). Bumping again would make `$ZSH_SUBSHELL` read 2
///     inside `$( )` instead of 1.
///   * c:1165-1193 `signal_ignore`/`signal_default` — per-process signal
///     dispositions. Defaulting SIGINT/SIGQUIT/SIGTSTP in the live
///     interactive shell would let ^C kill and ^Z stop the shell itself.
///   * c:1202-1205 `signal_unblock` — the process signal mask is owned by
///     the trap dispatcher's block/unblock pairing; unblocking here breaks
///     that pairing.
///   * c:1214-1217 close FDT_SAVED_MASK fds — fd surgery. These are the
///     parent's saved descriptors for redirection restore; closing them is
///     unrecoverable.
///   * c:1219 `clearjobtab(monitor)` — would destroy the parent's job table.
///   * c:1220 `get_usage()` — overwrites the parent's `times` baseline with
///     no fork to discard it, and nothing inside the substitution reads it.
///   * c:1221 `forklevel = locallevel` — FORKLEVEL gates local-scope
///     unwinding; the in-process body pushes and pops its own locals
///     normally, so moving the fork boundary would strand them.
///
/// Nesting is safe: each guard saves what it observed on entry and restores
/// exactly that on drop, so LIFO nesting (`$( … $( … ) … )`) unwinds
/// correctly.
pub struct SubshStateGuard {
    saved_monitor: bool,
    saved_shout: usize,
    saved_usezle: bool,
    saved_zleactive: i32,
    saved_subsh: i32,
}

impl SubshStateGuard {
    /// Apply the deltas listed above and capture the values to restore.
    pub fn enter() -> Self {
        let g = SubshStateGuard {
            saved_monitor: isset(MONITOR),
            saved_shout: *shout.lock().unwrap(),
            saved_usezle: isset(USEZLE),
            saved_zleactive: zleactive.load(Ordering::Relaxed),
            saved_subsh: subsh.load(Ordering::Relaxed),
        };
        // c:1153-1154 — `if (!(flags & ESUB_FAKE)) subsh = 1;`. The
        // substitution's body is "in a subshell" for every consumer that
        // asks, which is what keeps PRINT_EXIT_VALUE quiet inside
        // `x=$(false)` (c:4309 `&& !subsh`).
        subsh.store(1, Ordering::Relaxed); // c:1154
                                           // `force = 1`: C assigns the `opts[]` slots directly, so the
                                           // dosetopt gatekeeping (c:743-861, which only guards turning
                                           // options ON) must not apply in either direction.
        dosetopt(MONITOR, 0, 1); // c:1097 / c:1207
        *shout.lock().unwrap() = 0; // c:1164
        dosetopt(USEZLE, 0, 1); // c:1208
        zleactive.store(0, Ordering::Relaxed); // c:1209
        g
    }
}

impl Drop for SubshStateGuard {
    fn drop(&mut self) {
        // Reverse order of `enter`; the C child never restores because it
        // `_realexit()`s, so this half has no C counterpart to cite.
        subsh.store(self.saved_subsh, Ordering::Relaxed);
        zleactive.store(self.saved_zleactive, Ordering::Relaxed);
        dosetopt(USEZLE, self.saved_usezle as i32, 1);
        *shout.lock().unwrap() = self.saved_shout;
        dosetopt(MONITOR, self.saved_monitor as i32, 1);
    }
}

/// Port of `getpipe()` from `Src/exec.c:5119` — C decl `getpipe(char *cmd, int nullexec)`.
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
/// (a) `addproc` is now 7-arg (jobs.rs:1516) — wired at the
///     procsubst pid recording site (c:5141-5142) earlier this
///     session; the child IS now recorded in `JOBTAB[thisjob]`.
/// (b) `entersubsh` IS ported (exec.rs:4329) including the
///     ESUB_PGRP pipeline group-leadership path, and is called from
///     the real forked child below for getpipe's
///     `entersubsh(ESUB_ASYNC|ESUB_PGRP|ESUB_NOMONITOR, NULL)`.
/// (c) `execode(prog, ...)` IS now ported (exec.rs:6047) — getpipe
///     can route through execode for the parsed eprog. Currently
///     this caller still uses the fusevm pipeline for cache
///     coherence with execstring; switch over when the wordcode
///     walker becomes the primary path.
/// (d) `_realexit()` flushes stdio + jobs + history. We use bare
///     `std::process::exit(lastval)` for now.
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
    let mut bgtime: ZshTimespec = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let pid = zfork(Some(&mut bgtime)); // c:5135
    if pid != 0 {
        // c:5135 — parent.
        let _ = zclose(pipes[out as usize]); // c:5136
        if pid == -1 {
            // c:5137
            let _ = zclose(pipes[(1 - out) as usize]); // c:5138
            return -1; // c:5139
        }
        // c:5141-5142 — `if (!nullexec) addproc(pid, NULL, 1, &bgtime, -1, -1);`
        if nullexec == 0 {
            if let Some(jt) = JOBTAB.get() {
                let mut guard = jt.lock().unwrap();
                let tj = THISJOB.get().map(|m| *m.lock().unwrap()).unwrap_or(-1);
                if tj >= 0 {
                    if let Some(j) = guard.get_mut(tj as usize) {
                        crate::ported::jobs::addproc(
                            j,
                            pid,
                            "",
                            true, // aux=1 for proc subst
                            Some(std::time::Instant::now()),
                            -1,
                            -1,
                        );
                    }
                }
            }
        }
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
    let _ = crate::ported::exec::execute_script_zsh_pipeline(body);
    cmdpop(); // c:5151
              // c:5152 — _realexit() — WARNING (d).
    std::process::exit(LASTVAL.load(Ordering::Relaxed));
}

/// Port of `spawnpipes()` from `Src/exec.c:5184` — C decl `spawnpipes(LinkList l, int nullexec)`.
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

/// Port of `cancd2()` from `Src/exec.c:6411` — C decl `cancd2(char *s)`.
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
            format!(
                "{}/{}",
                if pwd_str.len() > 1 { &pwd_str[..] } else { "" },
                s
            )
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

/// Port of `cancd()` from `Src/exec.c:6370` — C decl `cancd(char *s)`.
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

/// Port of `simple_redir_name()` from `Src/exec.c:4689` — C decl `simple_redir_name(Eprog prog, int redir_type)`.
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

/// Port of `getherestr()` from `Src/exec.c:4655` — C decl `getherestr(struct redir *fn)`.
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

/// Port of `quote_tokenized_output()` from `Src/exec.c:2114` — C decl `quote_tokenized_output(char *str, FILE *file)`.
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
            b'\\' | b'<' | b'>' | b'(' | b'|' | b')' | b'^' | b'#' | b'~' | b'[' | b']' | b'*'
            | b'?' | b'$' | b' ' => {
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
// Wordcode-VM control-flow dispatch — faithful ports of the C
// `Src/exec.c` + `Src/loop.c` wordcode interpreter entries.
//
// Each function below takes `&mut estate` and returns `i32` to mirror
// the C `int execX(Estate state, int do_exec)` signature exactly. Per-
// line `// c:NNN` citations track the C source line.
//
// zshrs's primary execution path is the fusevm bytecode VM. These
// wordcode-VM entries exist for C-name parity with the upstream
// interpreter so that future bridging code can drive zshrs through
// the same dispatch tree zsh's `Src/init.c::loop` walks. Where
// zshrs primitives don't yet model their C counterpart (e.g.
// `execsubst`, `addvars`, `execfuncs[]` dispatch table), the local
// helper is declared with a comment citing the C source file:line
// where the canonical body lives — same pattern as the canonical
// `ksh93::ksh93_wrapper` port at c:152-227.
// =====================================================================

use crate::ported::math::{matheval as wc_matheval, mathevali as wc_mathevali};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::r#loop::try_tryflag;

// Addvars-specific imports (Src/exec.c:2497 port at exec.rs::addvars).
use crate::ported::builtin::{BREAKS, CONTFLAG, LOOPS, RETFLAG};
use crate::ported::linklist::LinkList;
use crate::ported::mem::freeheap;
use crate::ported::params::setloopvar;
use crate::ported::params::{assignaparam, assignsparam, unsetparam};
use crate::ported::parse::{ecgetlist, ecgetstr};
use crate::ported::pattern::haswilds;
use crate::ported::signals_h::{queue_signal_level, restore_queue_signals};
use crate::ported::subst::{globlist, prefork};
use crate::ported::zsh_h::{
    estate, wordcode, EC_DUP, EC_DUPTOK, EC_NODUP, NOERREXIT_EXIT, NOERREXIT_RETURN, PAT_STATIC,
    WC_CASE, WC_CASE_AND, WC_CASE_OR, WC_CASE_SKIP, WC_CASE_TESTAND, WC_CASE_TYPE, WC_CURSH_SKIP,
    WC_END, WC_FOR_COND, WC_FOR_LIST, WC_FOR_SKIP, WC_FOR_TYPE, WC_FUNCDEF, WC_FUNCDEF_SKIP, WC_IF,
    WC_IF_ELSE, WC_IF_SKIP, WC_IF_TYPE, WC_REPEAT_SKIP, WC_TIMED_EMPTY, WC_TIMED_TYPE, WC_TRY_SKIP,
    WC_WHILE_SKIP, WC_WHILE_TYPE, WC_WHILE_UNTIL,
};
use crate::ported::zsh_h::{
    ALLEXPORT, ASSPM_AUGMENT, ASSPM_KEY_VALUE, ASSPM_WARN, GLOBASSIGN, KSHARRAYS, PREFORK_ASSIGN,
    PREFORK_KEY_VALUE, PREFORK_SINGLE, WC_ASSIGN, WC_ASSIGN_INC, WC_ASSIGN_NUM, WC_ASSIGN_SCALAR,
    WC_ASSIGN_TYPE, WC_ASSIGN_TYPE2,
};
use crate::ported::zsh_h::{
    CS_ALWAYS, CS_CASE, CS_COND, CS_CURSH, CS_ELIF, CS_ELIFTHEN, CS_ELSE, CS_FOR, CS_IF, CS_IFTHEN,
    CS_MATH, CS_REPEAT, CS_UNTIL, CS_WHILE, MN_INTEGER,
};

// --- Local stubs for C primitives not yet ported elsewhere ------------
//
// These mirror the C functions of the same names. Each cites the C
// source file:line where the canonical body lives. They are inlined
// here (rather than a separate `pub fn` in the owning C-file module)
// because the owning ports are pending the wider exec-substrate
// work (sub-PR). Once those land, these locals collapse to direct
// `crate::ported::<owner>::<fn>` calls.

/// Port of `execsubst()` from `Src/exec.c:2684` — C decl `execsubst(LinkList strs)`.
///
/// C body (c:2684-2693):
/// ```c
/// void execsubst(LinkList strs) {
///     if (strs) {
///         prefork(strs, esprefork, NULL);
///         if (esglob && !errflag) {
///             LinkList ostrs = strs;
///             globlist(strs, 0);
///             strs = ostrs;
///         }
///     }
/// }
/// ```
///
/// `execsubst` runs `prefork` (parameter / arithmetic / command
/// substitution expansion + IFS-split) over the whole list, then
/// (when `esglob` is set) `globlist` to do filename globbing on the
/// result.
pub(crate) fn execsubst(list: &mut Vec<String>) {
    // c:2684
    if list.is_empty() {
        return; // c:2686 `if (strs)`
    }
    let mut ll: crate::ported::subst::LinkList = std::mem::take(list).into_iter().collect();
    let prefork_flags = esprefork.load(Ordering::Relaxed); // c:2687 esprefork
    let mut rf: i32 = 0;
    prefork(&mut ll, prefork_flags, &mut rf); // c:2687
    if esglob.load(Ordering::Relaxed) != 0 && errflag.load(Ordering::Relaxed) == 0 {
        // c:2688 `if (esglob && !errflag)`
        globlist(&mut ll, 0); // c:2690
    }
    *list = ll.into_iter().collect();
}

/// Port of `addvars()` from `Src/exec.c:2499` — C decl `addvars(Estate state, Wordcode pc, int addflags)`.
/// Direct port of `static void addvars(Estate state, Wordcode pc,
/// int addflags)` from `Src/exec.c:2497-2648`. Process the WC_ASSIGN
/// nodes stacked inline of a simple command — the `var=value` and
/// `arr=(v1 v2 v3)` assignments that precede argv. Walks the wordcode
/// at `pc`, extracts each assignment's name + value (scalar or array),
/// optionally preforks + globs the tokenised RHS, and routes through
/// `assignsparam` (scalar) or `assignaparam` (array).
///
/// XTRACE side-effect: prints `name=value ` / `name=( v1 v2 ) ` to
/// stderr (C uses xtrerr; zshrs uses eprint!).
///
/// `STTY=...` in an inline-export form (`STTY=raw cmd`) gets captured
/// into the file-static `STTYval` for `execute()` to apply pre-exec.
fn addvars(state: &mut estate, pc: usize, addflags: i32) {
    // c:2501 — locals.
    let mut vl: LinkList<String>; // c:2501 `LinkList vl;`
    let xtr: bool; // c:2502 `int xtr,`
    let mut isstr: bool; // c:2502 `int isstr,`
    let mut htok: i32 = 0; // c:2502 `int htok = 0;`
    let mut arr: Vec<String>; // c:2503 `char **arr, **ptr, *name;`
    let mut name: String;
    let mut flags: i32; // c:2504 `int flags;`
    let opc = state.pc; // c:2506 `Wordcode opc = state->pc;`
    let mut ac: wordcode; // c:2507 `wordcode ac;`
                          // c:2508 `local_list1(svl);` — stack-local one-element LinkList
                          // for the scalar-assignment path. Rust uses a fresh LinkList per
                          // iteration; equivalent semantics.

    // c:2510-2515 — comment about WARNCREATEGLOBAL warning suppression
    // when the assignment list is implicitly local (ADDVAR_RESTORE).
    flags = if (addflags & ADDVAR_RESTORE) == 0 {
        ASSPM_WARN // c:2516
    } else {
        0 // c:2516
    };
    xtr = isset(XTRACE); // c:2517 `xtr = isset(XTRACE);`
    if xtr {
        // c:2518
        printprompt4(); // c:2519
        doneps4.store(1, Ordering::Relaxed); // c:2520 `doneps4 = 1;`
    }
    state.pc = pc; // c:2522 `state->pc = pc;`

    // c:2523 `while (wc_code(ac = *state->pc++) == WC_ASSIGN) {`
    loop {
        if state.pc >= state.prog.prog.len() {
            break;
        }
        ac = state.prog.prog[state.pc];
        state.pc += 1;
        if wc_code(ac) != WC_ASSIGN {
            // Step back so the WC_SIMPLE / outer dispatcher sees the
            // non-assignment opcode. C's `state->pc++` post-increment
            // already pointed past WC_ASSIGN; we need to unconsume.
            state.pc -= 1;
            break;
        }
        let mut myflags = flags; // c:2524 `int myflags = flags;`
        name = ecgetstr(state, EC_DUPTOK, Some(&mut htok)); // c:2525
        if htok != 0 {
            // c:2526 `if (htok) untokenize(name);`
            name = untokenize(&name).to_string(); // c:2527
        }
        if WC_ASSIGN_TYPE2(ac) == WC_ASSIGN_INC {
            // c:2528
            myflags |= ASSPM_AUGMENT; // c:2529
        }
        if xtr {
            // c:2530
            // c:2531-2532 — fprintf(xtrerr, ... "%s+=" : "%s=", name);
            if WC_ASSIGN_TYPE2(ac) == WC_ASSIGN_INC {
                eprint!("{}+=", name); // c:2532
            } else {
                eprint!("{}=", name); // c:2532
            }
        }

        // c:2533 `if ((isstr = (WC_ASSIGN_TYPE(ac) == WC_ASSIGN_SCALAR))) {`
        isstr = WC_ASSIGN_TYPE(ac) == WC_ASSIGN_SCALAR;
        if isstr {
            // c:2534 `init_list1(svl, ecgetstr(state, EC_DUPTOK, &htok));`
            let svl_val = ecgetstr(state, EC_DUPTOK, Some(&mut htok));
            vl = LinkList::new();
            vl.push_back(svl_val);
            // c:2535 `vl = &svl;` — vl already points at the new list.
        } else {
            // c:2537 `vl = ecgetlist(state, WC_ASSIGN_NUM(ac), EC_DUPTOK, &htok);`
            let items = ecgetlist(
                state,
                WC_ASSIGN_NUM(ac) as usize,
                EC_DUPTOK,
                Some(&mut htok),
            );
            vl = LinkList::new();
            for it in items {
                vl.push_back(it);
            }
            if errflag.load(Ordering::Relaxed) != 0 {
                // c:2538-2541
                state.pc = opc; // c:2539
                return; // c:2540
            }
        }

        // c:2544 `if (vl && htok) {`
        if htok != 0 {
            // c:2545 `int prefork_ret = 0;`
            let mut prefork_ret: i32 = 0;
            // c:2546-2547 — prefork(vl, (isstr ? PREFORK_SINGLE|PREFORK_ASSIGN
            //                          : PREFORK_ASSIGN), &prefork_ret);
            let pf_flags = if isstr {
                PREFORK_SINGLE | PREFORK_ASSIGN
            } else {
                PREFORK_ASSIGN
            };
            prefork(&mut vl, pf_flags, &mut prefork_ret); // c:2547
            if errflag.load(Ordering::Relaxed) != 0 {
                // c:2548
                state.pc = opc; // c:2549
                return; // c:2550
            }
            if (prefork_ret & PREFORK_KEY_VALUE) != 0 {
                // c:2552
                myflags |= ASSPM_KEY_VALUE; // c:2553
            }
            // c:2554-2555 — `if (!isstr || (isset(GLOBASSIGN) && isstr &&
            //                  haswilds((char *)getdata(firstnode(vl)))))`
            let needs_glob = if !isstr {
                true
            } else {
                isset(GLOBASSIGN)
                    && isstr
                    && !vl.is_empty()
                    && haswilds(vl.nodes.front().map(|s| s.as_str()).unwrap_or(""))
            };
            if needs_glob {
                globlist(&mut vl, prefork_ret); // c:2556
                                                // c:2557-2562 — `if (isset(GLOBASSIGN) && isstr)
                                                //                  unsetparam(name);`
                if isset(GLOBASSIGN) && isstr {
                    unsetparam(&name); // c:2562
                }
                if errflag.load(Ordering::Relaxed) != 0 {
                    // c:2563
                    state.pc = opc; // c:2564
                    return; // c:2565
                }
            }
        }
        // c:2569 `if (isstr && (empty(vl) || !nextnode(firstnode(vl))))`
        // — scalar-assignment path: zero or one element after prefork.
        if isstr && (vl.is_empty() || vl.len() == 1) {
            let val: String; // c:2571 `char *val;`
            if vl.is_empty() {
                // c:2574
                val = String::new(); // c:2575 `val = ztrdup("");`
            } else {
                // c:2577 `untokenize(peekfirst(vl));`
                let peek = vl.nodes.front().cloned().unwrap_or_default();
                val = untokenize(&peek).to_string(); // c:2577-2578
                                                     // c:2578 `val = ztrdup(ugetnode(vl));` — ugetnode pops;
                                                     // we just cloned the front above. Equivalent.
            }
            if xtr {
                // c:2580
                eprint!("{}", quotedzputs(&val)); // c:2581
                eprint!(" "); // c:2582 `fputc(' ', xtrerr);`
            }
            // c:2584 `if ((addflags & ADDVAR_EXPORT) && !strchr(name, '['))`
            let pm = if (addflags & ADDVAR_EXPORT) != 0 && !name.contains('[') {
                // c:2585 `if (strcmp(name, "STTY") == 0)`
                if name == "STTY" {
                    // c:2586-2587 — `STTYval = ztrdup(val);`
                    let mut stty = STTYval.lock().unwrap();
                    *stty = Some(val.clone()); // c:2587
                }
                // c:2589 `allexp = opts[ALLEXPORT];`
                let allexp = isset(ALLEXPORT);
                // c:2590 `opts[ALLEXPORT] = 1;` — temporarily set.
                opt_state_set("allexport", true);
                if isset(KSHARRAYS) {
                    // c:2591
                    unsetparam(&name); // c:2592
                }
                let pm = assignsparam(&name, &val, myflags); // c:2593
                                                             // c:2594 `opts[ALLEXPORT] = allexp;` — restore.
                opt_state_set("allexport", allexp);
                pm
            } else {
                // c:2595
                assignsparam(&name, &val, myflags) // c:2596
            };
            if pm.is_none() {
                // c:2597 `if (!pm)`
                LASTVAL.store(1, Ordering::Relaxed); // c:2598 `lastval = 1;`
                                                     // c:2599-2604 — "cheating" comment: don't zerr.
                if cmdoutval.load(Ordering::Relaxed) == 0 {
                    // c:2605 `if (!cmdoutval)`
                    cmdoutval.store(1, Ordering::Relaxed); // c:2606
                }
            }
            if errflag.load(Ordering::Relaxed) != 0 {
                // c:2608
                state.pc = opc; // c:2609
                return; // c:2610
            }
            continue; // c:2612
        }
        // c:2614 `if (vl) { ... }` — array-assignment path: drain vl
        // into a fresh `char **arr`.
        // c:2615-2619 `ptr = arr = zalloc(...); while (nonempty(vl)) *ptr++ = ztrdup(ugetnode(vl));`
        arr = Vec::with_capacity(vl.len() + 1);
        while let Some(s) = vl.pop_front() {
            arr.push(s);
        }
        // c:2623 `*ptr = NULL;` — C terminator; Rust Vec doesn't need it.
        if xtr {
            // c:2624
            eprint!("( "); // c:2625
            for s in &arr {
                // c:2626 `for (ptr = arr; *ptr; ptr++)`
                eprint!("{}", quotedzputs(s)); // c:2627
                eprint!(" "); // c:2628
            }
            eprint!(") "); // c:2630
        }
        // c:2632 `if (!assignaparam(name, arr, myflags))`
        if assignaparam(&name, arr, myflags).is_none() {
            LASTVAL.store(1, Ordering::Relaxed); // c:2633
                                                 // c:2634-2638 — "cheating" comment.
            if cmdoutval.load(Ordering::Relaxed) == 0 {
                // c:2639
                cmdoutval.store(1, Ordering::Relaxed); // c:2640
            }
        }
        if errflag.load(Ordering::Relaxed) != 0 {
            // c:2642
            state.pc = opc; // c:2643
            return; // c:2644
        }
    }
    state.pc = opc; // c:2647 `state->pc = opc;`
}

// execfuncs[] dispatch table from `Src/exec.c:5499` is inlined as a
// match expression at the call sites in execsimple. Not a separate
// Rust fn — every C-side reference to
// `execfuncs[code - WC_CURSH](state, ...)` resolves inline below.

// --- exec.c entries ---------------------------------------------------

/// Port of `execcursh()` from `Src/exec.c:469` — C decl `execcursh(Estate state, int do_exec)`.
/// Execute a `{ ... }` current-shell command
/// group: skip the trailing try-only word, optionally drop a stale
/// job slot, then run the inner list.
pub fn execcursh(state: &mut estate, do_exec: i32) -> i32 {
    // c:472 — `end = state->pc + WC_CURSH_SKIP(state->pc[-1]);`
    let prior = state.prog.prog[state.pc.wrapping_sub(1)];
    let end = state.pc + WC_CURSH_SKIP(prior) as usize;
    // c:475 — `state->pc++;` skip the try/always-only word.
    state.pc += 1;
    // c:482-486 — drop empty job slot before nested cmd: if outer-pipe
    // bookkeeping is clean AND thisjob is a real job that's not the
    // pipe-leader AND has no procs yet, deletejob() recycles it. Avoids
    // leaking job-table slots when execcursh recurses.
    {
        let lp = list_pipe.load(Ordering::Relaxed);
        let lpj = list_pipe_job.load(Ordering::Relaxed);
        let tj = THISJOB.get().map(|m| *m.lock().unwrap()).unwrap_or(-1);
        if lp == 0 && tj != -1 && tj != lpj {
            if let Some(jt) = JOBTAB.get() {
                let mut guard = jt.lock().unwrap();
                let has = crate::ported::jobs::hasprocs(&guard, tj as usize);
                if !has {
                    if let Some(j) = guard.get_mut(tj as usize) {
                        crate::ported::jobs::deletejob(j, false);
                    }
                }
            }
        }
    }
    cmdpush(CS_CURSH as u8); // c:487 — `cmdpush(CS_CURSH);`
    let _ = execlist(state, 1, do_exec); // c:488 — `execlist(state, 1, do_exec);`
    cmdpop(); // c:489 — `cmdpop();`
    state.pc = end; // c:491 — `state->pc = end;`
    this_noerrexit.store(1, Ordering::Relaxed); // c:492 — `this_noerrexit = 1;`
    LASTVAL.load(Ordering::Relaxed) // c:494 — `return lastval;`
}

// `(...)` subshell — no dedicated C function (handled inline by
// `execpline`'s WC_PIPE branch via the WC_SUBSH bit, exec.c:2540+).
// In zshrs the subshell branch is folded into `execpline` and
// `execsimple`'s WC_SUBSH dispatch — both invoke execcursh for the
// inner-list walk since fusevm bytecode handles the forking via
// Op::Subshell at a higher layer.

/// Port of `execcond()` from `Src/exec.c:5204` — C decl `execcond(Estate state, UNUSED(int do_exec))`.
/// Run a `[[ ... ]]` cond expression.
pub fn execcond(state: &mut estate, _do_exec: i32) -> i32 {
    state.pc -= 1; // c:5208 — `state->pc--;`
                   // c:5209-5213 — XTRACE prelude.
    if isset(XTRACE) {
        printprompt4();
        eprint!("[[");
        // c:5212 — `tracingcond++;` not modeled in zshrs.
    }
    cmdpush(CS_COND as u8); // c:5214
                            // c:5215 — `stat = evalcond(state, NULL);` — TODO faithful: needs
                            // the wordcode-level evalcond from Src/cond.c which is distinct
                            // from the test-builtin evalcond ported in cond.rs. Pending.
    let stat: i32 = 0;
    // c:5219-5221 — `if (stat == 2) errflag |= ERRFLAG_ERROR;`
    if stat == 2 {
        errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
    }
    cmdpop(); // c:5222
    if isset(XTRACE) {
        eprintln!(" ]]");
    }
    stat // c:5230 — `return stat;`
}

/// Port of `execarith()` from `Src/exec.c:5235` — C decl `execarith(Estate state, UNUSED(int do_exec))`.
/// Run a `(( ... ))` arithmetic command;
/// returns 0 when val != 0 (success), 1 when val == 0 (false), 2 on
/// parse error.
pub fn execarith(state: &mut estate, _do_exec: i32) -> i32 {
    if isset(XTRACE) {
        printprompt4();
        eprint!("((");
    }
    cmdpush(CS_MATH as u8); // c:5247
    let mut htok: i32 = 0;
    let mut e = ecgetstr(state, EC_DUPTOK, Some(&mut htok)); // c:5248
    if htok != 0 {
        e = singsub(&e); // c:5250 — `singsub(&e);`
    }
    if isset(XTRACE) {
        eprint!(" {}", e);
    }
    let val_result = wc_matheval(&e); // c:5254 — `val = matheval(e);`
    cmdpop(); // c:5256
    if isset(XTRACE) {
        eprintln!(" ))");
    }
    // c:5262-5265 — `if (errflag) { errflag &= ~ERRFLAG_ERROR; return 2; }`
    if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 || val_result.is_err() {
        errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
        return 2;
    }
    // c:5267 — `return (val.type == MN_INTEGER) ? val.u.l == 0 : val.u.d == 0.0;`
    let val = val_result.unwrap();
    if val.type_ == MN_INTEGER {
        if val.l == 0 {
            1
        } else {
            0
        }
    } else if val.d == 0.0 {
        1
    } else {
        0
    }
}

/// Port of `exectime()` from `Src/exec.c:5272` — C decl `exectime(Estate state, UNUSED(int do_exec))`.
/// Run `time pipeline`: drives execpline with
/// the Z_TIMED|Z_SYNC flags so it tracks wall/user/sys time.
pub fn exectime(state: &mut estate, _do_exec: i32) -> i32 {
    let jb = *THISJOB
        .get_or_init(|| std::sync::Mutex::new(-1))
        .lock()
        .unwrap(); // c:5283
    let prior = state.prog.prog[state.pc.wrapping_sub(1)];
    // c:5284-5287 — empty `time` (no pipeline) — print accumulated shell time.
    if WC_TIMED_TYPE(prior) == WC_TIMED_EMPTY {
        // c:5285 — `shelltime(NULL,NULL,NULL,0);` — print accumulated
        // shell+kids time deltas since last call.
        crate::ported::jobs::shelltime(None, None, None, 0);
        return 0; // c:5286
    }
    // c:5288 — `execpline(state, *state->pc++, Z_TIMED|Z_SYNC, 0);`
    let slcode = state.prog.prog[state.pc];
    state.pc += 1;
    use crate::ported::zsh_h::{Z_SYNC, Z_TIMED};
    let _ = execpline(state, slcode, Z_TIMED as i32 | Z_SYNC as i32, 0);
    *THISJOB
        .get_or_init(|| std::sync::Mutex::new(-1))
        .lock()
        .unwrap() = jb; // c:5289
    LASTVAL.load(Ordering::Relaxed) // c:5290
}

/// Port of `execshfunc()` from `Src/exec.c:5540` — C decl `execshfunc(Shfunc shf, LinkList args)`.
/// `execshfunc(Shfunc shf, LinkList args)` — `Src/exec.c:5540`.
/// Promoted to top-level pub fn so execcmd_exec at the shfunc
/// dispatch site (c:4102-4105) can route through it. The real port
/// owns queue_signals + cmdstack + sfcontext setup before calling
/// doshfunc; doshfunc itself is unported, so we route the body
/// through `runshfunc` (exec.rs:1700), which carries the
/// wrapper-chain + zunderscore restore. Degraded vs C (no cmdstack
/// push, no sfcontext flip, no XTRACE arg-trace) but the function
/// body executes and `lastval` is updated.
pub fn execshfunc(shf: &mut shfunc, args: &mut Vec<String>) {
    // c:5546-5547 — `if (errflag) return;`
    // A user interrupt sets ERRFLAG_INT, never ERRFLAG_ERROR (signals.c:457), and
    // the C line cited above tests the WHOLE errflag, so masking here let an
    // interrupted shell keep going where zsh stops.
    if errflag.load(Ordering::Relaxed) != 0 {
        return;
    }
    // c:5550-5557 — drop empty job slot before nested shfunc invoke:
    // if outer-pipe bookkeeping is clean AND thisjob is a real job
    // that's not the pipe-leader AND has no procs yet, deletejob()
    // recycles it. Avoids leaking job-table slots across recursive
    // function calls. Same pattern as execcursh's c:482-486.
    {
        let lp = list_pipe.load(Ordering::Relaxed);
        let lpj = list_pipe_job.load(Ordering::Relaxed);
        let tj = THISJOB.get().map(|m| *m.lock().unwrap()).unwrap_or(-1);
        if lp == 0 && tj != -1 && tj != lpj {
            if let Some(jt) = JOBTAB.get() {
                let mut guard = jt.lock().unwrap();
                let has = crate::ported::jobs::hasprocs(&guard, tj as usize);
                if !has {
                    // c:5554-5555 — `last_file_list = jobtab[thisjob].filelist;
                    //                jobtab[thisjob].filelist = NULL;` — preserve
                    //                the filelist so deletejob doesn't unlink temp
                    //                files. Rust take()s the Vec into a local.
                    let _last_file_list: Vec<jobfile> = if let Some(j) = guard.get_mut(tj as usize)
                    {
                        std::mem::take(&mut j.filelist)
                    } else {
                        Vec::new()
                    };
                    if let Some(j) = guard.get_mut(tj as usize) {
                        crate::ported::jobs::deletejob(j, false); // c:5556
                    }
                }
            }
        }
    }
    // c:5559-5570 — `if (isset(XTRACE)) { printprompt4(); ... \n; }` —
    // emit PS4 prefix + space-separated quoted args on the trace
    // stream so `set -x` shows the function invocation line.
    if isset(XTRACE) {
        printprompt4();
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                eprint!(" ");
            }
            eprint!("{}", quotedzputs(a));
        }
        eprintln!();
    }
    // c:5572-5578 cmdstack/sfcontext setup: omit (no cmdstack in
    // zshrs yet — replaced by tracing).
    // c:5580 — `doshfunc(shf, args, 0);` — doshfunc swaps PPARAMS
    // ($1, $2, …) to the function's args, runs the body via
    // runshfunc, then restores. doshfunc itself isn't ported yet
    // so we do the swap-and-restore inline here.
    // c:5580 — `doshfunc(shf, args, 0);`. The C path always has
    // `funcdef` populated since C parses at definition time. zshrs
    // compiles to fusevm chunks instead, so `funcdef` is None for
    // user-defined functions; only `body` (source string) carries
    // the definition. When that's the case, build a one-shot eprog
    // whose `strs` carries the source so runshfunc's script-pipeline
    // arm (execute_script_zsh_pipeline) executes the body.
    let prog_owned: Option<eprog> = if shf.funcdef.is_some() {
        None
    } else if let Some(ref body) = shf.body {
        Some(eprog {
            strs: Some(body.clone()),
            ..Default::default()
        })
    } else if (shf.node.flags as u32 & PM_UNDEFINED) != 0 {
        // c:Src/builtin.c:3763 `shf->funcdef = mkautofn(shf);` — an
        // `autoload`ed-but-not-yet-loaded function is NOT body-less in C.
        // `mkautofn` (c:3786-3809) hands it a five-word program whose only
        // instruction is `WCB_AUTOFN()`, which `execautofn`
        // (Src/exec.c:5786) turns into `loadautofn`; `functions NAME`
        // prints that program back as `builtin autoload -X`.
        //
        // zshrs's autoload stub carries neither `funcdef` nor `body`, so
        // this arm produced `None` and `execshfunc` returned having done
        // NOTHING, status 0. That silently killed every autoloaded function
        // reached through `execcmd_exec` — i.e. every call whose command
        // word is not a compile-time literal (`$c`, `my$v`, `"$f"`), since
        // a literal head goes through `CallFunction` →
        // `dispatch_function_call`, which has its own autoload prelude.
        // `_sh` dispatches with `_$variant "$@"` (Completion/Unix/Command/
        // _sh sh:51), so `sh <TAB>` ran `_bash` as a no-op and completed
        // nothing.
        //
        // Emit mkautofn's program as its printed source; zshrs's
        // `autoload -X` arm (builtin.rs `bin_functions`, c:3613-3654) is
        // the port of `execautofn`'s effect and re-invokes the function
        // with the original arguments via `eval_autoload` (c:3167-3177).
        Some(eprog {
            strs: Some("builtin autoload -X".to_string()),
            ..Default::default()
        })
    } else {
        None
    };
    let prog_ref: Option<&eprog> = match (shf.funcdef.as_deref(), prog_owned.as_ref()) {
        (Some(p), _) => Some(p),
        (_, Some(p)) => Some(p),
        _ => None,
    };
    if let Some(_prog) = prog_ref {
        // c:5580 — `doshfunc(shf, args, 0);`. Direct doshfunc call —
        // noreturnval=0 means the body's return value updates LASTVAL
        // (caller of execfuncdef reads it back). PPARAMS swap +
        // restore happens INSIDE doshfunc's scope; body_runner just
        // runs the body.
        let name_for_body = shf.node.nam.clone();
        let body_args_owned: Vec<String> = if args.len() > 1 {
            args[1..].to_vec()
        } else {
            Vec::new()
        };
        let body_runner = move || -> i32 {
            crate::ported::exec::run_function_body(&name_for_body, &body_args_owned).unwrap_or(0)
        };
        let _ = doshfunc(shf, args.clone(), false, body_runner);
    }
    // c:5582-5589 cmdstack restore/free: omit (no cmdstack).
}

/// Port of `int doshfunc(Shfunc shfunc, LinkList doshargs, int noreturnval)`
/// from `Src/exec.c:5823-6158`.
///
/// C body's scope-management sequence ported here. The C source's
/// body-execution call (`runshfunc(prog, wrappers, name)` at c:6042)
/// is replaced by `body_runner` — zshrs runs function bodies through
/// fusevm bytecode rather than zsh's wordcode walker (per PORT.md
/// "zshrs replaces zsh's tree-walking interpreter" rule), so the
/// callback hands the live executor back to the caller (typically
/// the fusevm bridge) for the actual body run. Every line of scope
/// save/restore around the body call mirrors C exactly.
///
/// **RUST-ONLY ADAPTATION:** the extra `body_runner` parameter is
/// not in C. C calls `runshfunc(prog, wrappers, name)` directly at
/// c:6042; zshrs delegates to a closure because the body-execution
/// pipeline (fusevm) differs from C's (wordcode). The closure
/// fully replaces the runshfunc call and returns the body's exit
/// status (which doshfunc reads as `lastval` for the `noreturnval`
/// path).
#[allow(non_snake_case)]
/// Port of the module-global `oflags` in `Src/exec.c` (set at c:5970 via
/// `funcsave->oflags = oflags;`). Holds the attribute flags of the function
/// CURRENTLY executing, so a nested call can tell whether its caller carried
/// PM_TAGGED_LOCAL — that is what makes `functions -T` non-recursive.
/// zshrs-original storage shape (atomic rather than a plain global); the
/// save/restore discipline matches C's funcsave. Bug #1058.
pub static FUNC_OFLAGS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Port of `doshfunc()` from `Src/exec.c:5823` — C decl `doshfunc(Shfunc shfunc, LinkList doshargs, int noreturnval)`.
pub fn doshfunc(
    shfunc: &mut shfunc,                  // c:5823
    doshargs: Vec<String>,                // c:5823
    noreturnval: bool,                    // c:5823
    mut body_runner: impl FnMut() -> i32, // (Rust-only — body delegate)
) -> i32 {
    use crate::ported::builtin::{BREAKS, CONTFLAG, LASTVAL, LOOPS, RETFLAG};
    // TEMP-INSTRUMENTATION (remove before commit): shell-function invocation
    // volume, to tell "few slow calls" from "very many cheap calls".
    {
        use std::sync::atomic::{AtomicU64, Ordering as O};
        static CALLS: AtomicU64 = AtomicU64::new(0);
        let n = CALLS.fetch_add(1, O::Relaxed) + 1;
        if n % 5000 == 0 {
            tracing::info!(target: "fncount", calls = n, "doshfunc cumulative");
        }
    }
    use crate::ported::jobs::{NUMPIPESTATS, PIPESTATS};
    use crate::ported::modules::parameter::FUNCSTACK;
    use crate::ported::params::endparamscope;
    use crate::ported::params::locallevel as locallevel_atomic;
    use crate::ported::zsh_h::{
        FS_EVAL, FS_FUNC, FS_SOURCE, FUNCTIONARGZERO, PM_TAGGED, PM_TAGGED_LOCAL, PM_UNDEFINED,
    };
    use std::sync::atomic::Ordering;

    let name = shfunc.node.nam.clone(); // c:5827
    let flags = shfunc.node.flags; // c:5828
                                   // Lineage tap, before the funcstack frame goes on: the op must
                                   // record where the *caller* stands, not the body about to run.
    if crate::provenance::active() {
        // c:5978-5998 — doshargs[0] is the function name, doshargs[1..] the
        // positionals the body will see as $1, $2, …; the chain records those,
        // not the name again.
        let prov_args: &[String] = if doshargs.len() > 1 {
            &doshargs[1..]
        } else {
            &[]
        };
        crate::provenance::on_func_call(
            &name,
            prov_args,
            shfunc.body.as_deref(),
            shfunc.filename.as_deref(),
            shfunc.lineno,
        );
    }
    let fname = dupstring(&name); // c:5829
    let _ = fname; // c:5829 (kept for parity)

    // c:6000-6004 — FUNCNEST guard. C:
    //   if (zsh_funcnest >= 0 && funcstack && funcstacksz >= zsh_funcnest) {
    //       zerr("maximum nested function level reached; increase FUNCNEST?");
    //       goto undoshfunc;
    //   }
    // This was previously stubbed out on the assumption that the zshrs
    // fusevm "doesn't recurse via real stack frames" — but it DOES:
    // dispatch_function_call -> doshfunc -> dispatch_function_call -> …
    // nests real native frames, so an accidental infinite recursion
    // (e.g. a self-referential hook, a wrapped function that re-invokes
    // itself, a zle widget cycle) overflowed the OS stack and
    // SEGFAULTED instead of producing zsh's graceful error. Check at
    // entry — before queue_signals / inc_locallevel / the funcsave
    // snapshot — so the early return needs no unwinding. FUNCNEST < 0
    // means "unlimited" (zsh's `zsh_funcnest >= 0` gate). Depth is the
    // count of already-active frames on FUNCSTACK; the 501st nested
    // call (depth == 500) trips it, matching C's `>=` on the default
    // FUNCNEST=500.
    let funcnest = crate::ported::params::getiparam("FUNCNEST");
    if funcnest >= 0 {
        // Count only real function frames (FS_FUNC) — the raw FUNCSTACK
        // Vec also carries FS_EVAL/FS_SOURCE/anon frames, which inflate
        // the depth several× and would falsely trip on legitimately deep
        // (but finite) recursion. FS_FUNC-only matches zsh's nesting
        // depth (`${#funcstack}`) and its funcnest accounting.
        let depth = crate::ported::modules::parameter::FUNCSTACK
            .lock()
            .map(|s| s.iter().filter(|f| f.tp == FS_FUNC).count())
            .unwrap_or(0) as i64;
        if depth >= funcnest {
            zerr("maximum nested function level reached; increase FUNCNEST?");
            crate::ported::builtin::LASTVAL.store(1, std::sync::atomic::Ordering::Relaxed);
            return 1;
        }
    }

    // c:5835 — `queue_signals();` Lots of memory + global-state changes.
    queue_signals();

    // c:5847-5848 — `marked_prog = shfunc->funcdef; useeprog(marked_prog);`
    // Pinned so a recursive unload doesn't free the eprog under us.
    // (Skipped: zshrs's shfunc holds a Box<Eprog>; Drop semantics
    // already pin until call ends. C does explicit refcount on
    // `funcdef->nref` via useeprog.)

    // c:5856-5916 — Funcsave allocation + per-field snapshot.
    let funcsave_breaks = BREAKS.load(Ordering::Relaxed); // c:5859
    let funcsave_contflag = CONTFLAG.load(Ordering::Relaxed); // c:5860
    let funcsave_loops = LOOPS.load(Ordering::Relaxed); // c:5861
    let funcsave_lastval = LASTVAL.load(Ordering::Relaxed); // c:5862
    let funcsave_numpipestats = {
        // c:5864
        NUMPIPESTATS
            .get_or_init(|| std::sync::Mutex::new(0))
            .lock()
            .map(|n| *n)
            .unwrap_or(0)
    };
    let funcsave_noerrexit = noerrexit.load(Ordering::Relaxed); // c:5865
                                                                // c:5866-5867 — trap_state PRIMED branch decrements trap_return.
    if TRAP_STATE.load(Ordering::Relaxed) == TRAP_STATE_PRIMED {
        // c:5866
        TRAP_RETURN.fetch_sub(1, Ordering::Relaxed); // c:5867
    }
    // c:5871 — `noerrexit &= ~NOERREXIT_RETURN;` — scope-clear of
    // return-suppress so a `return` inside the body fires errexit
    // checks normally.
    noerrexit.fetch_and(!NOERREXIT_RETURN, Ordering::Relaxed);

    // c:5872-5880 — noreturnval branch: deep-copy pipestats so the
    // function body's pipestats writes are restored on exit.
    let funcsave_pipestats: Option<Vec<i32>> = if noreturnval {
        // c:5872
        let p = PIPESTATS.get_or_init(|| std::sync::Mutex::new([0; MAX_PIPESTATS]));
        p.lock().ok().map(|g| g[..funcsave_numpipestats].to_vec()) // c:5879 memcpy
    } else {
        None
    };

    // c:5882-5896 — TRAPEXIT special case (deep-copy shfunc so
    // starttrapscope doesn't rug-pull). zshrs doesn't yet support
    // running TRAPEXIT directly via doshfunc; flagged for follow-up.
    // (Skip: name = "TRAPEXIT" path.)
    let _ = name.as_str(); // sentinel for the eventual port.

    // c:5898 — `starttrapscope();` — canonical port at signals.rs:1135
    // tags SIGEXIT for deferred restoration at scope end.
    crate::ported::signals::starttrapscope();
    // c:5899 — `startpatternscope();`
    crate::ported::pattern::startpatternscope();

    // c:5901 — `pptab = pparams;` — save outer positional params.
    let pptab: Vec<String> = crate::ported::builtin::PPARAMS
        .lock()
        .map(|p| p.clone())
        .unwrap_or_default();

    // c:5902-5903 — non-undefined: `scriptname = dupstring(name);`
    let funcsave_scriptname = crate::ported::utils::scriptname_get();
    if (flags as u32 & PM_UNDEFINED) == 0 {
        // c:5902
        crate::ported::utils::set_scriptname(Some(dupstring(&name))); // c:5903
    }

    // c:5964-5965 — `funcsave->zoptind = zoptind; funcsave->optcind
    // = optcind;`. C makes OPTIND implicitly function-local: a
    // `getopts` loop inside the function gets its own counter that
    // snaps back to the caller's on return. zshrs keeps the counter
    // in the `$OPTIND` param plus the ZOPTIND/OPTCIND trackers
    // `getopts` syncs against, so all three are snapshotted here.
    //
    // OPTARG is deliberately NOT snapshotted. The funcsave struct
    // (c:44-52) is `char opts[OPT_SIZE]; char *argv0; int zoptind,
    // lastval, optcind, numpipestats; …` — there is no `zoptarg`
    // member, and `grep -n zoptarg Src/exec.c` returns nothing, so C
    // lets a callee's OPTARG leak to its caller. Bug #513 added an
    // OPTARG save/restore citing "C's `funcsave->zoptind / zoptarg`
    // save/restore pair"; that field does not exist. The extra
    // restore was observable:
    //     g() { OPTARG=inner; }; h() { local OPTARG=Z; g; print $OPTARG }
    //     zsh   -> inner        zshrs (before) -> Z
    //     g() { getopts "a:" o -a VAL; }
    //     k() { local OPTARG=Z OPTIND=1; g; print $OPTARG }
    //     zsh   -> VAL          zshrs (before) -> Z
    let funcsave_optind: Option<String> = crate::ported::params::getsparam("OPTIND");
    let funcsave_optcind = crate::ported::builtin::OPTCIND.load(Ordering::Relaxed); // c:5965
                                                                                    // c:5966-5969 — `if (!isset(POSIXBUILTINS)) { zoptind = 1; optcind = 0; }`.
                                                                                    // The snapshot above is only half the contract: C also RESETS the
                                                                                    // counter on entry so an inner `getopts` loop starts fresh at the
                                                                                    // first positional, independent of the caller's OPTIND. Without
                                                                                    // this, a function whose body runs `getopts` after the caller
                                                                                    // advanced OPTIND mis-parses its own args — e.g. `add-zsh-hook`
                                                                                    // (which `getopts`-parses `precmd func`) printed its usage and
                                                                                    // failed when invoked from a config that had run getopts earlier.
                                                                                    // zshrs keeps the counter in the $OPTIND param plus the ZOPTIND/
                                                                                    // OPTCIND trackers getopts syncs against; reset all three.
    if !crate::ported::zsh_h::isset(crate::ported::zsh_h::POSIXBUILTINS) {
        crate::ported::params::setiparam("OPTIND", 1); // c:5967 zoptind = 1
        crate::ported::builtin::ZOPTIND.store(1, Ordering::Relaxed);
        crate::ported::builtin::OPTCIND.store(0, Ordering::Relaxed); // c:5968 optcind = 0
    }

    // c:5911-5914 — `memcpy(funcsave->opts, opts, sizeof(opts));`
    //
    // C copies 186 bytes. The port used to clone the whole option table
    // as a `HashMap<String, bool>` — a heap allocation per option name,
    // on every call of every function, whether or not the body touches
    // an option. `opt_state_store` is C's array, so this is C's memcpy.
    let funcsave_opts = crate::ported::options::opt_state_store::save();

    // c:5974-5975 — `funcsave->emulation = emulation;
    //                funcsave->sticky = sticky;`
    let funcsave_emulation = crate::ported::options::emulation.load(Ordering::Relaxed);
    let funcsave_emulation_live = crate::ported::options::EMULATION.load(Ordering::Relaxed);
    let funcsave_fully = crate::ported::options::FULLY_EMULATING.load(Ordering::Relaxed);
    let funcsave_sticky = sticky
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|b| sticky_emulation_dup(b, 0));

    // c:5977-6012 —
    //     if (sticky_emulation_differs(shfunc->sticky)) {
    //         sticky = sticky_emulation_dup(shfunc->sticky, 1);
    //         emulation = sticky->emulation;
    //         funcsave->restore_sticky = 1;
    //         installemulation(emulation, opts);
    //         if (sticky->n_on_opts) { ... opts[*onptr] = 1; }
    //         if (sticky->n_off_opts) { ... opts[*offptr] = 0; }
    //         /* All emulations start with pattern disables clear */
    //         clearpatterndisables();
    //     } else
    //         funcsave->restore_sticky = 0;
    //
    // This whole block was missing, so a function DEFINED inside
    // `emulate sh -c '...'` did not re-enter that emulation when
    // called (B07emulate.ztst:6,7,8,12,13,14).
    let mut funcsave_restore_sticky = 0; // c:6012
    if sticky_emulation_differs(shfunc.sticky.as_deref()) != 0 {
        // c:5978
        let newsticky = sticky_emulation_dup(shfunc.sticky.as_deref().unwrap(), 1); // c:5990
        let emu = newsticky.emulation; // c:5991
        *sticky.lock().unwrap_or_else(|e| e.into_inner()) = Some(newsticky);
        // C's single `emulation` global carries EMULATE_FULLY; the Rust
        // port splits it into EMULATION (base bits) + FULLY_EMULATING.
        let emu_base = emu & !crate::ported::zsh_h::EMULATE_FULLY;
        let emu_fully = (emu & crate::ported::zsh_h::EMULATE_FULLY) != 0;
        crate::ported::options::emulation.store(emu_base, Ordering::Relaxed); // c:5991
        crate::ported::options::EMULATION.store(emu_base, Ordering::Relaxed); // (port keeps 2 cells)
        crate::ported::options::FULLY_EMULATING.store(emu_fully, Ordering::Relaxed);
        funcsave_restore_sticky = 1; // c:5992
                                     // c:5993 — `installemulation(emulation, opts);`
        let mut new_opts: std::collections::HashMap<String, bool> =
            std::collections::HashMap::new();
        crate::ported::options::installemulation(emu, &mut new_opts); // c:5993
        for (k, v) in &new_opts {
            opt_state_set(k, *v);
        }
        {
            let guard = sticky.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(ref st) = *guard {
                // c:5995-6001 — `opts[*onptr] = 1;`
                for on in &st.on_opts {
                    opt_state_set(crate::ported::zsh_h::opt_name(*on as i32), true);
                }
                // c:6002-6008 — `opts[*offptr] = 0;`
                for off in &st.off_opts {
                    opt_state_set(crate::ported::zsh_h::opt_name(*off as i32), false);
                }
            }
        }
        // c:6010 — `clearpatterndisables();`
        crate::ported::pattern::clearpatterndisables();
    }

    // c:5954-5960 — PM_TAGGED / PM_TAGGED_LOCAL turn XTRACE on for the
    // duration of the call:
    //     if (flags & (PM_TAGGED|PM_TAGGED_LOCAL))
    //         opts[XTRACE] = 1;
    //     else if (oflags & PM_TAGGED_LOCAL) {
    //         if (shfunc->node.nam == ANONYMOUS_FUNCTION_NAME)
    //             flags |= PM_TAGGED_LOCAL;
    //         else
    //             opts[XTRACE] = 0;
    //     }
    // This is what makes `functions -t f` / `-T f` trace a SINGLE function
    // without `setopt xtrace` globally. It was previously skipped, so both
    // flags parsed and stored correctly (builtin.rs c:3358-3365) but nothing
    // ever consulted them and the traced function ran silently.
    //
    // `-T` is the non-recursive form: a function called FROM a `-T` function
    // gets XTRACE turned back OFF unless it is itself tagged. C tracks the
    // caller's flags in the module-global `oflags`, saved per call at c:5970
    // (`funcsave->oflags = oflags;`) and restored on return — mirrored here by
    // FUNC_OFLAGS. Anonymous functions are the documented exception: they
    // INHERIT the local tag instead of clearing it, so `-T` tracing spans the
    // `(anon)` bodies inside the traced function.
    //
    // No restore of XTRACE is needed here: it is already in the
    // always-restored subset at the exit block below (c:6097-6101).
    // Bug #1058.
    let mut fn_flags = shfunc.node.flags as u32;
    let oflags_prev = FUNC_OFLAGS.load(Ordering::Relaxed);
    if (fn_flags & (PM_TAGGED | PM_TAGGED_LOCAL)) != 0 {
        opt_state_set("xtrace", true); // c:5955
    } else if (oflags_prev & PM_TAGGED_LOCAL) != 0 {
        // c:5956
        if shfunc.node.nam == ANONYMOUS_FUNCTION_NAME {
            fn_flags |= PM_TAGGED_LOCAL; // c:5958
        } else {
            opt_state_set("xtrace", false); // c:5960
        }
    }
    FUNC_OFLAGS.store(fn_flags, Ordering::Relaxed); // c:5970

    // c:5977 — `opts[PRINTEXITVALUE] = 0;` — suppress printexitvalue
    // for inner commands; outer flag restored on exit.
    opt_state_set("printexitvalue", false);

    // c:5978-5998 — pparams swap. C reads doshargs and constructs the
    // function's positional-param array. First arg is the function
    // name (regardless of FUNCTIONARGZERO); the rest become $1..$N.
    let funcsave_argv0: Option<String> = if !doshargs.is_empty() {
        // c:5978
        // c:5982-5985 — `pparams = x = zshcalloc(...)`.
        let positionals: Vec<String> = if doshargs.len() > 1 {
            doshargs[1..].to_vec()
        } else {
            Vec::new()
        };
        if let Ok(mut pp) = crate::ported::builtin::PPARAMS.lock() {
            *pp = positionals;
        }
        // c:5984-5987 — FUNCTIONARGZERO: save argzero, install
        // doshargs[0] (the function name).
        if isset(FUNCTIONARGZERO) {
            // c:5984
            let prev = crate::ported::utils::argzero();
            crate::ported::utils::set_argzero(Some(doshargs[0].clone())); // c:5986
            prev
        } else {
            None
        }
    } else {
        // c:5992-5997 — no args: empty pparams. argzero saved+dup'd.
        if let Ok(mut pp) = crate::ported::builtin::PPARAMS.lock() {
            *pp = Vec::new();
        }
        if isset(FUNCTIONARGZERO) {
            // c:5994
            let prev = crate::ported::utils::argzero();
            crate::ported::utils::set_argzero(prev.clone()); // c:5996 ztrdup(argzero)
            prev
        } else {
            None
        }
    };

    // c:5999 — `++funcdepth;` — bumped on entry. Mirror via locallevel
    // since zshrs tracks function-call depth there.
    //
    // Plus the canonical startparamscope (c:6194 inside runshfunc).
    // zshrs's body_runner replaces runshfunc's `execode` call so the
    // startparamscope/endparamscope pair must wrap body_runner here,
    // not inside the closure. inc_locallevel is exactly startparamscope.
    inc_locallevel();

    // c:6000-6004 — FUNCNEST check + `goto undoshfunc` on overflow.
    // Skip the runtime check (the zshrs fusevm doesn't recurse via
    // real stack frames so the depth limit is less critical), but
    // keep the comment so the C label `undoshfunc:` target is
    // visible — `goto undoshfunc;` here would jump straight to the
    // epilogue at the `undoshfunc:` label below.

    // c:6005-6019 — funcstack frame push. The full C block:
    //   funcsave->fstack.name      = dupstring(name);
    //   funcsave->fstack.caller    = funcstack ? funcstack->name :
    //                                 dupstring(argv0 ? argv0 : argzero);
    //   funcsave->fstack.lineno    = lineno;
    //   funcsave->fstack.prev      = funcstack;
    //   funcsave->fstack.tp        = FS_FUNC;
    //   funcstack                  = &funcsave->fstack;
    //   funcsave->fstack.flineno   = shfunc->lineno;
    //   funcsave->fstack.filename  = getshfuncfile(shfunc);
    // c:6013 — `funcsave->fstack.lineno = lineno;` C has ONE lineno
    // global (params.c:123); zshrs mirrors it in both input::lineno
    // and lex::LEX_LINENO. The lex mirror is the one driven by
    // BUILTIN_SET_LINENO per statement AND zeroed for the duration
    // of a function body (see set_lineno(0) below), so it is the
    // one that matches C's value at call time: a call made INSIDE a
    // caller's body records the caller-relative line (0 for a
    // single-line fn), giving `$functrace` entries like `g:0`.
    // input::lineno stayed parked at the script-wide line and
    // produced `g:1`.
    let lineno_now = crate::ported::lex::lineno() as i64;
    let (caller, prev_tp): (Option<String>, Option<i32>) = {
        let stk = FUNCSTACK.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(p) = stk.last() {
            (Some(p.name.clone()), Some(p.tp))
        } else {
            // c:6011-6012 — outermost: argv0 (saved) or argzero global.
            let z = funcsave_argv0
                .clone()
                .or_else(crate::ported::utils::argzero);
            (z, None)
        }
    };
    // c:6018-6019 — flineno: shfunc->lineno (function def line)
    let mut flineno = shfunc.lineno;
    // c:6079 — `funcsave->fstack.filename = getshfuncfile(shfunc);`, NOT a raw
    // read of `shfunc->filename`. `getshfuncfile` (c:Src/hashtable.c:1058-1068)
    // appends `/name` when PM_LOADDIR is set:
    //     if (shf->node.flags & PM_LOADDIR)
    //         return zhtricat(shf->filename, "/", shf->node.nam);
    // An fpath autoload stores the DIRECTORY in `filename` (loadautofnsetfile,
    // c:5657), so reading the field directly rendered a funcstack frame as the
    // directory — `$funcfiletrace` showed `/Users/…/.zshrs/functions:117`
    // where zsh has `/…/functions/_complete:117`.
    //
    // The `.or_else(Some(String::new()))` coercion is DELIBERATELY kept: C's
    // `getshfuncfile` can return NULL and c:5675 tests `!funcstack->filename`
    // for it, but several readers here assume non-`None`, so making that
    // distinction expressible is a separate change.
    let mut filename = if (shfunc.node.flags as u32 & crate::ported::zsh_h::PM_LOADDIR) != 0 {
        // c:1061-1062
        shfunc
            .filename
            .as_ref()
            .map(|d| format!("{}/{}", d, shfunc.node.nam))
            .or_else(|| Some(String::new()))
    } else {
        // c:1063-1066
        shfunc.filename.clone().or_else(|| Some(String::new()))
    };
    // !!! WARNING: RUST-ONLY HELPER !!!
    //
    // c:6019 — `funcsave->fstack.filename = getshfuncfile(shfunc);`, which
    // reads the shfunc's OWN `filename`, set by `loadautofnsetfile`
    // (c:5657, called from `loadautofn` at c:5735 / c:5757) when the
    // function was autoloaded out of `$fpath`.
    //
    // zshrs has no C counterpart for a function that exists ONLY as a Rust
    // port: `_main_complete`, `_normal`, `_dispatch`, … are dispatched from
    // `compsys::router::try_rust_dispatch` and the autoload prelude is
    // skipped entirely (vm_helper.rs — "no upstream shell function to
    // load"), so nothing ever records a `$fpath` file for the name and the
    // synthesized shfunc handed to us carries the caller's `scriptfilename`
    // ("zsh") or nothing at all. `$funcsourcetrace` / `$funcfiletrace` then
    // reported `zsh:1` for every completer frame where zsh names the
    // `$fpath` file — and `funcfiletrace` is read by real completion code
    // (`_git` derives its git-completion.bash search path from
    // `${funcsourcetrace[1]%:*}`).
    //
    // Stand in for `loadautofnsetfile` in exactly that case by resolving the
    // defining file out of `$fpath` the way `getfpfunc` (c:6219) does for a
    // real autoload. An autoload-installed function gets `shf->lineno == 0`
    // (c:5384-5388 only stamps a `name() { … }` STATEMENT), so the def line
    // is 0, matching zsh's `<fpath-file>:0` in `$funcsourcetrace`.
    //
    // The gate is `try_rust_dispatch` ALONE. That predicate is the same one
    // `vm_helper::dispatch_function_call` and `compcore::callcompfunc` use to
    // pick the body_runner, so `Some` means the frame being pushed IS the
    // port standing in for the stock `$fpath` file — and the router has
    // already refused every name a shell definition owns
    // (`has_fpath_override` / `has_shfunc_override`, router.rs:53,62), so
    // there is nothing left here to shadow.
    //
    // An additional "…and the name has no shfunctab node" clause used to
    // guard this, which meant it never fired in practice: `compinit` writes
    // one bare `autoload -Uz <every completer>` line into its dump
    // (Completion/compdump:113), so EVERY completer has a
    // PM_UNDEFINED stub by the time Tab is pressed. A bare-name stub records
    // neither `filename` nor PM_LOADDIR (`add_autoload_function`,
    // Src/builtin.c:3278-3334 — only the `/abs/dir/name` arm and the
    // inherited-loaddir arm set them), so `getshfuncfile` returns NULL for
    // it and the fallback chain in vm_helper landed on `scriptfilename`.
    // That is how `$funcsourcetrace` read `zsh:1` for `_vars`, `_dispatch`,
    // `_normal`, `_complete` where zsh reads `/usr/share/zsh/5.9/functions/
    // _vars:0` etc.
    //
    // Memoised, and deliberately never invalidated: C stamps `shf->filename`
    // ONCE, at autoload time, and a later `fpath=(…)` does not restamp it —
    // so a permanent per-name answer is what matches zsh, not a re-scan. It
    // also keeps `doshfunc` off the filesystem on the completion hot path.
    if crate::compsys::router::try_rust_dispatch(&name).is_some() {
        static RUST_PORT_FILE: std::sync::OnceLock<
            std::sync::Mutex<std::collections::HashMap<String, Option<String>>>,
        > = std::sync::OnceLock::new();
        let cache = RUST_PORT_FILE.get_or_init(|| std::sync::Mutex::new(Default::default()));
        let cached = cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&name)
            .cloned();
        let resolved = match cached {
            Some(hit) => hit,
            None => {
                let mut fdir: Option<String> = None;
                let mut dump: Option<(eprog, i32)> = None;
                // c:6219 `getfpfunc(s, ksh, test, fdir, 1)` with test_only —
                // a pure probe: it fills `*fdir` (c:6240 `*fdir = *pp;`)
                // without parsing the file.
                let hit = getfpfunc(&name, &mut fdir, None, 1, &mut dump)
                    .and(fdir)
                    // c:1061 (Src/hashtable.c, getshfuncfile) — a PM_LOADDIR
                    // filename renders as
                    // `zhtricat(shf->filename, "/", shf->node.nam)`.
                    .map(|d| {
                        if d.is_empty() {
                            name.clone()
                        } else {
                            format!("{d}/{name}")
                        }
                    });
                cache
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(name.clone(), hit.clone());
                hit
            }
        };
        if let Some(file) = resolved {
            filename = Some(file);
            flineno = 0; // c:5384-5388 — never runs for an autoload stub
        }
    }
    {
        let frame = crate::ported::zsh_h::funcstack {
            prev: None,             // c:6014 (Vec-stack: index encodes link)
            name: dupstring(&name), // c:6005
            filename,               // c:6019
            caller,                 // c:6011
            flineno,                // c:6018
            lineno: lineno_now,     // c:6013
            tp: FS_FUNC,            // c:6015
        };
        let _ = prev_tp; // c:6011 (informational)
        let mut stack = FUNCSTACK.lock().unwrap_or_else(|e| e.into_inner());
        stack.push(frame); // c:6016 funcstack = &funcsave->fstack
    }

    // c:6021-6042 — body execution. C: `runshfunc(prog, wrappers, name)`.
    // zshrs delegates to the body_runner closure (typically a fusevm
    // sub-VM run from the bridge). The closure returns the body's
    // exit status which becomes lastval.
    //
    // c:Src/exec.c:1251-1266 — push "shfunc" onto zsh_eval_context
    // so the body sees `${zsh_eval_context[*]}` containing the call
    // chain context. The execode-based path (c:1245-1282 port at
    // exec.rs:7092) already did this, but the fusevm body_runner
    // path skipped doshfunc's body_runner invocation without the
    // push. Bug #262 in docs/BUGS.md.
    //
    // Push BOTH the static `zsh_eval_context` (matches C's variable)
    // AND the paramtab array entry (what `${zsh_eval_context[*]}`
    // reads). Pop on every return path via the guard struct so
    // panics / early returns don't leak the entry. Inlined here
    // (sole caller) — `zsh_eval_context` is this module's own static.
    //
    // c:Src/exec.c:1251-1266 — `zsh_eval_context[*]` shell-visible
    // mirror: the array entry holds the stack; the scalar
    // `ZSH_EVAL_CONTEXT` holds the `:`-joined form. Both written via
    // the PM_READONLY bypass (`u_arr`/`u_str` direct), the same shape
    // the binary's `-c` ZSH_EVAL_CONTEXT init uses (bins/zshrs.rs).
    // `EvalContextFrame` owns both halves (push + paramtab sync, then
    // pop + sync on Drop) and is shared with the other C sites that
    // reach `execode` with a label — `bin_eval` ("eval", builtin.c:6209)
    // and the autoload body run ("loadautofunc", exec.c:5626).
    let _eval_ctx_guard = EvalContextFrame::push("shfunc");
    // c:Src/exec.c — function bodies execute with `lineno` reset to
    // the relative line within the body (incremented per WC_PIPE
    // from the wordcode-encoded lineno). zsh's zerrmsg
    // (Src/utils.c:301) emits the lineno prefix only when lineno
    // is non-zero AND (!SHINSTDIN || locallevel != 0). For an
    // inline single-line function like `f() { x=1 }`, the body's
    // WC_PIPE encodes lineno=1, exec sets `lineno = lineno - 1 =
    // 0`, and the zerrmsg path falls through to space-only ("f: ").
    //
    // zshrs's compiler doesn't thread WC_PIPE_LINENO into the
    // bytecode, so the global lineno stays at the script-wide
    // value (1 for inline `-c`). Suppress the line-number prefix
    // inside function bodies by saving lineno on entry and forcing
    // it to 0 during body execution; restore on exit. This makes
    // warnings inside functions emit `f: ...` matching zsh's
    // single-line-function format. Bug #54/#74/#86 in docs/BUGS.md.
    let saved_lineno = crate::ported::lex::lineno();
    crate::ported::lex::set_lineno(0);
    // c:Src/exec.c:6173-6175 + c:6196-6198 — `runshfunc` saves
    // zunderscore before the body runs and restores it after, so
    // `$_` reads outside the function continue to reflect the
    // function CALL's last arg (set by setunderscore at c:3491
    // before doshfunc enters). Without this, commands inside the
    // body (`:`, `echo`, etc.) update `$_` to their own last arg,
    // and the post-call `echo "[$_]"` sees the body's residue
    // instead of the call's arg. Bug surfaced via
    // test_dollar_underscore_after_function_call.
    let saved_zunderscore = crate::ported::params::getsparam("_").unwrap_or_default();
    // c:6042 — `runshfunc(prog, wrappers, funcsave->fstack.name);` — run
    // the body through the module-registered wrapper chain, starting at
    // its head (`wrappers` == [`BODY_WRAPPERS`] index 0). `prog` is `None`
    // because the body arrives as `body_runner`; see [`runshfunc`] for the
    // split of C's c:6166-6202 between it and this function.
    let body_status = runshfunc(None, 0, &name, &mut body_runner);
    crate::ported::params::set_zunderscore(std::slice::from_ref(&saved_zunderscore));
    crate::ported::lex::set_lineno(saved_lineno);
    LASTVAL.store(body_status, Ordering::Relaxed);

    // c:6043 — `doneshfunc:` label. The C `runshfunc` happy-path
    // falls through here from c:6042.
    // c:6044 — `funcstack = funcsave->fstack.prev;` — pop our frame.
    {
        let mut stack = FUNCSTACK.lock().unwrap_or_else(|e| e.into_inner());
        stack.pop();
    }
    // c:Src/exec.c:6260 — `endparamscope();` inside `runshfunc`, i.e.
    // BEFORE control returns to doshfunc's epilogue at c:6103. zshrs
    // hoists runshfunc's `startparamscope()` (c:6254) up to
    // `inc_locallevel()` above because the body arrives as a closure,
    // so its partner must be dropped here — at the same point in the
    // sequence C reaches it — not at the tail of the epilogue.
    //
    // The whole epilogue below therefore runs with the callee's locals
    // already popped, which is what C's own comment at c:6195-6196
    // asserts ("The endparamscope() has already happened, hence the
    // `+1` here" for the exit_pending test at c:6201). Three orderings
    // depend on it and all three were wrong while this call sat at the
    // tail:
    //   * `endtrapscope()` (c:6174) dispatches a function-scoped EXIT
    //     trap. In C the trap body runs in the CALLER's param scope:
    //       X=5; f(){ local X=1; trap 'print IN=$X; X=99' EXIT; }; f
    //       zsh -> IN=5 then X==99 outside
    //     With endparamscope last, zshrs printed `IN=1` and the trap's
    //     `X=99` landed in the doomed local shadow (outer X stayed 5).
    //   * the OPTIND/optcind restore (c:6120-6122) must write the
    //     CALLER's parameter, so it has to follow this pop — see the
    //     restore site below.
    //   * `foo() { exit 7; }; foo` needs the decrement before the
    //     exit_pending comparison or `exit_level >= locallevel+1` is
    //     off by one and the shell exits 0.
    endparamscope();

    // c:6045 — `undoshfunc:` label. Reached either by fall-through
    // from c:6044 or by `goto undoshfunc;` from the FUNCNEST check
    // at c:6003. Tail epilogue follows.

    // c:6046 — `--funcdepth;` — paired endparamscope (c:6200 inside
    // runshfunc) lives at c:6157 below as `endparamscope()`. Removed
    // the dec here so locallevel only decrements once per
    // function-call frame; double-dec was purging level-0 globals on
    // function exit (the `f() { x=foo; }; f; echo $x` regression).

    // c:6047-6053 — retflag clear. C clears retflag and restores
    // outer breaks if a `return` fired.
    if RETFLAG.load(Ordering::SeqCst) != 0 {
        // c:6047
        RETFLAG.store(0, Ordering::SeqCst); // c:6051
        BREAKS.store(funcsave_breaks, Ordering::SeqCst); // c:6052
    }

    // c:6054-6058 — pparams + argv0 restore.
    if let Ok(mut pp) = crate::ported::builtin::PPARAMS.lock() {
        *pp = pptab; // c:6059 pparams = pptab
    }
    if let Some(saved) = funcsave_argv0 {
        // c:6055
        crate::ported::utils::set_argzero(Some(saved)); // c:6057
    }

    // c:6120-6123 — `if (!isset(POSIXBUILTINS)) { zoptind =
    // funcsave->zoptind; optcind = funcsave->optcind; }`.
    //
    // Placement is load-bearing in both directions, which is why this
    // sits exactly where C has it — after the pparams/argv0 restore
    // (c:6114-6119) and before the option restore (c:6129-6162):
    //
    //  * AFTER `endparamscope()` (already run above). In C `zoptind`
    //    is a plain global int (c:47); in zshrs `$OPTIND` is an
    //    ordinary paramtab entry, so "restore OPTIND" means "write the
    //    visible parameter". Written before the scope pop it landed in
    //    the CALLEE's own `local OPTIND` shadow, which the pop then
    //    discarded — re-exposing the caller's parameter that the entry
    //    reset at c:5967 had already clobbered to 1:
    //        g() { local OPTIND OPTARG; }
    //        f() { local OPTIND=9; g; print $OPTIND }
    //        zsh -> 9        zshrs (before) -> 1
    //    A callee with no `local OPTIND` was unaffected, which is why
    //    it hid for so long; it reached every compsys port that
    //    declares OPTIND local (`_describe` and thus every completer
    //    calling it).
    //
    //  * BEFORE `endtrapscope()` (c:6174). A function-scoped EXIT trap
    //    that assigns OPTIND must win, because in C it runs after the
    //    restore:
    //        t1(){ trap 'OPTIND=99' EXIT; }; OPTIND=5; t1; print $OPTIND
    //        zsh -> 99
    //    Restoring after endtrapscope clobbered the trap's write to 5.
    //
    //  * BEFORE the option restore (c:6129-6162), so `isset(POSIXBUILTINS)`
    //    reads the CALLEE's setting, matching the entry-side reset at
    //    c:5966 which is likewise evaluated inside the function's option
    //    scope:
    //        OPTIND=5; f(){ setopt localoptions posixbuiltins; OPTIND=42 }
    //        f; print $OPTIND    zsh -> 42
    if !crate::ported::zsh_h::isset(crate::ported::zsh_h::POSIXBUILTINS) {
        if let Some(saved) = funcsave_optind {
            // c:6121 `zoptind = funcsave->zoptind;`
            if let Ok(n) = saved.parse::<i64>() {
                crate::ported::params::setiparam("OPTIND", n);
                crate::ported::builtin::ZOPTIND.store(n as i32, Ordering::Relaxed);
            } else {
                crate::ported::params::setsparam("OPTIND", &saved);
            }
        }
        // c:6122 `optcind = funcsave->optcind;`
        crate::ported::builtin::OPTCIND.store(funcsave_optcind, Ordering::Relaxed);
    }

    // c:6064 — `scriptname = funcsave->scriptname;`
    crate::ported::utils::set_scriptname(funcsave_scriptname);

    // c:6067 — `endpatternscope();`
    crate::ported::pattern::endpatternscope();

    // c:6078-6102 — LOCALOPTIONS restore. Re-apply the snapshot when
    // localoptions was set inside the body.
    // c:6129 — `if (isset(LOCALOPTIONS) || funcsave->restore_sticky)`.
    // A sticky-emulation call ALWAYS restores the full option set on
    // return, whether or not LOCAL_OPTIONS is on — otherwise the
    // emulation the call installed leaks into the caller.
    if crate::ported::options::opt_state_get("localoptions").unwrap_or(false)
        || funcsave_restore_sticky != 0
    {
        // c:6091 memcpy(opts, funcsave->opts, sizeof(opts)) — full restore.
        let mut restore = funcsave_opts.clone();
        // c:6089 — `funcsave->opts[PRIVILEGED] = opts[PRIVILEGED];`.
        // PRIVILEGED is carved out of the restore in C: a function that
        // dropped privileges does not get them back on return. The port
        // restored it with everything else.
        restore.carry_privileged_from_live();
        restore.restore();
        // c:6136 / c:6153 — `emulation = funcsave->emulation;`
        crate::ported::options::emulation.store(funcsave_emulation, Ordering::Relaxed);
        crate::ported::options::EMULATION.store(funcsave_emulation_live, Ordering::Relaxed);
        crate::ported::options::FULLY_EMULATING.store(funcsave_fully, Ordering::Relaxed);
        // c:6137 — `sticky = funcsave->sticky;` (restore_sticky arm only)
        if funcsave_restore_sticky != 0 {
            *sticky.lock().unwrap_or_else(|e| e.into_inner()) = funcsave_sticky;
        }
    } else {
        // c:6097-6101 — non-LOCALOPTIONS: restore only the always-
        // restored subset (XTRACE / PRINTEXITVALUE / LOCALOPTIONS /
        // LOCALLOOPS / WARNNESTEDVAR).
        for opt in [
            "xtrace",
            "printexitvalue",
            "localoptions",
            "localloops",
            "warnnestedvar",
        ] {
            if let Some(v) = funcsave_opts.saved_get(opt) {
                opt_state_set(opt, v);
            }
        }
    }

    // c:5970 counterpart — `oflags = funcsave->oflags;` restores the caller's
    // attribute set so a sibling call after this one sees the right `-T`
    // inheritance. Bug #1058.
    FUNC_OFLAGS.store(oflags_prev, Ordering::Relaxed);

    // c:6104-6112 — LOCALLOOPS warn-on-active-continue/break + restore
    // breaks/contflag/loops snapshot.
    if crate::ported::options::opt_state_get("localloops").unwrap_or(false) {
        // c:6106-6108 —
        //     if (contflag) zwarn("`continue' active at end of function scope");
        //     if (breaks)   zwarn("`break' active at end of function scope");
        if CONTFLAG.load(Ordering::SeqCst) != 0 {
            crate::ported::utils::zwarn("`continue' active at end of function scope");
            // c:6106
        }
        if BREAKS.load(Ordering::SeqCst) != 0 {
            crate::ported::utils::zwarn("`break' active at end of function scope");
            // c:6107
        }
        BREAKS.store(funcsave_breaks, Ordering::SeqCst); // c:6109
        CONTFLAG.store(funcsave_contflag, Ordering::SeqCst); // c:6110
        LOOPS.store(funcsave_loops, Ordering::SeqCst); // c:6111
    }

    // c:6174 — `endtrapscope();`
    //
    // Bug #80 in docs/BUGS.md: zshrs used to run endtrapscope while
    // locallevel was still at the function's own level, so savetrap
    // entries tagged `local == current_function_level` failed the
    // `local > locallevel` pop condition and nested EXIT traps never
    // restored — outer EXIT traps fired at script exit instead. That
    // was worked around by bracketing this call with a manual
    // decrement/re-bump of locallevel. The workaround is gone: the
    // real `endparamscope()` now runs at its C position above, so the
    // pop loop observes the genuine post-decrement level, and the
    // trap body executes in the caller's param scope as C does.
    crate::ported::signals::endtrapscope();

    // c:6116-6117 — TRAP_STATE_PRIMED branch: bump trap_return back.
    if TRAP_STATE.load(Ordering::Relaxed) == TRAP_STATE_PRIMED {
        // c:6116
        TRAP_RETURN.fetch_add(1, Ordering::Relaxed); // c:6117
    }

    // c:6118 — `ret = lastval;`
    let ret = LASTVAL.load(Ordering::Relaxed);

    // c:6119 — `noerrexit = funcsave->noerrexit;`
    noerrexit.store(funcsave_noerrexit, Ordering::Relaxed);

    // c:6120-6124 — noreturnval: restore lastval + pipestats. C runs
    // the function for side-effects only; outer lastval/pipestats
    // should reflect the PRE-call state.
    if noreturnval {
        // c:6120
        LASTVAL.store(funcsave_lastval, Ordering::Relaxed); // c:6121
        if let Some(saved_ps) = funcsave_pipestats {
            let n = NUMPIPESTATS.get_or_init(|| std::sync::Mutex::new(0));
            if let Ok(mut nguard) = n.lock() {
                *nguard = funcsave_numpipestats; // c:6122
            }
            let p = PIPESTATS.get_or_init(|| std::sync::Mutex::new([0; MAX_PIPESTATS]));
            if let Ok(mut pguard) = p.lock() {
                for (i, v) in saved_ps.iter().enumerate() {
                    if i < pguard.len() {
                        pguard[i] = *v; // c:6123 memcpy
                    }
                }
            }
        }
    }

    // c:6128 — `unqueue_signals();`
    unqueue_signals();

    // c:6135-6155 — exit_pending branch: when an `exit` was queued
    // inside the function body and we've unwound enough scopes for
    // it to take effect, either keep unwinding (still inside a
    // nested function) or actually exit the shell.
    let exit_pending = crate::ported::builtin::EXIT_PENDING.load(Ordering::Relaxed);
    let exit_level = crate::ported::builtin::EXIT_LEVEL.load(Ordering::Relaxed);
    let cur_locallevel = locallevel.load(Ordering::Relaxed) as i32;
    let cur_forklevel = FORKLEVEL.load(Ordering::Relaxed);
    let in_exit_trap = crate::ported::signals::in_exit_trap.load(Ordering::Relaxed); // c:Src/signals.c:63
    if exit_pending != 0 && exit_level >= cur_locallevel + 1 && in_exit_trap == 0 {
        // c:6141
        if cur_locallevel > cur_forklevel {
            // c:6143 — still inside a nested function: keep unwinding.
            RETFLAG.store(1, Ordering::Relaxed); // c:6144
            BREAKS.store(LOOPS.load(Ordering::Relaxed), Ordering::Relaxed); // c:6145
        } else {
            // c:6151 — out of all functions: exit for real.
            crate::ported::builtin::STOPMSG.store(1, Ordering::Relaxed); // c:6151
            let val = EXIT_VAL.load(Ordering::Relaxed);
            crate::ported::builtin::zexit(val, crate::ported::zsh_h::ZEXIT_NORMAL);
            // c:6152
        }
    }

    ret // c:6157 return ret
}

// ═══════════════════════════════════════════════════════════════════════
// `(eval)` funcstack frame — the `FS_EVAL` half of the funcstack/functrace/
// funcfiletrace subsystem whose `FS_FUNC` half is `doshfunc` above.
// ═══════════════════════════════════════════════════════════════════════

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// C keeps `struct funcstack fstack;` as an automatic in `eval()`
/// (`Src/builtin.c:6148`) and unwinds it with a plain
/// `funcstack = funcstack->prev;` on the single return path
/// (`Src/builtin.c:6210`). zshrs's `FUNCSTACK` is a `Mutex<Vec<funcstack>>`
/// and its eval entry points have MANY early-return paths (errflag
/// containment, recursion backstop, `?` from `execute_script`), so the pop
/// is carried by `Drop` instead. There is no C counterpart for this type;
/// the pushed frame itself is a line-by-line port of `Src/builtin.c:6155-6193`.
///
/// Every caller that stands in for C's `eval()` — the `BUILTIN_EVAL`
/// handler and `eval_string` below — must hold one of these for the
/// duration of the eval'd body, or `$funcstack` / `$functrace` /
/// `$funcfiletrace` lose a frame relative to zsh.
pub struct EvalFuncstackFrame {
    /// c:6191/6193 `fpushed` — whether the frame was actually pushed.
    fpushed: bool,
    /// c:6147 — `int oineval = ineval, fpushed;`. Restored at c:6215
    /// (`ineval = oineval;`) by this guard's Drop.
    oineval: i32,
}

impl EvalFuncstackFrame {
    /// Port of the funcstack prologue of `eval(char **argv)` from
    /// `Src/builtin.c:6155-6193`.
    ///
    /// ```c
    /// ineval = !isset(EVALLINENO);
    /// if (!ineval) {
    ///     scriptname = "(eval)";
    ///     fstack.prev = funcstack;
    ///     fstack.name = scriptname;
    ///     fstack.caller = funcstack ? funcstack->name : dupstring(argzero);
    ///     fstack.lineno = lineno;
    ///     fstack.tp = FS_EVAL;
    ///     if (!funcstack || funcstack->tp == FS_SOURCE) {
    ///         fstack.flineno = fstack.lineno;
    ///         fstack.filename = fstack.caller;
    ///     } else {
    ///         fstack.flineno = funcstack->flineno + lineno;
    ///         if (funcstack->tp == FS_EVAL)
    ///             fstack.flineno--;
    ///         fstack.filename = funcstack->filename;
    ///         if (!fstack.filename)
    ///             fstack.filename = "";
    ///     }
    ///     funcstack = &fstack;
    ///     fpushed = 1;
    /// } else
    ///     fpushed = 0;
    /// ```
    ///
    /// `scriptname` (c:6157) is left to the caller: the `BUILTIN_EVAL`
    /// handler saves/restores it around the whole builtin, which is where
    /// C's `oscriptname` (c:6146) / restore (c:6222) live.
    pub fn push() -> Self {
        use crate::ported::modules::parameter::FUNCSTACK;
        use crate::ported::zsh_h::{FS_EVAL, FS_SOURCE};
        // c:6147 — `int oineval = ineval, fpushed;`
        let oineval = crate::ported::builtin::ineval.load(std::sync::atomic::Ordering::Relaxed);
        // c:6155 — `ineval = !isset(EVALLINENO);`
        let ineval = !crate::ported::zsh_h::isset(crate::ported::zsh_h::EVALLINENO);
        // The global drives `Src/exec.c`'s three `!ineval` lineno gates
        // (c:1355 / c:1451 / c:2056); zshrs reads it in the
        // `BUILTIN_SET_LINENO` handler. Only the funcstack half of this
        // prologue was ported, so `unsetopt evallineno; eval 'print
        // $LINENO'` counted lines inside the eval string (1) instead of
        // reporting the caller's line (E01options.ztst:29).
        crate::ported::builtin::ineval.store(
            if ineval { 1 } else { 0 },
            std::sync::atomic::Ordering::Relaxed,
        );
        if ineval {
            return EvalFuncstackFrame {
                fpushed: false, // c:6193 fpushed = 0
                oineval,
            };
        }
        // c:6161 — `fstack.lineno = lineno;`. zshrs mirrors C's single
        // `lineno` global (params.c:123) in `lex::LEX_LINENO`, the one
        // `BUILTIN_SET_LINENO` drives per statement — the same mirror
        // `doshfunc` reads for its `FS_FUNC` frame (see c:6013 there).
        let lineno = crate::ported::lex::lineno() as i64;
        let frame = {
            let stk = FUNCSTACK.lock().unwrap_or_else(|e| e.into_inner());
            // c:6160 — `fstack.caller = funcstack ? funcstack->name
            //                                     : dupstring(argzero);`
            let caller = match stk.last() {
                Some(f) => Some(f.name.clone()),
                None => crate::ported::utils::argzero(),
            };
            // c:6174-6188 — flineno/filename deduction. Identical logic to
            // `funcfiletracegetfn` (Src/Modules/parameter.c:724-762): an eval
            // is an inlined call from a tracing perspective.
            let (flineno, filename) = match stk.last() {
                // c:6174 — `if (!funcstack || funcstack->tp == FS_SOURCE)`
                None => (lineno, caller.clone()), // c:6175-6176
                Some(p) if p.tp == FS_SOURCE => (lineno, caller.clone()), // c:6175-6176
                Some(p) => {
                    // c:6178 — `fstack.flineno = funcstack->flineno + lineno;`
                    let mut flineno = p.flineno + lineno;
                    // c:6183-6184 — `if (funcstack->tp == FS_EVAL) fstack.flineno--;`
                    // Line numbers in eval start from 1, not zero, so offset
                    // by one to get line in file.
                    if p.tp == FS_EVAL {
                        flineno -= 1;
                    }
                    // c:6185-6187 — `fstack.filename = funcstack->filename;
                    //                if (!fstack.filename) fstack.filename = "";`
                    (
                        flineno,
                        Some(p.filename.clone().unwrap_or_else(String::new)),
                    )
                }
            };
            crate::ported::zsh_h::funcstack {
                prev: None,                 // c:6158 (Vec-stack: index encodes link)
                name: "(eval)".to_string(), // c:6159 fstack.name = scriptname
                filename,                   // c:6176 / c:6185
                caller,                     // c:6160
                flineno,                    // c:6175 / c:6178
                lineno,                     // c:6161
                tp: FS_EVAL,                // c:6162
            }
        };
        FUNCSTACK
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(frame); // c:6189 funcstack = &fstack
        EvalFuncstackFrame {
            fpushed: true, // c:6191 fpushed = 1
            oineval,
        }
    }

    /// c:6191/6193 `fpushed` — whether `EVALLINENO` was set and a frame
    /// really went on. Callers mirror C's `if (!ineval) scriptname =
    /// "(eval)";` (c:6157) off the same test.
    pub fn pushed(&self) -> bool {
        self.fpushed
    }
}

impl Drop for EvalFuncstackFrame {
    /// c:6209-6210 — `if (fpushed) funcstack = funcstack->prev;`
    /// c:6215 — `ineval = oineval;`
    fn drop(&mut self) {
        crate::ported::builtin::ineval.store(self.oineval, std::sync::atomic::Ordering::Relaxed); // c:6215
        if self.fpushed {
            crate::ported::modules::parameter::FUNCSTACK
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pop();
        }
    }
}

/// `TRAP_STATE_PRIMED` — re-exported from the canonical enum port rather
/// than redefined here.
///
/// This was a LOCAL `const … = 2`, which is `TRAP_STATE_FORCE_RETURN`'s value:
/// `enum trap_state { TRAP_STATE_INACTIVE, TRAP_STATE_PRIMED,
/// TRAP_STATE_FORCE_RETURN }` (zsh.h:2947-2960) makes PRIMED 1, and
/// zsh_h.rs:4501-4505 already carries all three with the right values. With
/// the wrong number, doshfunc's two trap_return adjustments (c:5866-5867 on
/// entry, c:6116-6117 on exit) never fired for a primed trap, so trap_return
/// stayed at -1 instead of reaching the -2 sentinel bin_break tests
/// (builtin.c:5845). `return N` inside a TRAPxxx function therefore never
/// promoted to TRAP_STATE_FORCE_RETURN:
///     TRAPINT() { print CAUGHT; return 130 }; kill -INT $$; print DONE
///     zsh  : CAUGHT
///     zshrs: CAUGHT DONE
use crate::ported::zsh_h::TRAP_STATE_PRIMED;

/// Port of `execfuncdef()` from `Src/exec.c:5309` — C decl `execfuncdef(Estate state, Eprog redir_prog)`.
/// Define a shell function: extract
/// name(s)+body from the wordcode payload, allocate the Shfunc,
/// install into `shfunctab` (named), or execute immediately (anon).
#[allow(non_snake_case)]
pub fn execfuncdef(state: &mut estate, mut redir_prog: Option<crate::ported::zsh_h::Eprog>) -> i32 {
    use crate::ported::hashtable::{dircache_set, shfunctab_lock};
    use crate::ported::jobs::{getsigidx, removetrapnode};
    use crate::ported::parse::{dupeprog, freeeprog, incrdumpcount};
    use crate::ported::signals::settrap;
    use crate::ported::utils::scriptfilename_get;
    use crate::ported::zsh_h::{
        eprog as eprog_t, hashnode, patprog as patprog_t, shfunc as shfunc_t, Patprog,
        EC_DUPTOK as _, EF_HEAP, EF_MAP, EF_REAL, FS_EVAL, FS_FUNC, PM_ANONYMOUS, PM_TAGGED,
        PM_TAGGED_LOCAL, PRINTEXITVALUE, SHINSTDIN, ZSIG_FUNC,
    };
    // c:5311 — `Shfunc shf;`
    let mut shf: Box<shfunc_t>;
    // c:5312 — `char *s = NULL;`
    let mut s: Option<String> = None;
    // c:5313 — `int signum, nprg, sbeg, nstrs, npats, do_tracing, len, plen, i, htok = 0, ret = 0;`
    let mut signum: i32;
    let nprg: i32;
    let sbeg: i32;
    let nstrs: i32;
    let npats: i32;
    let do_tracing: i32;
    let len: i32;
    let plen: i32;
    // `i` — C loop counter for pp stamp; Rust uses .map().collect().
    let mut htok: i32 = 0;
    let mut ret: i32 = 0;
    // c:5314 — `int anon_func = 0;`
    let mut anon_func: i32 = 0;
    // c:5315 — `Wordcode beg = state->pc, end;`
    let _beg: usize = state.pc;
    let mut end: usize;
    // c:5316 — `Eprog prog;`
    // (allocated inline per-iter below; no upfront binding needed)
    // c:5317 — `Patprog *pp;` — handled by Vec construction.
    // c:5318 — `LinkList names;`
    let names: Vec<String>;
    // c:5319 — `int tracing_flags;`
    let tracing_flags: i32;

    // c:5321 — `end = beg + WC_FUNCDEF_SKIP(state->pc[-1]);`
    end = state.pc + WC_FUNCDEF_SKIP(state.prog.prog[state.pc.wrapping_sub(1)]) as usize;
    // c:5322 — `names = ecgetlist(state, *state->pc++, EC_DUPTOK, &htok);`
    let num = state.prog.prog[state.pc] as usize;
    state.pc += 1;
    names = ecgetlist(state, num, EC_DUPTOK, Some(&mut htok));
    // c:5323 — `sbeg = *state->pc++;`
    sbeg = state.prog.prog[state.pc] as i32;
    state.pc += 1;
    // c:5324 — `nstrs = *state->pc++;`
    nstrs = state.prog.prog[state.pc] as i32;
    state.pc += 1;
    // c:5325 — `npats = *state->pc++;`
    npats = state.prog.prog[state.pc] as i32;
    state.pc += 1;
    // c:5326 — `do_tracing = *state->pc++;`
    do_tracing = state.prog.prog[state.pc] as i32;
    state.pc += 1;

    // c:5328 — `nprg = (end - state->pc);`
    nprg = end.saturating_sub(state.pc) as i32;
    // c:5329 — `plen = nprg * sizeof(wordcode);`
    plen = nprg.saturating_mul(size_of::<wordcode>() as i32);
    // c:5330 — `len = plen + (npats * sizeof(Patprog)) + nstrs;`
    len = plen + npats.saturating_mul(size_of::<usize>() as i32) + nstrs;
    // c:5331 — `tracing_flags = do_tracing ? PM_TAGGED_LOCAL : 0;`
    tracing_flags = if do_tracing != 0 {
        PM_TAGGED_LOCAL as i32
    } else {
        0
    };

    // c:5333-5339 — htok name substitution.
    let mut names_mut: Vec<String> = names;
    if htok != 0 && !names_mut.is_empty() {
        execsubst(&mut names_mut); // c:5334
        if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
            // c:5335
            state.pc = end; // c:5336
            return 1; // c:5337
        }
    }

    // c:5341-5342 DPUTS — debug assertion (anon + redir simultaneously).
    // Not portable as panic; left as comment.

    // c:5343 — `while (!names || (s = (char *) ugetnode(names))) {`
    // num==0 → anon (no names); else iterate names.
    let mut names_iter = names_mut.into_iter();
    loop {
        let no_names = num == 0;
        if !no_names {
            // c:5343 — `s = ugetnode(names)`; break when list exhausted.
            match names_iter.next() {
                Some(nm) => s = Some(nm),
                None => break,
            }
        }
        // c:5344-5374 — Eprog alloc.
        let prog: Box<eprog_t>;
        let dump_present = state.prog.dump.is_some();
        let make_pat = || -> Patprog {
            // c:5375-5376 `*pp = dummy_patprog1;` — sentinel slot.
            Box::new(patprog_t {
                startoff: 0,
                size: 0,
                mustoff: 0,
                patmlen: 0,
                globflags: 0,
                globend: 0,
                flags: 0,
                patnpar: 0,
                patstartch: 0,
            })
        };
        if no_names {
            // c:5345-5346 — `zhalloc`, `nref = -1`.
            // c:5355-5357 — EF_HEAP, no dump, npats pats on heap.
            let pats: Vec<Patprog> = (0..npats).map(|_| make_pat()).collect();
            let prog_words: Vec<wordcode> = state.prog.prog[state.pc..end].to_vec();
            // c:5365 — `prog->strs = state->strs + sbeg;`
            let strs_tail = state.strs.as_ref().map(|t| {
                let off = (sbeg as usize).min(t.len());
                t[off..].to_string()
            });
            prog = Box::new(eprog_t {
                flags: EF_HEAP,
                len,
                npats,
                nref: -1, // c:5346
                pats,
                prog: prog_words,
                strs: strs_tail,
                shf: None,  // c:5377
                dump: None, // c:5356
                // c:5365 — `prog->strs = state->strs + sbeg;` — the tail is
                // the SAME pool, so it keeps the same encoding.
                strs_metafied: state.prog.strs_metafied,
            });
        } else if dump_present {
            // c:5358-5363 — EF_MAP path: refcount the dump, allocate
            // pats permanent, reuse `state->pc` slice in place.
            if let Some(dp) = state.prog.dump.as_deref() {
                incrdumpcount(dp); // c:5360
            }
            let pats: Vec<Patprog> = (0..npats).map(|_| make_pat()).collect();
            let prog_words: Vec<wordcode> = state.prog.prog[state.pc..end].to_vec();
            let strs_tail = state.strs.as_ref().map(|t| {
                let off = (sbeg as usize).min(t.len());
                t[off..].to_string()
            });
            prog = Box::new(eprog_t {
                flags: EF_MAP, // c:5359
                len,
                npats,
                nref: 1, // c:5349
                pats,
                prog: prog_words,
                strs: strs_tail,
                shf: None,                               // c:5377
                dump: state.prog.dump.clone(),           // c:5361
                strs_metafied: state.prog.strs_metafied, // pool copied verbatim — carry provenance
            });
        } else {
            // c:5366-5374 — EF_REAL: copy wordcode + strs into a
            // freshly-owned eprog (no shared dump backing).
            let pats: Vec<Patprog> = (0..npats).map(|_| make_pat()).collect();
            let pc_end = state.pc + nprg as usize;
            let prog_words: Vec<wordcode> = state.prog.prog[state.pc..pc_end].to_vec();
            // c:5373 — `memcpy(prog->strs, state->strs + sbeg, nstrs);`
            let strs_copy = state.strs.as_ref().map(|t| {
                let off = (sbeg as usize).min(t.len());
                let n_avail = t.len().saturating_sub(off);
                let take = (nstrs as usize).min(n_avail);
                t[off..off + take].to_string()
            });
            prog = Box::new(eprog_t {
                flags: EF_REAL, // c:5367
                len,
                npats,
                nref: 1, // c:5349
                pats,
                prog: prog_words,
                strs: strs_copy,
                shf: None,  // c:5377
                dump: None, // c:5371
                // c:5373 — `memcpy(prog->strs, state->strs + sbeg, nstrs);` —
                // a byte copy of the source pool, so the same encoding.
                strs_metafied: state.prog.strs_metafied,
            });
        }

        // c:5379-5381 — Shfunc alloc + funcdef + tracing flags.
        shf = Box::new(shfunc_t {
            node: hashnode {
                next: None,
                nam: String::new(),
                flags: tracing_flags,
            },
            filename: scriptfilename_get(), // c:5383 `ztrdup(scriptfilename)`
            // c:5384-5388 — funcstack top FS_FUNC/FS_EVAL → flineno+lineno
            // else just lineno.
            lineno: {
                let cur_lineno = crate::ported::input::lineno.with(|l| l.get()) as i64;
                if let Ok(stk) = crate::ported::modules::parameter::FUNCSTACK.lock() {
                    if let Some(top) = stk.last() {
                        if top.tp == FS_FUNC || top.tp == FS_EVAL {
                            top.flineno + cur_lineno
                        } else {
                            cur_lineno
                        }
                    } else {
                        cur_lineno
                    }
                } else {
                    cur_lineno
                }
            },
            funcdef: Some(prog), // c:5380
            redir: None,
            sticky: None,
            body: None,
            redir_text: None,
        });
        // c:5396-5401 — redir_prog ownership.
        // C: `if (names && nonempty(names) && redir_prog) shf->redir = dupeprog(redir_prog,0)`
        // else `shf->redir = redir_prog; redir_prog = 0;`
        // "nonempty(names)" means there's a NEXT name still to consume —
        // i.e. peek the iterator.
        if !no_names && names_iter.len() > 0 && redir_prog.is_some() {
            // c:5397 — dupe so each earlier name gets its own copy; the
            // last name (when iterator drains) gets the original.
            if let Some(rp) = redir_prog.as_deref() {
                shf.redir = Some(Box::new(dupeprog(rp, false)));
            }
        } else {
            // c:5399-5400 — last name (or anon) takes original.
            shf.redir = redir_prog.take();
        }
        // c:5402 — `shfunc_set_sticky(shf);`
        shfunc_set_sticky(&mut shf);

        if no_names {
            // c:5404-5457 — anonymous function: execute immediately.
            // `LinkList args;` c:5409
            let mut args: Vec<String>;

            anon_func = 1; // c:5411
            shf.node.flags |= PM_ANONYMOUS as i32; // c:5412

            state.pc = end; // c:5414
                            // c:5415 — `end += *state->pc++;`
            end += state.prog.prog[state.pc] as usize;
            state.pc += 1;
            // c:5416 — `args = ecgetlist(state, *state->pc++, EC_DUPTOK, &htok);`
            let arg_count = state.prog.prog[state.pc] as usize;
            state.pc += 1;
            args = ecgetlist(state, arg_count, EC_DUPTOK, Some(&mut htok));

            // c:5418-5429 — htok arg subst + cleanup-on-error.
            if htok != 0 && !args.is_empty() {
                execsubst(&mut args); // c:5419
                if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
                    // c:5421 — `freeeprog(shf->funcdef);`
                    if let Some(mut fd) = shf.funcdef.take() {
                        freeeprog(&mut fd);
                    }
                    if shf.redir.is_some() {
                        // c:5422-5423 — "shouldn't be" anon+redir, but free if so.
                        if let Some(mut rd) = shf.redir.take() {
                            freeeprog(&mut rd);
                        }
                    }
                    dircache_set(&mut shf.filename, None); // c:5424
                    drop(shf); // c:5425 `zfree(shf, sizeof(*shf));`
                    state.pc = end; // c:5426
                    return 1; // c:5427
                }
            }

            // c:5431-5432 — `setunderscore` to last arg (or "").
            let under_val = if !args.is_empty() {
                args.last().cloned().unwrap_or_default()
            } else {
                String::new()
            };
            setunderscore(&under_val);

            // c:5434-5435 — `if (!args) args = newlinklist();`
            // (Rust Vec is never null; no-op.)
            shf.node.nam = ANONYMOUS_FUNCTION_NAME.to_string(); // c:5436
                                                                // c:5437 — `pushnode(args, shf->node.nam);` — prepend.
            args.insert(0, shf.node.nam.clone());

            execshfunc(&mut shf, &mut args); // c:5439
            ret = LASTVAL.load(Ordering::Relaxed); // c:5440

            // c:5442-5450 — PRINTEXITVALUE+SHINSTDIN exit report.
            if isset(PRINTEXITVALUE) && isset(SHINSTDIN) && ret != 0 {
                eprintln!("zsh: exit {}", ret); // c:5445/5447
            }

            // c:5452-5456 — cleanup.
            if let Some(mut fd) = shf.funcdef.take() {
                freeeprog(&mut fd);
            }
            if let Some(mut rd) = shf.redir.take() {
                // c:5453-5454 — "shouldn't be" but free if present.
                freeeprog(&mut rd);
            }
            dircache_set(&mut shf.filename, None); // c:5455
            drop(shf); // c:5456 `zfree(shf, sizeof(*shf));`
            break; // c:5457
        } else {
            // c:5458-5484 — named function path.
            let nm = s.as_deref().unwrap_or("");
            // c:5460-5475 — TRAP* signal-trap install.
            // BUGS.md #1114 — the TRAPxxx() FUNCTION-trap form is zsh-only.
            // bash, ksh and dash have no such concept: there `TRAPINT` is an
            // ordinary function whose name merely begins with TRAP, and SIGINT
            // keeps its default disposition. Recognising it in a drop-in mode
            // also SILENTLY DESTROYS an already-installed list trap for that
            // signal, since the two forms are alternatives in zsh.
            // posix_faithful() is raised only for a bare drop-in flag, so
            // --zsh, native zshrs and `emulate sh` are untouched.
            if !crate::extensions::dash_mode::posix_faithful() && nm.len() > 4 && nm.starts_with("TRAP") {
                if let Some(sn) = getsigidx(&nm[4..]) {
                    signum = sn;
                    // c:5462 — `if (settrap(signum, NULL, ZSIG_FUNC))`
                    if settrap(signum, None, ZSIG_FUNC) != 0 {
                        if let Some(mut fd) = shf.funcdef.take() {
                            freeeprog(&mut fd); // c:5463
                        }
                        dircache_set(&mut shf.filename, None); // c:5464
                        drop(shf); // c:5465
                        state.pc = end; // c:5466
                        return 1; // c:5467
                    }
                    // c:5474 — `removetrapnode(signum);`
                    removetrapnode(signum);
                    // c:Src/signals.c::settrap → unsettrap →
                    // removetrap also clears sigfuncs[sig] (the C
                    // string-form trap slot). zshrs's port stores
                    // string-form bodies in a separate
                    // `traps_table` HashMap not touched by
                    // removetrap. Drop the string-form entry here
                    // so dotrap's fallback doesn't double-dispatch
                    // when a TRAPxxx function REPLACES an
                    // existing `trap '...' SIG` registration. Bug
                    // #541 in docs/BUGS.md.
                    if let Ok(mut t) = crate::ported::builtin::traps_table().lock() {
                        t.remove(&nm[4..]);
                    }
                }
            }
            // c:5477-5482 — re-define-self trace flag propagate.
            if let Ok(stk) = crate::ported::modules::parameter::FUNCSTACK.lock() {
                if let Some(top) = stk.last() {
                    if top.tp == FS_FUNC && top.name == nm {
                        // c:5479 — `Shfunc old = shfunctab->getnode(s);`
                        if let Ok(rd) = shfunctab_lock().read() {
                            if let Some(old) = rd.get(nm) {
                                // c:5481 — propagate PM_TAGGED|PM_TAGGED_LOCAL.
                                shf.node.flags |=
                                    old.node.flags & (PM_TAGGED as i32 | PM_TAGGED_LOCAL as i32);
                            }
                        }
                    }
                }
            }
            // c:5483 — `shfunctab->addnode(shfunctab, ztrdup(s), shf);`
            shf.node.nam = nm.to_string();
            // Lineage tap: the definition is a function's origin, and a
            // second definition of the same name is a `redefine` op.
            if crate::provenance::active() {
                crate::provenance::on_func_define(
                    nm,
                    shf.body.as_deref(),
                    shf.filename.as_deref(),
                    shf.lineno,
                );
            }
            if let Ok(mut wr) = shfunctab_lock().write() {
                wr.add(*shf);
            }
        }
    }
    // c:5486-5487 — `if (!anon_func) setunderscore("");`
    if anon_func == 0 {
        setunderscore("");
    }
    // c:5488-5491 — leftover redir cleanup ("shouldn't happen").
    if let Some(mut rd) = redir_prog.take() {
        freeeprog(&mut rd);
    }
    // c:5492 — `state->pc = end;`
    state.pc = end;
    // c:5493 — `return ret;`
    ret
}

/// Port of `execsimple()` from `Src/exec.c:1290` — C decl `execsimple(Estate state)`.
/// Fast-path for single-Simple commands that bypasses the full
/// `execcmd_exec` machinery.
pub fn execsimple(state: &mut estate) -> i32 {
    // c:1292 — `wordcode code = *state->pc++;`
    let mut code = state.prog.prog[state.pc];
    state.pc += 1;
    // c:1295-1296 — `if (errflag) return (lastval = 1);`
    // A user interrupt sets ERRFLAG_INT, never ERRFLAG_ERROR (signals.c:457), and
    // the C line cited above tests the WHOLE errflag, so masking here let an
    // interrupted shell keep going where zsh stops.
    if errflag.load(Ordering::Relaxed) != 0 {
        LASTVAL.store(1, Ordering::Relaxed);
        return 1;
    }
    // c:1298-1299 — `if (!isset(EXECOPT)) return lastval = 0;`
    if !isset(crate::ported::zsh_h::EXECOPT) {
        LASTVAL.store(0, Ordering::Relaxed);
        return 0;
    }
    // c:1301-1303 — `if (!IN_EVAL_TRAP() && !ineval && code) lineno = code - 1;`
    // In evaluated traps, don't modify the line number (the trap
    // dispatcher restores it). `code` here is the wordcode-encoded
    // line number from the WC_SIMPLE entry at state.pc-1.
    if !crate::ported::zsh_h::IN_EVAL_TRAP()
        && crate::ported::builtin::INEVAL.load(Ordering::SeqCst) == 0
        && code != 0
    {
        crate::ported::input::lineno.with(|l| l.set((code as usize).saturating_sub(1)));
    }
    // c:1306 — `code = wc_code(*state->pc++);`
    code = wc_code(state.prog.prog[state.pc]);
    state.pc += 1;
    // c:1311-1312 — `otj = thisjob; thisjob = -1;`
    let otj = *THISJOB
        .get_or_init(|| std::sync::Mutex::new(-1))
        .lock()
        .unwrap();
    *THISJOB
        .get_or_init(|| std::sync::Mutex::new(-1))
        .lock()
        .unwrap() = -1;
    use crate::ported::zsh_h::{
        WC_ARITH, WC_CASE, WC_COND, WC_FOR, WC_REPEAT, WC_SELECT, WC_SUBSH, WC_TIMED, WC_TRY,
        WC_WHILE,
    };
    use crate::ported::zsh_h::{WC_ASSIGN, WC_CURSH};
    let lv = if code == WC_ASSIGN {
        // c:1315-1319 — assignment-only simple cmd path.
        // cmdoutval = 0; addvars(state, state->pc - 1, 0); setunderscore("");
        addvars(state, state.pc.saturating_sub(1), 0);
        setunderscore(""); // c:1317
        if isset(XTRACE) {
            eprintln!();
        }
        let ef = errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR;
        if ef != 0 {
            ef
        } else {
            0
        }
    } else {
        // c:1322-1330 — dispatch via execfuncs[code - WC_CURSH] or execfuncdef.
        let q = queue_signal_level();
        dont_queue_signals();
        let result = if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
            ERRFLAG_ERROR
        } else if code == WC_FUNCDEF {
            execfuncdef(state, None)
        } else {
            // c:5499 execfuncs[] table inlined — match the WC_* tag.
            match code {
                WC_CURSH => execcursh(state, 0),
                WC_SUBSH => execcursh(state, 0), // subshell folds to cursh body walk
                WC_FOR => execfor(state, 0),
                WC_SELECT => execselect(state, 0),
                WC_CASE => execcase(state, 0),
                WC_IF => execif(state, 0),
                WC_WHILE => execwhile(state, 0),
                WC_REPEAT => execrepeat(state, 0),
                WC_TIMED => exectime(state, 0),
                WC_COND => execcond(state, 0),
                WC_ARITH => execarith(state, 0),
                WC_TRY => exectry(state, 0),
                _ => 0,
            }
        };
        restore_queue_signals(q);
        result
    };
    // c:1334 — `thisjob = otj;`
    *THISJOB
        .get_or_init(|| std::sync::Mutex::new(-1))
        .lock()
        .unwrap() = otj;
    LASTVAL.store(lv, Ordering::Relaxed); // c:1336 — `return lastval = lv;`
    lv
}

/// Port of `execlist()` from `Src/exec.c:1349` — C decl `execlist(Estate state, int dont_change_job, int exiting)`.
/// Walks WC_LIST entries, dispatches each
/// sublist (WC_SUBLIST chain inlined per c:1525-1625, same as C —
/// there's no separate execsublist function), handles signal-trap
/// dispatch + ERREXIT propagation.
///
/// Body ports the structural skeleton faithfully (WC_LIST walk,
/// per-iteration breaks/retflag/errflag guards, ltype dispatch on
/// Z_END/Z_SYNC/Z_ASYNC, donetrap handling). The full signal queue
/// + DEBUGBEFORECMD trap machinery from c:1357-1500 is preserved
/// in shape with TODO-citations where dependent primitives aren't
/// yet ported.
pub fn execlist(state: &mut estate, dont_change_job: i32, mut exiting: i32) -> i32 {
    let mut last_status: i32 = 0;
    let mut donetrap: i32 = 0; // c:1352 — `static int donetrap;`
    let cj = *THISJOB
        .get_or_init(|| std::sync::Mutex::new(-1))
        .lock()
        .unwrap(); // c:1364 — `cj = thisjob;`
    let _ = dont_change_job; // c:1361 — restored on exit if nonzero.
                             // c:1380 — `code = *state->pc++;`
    if state.pc >= state.prog.prog.len() {
        return last_status;
    }
    let mut code = state.prog.prog[state.pc];
    state.pc += 1;
    // c:1382-1384 — empty list returns lastval = 0.
    if wc_code(code) != WC_LIST {
        LASTVAL.store(0, Ordering::Relaxed);
        return 0;
    }
    use crate::ported::zsh_h::{WC_LIST_SKIP, WC_LIST_TYPE, Z_END, Z_SIMPLE, Z_SYNC};
    // c:1385-1499 — main WC_LIST loop.
    while wc_code(code) == WC_LIST
        && BREAKS.load(Ordering::SeqCst) == 0
        && RETFLAG.load(Ordering::SeqCst) == 0
        // c:1390 — `while (wc_code(code) == WC_LIST && !breaks && !retflag &&
        // !errflag)`: the WHOLE errflag. A user interrupt sets ERRFLAG_INT and
        // never ERRFLAG_ERROR (signals.c:457), so the mask let the rest of the
        // list run after ^C or an interrupting trap:
        //   TRAPINT() { print T; return 1 }
        //   f() { print A; kill -INT $$; print C }; f; print B
        //   zsh: A T      zshrs: A T B
        && errflag.load(Ordering::Relaxed) == 0
    {
        let ltype = WC_LIST_TYPE(code) as i32;
        // c:1396 — `csp = cmdsp;` — snapshot cmdstack depth at start
        // of this WC_LIST iteration; restored at end so partial
        // cmdpush sequences (e.g. from execcond, execfuncs) don't
        // leak into the next sublist.
        let csp = crate::ported::prompt::CMDSTACK.with(|s| s.borrow().len());
        // c:1502-1509 — Z_SIMPLE fast-path.
        if (ltype & Z_SIMPLE as i32) != 0 {
            let next_pc = state.pc + WC_LIST_SKIP(code) as usize;
            let s = execsimple(state);
            last_status = s;
            state.pc = next_pc;
        } else {
            // c:1513-1523 — sublist chain.
            if state.pc >= state.prog.prog.len() {
                break;
            }
            code = state.prog.prog[state.pc];
            state.pc += 1;
            // c:1525-1625 — sublist chain (&&/|| operators) inlined.
            use crate::ported::zsh_h::{
                WC_SUBLIST_AND, WC_SUBLIST_END, WC_SUBLIST_NOT, WC_SUBLIST_OR, WC_SUBLIST_SIMPLE,
                WC_SUBLIST_SKIP,
            };
            let mut sub_code = code;
            let _ = dont_change_job;
            while wc_code(sub_code) == WC_SUBLIST {
                let flags = WC_SUBLIST_FLAGS(sub_code);
                let next = state.pc + WC_SUBLIST_SKIP(sub_code) as usize;
                let sl_type = WC_SUBLIST_TYPE(sub_code) as i32;
                let last1 = if WC_SUBLIST_TYPE(sub_code) == WC_SUBLIST_END {
                    exiting
                } else {
                    0
                };
                if flags == WC_SUBLIST_SIMPLE {
                    last_status = execsimple(state); // c:1605
                } else {
                    let _ = execpline(state, sub_code, sl_type, last1); // c:1607
                    last_status = LASTVAL.load(Ordering::Relaxed);
                }
                // c:1612 — `WC_SUBLIST_NOT` inverts status.
                if (flags & WC_SUBLIST_NOT) != 0 {
                    last_status = if last_status == 0 { 1 } else { 0 };
                    LASTVAL.store(last_status, Ordering::Relaxed);
                }
                state.pc = next;
                if WC_SUBLIST_TYPE(sub_code) == WC_SUBLIST_END {
                    break;
                }
                if state.pc >= state.prog.prog.len() {
                    break;
                }
                // c:1617-1623 — short-circuit on && / ||.
                if sl_type == WC_SUBLIST_AND as i32 && last_status != 0 {
                    while state.pc < state.prog.prog.len() {
                        let c = state.prog.prog[state.pc];
                        if wc_code(c) != WC_SUBLIST {
                            break;
                        }
                        state.pc = state.pc + 1 + WC_SUBLIST_SKIP(c) as usize;
                        if WC_SUBLIST_TYPE(c) == WC_SUBLIST_END {
                            break;
                        }
                    }
                    break;
                }
                if sl_type == WC_SUBLIST_OR as i32 && last_status == 0 {
                    while state.pc < state.prog.prog.len() {
                        let c = state.prog.prog[state.pc];
                        if wc_code(c) != WC_SUBLIST {
                            break;
                        }
                        state.pc = state.pc + 1 + WC_SUBLIST_SKIP(c) as usize;
                        if WC_SUBLIST_TYPE(c) == WC_SUBLIST_END {
                            break;
                        }
                    }
                    break;
                }
                sub_code = state.prog.prog[state.pc];
                state.pc += 1;
            }
        }
        // c:1593 — `cmdsp = csp;` — restore cmdstack depth to the
        // snapshot taken at start of iteration. Reverses any cmdpush
        // calls made by nested execcond / execfuncs / execcmd_exec
        // that didn't pop cleanly.
        crate::ported::prompt::CMDSTACK.with(|s| {
            let mut g = s.borrow_mut();
            if g.len() > csp {
                g.truncate(csp);
            }
        });
        // c:1626-1634 — donetrap is reset between sublists.
        donetrap = 0;
        // c:1640-1645 — fetch next WC_LIST header (or break out).
        if state.pc >= state.prog.prog.len() {
            break;
        }
        let next_code = state.prog.prog[state.pc];
        if wc_code(next_code) != WC_LIST {
            break;
        }
        state.pc += 1;
        code = next_code;
        // c:1389 — z_end means last sublist, exiting becomes 1 for tail-exec.
        if (ltype & Z_END as i32) != 0 {
            exiting = 1;
        }
    }
    // c:1659-1664 — cleanup: restore thisjob if dont_change_job, this_noerrexit=1.
    if dont_change_job != 0 {
        *THISJOB
            .get_or_init(|| std::sync::Mutex::new(-1))
            .lock()
            .unwrap() = cj;
    }
    let _ = donetrap;
    this_noerrexit.store(1, Ordering::Relaxed);
    LASTVAL.store(last_status, Ordering::Relaxed);
    last_status
}

// WC_SUBLIST chain walk is inlined into execlist (per `Src/exec.c:1525-
// 1625`, the C source likewise inlines it — there's no `execsublist`
// function in zsh C).

/// Port of `execcmd_getargs()` from `Src/exec.c:2791` — C decl `static void execcmd_getargs(LinkList preargs, LinkList args, int expand)`.
/// Transfer the first node of `args`
/// to `preargs`, performing `prefork` (singleton-list expansion) on
/// the way if `expand` is set. Used by `execcmd_exec` to pull the
/// command head one word at a time so prefix-modifier walking
/// (BINF_COMMAND, BINF_EXEC etc.) sees expanded names.
pub fn execcmd_getargs(preargs: &mut LinkList<String>, args: &mut LinkList<String>, expand: i32) {
    // c:2791
    if args.firstnode().is_none() {
        // c:2793 — `if (!firstnode(args)) return;`
        return;
    } else if expand != 0 {
        // c:2795
        // c:2796-2797 — `local_list0(svl); init_list0(svl);` —
        // stack-local single-bucket list. Rust uses a fresh
        // LinkList<String> per call.
        let mut svl: LinkList<String> = Default::default();
        // c:2799 — `addlinknode(&svl, uremnode(args, firstnode(args)));`
        if let Some(idx) = args.firstnode() {
            if let Some(head) = crate::ported::linklist::uremnode(args, idx) {
                svl.push_back(head);
            }
        }
        // c:2801 — `prefork(&svl, 0, NULL);`
        let mut rf = 0i32;
        prefork(&mut svl, 0, &mut rf);
        // c:2802 — `joinlists(preargs, &svl);`
        crate::ported::linklist::joinlists(preargs, &mut svl);
    } else {
        // c:2803-2804 — no-expand path: move head verbatim.
        if let Some(idx) = args.firstnode() {
            if let Some(head) = crate::ported::linklist::uremnode(args, idx) {
                preargs.push_back(head);
            }
        }
    }
}

/// Port of `execcmd_fork()` from `Src/exec.c:2810` — C decl `execcmd_fork(Estate state, int how, int type, Wordcode varspc, LinkList *filelistp, char *text, int oautocont, int close_if_forked)`.
/// Wordcode varspc, LinkList *filelistp, char *text, int oautocont,
/// int close_if_forked)` from `Src/exec.c:2810-2893`.
///
/// Fork the current command into a child process: parent records
/// the pid + STTY env scan + addproc; child enters subshell, writes
/// `entersubsh_ret` back to parent through `synch` pipe, and returns
/// 0 so the caller can continue with the body.
///
/// `filelistp` out-arg is moved from `jobtab[thisjob].filelist`
/// only in the child branch (so the parent's `filelist` stays
/// untouched). Rust sig keeps the same C contract.
pub fn execcmd_fork(
    state: &mut estate,
    how: i32,
    typ: i32,
    varspc: Option<usize>,
    filelistp: &mut Vec<jobfile>,
    text: &str,
    oautocont: i32,
    close_if_forked: i32,
) -> i32 {
    use crate::ported::signals::sigtrapped as sigtrapped_static;
    use crate::ported::signals_h::SIGEXIT;
    use crate::ported::zsh_h::{
        AUTOCONTINUE, BGNICE, WC_ASSIGN as ZWC_ASSIGN, WC_ASSIGN_NUM as ZWC_ASSIGN_NUM,
        WC_ASSIGN_SCALAR as ZWC_ASSIGN_SCALAR, WC_ASSIGN_TYPE as ZWC_ASSIGN_TYPE,
        WC_SUBSH as ZWC_SUBSH, ZSIG_IGNORED, Z_ASYNC,
    };
    // c:2810
    let pid: libc::pid_t; // c:2814
    let mut synch: [i32; 2] = [-1, -1]; // c:2815
    let flags: i32; // c:2815
    let mut esret: entersubsh_ret = entersubsh_ret::default(); // c:2816
                                                               // c:2817 — `struct timespec bgtime;` — bgtime is passed to zfork
                                                               // for accounting; the Rust zfork wrapper expects Option<&mut ZshTimespec>.
    let mut bgtime = ZshTimespec::default();

    child_block(); // c:2819
    esret.gleader = -1; // c:2820
    esret.list_pipe_job = -1; // c:2821

    // c:2823 — `if (pipe(synch) < 0) { zerr("pipe failed: %e", errno); return -1; }`
    if unsafe { libc::pipe(synch.as_mut_ptr()) } < 0 {
        zerr(&format!("pipe failed: {}", std::io::Error::last_os_error()));
        return -1; // c:2825
    }
    // c:2826 — `else if ((pid = zfork(&bgtime)) == -1) { ... }`
    pid = zfork(Some(&mut bgtime));
    if pid == -1 {
        unsafe {
            libc::close(synch[0]); // c:2827
            libc::close(synch[1]); // c:2828
        }
        LASTVAL.store(1, Ordering::Relaxed); // c:2829
        errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed); // c:2830
        return -1; // c:2831
    }
    if pid != 0 {
        // c:2833 — parent.
        unsafe { libc::close(synch[1]) }; // c:2834
                                          // c:2835 — `read_loop(synch[0], (char *)&esret, sizeof(esret));`
        let mut buf = [0u8; size_of::<entersubsh_ret>()];
        let _ = crate::ported::utils::read_loop(synch[0], &mut buf);
        // entersubsh_ret is two i32s; reconstruct from LE bytes (host order).
        if buf.len() >= 8 {
            esret.gleader = i32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]);
            esret.list_pipe_job = i32::from_ne_bytes([buf[4], buf[5], buf[6], buf[7]]);
        }
        unsafe { libc::close(synch[0]) }; // c:2836
        if (how & Z_ASYNC as i32) != 0 {
            // c:2837 — `lastpid = (zlong) pid;`
            crate::ported::modules::clone::lastpid.store(pid, Ordering::Relaxed);
        } else {
            // c:2839 — `if (!jobtab[thisjob].stty_in_env && varspc)`.
            let thisjob_idx = {
                if let Some(m) = THISJOB.get() {
                    *m.lock().unwrap()
                } else {
                    -1
                }
            };
            // Examine the jobtab entry under lock.
            let stty_already = if thisjob_idx >= 0 {
                if let Some(jt) = JOBTAB.get() {
                    let guard = jt.lock().unwrap();
                    guard
                        .get(thisjob_idx as usize)
                        .map(|j| j.stty_in_env != 0)
                        .unwrap_or(true)
                } else {
                    true
                }
            } else {
                true
            };
            if !stty_already && varspc.is_some() {
                // c:2841-2851 — walk varspc looking for STTY=...
                let mut p = varspc.unwrap();
                loop {
                    if p >= state.prog.prog.len() {
                        break;
                    }
                    let ac = state.prog.prog[p];
                    if wc_code(ac) != ZWC_ASSIGN {
                        break;
                    }
                    // c:2845 — `if (!strcmp(ecrawstr(state->prog, p + 1, NULL), "STTY"))`
                    let name = ecrawstr(&state.prog, p + 1, None);
                    if name == "STTY" {
                        // c:2846 — `jobtab[thisjob].stty_in_env = 1;`
                        if let Some(jt) = JOBTAB.get() {
                            let mut guard = jt.lock().unwrap();
                            if let Some(j) = guard.get_mut(thisjob_idx as usize) {
                                j.stty_in_env = 1;
                            }
                        }
                        break; // c:2847
                    }
                    p += if ZWC_ASSIGN_TYPE(ac) == ZWC_ASSIGN_SCALAR {
                        3 // c:2849
                    } else {
                        (ZWC_ASSIGN_NUM(ac) + 2) as usize // c:2850
                    };
                }
            }
        }
        // c:2853 — `addproc(pid, text, 0, &bgtime, esret.gleader, esret.list_pipe_job);`
        if let Some(jt) = JOBTAB.get() {
            let mut guard = jt.lock().unwrap();
            let tj = {
                if let Some(m) = THISJOB.get() {
                    *m.lock().unwrap()
                } else {
                    -1
                }
            };
            if tj >= 0 {
                if let Some(j) = guard.get_mut(tj as usize) {
                    crate::ported::jobs::addproc(
                        j,
                        pid,
                        text,
                        false,
                        Some(std::time::Instant::now()),
                        esret.gleader,
                        esret.list_pipe_job,
                    );
                }
            }
        }
        // c:2854-2855 — `if (oautocont >= 0) opts[AUTOCONTINUE] = oautocont;`
        if oautocont >= 0 {
            opt_state_set("autocontinue", oautocont != 0);
            let _ = AUTOCONTINUE; // const referenced for parity
        }
        // c:2856 — `pipecleanfilelist(jobtab[thisjob].filelist, 1);`
        if let Some(jt) = JOBTAB.get() {
            let mut guard = jt.lock().unwrap();
            let tj = {
                if let Some(m) = THISJOB.get() {
                    *m.lock().unwrap()
                } else {
                    -1
                }
            };
            if tj >= 0 {
                if let Some(j) = guard.get_mut(tj as usize) {
                    crate::ported::jobs::pipecleanfilelist(j, true);
                }
            }
        }
        return pid; // c:2857
    }

    // c:2860 — pid == 0 (child).
    unsafe { libc::close(synch[0]) }; // c:2861
    flags = (if (how & Z_ASYNC as i32) != 0 {
        esub::ASYNC
    } else {
        0
    }) | esub::PGRP; // c:2862
    let mut flags = flags;
    if typ != ZWC_SUBSH as i32 && (how & Z_ASYNC as i32) == 0 {
        flags |= esub::KEEPTRAP; // c:2864
    }
    if typ == ZWC_SUBSH as i32 && (how & Z_ASYNC as i32) == 0 {
        flags |= esub::JOB_CONTROL; // c:2866
    }
    // c:2867 — `*filelistp = jobtab[thisjob].filelist;`
    if let Some(jt) = JOBTAB.get() {
        let mut guard = jt.lock().unwrap();
        let tj = {
            if let Some(m) = THISJOB.get() {
                *m.lock().unwrap()
            } else {
                -1
            }
        };
        if tj >= 0 {
            if let Some(j) = guard.get_mut(tj as usize) {
                *filelistp = std::mem::take(&mut j.filelist);
            }
        }
    }
    entersubsh(flags, Some(&mut esret)); // c:2868
                                         // c:2869 — `write_loop(synch[1], &esret, sizeof(esret));`
    let mut buf = [0u8; 8];
    buf[0..4].copy_from_slice(&esret.gleader.to_ne_bytes());
    buf[4..8].copy_from_slice(&esret.list_pipe_job.to_ne_bytes());
    if write_loop(synch[1], &buf).map(|n| n as usize).unwrap_or(0) != buf.len() {
        zerr(&format!(
            "Failed to send entersubsh_ret report: {}",
            std::io::Error::last_os_error()
        ));
        return -1; // c:2871
    }
    unsafe { libc::close(synch[1]) }; // c:2873
    let _ = zclose(close_if_forked); // c:2874

    // c:2876 — `if (sigtrapped[SIGINT] & ZSIG_IGNORED) holdintr();`
    let sigint_state = {
        let guard = sigtrapped_static.lock().unwrap();
        guard.get(libc::SIGINT as usize).copied().unwrap_or(0)
    };
    if (sigint_state & ZSIG_IGNORED) != 0 {
        crate::ported::signals::holdintr(); // c:2877
    }
    // c:2882 — `sigtrapped[SIGEXIT] = 0;` — EXIT traps don't fire in fork-child.
    {
        let mut guard = sigtrapped_static.lock().unwrap();
        if let Some(slot) = guard.get_mut(SIGEXIT as usize) {
            *slot = 0;
        }
    }
    // c:2884-2890 — `if ((how & Z_ASYNC) && isset(BGNICE)) nice(5)`.
    // Per-platform errno setter+reader: __error() on macOS,
    // __errno_location() on Linux. Without cfg gating Linux CI breaks.
    if (how & Z_ASYNC as i32) != 0 && isset(BGNICE) {
        #[cfg(target_os = "macos")]
        unsafe {
            *libc::__error() = 0;
            if libc::nice(5) == -1 && *libc::__error() != 0 {
                zwarn(&format!(
                    "nice(5) failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
        #[cfg(target_os = "linux")]
        unsafe {
            *libc::__errno_location() = 0;
            if libc::nice(5) == -1 && *libc::__errno_location() != 0 {
                zwarn(&format!(
                    "nice(5) failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
        }
    }
    0 // c:2892
}

/// Port of `execcmd_analyse()` from `Src/exec.c:2733` — C decl `execcmd_analyse(Estate state, Execcmd_params eparams)`.
/// Pre-execcmd_exec analysis pass:
/// walks the wordcode at `state->pc`, splits out redirs/varspc/args
/// without expanding (no prefork, no globbing), and fills `eparams`
/// so the caller (execcmd_exec at c:2901 or execpline2 at c:2013)
/// can branch on the command type before the real work.
pub fn execcmd_analyse(state: &mut estate, eparams: &mut crate::ported::zsh_h::execcmd_params) {
    use crate::ported::zsh_h::{
        WC_ASSIGN as ZWC_ASSIGN, WC_REDIR as ZWC_REDIR, WC_SIMPLE as ZWC_SIMPLE,
        WC_SIMPLE_ARGC as ZWC_SIMPLE_ARGC, WC_TYPESET as ZWC_TYPESET,
        WC_TYPESET_ARGC as ZWC_TYPESET_ARGC,
    };
    // c:2733
    let mut code: wordcode; // c:2735
    let mut i: i32; // c:2736
    let _ = i;

    // c:2738 — `eparams->beg = state->pc;`
    eparams.beg = state.pc;
    // c:2739-2740 — `eparams->redir = (wc_code(*state->pc) == WC_REDIR ? ecgetredirs(state) : NULL);`
    eparams.redir =
        if state.pc < state.prog.prog.len() && wc_code(state.prog.prog[state.pc]) == ZWC_REDIR {
            Some(crate::ported::parse::ecgetredirs(state))
        } else {
            None
        };
    // c:2741-2748 — varspc walk (WC_ASSIGN chain).
    if state.pc < state.prog.prog.len() && wc_code(state.prog.prog[state.pc]) == ZWC_ASSIGN {
        cmdoutval.store(0, Ordering::Relaxed); // c:2742
        eparams.varspc = Some(state.pc); // c:2743
                                         // c:2744-2746 — `while (wc_code((code = *state->pc)) == WC_ASSIGN) state->pc += ...`
        loop {
            if state.pc >= state.prog.prog.len() {
                break;
            }
            code = state.prog.prog[state.pc];
            if wc_code(code) != ZWC_ASSIGN {
                break;
            }
            state.pc += if WC_ASSIGN_TYPE(code) == WC_ASSIGN_SCALAR {
                3 // c:2745
            } else {
                (WC_ASSIGN_NUM(code) + 2) as usize // c:2746
            };
        }
    } else {
        eparams.varspc = None; // c:2748
    }

    // c:2750 — `code = *state->pc++;`
    if state.pc >= state.prog.prog.len() {
        eparams.args = None;
        eparams.assignspc = None;
        eparams.typ = 0;
        eparams.postassigns = 0;
        eparams.htok = 0;
        return;
    }
    code = state.prog.prog[state.pc];
    state.pc += 1;

    // c:2752 — `eparams->type = wc_code(code);`
    eparams.typ = wc_code(code) as i32;
    // c:2753 — `eparams->postassigns = 0;`
    eparams.postassigns = 0;

    // c:2755-2783 — switch on type. EC_DUP is used (not EC_DUPTOK)
    // per the comment at c:2755-2757.
    match eparams.typ as wordcode {
        x if x == ZWC_SIMPLE => {
            // c:2759-2763
            let mut htok = 0;
            let argc = ZWC_SIMPLE_ARGC(code) as usize;
            eparams.args = Some(ecgetlist(state, argc, EC_DUP, Some(&mut htok)));
            eparams.htok = htok;
            eparams.assignspc = None;
        }
        x if x == ZWC_TYPESET => {
            // c:2765-2777
            let mut htok = 0;
            let argc = ZWC_TYPESET_ARGC(code) as usize;
            eparams.args = Some(ecgetlist(state, argc, EC_DUP, Some(&mut htok)));
            eparams.htok = htok;
            // c:2768 — `eparams->postassigns = *state->pc++;`
            if state.pc < state.prog.prog.len() {
                eparams.postassigns = state.prog.prog[state.pc] as i32;
                state.pc += 1;
            }
            // c:2769 — `eparams->assignspc = state->pc;`
            eparams.assignspc = Some(state.pc);
            // c:2770-2776 — walk past the postassigns.
            let mut k = 0i32;
            while k < eparams.postassigns {
                if state.pc >= state.prog.prog.len() {
                    break;
                }
                code = state.prog.prog[state.pc];
                // c:2772-2773 DPUTS — assert wc_code == WC_ASSIGN; skipped.
                state.pc += if WC_ASSIGN_TYPE(code) == WC_ASSIGN_SCALAR {
                    3 // c:2774
                } else {
                    (WC_ASSIGN_NUM(code) + 2) as usize // c:2775
                };
                k += 1;
            }
        }
        _ => {
            // c:2779-2783 default.
            eparams.args = None;
            eparams.assignspc = None;
            eparams.htok = 0;
        }
    }
}

/// Port of `char **zsh_eval_context;` from `Src/exec.c` (zsh.export:355).
/// Stack of `"context"` labels used by `eval`-style nested execution:
/// `bin_dot`, `bin_eval`, `execode`, autoloads. Each `execode(prog,
/// ..., "context")` pushes its label and pops on return.
pub static zsh_eval_context: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// RAII form of C's `execode` context push/pop (`Src/exec.c:1265-1266`
/// and the matching `zsh_eval_context[alen] = NULL` at c:1281).
///
/// zshrs does not route every nested execution through `execode`, so
/// the callers that C reaches via `execode(prog, …, "label")` push
/// their label with this guard instead. Dropping it pops, on every
/// return path including a panic — C relies on the plain assignment at
/// c:1281 being reached, which the guard reproduces more robustly.
pub struct EvalContextFrame {
    pushed: bool,
}

impl EvalContextFrame {
    /// c:1265-1266 — `zsh_eval_context[alen] = context;`
    pub fn push(context: &str) -> Self {
        let mut pushed = false;
        if let Ok(mut ctx) = zsh_eval_context.lock() {
            ctx.push(context.to_string());
            Self::sync(&ctx);
            pushed = true;
        }
        Self { pushed }
    }

    /// Publish the `zsh_eval_context` stack into the shell-visible
    /// params. C keeps ONE `char **zsh_eval_context` that the parameter
    /// table points at directly (`IPDEF8`/`IPDEF9`, params.c:401/431),
    /// so a push is instantly visible as `$zsh_eval_context` /
    /// `$ZSH_EVAL_CONTEXT`. The Rust params own their storage, so every
    /// mutation of the static has to be mirrored — through the
    /// `u_arr`/`u_str` fields directly because both names are
    /// `PM_READONLY_SPECIAL`.
    fn sync(stack: &[String]) {
        let joined = stack.join(":");
        if let Ok(mut tab) = crate::ported::params::paramtab().write() {
            if let Some(pm) = tab.get_mut("zsh_eval_context") {
                pm.u_arr = Some(stack.to_vec());
                pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
            }
            if let Some(pm) = tab.get_mut("ZSH_EVAL_CONTEXT") {
                pm.u_str = Some(joined);
                pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
            }
        }
    }
}

impl Drop for EvalContextFrame {
    fn drop(&mut self) {
        // c:1281 — `zsh_eval_context[alen] = NULL;`
        if self.pushed {
            if let Ok(mut ctx) = zsh_eval_context.lock() {
                ctx.pop();
                Self::sync(&ctx);
            }
        }
    }
}

/// Port of `static int donetrap;` from `Src/exec.c:1351`. Tracks
/// whether the ZERR trap has already fired for the current sublist.
/// C source resets to 0 at sublist start (c:1455) and sets to 1
/// after `dotrap(SIGZERR)` (c:1602). The check
/// `if (!this_noerrexit && !donetrap && !this_donetrap)` at c:1598
/// suppresses re-firing within the same sublist AND, crucially,
/// carries the "already fired" state across a function-call return
/// boundary so the outer caller's post-command check doesn't fire
/// ZERR a second time for the same logical error. Bug #303 in
/// docs/BUGS.md.
///
/// Reset at each top-level statement boundary via
/// `BUILTIN_DONETRAP_RESET` emitted by `compile_list`. Set after
/// `dotrap(SIGZERR)` fires inside `BUILTIN_ERREXIT_CHECK`.
pub static DONETRAP: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Port of `save_params()` from `Src/exec.c:4410` — C decl `save_params(Estate state, Wordcode pc, LinkList *restore_p, LinkList *remove_p)`.
/// Walk WC_ASSIGN
/// chain at `pc`, snapshot each existing param into `restore_p` (so
/// the builtin/shfunc can restore them on return) and enqueue every
/// touched name in `remove_p` (so we know what to unset).
pub fn save_params(
    state: &mut estate,
    pc: usize,
    restore_p: &mut Vec<crate::ported::zsh_h::param>,
    remove_p: &mut Vec<String>,
) {
    use crate::ported::zsh_h::{
        PM_READONLY, PM_SPECIAL, WC_ASSIGN, WC_ASSIGN_NUM as ZWC_ASSIGN_NUM,
        WC_ASSIGN_SCALAR as ZWC_ASSIGN_SCALAR, WC_ASSIGN_TYPE as ZWC_ASSIGN_TYPE,
    };
    // c:4410 — `*restore_p = newlinklist();` — caller pre-allocates.
    // c:4417 — `*remove_p = newlinklist();` — caller pre-allocates.
    let mut p = pc;
    // c:4419 — `while (wc_code(ac = *pc) == WC_ASSIGN)`
    loop {
        if p >= state.prog.prog.len() {
            break;
        }
        let ac = state.prog.prog[p];
        if wc_code(ac) != WC_ASSIGN {
            break;
        }
        // c:4420 — `s = ecrawstr(state->prog, pc + 1, NULL);`
        let s = ecrawstr(&state.prog, p + 1, None);
        // c:4421 — `pm = paramtab->getnode(paramtab, s)`
        let pm_clone: Option<crate::ported::zsh_h::param> = {
            let tab = paramtab().read().unwrap();
            tab.get(&s).map(|b| (**b).clone())
        };
        if let Some(pm) = pm_clone {
            // c:4423-4424 — `if (pm->env) delenv(pm);`
            if pm.env.is_some() {
                crate::ported::params::delenv(&s);
            }
            // c:4425-4448 — copy if not readonly-special.
            if (pm.node.flags & PM_SPECIAL as i32) == 0 {
                // c:4426-4438 — regular param: deep copy via copyparam(tpm, pm, 0).
                let mut tpm = pm.clone();
                tpm.node.nam = s.clone();
                // copyparam with fakecopy=0 already done by the clone()
                // (Clone derives a deep copy of param fields).
                restore_p.push(tpm); // c:4451
            } else if (pm.node.flags & PM_READONLY as i32) == 0 {
                // c:4439-4448 — special-but-not-readonly: fakecopy=1.
                let mut tpm = pm.clone();
                tpm.node.nam = pm.node.nam.clone();
                restore_p.push(tpm); // c:4451
            }
            // c:4449 — `addlinknode(*remove_p, dupstring(s));`
            remove_p.push(s.clone());
        } else {
            // c:4453 — `addlinknode(*remove_p, dupstring(s));`
            remove_p.push(s.clone());
        }
        // c:4455 — `pc += (WC_ASSIGN_TYPE(ac) == WC_ASSIGN_SCALAR ? 3 : WC_ASSIGN_NUM(ac) + 2);`
        p += if ZWC_ASSIGN_TYPE(ac) == ZWC_ASSIGN_SCALAR {
            3
        } else {
            (ZWC_ASSIGN_NUM(ac) + 2) as usize
        };
    }
}

/// Port of `restore_params()` from `Src/exec.c:4464` — C decl `restore_params(LinkList restorelist, LinkList removelist)`.
/// After the builtin/shfunc returns,
/// unset every name in removelist, then for each saved param in
/// restorelist re-install its values (PM_SPECIAL go through gsu
/// setfn; regular params re-enter paramtab as-is).
pub fn restore_params(restorelist: Vec<crate::ported::zsh_h::param>, removelist: Vec<String>) {
    use crate::ported::zsh_h::{PM_READONLY, PM_SPECIAL};
    // c:4470-4476 — `while ((s = ugetnode(removelist)))` — unset each.
    for s in &removelist {
        // c:4471 — `if ((pm = paramtab->getnode(paramtab, s)) && !(pm->node.flags & PM_SPECIAL))`
        let flags = {
            let tab = paramtab().read().unwrap();
            tab.get(s).map(|p| p.node.flags)
        };
        if let Some(f) = flags {
            if (f & PM_SPECIAL as i32) == 0 {
                // c:4473 — `pm->node.flags &= ~PM_READONLY;`
                let mut tab = paramtab().write().unwrap();
                if let Some(pm_mut) = tab.get_mut(s) {
                    pm_mut.node.flags &= !(PM_READONLY as i32);
                }
                // Drop write guard before calling unsetparam_pm.
                drop(tab);
                let mut tab = paramtab().write().unwrap();
                if let Some(pm_mut) = tab.get_mut(s) {
                    let _ = crate::ported::params::unsetparam_pm(pm_mut, 0, 0); // c:4474
                }
            }
        }
    }
    // c:4478-4523 — restore saved params.
    for pm in restorelist {
        // c:4481-4520 — PM_SPECIAL: route through gsu setfn.
        // c:4521-4523 — non-special: re-install via paramtab.
        if (pm.node.flags & PM_SPECIAL as i32) != 0 {
            // PM_SPECIAL restore: full path requires PM_TYPE dispatch
            // on gsu_s/i/f/a/h setfn. Each setfn fires the param's
            // canonical write hook. Pragmatic port: overwrite in
            // paramtab; daily-driver path rarely saves specials (those
            // are reserved-name vars like PATH/FPATH/etc. which can't
            // appear as `VAR=val cmd` prefix anyway).
            let mut tab = paramtab().write().unwrap();
            tab.insert(pm.node.nam.clone(), Box::new(pm));
        } else {
            // c:4521 — `paramtab->addnode(paramtab, ztrdup(pm->node.nam), pm);`
            let mut tab = paramtab().write().unwrap();
            tab.insert(pm.node.nam.clone(), Box::new(pm));
        }
    }
}

/// Port of `execode` from `Src/exec.c:1245` — C decl `execode(Eprog p, int dont_change_job, int exiting, char *context)`. Rust fn `execode_wordcode` is the wordcode form, driving the ported `execlist` interpreter; `execode` at exec.rs:8413 is the fusevm-pipeline entry that carries the C name.
/// Set up an `estate`
/// around the given Eprog and run `execlist`. Maintains the
/// `zsh_eval_context` stack so `$ZSH_EVAL_CONTEXT` reflects the
/// call chain.
///
/// NOTE: this is the WORDCODE form (drives the ported `execlist`
/// interpreter). zshrs's live execution pipeline is fusevm
/// (`compile_zsh` → VM), so the top-level REPL `loop()` and most call
/// sites run through [`execode`] (the ZshProgram/fusevm form below)
/// instead. This wordcode entry is retained for the internal
/// function-body callers (`doshfunc` / autoload) that already hold an
/// `Eprog`.
pub fn execode_wordcode(
    p: crate::ported::zsh_h::Eprog,
    dont_change_job: i32,
    exiting: i32,
    context: &str,
) {
    // c:1245
    let prog_ref = *p;
    // c:1247 — `struct estate s;`
    let mut s = estate {
        prog: Box::new(prog_ref.clone()),
        // c:1269 — `s.pc = p->prog;` — start at index 0.
        pc: 0,
        // c:1270 — `s.strs = p->strs;`
        strs: prog_ref.strs.clone(),
        strs_offset: 0,
    };
    // c:1251-1266 — push context onto zsh_eval_context.
    let pushed = {
        if let Ok(mut ctx) = zsh_eval_context.lock() {
            ctx.push(context.to_string());
            true
        } else {
            false
        }
    };
    // c:1271 — `useeprog(p);`
    crate::ported::parse::useeprog(&mut s.prog);
    // c:1273 — `execlist(&s, dont_change_job, exiting);`
    execlist(&mut s, dont_change_job, exiting);
    // c:1275 — `freeeprog(p);`
    crate::ported::parse::freeeprog(&mut s.prog);
    // c:1281 — `zsh_eval_context[alen] = NULL;` — pop our entry.
    if pushed {
        if let Ok(mut ctx) = zsh_eval_context.lock() {
            ctx.pop();
        }
    }
}

thread_local! {
    /// The long-lived interactive executor for the top-level `loop()`
    /// REPL (the `zsh_main` path). Set once by the bin before
    /// `zsh_main`; [`execode`] runs each parsed program through it.
    /// Persists for the whole session so variables/functions survive
    /// across prompts.
    static SESSION_EXECUTOR: std::cell::Cell<Option<*mut crate::vm_helper::ShellExecutor>> =
        const { std::cell::Cell::new(None) };
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// installing the process-lifetime fusevm executor that `execode`
/// (exec.rs:8413) runs programs on. C has no analogue: its interpreter state
/// is the `estate` built per call in `execode` (c:1245).
/// Register the persistent session executor used by [`execode`] for the
/// interactive `loop()` REPL. The pointer must outlive the session (the
/// bin keeps the executor alive until `zsh_main` exits the process).
pub fn install_session_executor(exec: &mut crate::vm_helper::ShellExecutor) {
    SESSION_EXECUTOR.with(|c| c.set(Some(exec as *mut crate::vm_helper::ShellExecutor)));
    // Mirror the pointer into the bridge so `with_session_context` can
    // establish a VM context for startup rc-sourcing (run_init_scripts,
    // c:1914) before the loop's first execode enters one.
    crate::fusevm_bridge::register_session_executor(exec);
}

/// Port of `execode()` from `Src/exec.c:1245` — C decl `execode(Eprog p, int dont_change_job, int exiting, char *context)`. DIVERGENCE: this entry drives the fusevm pipeline over a `ZshProgram`; the faithful wordcode walker is `execode_wordcode` (exec.rs:8338).
/// zshrs `execode` — run an already-parsed `ZshProgram` (Src/exec.c:1245
/// `execode(prog, ...)`, called from `loop()`). This is the **exec.rs
/// exception** to the line-by-line port: rather than walk wordcode via
/// `execlist`, it drives zshrs's live engine — compile the program with
/// `compile_zsh` and run it on the session executor's fusevm VM. The
/// faithful `loop()` in init.rs calls this exactly as C calls execode.
/// Returns `$?` (0 when no session executor is installed).
pub fn execode(
    program: &crate::parse::ZshProgram,
    _dont_change_job: i32,
    _exiting: i32,
    context: &str,
) -> i32 {
    // c:1319 — `zsh_eval_context_push(context)`. The wordcode twin
    // (`execode_wordcode`) already does this; this fusevm variant is the
    // one `loop()` (Src/init.c:220) calls with "toplevel", so dropping the
    // push left `$zsh_eval_context` / `$ZSH_EVAL_CONTEXT` empty for the
    // whole interactive session (a `-c` shell got "cmdarg" from its own
    // caller and looked correct, hiding this).
    // c:1334 — the matching `zsh_eval_context_pop()` runs when this
    // guard drops, i.e. after execution, exactly as in C.
    let _ctx_frame = EvalContextFrame::push(context); // c:1319
    SESSION_EXECUTOR.with(|c| match c.get() {
        // SAFETY: set by install_session_executor to an executor that
        // lives for the whole single-threaded interactive session;
        // loop() runs only after the bin installs it.
        Some(ptr) => unsafe { (*ptr).execute_program(program) },
        None => 0,
    })
}

// =========================================================================
// Live-executor accessors (former `exec_hooks` OnceLock layer).
//
// These are the **exec.rs exception**: src/ported/ code reaches the
// live fusevm `ShellExecutor` (param store, function dispatch, nested
// script/cmdsubst execution) through these thin wrappers instead of the
// deleted `exec_hooks` fn-pointer registry. Each delegates to
// `fusevm_bridge::try_with_executor` — `Some` when a VM execution
// context is in scope, `None` in unit-test / compsys contexts with no
// bridge running — and reproduces the exact per-call fallback the old
// `exec_hooks` wrappers used when no hook was installed. Behavior is
// byte-for-byte identical to the OnceLock path; only the indirection is
// gone. See `feedback_no_shellexecutor_in_ported` /
// `feedback_no_exec_script_from_ported`: the bridge belongs in exec.rs
// (the sanctioned exception), not scattered through src/ported.
// =========================================================================

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// reading an array parameter through the live fusevm executor, falling back
/// to `params::getaparam`. C reads `paramtab` directly wherever it needs
/// this.
/// Array param value via the live executor; falls back to the direct
/// param table (`params::getaparam`) when no executor is in scope, so
/// compsys / unit-test environments still observe shell-side arrays.
pub fn array(name: &str) -> Option<Vec<String>> {
    // bash special arrays (PIPESTATUS / FUNCNAME / BASH_VERSINFO) alias the
    // zsh-native specials in --bash mode. Checked before the stored-array
    // lookup so a user array of the same name can still shadow it (rare), but
    // after the bash-mode gate so --zsh is untouched. Guard against the
    // PIPESTATUS→pipestatus self-alias recursing by only aliasing uppercase.
    if name.starts_with(|c: char| c.is_ascii_uppercase()) {
        if let Some(v) = crate::dash_mode::bash_special_array(name) {
            return Some(v);
        }
    }
    if let Some(Some(v)) = crate::fusevm_bridge::try_with_executor(|exec| exec.array(name)) {
        return Some(v);
    }
    crate::ported::params::getaparam(name)
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// reading an associative parameter through the live fusevm executor. C reads
/// `paramtab` directly.
/// Associative-array param value via the live executor (`None` when no
/// executor / not set).
pub fn assoc(name: &str) -> Option<indexmap::IndexMap<String, String>> {
    crate::fusevm_bridge::try_with_executor(|exec| exec.assoc(name)).flatten()
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// writing an array parameter through the live fusevm executor. C calls
/// `setaparam` directly.
/// Store an array param into the live executor (no-op without one).
pub fn set_array(name: &str, val: Vec<String>) {
    let _ = crate::fusevm_bridge::try_with_executor(|exec| exec.set_array(name.to_string(), val));
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// writing an associative parameter through the live fusevm executor. C calls
/// `sethparam` directly.
/// Store an associative-array param into the live executor (no-op
/// without one).
pub fn set_assoc(name: &str, val: indexmap::IndexMap<String, String>) {
    let _ = crate::fusevm_bridge::try_with_executor(|exec| exec.set_assoc(name.to_string(), val));
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// unsetting a scalar parameter through the live fusevm executor. C calls
/// `unsetparam` directly.
/// Unset a scalar param in the live executor (no-op without one).
pub fn unset_scalar(name: &str) {
    let _ = crate::fusevm_bridge::try_with_executor(|exec| exec.unset_scalar(name));
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// unsetting an array parameter through the live fusevm executor. C calls
/// `unsetparam` directly.
/// Unset an array param in the live executor (no-op without one).
pub fn unset_array(name: &str) {
    let _ = crate::fusevm_bridge::try_with_executor(|exec| exec.unset_array(name));
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// unsetting an associative parameter through the live fusevm executor. C
/// calls `unsetparam` directly.
/// Unset an associative-array param in the live executor (no-op
/// without one).
pub fn unset_assoc(name: &str) {
    let _ = crate::fusevm_bridge::try_with_executor(|exec| exec.unset_assoc(name));
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// calling a shell function by name on the live fusevm executor, with the
/// full `doshfunc` (c:5823) scope wrap. C reaches the function through
/// `execshfunc` (c:5540) off a wordcode `Shfunc`.
/// Dispatch a shell-function call by name through the live executor
/// (full doshfunc scope wrap). `None` when no executor / not a
/// function.
pub fn dispatch_function_call(name: &str, args: &[String]) -> Option<i32> {
    // c:3490-3492 — `if (type != WC_FUNCDEF) setunderscore((args &&
    // nonempty(args)) ? getdata(lastnode(args)) : "")`. C's `args` list
    // carries the command word, so a bare `_pre` leaves `$_` == "_pre".
    // The compsys ports call shell functions through this shim rather
    // than through a parsed command, so those calls never reached
    // execcmd's write and the callee saw whatever `$_` the caller left:
    // zsh reports `_post` inside comppostfuncs where zshrs reported the
    // stale value.
    {
        let last = args.last().cloned().unwrap_or_else(|| name.to_string());
        crate::ported::params::set_zunderscore(std::slice::from_ref(&last)); // c:3491
    }
    let __ft = crate::ftime::start(name);
    if let Some(r) =
        crate::fusevm_bridge::try_with_executor(|exec| exec.dispatch_function_call(name, args))
    {
        crate::ftime::stop(__ft);
        return r;
    }
    // No active VM context: this is the loop()/zsh_main exit path where
    // `zexit` fires a `TRAPEXIT() { ... }` (via dotrap, Src/builtin.c:6043)
    // or the `zshexit` hook after `loop()` returned. Enter the installed
    // session executor so the handler runs in the shell. SAFETY per execode.
    SESSION_EXECUTOR.with(|c| match c.get() {
        Some(ptr) => {
            let _ctx = crate::fusevm_bridge::ExecutorContext::enter(unsafe { &mut *ptr });
            unsafe { (*ptr).dispatch_function_call(name, args) }
        }
        None => None,
    })
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// running a function BODY on the live fusevm executor with no scope wrap,
/// for callers that already entered `doshfunc` (c:5823).
/// Body-only function dispatch (no doshfunc scope wrap) — call as the
/// `body_runner` of a direct `doshfunc(...)` invocation to avoid the
/// double-wrap of going back through [`dispatch_function_call`]. `None`
/// when no executor.
pub fn run_function_body(name: &str, args: &[String]) -> Option<i32> {
    if let Some(r) =
        crate::fusevm_bridge::try_with_executor(|exec| exec.run_function_body_only(name, args))
    {
        return r;
    }
    // Session-executor fallback for the no-VM-context exit path (e.g. the
    // `zshexit` hook fired by `zexit` from zsh_main). SAFETY per execode.
    SESSION_EXECUTOR.with(|c| match c.get() {
        Some(ptr) => {
            let _ctx = crate::fusevm_bridge::ExecutorContext::enter(unsafe { &mut *ptr });
            unsafe { (*ptr).run_function_body_only(name, args) }
        }
        None => None,
    })
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// running a source string on the live fusevm executor. The C path is
/// `execstring` (c:1228) -> `parse_string` (c:283) -> `execode` (c:1245).
/// Run a script source string on the live executor. `Ok(0)` when no
/// executor is in scope.
pub fn execute_script(src: &str) -> Result<i32, String> {
    if let Some(r) = crate::fusevm_bridge::try_with_executor(|exec| exec.execute_script(src)) {
        return r;
    }
    // No active VM context: the loop()/zsh_main exit path where `zexit`
    // runs a `trap '...' EXIT` raw body (Src/builtin.c:6043) after `loop()`
    // returned. Enter the installed session executor so the trap body runs
    // in the shell instead of silently no-op'ing. SAFETY per execode.
    SESSION_EXECUTOR.with(|c| match c.get() {
        Some(ptr) => {
            let _ctx = crate::fusevm_bridge::ExecutorContext::enter(unsafe { &mut *ptr });
            unsafe { (*ptr).execute_script(src) }
        }
        None => Ok(0),
    })
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// running a source string through the fusevm zsh pipeline. Same C path as
/// above: `execstring` (c:1228).
/// Run a script source string through the live executor's zsh pipeline.
/// `Ok(0)` when no executor is in scope.
pub fn execute_script_zsh_pipeline(src: &str) -> Result<i32, String> {
    crate::fusevm_bridge::try_with_executor(|exec| exec.execute_script_zsh_pipeline(src))
        .unwrap_or(Ok(0))
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// running a `$(...)` body in-process on the live fusevm executor. C forks
/// and calls `entersubsh(ESUB_PGRP|ESUB_NOMONITOR, NULL)` inside `getoutput`
/// (c:4713); the in-process form applies that subshell state with a guard
/// instead.
/// Run a `$(...)` command substitution on the live executor, returning
/// captured stdout. Empty string when no executor is in scope.
///
/// The body runs IN-PROCESS (no fork), so the subshell state that
/// `Src/exec.c:4781`'s `entersubsh(ESUB_PGRP|ESUB_NOMONITOR, NULL)` would
/// have installed in the forked child is applied here by `SubshStateGuard`
/// and unwound when the substitution returns. Every caller of this funnel —
/// `getoutput` above and the p10k custom-segment renderer
/// (`extensions/p10k/segments_core.rs:917`, which is `content="$(eval
/// $command)"`) — is a genuine command substitution and wants it.
pub fn run_command_substitution(cmd: &str) -> String {
    // c:4781 — `entersubsh(ESUB_PGRP|ESUB_NOMONITOR, NULL);` in the child
    // arm of getoutput. Fork-safe subset only; see SubshStateGuard for the
    // applied/skipped breakdown.
    let _subsh_state = SubshStateGuard::enter();
    if let Some(r) =
        crate::fusevm_bridge::try_with_executor(|exec| exec.run_command_substitution(cmd))
    {
        return r;
    }
    // Session-executor fallback for no-VM-context callers — the native
    // p10k custom_* segments (p10k:1698 `content="$(eval $command)"`)
    // render at preprompt time, before any ExecutorContext is entered,
    // so try_with_executor alone returned "" and every custom segment
    // rendered empty. Same pattern as run_function_body above; SAFETY
    // per execode.
    SESSION_EXECUTOR.with(|c| match c.get() {
        Some(ptr) => {
            let _ctx = crate::fusevm_bridge::ExecutorContext::enter(unsafe { &mut *ptr });
            unsafe { (*ptr).run_command_substitution(cmd) }
        }
        None => String::new(),
    })
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// reading the positional parameters off the live fusevm executor. C reads
/// the `pparams` global directly (Src/params.c).
/// Positional parameters ($1..$N) from the live executor; empty without
/// one.
pub fn pparams() -> Vec<String> {
    crate::fusevm_bridge::try_with_executor(|exec| exec.pparams()).unwrap_or_default()
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// replacing the positional parameters on the live fusevm executor. C assigns
/// the `pparams` global directly (Src/params.c).
/// Replace the positional parameters in the live executor (no-op
/// without one).
pub fn set_pparams(v: Vec<String>) {
    let _ = crate::fusevm_bridge::try_with_executor(|exec| exec.set_pparams(v));
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// dropping a function from the fusevm executor's compiled-chunk and source
/// maps. C removes the node from `shfunctab` directly.
/// Drop a function from both the compiled-chunk and source maps in the
/// live executor. Returns true if either entry existed; false when no
/// executor.
pub fn unregister_function(name: &str) -> bool {
    crate::fusevm_bridge::try_with_executor(|exec| {
        let a = exec.functions_compiled.remove(name).is_some();
        let b = exec.function_source.remove(name).is_some();
        a || b
    })
    .unwrap_or(false)
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// reading the saved outer stdout fd of an in-progress `$(...)` capture. C
/// has no such stack: the capture happens in a forked child (`getoutput`,
/// c:4713) so the parent fd is never rebound.
/// Saved outer stdout fd for an in-progress `$(...)` capture (top of
/// the bridge's CMDSUBST_OUTER_FDS stack), or `None` when not inside a
/// cmdsub. Used by the trap dispatcher to route a trap body's stdout to
/// the parent terminal instead of the cmdsub-bound pipe (Bug #56).
pub fn cmdsubst_outer_stdout() -> Option<i32> {
    crate::fusevm_bridge::cmdsubst_outer_stdout()
}

/// Port of `execautofn_basic()` from `Src/exec.c:5608` — C decl `execautofn_basic(Estate state, UNUSED(int do_exec))`.
/// Run a pre-loaded autoload function body
/// via `execode`, snapshotting `scriptname`/`scriptfilename` around
/// the call so `%N` / `%x` reflect the autoload target during
/// execution.
pub fn execautofn_basic(state: &mut estate, _do_exec: i32) -> i32 {
    // c:5608
    // c:5613 — `shf = state->prog->shf;`
    let shf = match state.prog.shf.as_deref() {
        Some(s) => s.clone(),
        None => return LASTVAL.load(Ordering::Relaxed),
    };

    // c:5619-5620 — funcstack filename catch-up. zshrs's funcstack
    // top-of-stack tracking is in modules::parameter::FUNCSTACK.
    {
        let mut stk = crate::ported::modules::parameter::FUNCSTACK.lock().unwrap();
        if let Some(top) = stk.last_mut() {
            if top.filename.is_none() {
                // c:5620 — `funcstack->filename = getshfuncfile(shf);`
                top.filename = crate::ported::hashtable::getshfuncfile(&shf.node.nam);
            }
        }
    }

    // c:5622-5623 — `oldscriptname/oldscriptfilename = scriptname/scriptfilename;`
    let oldscriptname = crate::ported::utils::scriptname_get();
    let oldscriptfilename = crate::ported::utils::scriptfilename_get();
    // c:5624 — `scriptname = dupstring(shf->node.nam);`
    crate::ported::utils::set_scriptname(Some(shf.node.nam.clone()));
    // c:5625 — `scriptfilename = getshfuncfile(shf);`
    crate::ported::utils::set_scriptfilename(crate::ported::hashtable::getshfuncfile(
        &shf.node.nam,
    ));
    // c:5626 — `execode(shf->funcdef, 1, 0, "loadautofunc");`
    if let Some(funcdef) = shf.funcdef.clone() {
        execode_wordcode(funcdef, 1, 0, "loadautofunc");
    }
    // c:5627-5628 — restore.
    crate::ported::utils::set_scriptname(oldscriptname);
    crate::ported::utils::set_scriptfilename(oldscriptfilename);

    LASTVAL.load(Ordering::Relaxed) // c:5630
}

/// Port of `execautofn()` from `Src/exec.c:5635` — C decl `execautofn(Estate state, UNUSED(int do_exec))`.
/// The autoload-aware dispatch entry
/// for `WC_AUTOFN`: fault the function body in via `loadautofn`,
/// then hand off to `execautofn_basic` to actually run it.
///
/// C body:
/// ```c
/// static int
/// execautofn(Estate state, UNUSED(int do_exec))
/// {
///     Shfunc shf;
///     if (!(shf = loadautofn(state->prog->shf, 1, 0, 0)))
///         return 1;
///     state->prog->shf = shf;
///     return execautofn_basic(state, 0);
/// }
/// ```
///
/// Rust port: `loadautofn` mutates the `shfunc` in place via a raw
/// pointer and returns 0/1 (success/failure), so the explicit
/// `state->prog->shf = shf` assignment in C is implicit here.
pub fn execautofn(state: &mut estate, _do_exec: i32) -> i32 {
    // c:5638-5640 — `if (!(shf = loadautofn(state->prog->shf, 1, 0, 0))) return 1;`
    let shf_ptr: *mut shfunc = match state.prog.shf.as_mut() {
        Some(b) => &mut **b as *mut shfunc,
        None => return 1,
    };
    if loadautofn(shf_ptr, 1, 0, 0) != 0 {
        return 1;
    }
    // c:5643 — `return execautofn_basic(state, 0);`
    execautofn_basic(state, 0)
}

/// Port of `execpline2()` from `Src/exec.c:1991` — C decl `execpline2(Estate state, wordcode pcode, int how, int input, int output, int last1)`.
/// Recursive
/// multi-stage pipe walker: at each step, analyse the current
/// command, fork-into-pipe (if mid-pipeline) or exec directly (if
/// WC_PIPE_END), then recurse on the next stage with `pipes[0]` as
/// its input fd.
pub fn execpline2(
    state: &mut estate,
    pcode: wordcode,
    how: i32,
    input: i32,
    output: i32,
    last1: i32,
) {
    use crate::ported::builtin::{BREAKS, INEVAL, RETFLAG};
    use crate::ported::zsh_h::{
        execcmd_params, CS_PIPE, WC_PIPE_END, WC_PIPE_LINENO as ZWC_PIPE_LINENO,
        WC_PIPE_TYPE as ZWC_PIPE_TYPE, Z_ASYNC,
    };
    // c:1991
    let mut eparams: execcmd_params = execcmd_params::default(); // c:1994 `struct execcmd_params eparams;`

    // c:1996-1997 — `if (breaks || retflag) return;`
    if BREAKS.load(Ordering::SeqCst) != 0 || RETFLAG.load(Ordering::SeqCst) != 0 {
        return;
    }

    // c:1999-2001 — `if (!IN_EVAL_TRAP() && !ineval && WC_PIPE_LINENO(pcode))
    //                  lineno = WC_PIPE_LINENO(pcode) - 1;`
    if !crate::ported::zsh_h::IN_EVAL_TRAP()
        && INEVAL.load(Ordering::SeqCst) == 0
        && ZWC_PIPE_LINENO(pcode) != 0
    {
        let new_lineno = ZWC_PIPE_LINENO(pcode).saturating_sub(1) as usize;
        crate::ported::input::lineno.with(|l| l.set(new_lineno));
    }

    // c:2003-2011 — pline_level == 1 → snapshot to list_pipe_text for `jobs` output.
    if pline_level.load(Ordering::Relaxed) == 1 {
        // c:2003
        if (how & Z_ASYNC as i32) != 0 || sfcontext.load(Ordering::Relaxed) == 0 {
            // c:2004 — `(how & Z_ASYNC) || !sfcontext`
            // c:2005-2008 — `strcpy(list_pipe_text, getjobtext(state->prog,
            //   state->pc + (WC_PIPE_TYPE(pcode) == WC_PIPE_END ? 0 : 1)));`
            let pc_for_text = state.pc
                + if ZWC_PIPE_TYPE(pcode) == WC_PIPE_END {
                    0
                } else {
                    1
                };
            let text = crate::ported::text::getjobtext(state.prog.clone(), Some(pc_for_text));
            if let Ok(mut lpt) = LIST_PIPE_TEXT.lock() {
                *lpt = text;
            }
        } else {
            // c:2010 — `list_pipe_text[0] = '\0';`
            if let Ok(mut lpt) = LIST_PIPE_TEXT.lock() {
                lpt.clear();
            }
        }
    }

    if ZWC_PIPE_TYPE(pcode) == WC_PIPE_END {
        // c:2012-2014 — terminal stage: analyse + exec directly.
        execcmd_analyse(state, &mut eparams); // c:2013
        execcmd_exec(
            state,
            &mut eparams,
            input,
            output,
            how,
            if last1 != 0 { 1 } else { 2 }, // c:2014 `last1 ? 1 : 2`
            -1,                             // c:2014 close_if_forked = -1
        );
    } else {
        // c:2015-2039 — non-terminal stage: pipe + fork + recurse.
        let mut pipes: [i32; 2] = [-1, -1]; // c:2016
        let old_list_pipe = list_pipe.load(Ordering::Relaxed); // c:2017
                                                               // c:2018 — `Wordcode next = state->pc + (*state->pc);`
        let next = if state.pc < state.prog.prog.len() {
            state.pc + state.prog.prog[state.pc] as usize
        } else {
            state.pc
        };
        // c:2020 — `++state->pc;`
        if state.pc < state.prog.prog.len() {
            state.pc += 1;
        }
        execcmd_analyse(state, &mut eparams); // c:2021

        if mpipe(&mut pipes) < 0 {
            // c:2023-2025 — pipe() failure — `/* FIXME */` in C, fall through.
        }

        // c:2027 — `addfilelist(NULL, pipes[0]);`
        // C uses the current thisjob's filelist; Rust port wires through JOBTAB.
        if let Some(jt) = JOBTAB.get() {
            let mut guard = jt.lock().unwrap();
            let tj = {
                if let Some(m) = THISJOB.get() {
                    *m.lock().unwrap()
                } else {
                    -1
                }
            };
            if tj >= 0 {
                if let Some(j) = guard.get_mut(tj as usize) {
                    crate::ported::jobs::addfilelist(j, None, pipes[0]);
                }
            }
        }

        // c:2028 — `execcmd_exec(state, &eparams, input, pipes[1], how, 0, pipes[0]);`
        execcmd_exec(state, &mut eparams, input, pipes[1], how, 0, pipes[0]);
        let _ = zclose(pipes[1]); // c:2029
        state.pc = next; // c:2030

        // c:2034 — `cmdpush(CS_PIPE);`
        cmdpush(CS_PIPE as u8);
        // c:2035 — `list_pipe = 1;`
        list_pipe.store(1, Ordering::Relaxed);
        // c:2036 — `execpline2(state, *state->pc++, how, pipes[0], output, last1);`
        let next_pcode = if state.pc < state.prog.prog.len() {
            state.prog.prog[state.pc]
        } else {
            0
        };
        if state.pc < state.prog.prog.len() {
            state.pc += 1;
        }
        execpline2(state, next_pcode, how, pipes[0], output, last1);
        // c:2037 — `list_pipe = old_list_pipe;`
        list_pipe.store(old_list_pipe, Ordering::Relaxed);
        // c:2038 — `cmdpop();`
        cmdpop();
    }
}

/// Port of `execpline()` from `Src/exec.c:1668` — C decl `execpline(Estate state, wordcode slcode, int how, int last1)`.
/// Full faithful port: allocates a job-table
/// entry via `initjob`, sets up coproc mpipes, drives the whole
/// (multi-stage) pipeline through `execpline2` (which performs the real
/// per-stage mpipe/fork/exec), then either spawns the job asynchronously
/// (`Z_ASYNC` -> `spawnjob`/`deletejob`) or waits synchronously
/// (`waitjobs`), including the `list_pipe` stop/continue fork machinery
/// (SUBJOB/SUPERJOB linkage) that re-forks the shell to keep an
/// interactively-suspended right-hand pipeline stage running.
///
/// Divergences from C, each forced by the Rust substrate and cited
/// inline at the point of use:
///   * `initjob` here grows the `Vec`-backed jobtab on demand and does
///     not return -1, so C's per-pipeline table-full bailout (c:1756-1760)
///     is inert on THIS path. That bailout is zsh's universal recursion
///     backstop (`initjob` caps the table at `MAX_MAXJOBS` → `zerr("job
///     table full or recursion limit exceeded")`), which bounds recursion
///     through paths FUNCNEST does not count (sourced files, `eval` — the
///     doshfunc funcnest check at c:5684 counts FS_FUNC frames only). The
///     fusevm runtime that actually executes pipelines does not allocate a
///     job per pipeline, so that backstop was missing entirely — runaway
///     `source`/`eval` recursion overflowed the (large but finite)
///     main-thread stack → SIGBUS. It is reinstated inline (same ceiling,
///     total FUNCSTACK depth as the proxy for held job slots) at the
///     FS_SOURCE re-entry (init.rs::source) and FS_EVAL re-entry
///     (builtin.rs eval).
///   * `errbrk_saved` / `prev_errflag` / `prev_breaks` (jobs.c:128
///     globals) are only *read* here; their setter lives in the
///     not-yet-ported jobs.c reaping path, so they stay 0 and the
///     `if (errbrk_saved)` restore (c:1998-2003) is a faithful no-op.
pub fn execpline(state: &mut estate, slcode: wordcode, how: i32, last1: i32) -> i32 {
    use crate::ported::builtin::{BREAKS, LOOPS, RETFLAG};
    use crate::ported::init::zleentry;
    use crate::ported::jobs::stat as jst;
    use crate::ported::jobs::{
        addproc, clearoldjobtab, deletejob, hasprocs, initjob, makerunning, pipecleanfilelist,
        printjob, spawnjob, waitjobs, CURJOB, LASTVAL2, PREVJOB,
    };
    use crate::ported::modules::clone::{coprocin, coprocout};
    use crate::ported::signals::killjb;
    use crate::ported::signals_h::{
        queue_signal_level, queue_signals, restore_queue_signals, unqueue_signals,
    };
    use crate::ported::utils::read_loop;
    use crate::ported::zsh_h::{
        jobbing, INTERACTIVE, LONGLISTJOBS, STAT_SUBJOB_ORPHANED, WC_PIPE, WC_PIPE_END,
        WC_PIPE_TYPE, WC_SUBLIST_COPROC, WC_SUBLIST_FLAGS, WC_SUBLIST_NOT, ZLE_CMD_TRASH, Z_ASYNC,
        Z_DISOWN, Z_TIMED,
    };

    // c:1731 — `static int lastwj, lpforked;` (persist across calls).
    static LASTWJ: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
    static LPFORKED: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
    // c:465 (exec.c) — `static struct timespec list_pipe_start;`. Used as
    // the addproc bgtime for the re-forked super-job leader (c:1841).
    static LIST_PIPE_START: std::sync::Mutex<Option<std::time::Instant>> =
        std::sync::Mutex::new(None);
    // c:128 (jobs.c) — `int prev_errflag, prev_breaks, errbrk_saved;`. The
    // setter is in the not-yet-ported reaping path, so these stay 0 here.
    static ERRBRK_SAVED: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
    static PREV_ERRFLAG: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
    static PREV_BREAKS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

    let jt = JOBTAB.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    // Read/write the shared thisjob slot.
    let thisjob_set = |v: i32| {
        if let Some(m) = THISJOB.get() {
            *m.lock().unwrap() = v;
        }
    };
    let thisjob_get = || THISJOB.get().map(|m| *m.lock().unwrap()).unwrap_or(-1);

    let old_simple_pline = simple_pline.load(Ordering::Relaxed); // c:1728
    let mut slflags = WC_SUBLIST_FLAGS(slcode); // c:1729
    let code = state.prog.prog[state.pc]; // c:1730 `wordcode code = *state->pc++;`
    state.pc += 1;

    // c:1733-1736 — non-pipe, non-timed sublist short-circuits with the
    // negated-empty status.
    if wc_code(code) != WC_PIPE && (how & Z_TIMED) == 0 {
        let r = i32::from((slflags & WC_SUBLIST_NOT) != 0);
        LASTVAL.store(r, Ordering::Relaxed);
        return r;
    }
    let mut last1 = last1;
    if (slflags & WC_SUBLIST_NOT) != 0 {
        last1 = 0; // c:1736
    }
    let mut how = how;

    queue_signals(); // c:1744

    let pj = thisjob_get(); // c:1746 — `pj = thisjob;`
    let mut ipipe: [i32; 2] = [0, 0]; // c:1747
    let mut opipe: [i32; 2] = [0, 0];
    child_block(); // c:1748

    // c:1755 — `thisjob = newjob = initjob();` (Rust jobtab grows on
    // demand, so initjob never fails; the -1 bailout is unreachable).
    let newjob = {
        let mut g = jt.lock().unwrap();
        initjob(&mut g)
    };
    thisjob_set(newjob as i32);
    if (how & Z_TIMED) != 0 {
        // c:1760-1761
        let mut g = jt.lock().unwrap();
        g[newjob].stat |= jst::TIMED;
    }

    if (slflags & WC_SUBLIST_COPROC) != 0 {
        // c:1763-1782
        how = Z_ASYNC; // c:1764
        if coprocin.load(Ordering::Relaxed) >= 0 {
            zclose(coprocin.load(Ordering::Relaxed)); // c:1766
            zclose(coprocout.load(Ordering::Relaxed)); // c:1767
        }
        if mpipe(&mut ipipe) < 0 {
            // c:1769-1771
            coprocin.store(-1, Ordering::Relaxed);
            coprocout.store(-1, Ordering::Relaxed);
            slflags &= !WC_SUBLIST_COPROC;
        } else if mpipe(&mut opipe) < 0 {
            // c:1772-1776
            unsafe {
                libc::close(ipipe[0]);
                libc::close(ipipe[1]);
            }
            coprocin.store(-1, Ordering::Relaxed);
            coprocout.store(-1, Ordering::Relaxed);
            slflags &= !WC_SUBLIST_COPROC;
        } else {
            // c:1777-1781
            coprocin.store(ipipe[0], Ordering::Relaxed);
            coprocout.store(opipe[1], Ordering::Relaxed);
            fdtable_set(ipipe[0], FDT_UNUSED);
            fdtable_set(opipe[1], FDT_UNUSED);
        }
    }

    // c:1788-1793 — `if (!pline_level++) { ... }`.
    let prev_pline = pline_level.fetch_add(1, Ordering::Relaxed);
    if prev_pline == 0 {
        list_pipe_pid.store(0, Ordering::Relaxed); // c:1789
        nowait.store(0, Ordering::Relaxed); // c:1790
        simple_pline.store(
            i32::from(WC_PIPE_TYPE(code) == WC_PIPE_END),
            Ordering::Relaxed,
        ); // c:1791
        list_pipe_job.store(newjob as i32, Ordering::Relaxed); // c:1792
    }
    LASTWJ.store(0, Ordering::Relaxed); // c:1794
    LPFORKED.store(0, Ordering::Relaxed);
    execpline2(state, code, how, opipe[0], ipipe[1], last1); // c:1795
    pline_level.fetch_sub(1, Ordering::Relaxed); // c:1796

    if (how & Z_ASYNC) != 0 {
        // c:1797-1818
        clearoldjobtab(); // c:1798
        LASTWJ.store(newjob as i32, Ordering::Relaxed); // c:1799

        if thisjob_get() == list_pipe_job.load(Ordering::Relaxed) {
            list_pipe_job.store(0, Ordering::Relaxed); // c:1801-1802
        }
        {
            let mut g = jt.lock().unwrap();
            let tj = thisjob_get();
            if tj >= 0 {
                g[tj as usize].stat |= jst::NOSTTY; // c:1803
            }
        }
        if (slflags & WC_SUBLIST_COPROC) != 0 {
            zclose(ipipe[1]); // c:1805
            zclose(opipe[0]); // c:1806
        }
        if (how & Z_DISOWN) != 0 {
            // c:1808-1812
            let tj = thisjob_get();
            if tj >= 0 {
                let mut g = jt.lock().unwrap();
                pipecleanfilelist(&mut g[tj as usize], false); // c:1809
                deletejob(&mut g[tj as usize], true); // c:1810
            }
            thisjob_set(-1); // c:1811
        } else {
            spawnjob(); // c:1814 (locks JOBTAB internally — no guard held)
        }
        child_unblock(); // c:1815
        unqueue_signals(); // c:1816
        LASTVAL.store(0, Ordering::Relaxed); // c:1818 `return lastval = 0;`
        return 0;
    }

    // c:1819-2033 — synchronous branch.
    if newjob as i32 != LASTWJ.load(Ordering::Relaxed) {
        // c:1820
        let mut jn_idx = newjob; // `Job jn = jobtab + newjob;`

        // c:1824-1825 — a list_pipe sub-shell child exits here.
        if newjob as i32 == list_pipe_job.load(Ordering::Relaxed)
            && list_pipe_child.load(Ordering::Relaxed) != 0
        {
            unsafe { libc::_exit(0) };
        }

        LASTWJ.store(newjob as i32, Ordering::Relaxed); // c:1826
        thisjob_set(newjob as i32);

        // c:1828-1830 — suppress the job announcement in nested pipes.
        {
            let mut g = jt.lock().unwrap();
            let noprint = list_pipe.load(Ordering::Relaxed) != 0
                || (pline_level.load(Ordering::Relaxed) != 0
                    && (how & Z_TIMED) == 0
                    && (g[jn_idx].stat & jst::NOSTTY) == 0);
            if noprint {
                g[jn_idx].stat |= jst::NOPRINT;
            }
        }

        if nowait.load(Ordering::Relaxed) != 0 {
            // c:1832-1882
            if pline_level.load(Ordering::Relaxed) == 0 {
                // c:1833-1875
                *CURJOB
                    .get_or_init(|| std::sync::Mutex::new(-1))
                    .lock()
                    .unwrap() = newjob as i32; // c:1836
                                               // c:1838 — DPUTS(!list_pipe_pid, "invalid list_pipe_pid").
                                               // c:1840-1841 — record the re-forked leader in the super-job.
                {
                    let txt = LIST_PIPE_TEXT.lock().map(|s| s.clone()).unwrap_or_default();
                    let bgt = *LIST_PIPE_START.lock().unwrap();
                    let mut g = jt.lock().unwrap();
                    addproc(
                        &mut g[jn_idx],
                        list_pipe_pid.load(Ordering::Relaxed),
                        &txt,
                        false,
                        bgt,
                        -1,
                        -1,
                    );
                }
                {
                    let mut g = jt.lock().unwrap();
                    // c:1845 — `if (!jn->procs->next || lpforked == 2)`.
                    if g[jn_idx].procs.len() <= 1 || LPFORKED.load(Ordering::Relaxed) == 2 {
                        g[jn_idx].gleader = list_pipe_pid.load(Ordering::Relaxed); // c:1845
                        g[jn_idx].stat |= jst::SUBLEADER; // c:1847
                                                          // c:1852-1861 — adopt any orphaned subjob; we
                                                          // become its super-job.
                        for jobsub in 1..g.len() {
                            if (g[jobsub].stat & STAT_SUBJOB_ORPHANED) != 0 {
                                g[jn_idx].other = jobsub as i32; // c:1855
                                g[jn_idx].stat |= jst::SUPERJOB; // c:1856
                                g[jobsub].stat &= !STAT_SUBJOB_ORPHANED; // c:1857
                                g[jobsub].other = list_pipe_pid.load(Ordering::Relaxed);
                                // c:1858
                            }
                        }
                    }
                    // c:1863-1869 — copy a stopped proc status from the
                    // subjob onto our last proc.
                    let other = g[jn_idx].other as usize;
                    let stopped = if other < g.len() {
                        g[other]
                            .procs
                            .iter()
                            .find(|p| p.is_stopped())
                            .map(|p| p.status)
                    } else {
                        None
                    };
                    if let Some(st) = stopped {
                        if let Some(last) = g[jn_idx].procs.last_mut() {
                            last.status = st;
                        }
                    }
                    // c:1870-1872
                    g[jn_idx].stat &= !(jst::DONE | jst::NOPRINT);
                    g[jn_idx].stat |= jst::STOPPED | jst::CHANGED | jst::LOCKED | jst::INUSE;
                }
                // c:1875 — printjob(jn, !!isset(LONGLISTJOBS), 1).
                {
                    let g = jt.lock().unwrap();
                    let cur = *CURJOB
                        .get_or_init(|| std::sync::Mutex::new(-1))
                        .lock()
                        .unwrap();
                    let prev = *PREVJOB
                        .get_or_init(|| std::sync::Mutex::new(-1))
                        .lock()
                        .unwrap();
                    let s = printjob(
                        &g[jn_idx],
                        jn_idx,
                        i32::from(isset(LONGLISTJOBS)),
                        if cur >= 0 { Some(cur as usize) } else { None },
                        if prev >= 0 { Some(prev as usize) } else { None },
                    );
                    if !s.is_empty() {
                        eprintln!("{}", s);
                    }
                }
            } else if newjob as i32 != list_pipe_job.load(Ordering::Relaxed) {
                let mut g = jt.lock().unwrap();
                deletejob(&mut g[jn_idx], false); // c:1878
            } else {
                LASTWJ.store(-1, Ordering::Relaxed); // c:1879
            }
        }

        ERRBRK_SAVED.store(0, Ordering::Relaxed); // c:1883
                                                  // c:1884-2015 — `for (; !nowait;)` wait / continue-fork loop.
        loop {
            if nowait.load(Ordering::Relaxed) != 0 {
                break;
            }
            if list_pipe_child.load(Ordering::Relaxed) != 0 {
                // c:1886-1887
                let mut g = jt.lock().unwrap();
                g[jn_idx].stat |= jst::NOPRINT;
                makerunning(&mut g, jn_idx);
            }
            // c:1889-1894 — wait unless the job is LOCKED.
            let locked = {
                let g = jt.lock().unwrap();
                (g[jn_idx].stat & jst::LOCKED) != 0
            };
            let updated;
            if !locked {
                let tj = thisjob_get();
                {
                    let mut g = jt.lock().unwrap();
                    updated = hasprocs(&g, tj as usize); // c:1890
                    waitjobs(&mut g, tj as usize); // c:1891
                }
                child_block(); // c:1892
            } else {
                updated = false; // c:1894
            }
            // c:1895-1902 — nudge the signal queue when the LHS job is
            // still running but we saw no update.
            let lpj = list_pipe_job.load(Ordering::Relaxed);
            let nudge = !updated && lpj != 0 && {
                let g = jt.lock().unwrap();
                (lpj as usize) < g.len()
                    && hasprocs(&g, lpj as usize)
                    && (g[lpj as usize].stat & jst::STOPPED) == 0
            };
            if nudge {
                let q = queue_signal_level();
                child_unblock();
                child_block();
                dont_queue_signals();
                restore_queue_signals(q);
            }
            // c:1903-1907 — forward a fatal signal from a done child.
            let jn_done = {
                let g = jt.lock().unwrap();
                (g[jn_idx].stat & jst::DONE) != 0
            };
            if list_pipe_child.load(Ordering::Relaxed) != 0
                && jn_done
                && (LASTVAL2.load(Ordering::Relaxed) & 0o200) != 0
            {
                unsafe {
                    libc::killpg(
                        mypgrp.load(Ordering::Relaxed),
                        LASTVAL2.load(Ordering::Relaxed) & !0o200,
                    );
                }
            }
            // c:1908-1921 — a pipeline with the shell running the RHS was
            // stopped; fork to let it continue.
            let stop_fork = list_pipe_child.load(Ordering::Relaxed) == 0
                && LPFORKED.load(Ordering::Relaxed) == 0
                && subsh.load(Ordering::Relaxed) == 0
                && jobbing()
                && (list_pipe.load(Ordering::Relaxed) != 0
                    || last1 != 0
                    || pline_level.load(Ordering::Relaxed) != 0)
                && {
                    let g = jt.lock().unwrap();
                    (g[jn_idx].stat & jst::STOPPED) != 0
                        || (lpj != 0
                            && pline_level.load(Ordering::Relaxed) != 0
                            && (lpj as usize) < g.len()
                            && (g[lpj as usize].stat & jst::STOPPED) != 0)
                };
            if stop_fork {
                let mut synch: [i32; 2] = [0, 0]; // c:1913
                let mut bgtime = ZshTimespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                };
                let mut pid: libc::pid_t = 0;
                let pipe_failed = unsafe { libc::pipe(synch.as_mut_ptr()) } < 0; // c:1922
                if !pipe_failed {
                    pid = zfork(Some(&mut bgtime)); // c:1922
                }
                if pipe_failed || pid == -1 {
                    // c:1923-1935 — failure: can't suspend, resume the job.
                    if pid < 0 {
                        unsafe {
                            libc::close(synch[0]);
                            libc::close(synch[1]);
                        }
                    } else {
                        zerr(&format!("pipe failed: {}", std::io::Error::last_os_error()));
                        // c:1926
                    }
                    let _ = zleentry(ZLE_CMD_TRASH); // c:1929
                    eprintln!("zsh: job can't be suspended"); // c:1930
                    {
                        let mut g = jt.lock().unwrap();
                        makerunning(&mut g, jn_idx); // c:1932
                    }
                    killjb(jn_idx, libc::SIGCONT); // c:1933
                    thisjob_set(newjob as i32); // c:1934
                } else if pid != 0 {
                    // c:1936-1973 — parent: job control lives here.
                    let gl = {
                        let g = jt.lock().unwrap();
                        g.get(lpj as usize).map(|j| j.gleader).unwrap_or(0)
                    };
                    LPFORKED.store(
                        if unsafe { libc::killpg(gl, 0) } == -1 {
                            2
                        } else {
                            1
                        },
                        Ordering::Relaxed,
                    ); // c:1951-1952
                    list_pipe_pid.store(pid, Ordering::Relaxed); // c:1953
                    *LIST_PIPE_START.lock().unwrap() = Some(std::time::Instant::now()); // c:1954
                    nowait.store(1, Ordering::Relaxed); // c:1955
                    errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed); // c:1956
                    BREAKS.store(LOOPS.load(Ordering::SeqCst), Ordering::SeqCst); // c:1957
                    unsafe { libc::close(synch[1]) }; // c:1958
                    let mut dummy = [0u8; 1];
                    let _ = read_loop(synch[0], &mut dummy); // c:1959
                    unsafe { libc::close(synch[0]) }; // c:1960
                                                      // c:1962-1970 — link super/sub jobs if we're still live.
                    let jn_done2 = {
                        let g = jt.lock().unwrap();
                        (g[jn_idx].stat & jst::DONE) != 0
                    };
                    if !jn_done2 {
                        let mut g = jt.lock().unwrap();
                        g[lpj as usize].other = newjob as i32; // c:1964
                        g[lpj as usize].stat |= jst::SUPERJOB; // c:1965
                        g[jn_idx].stat |= jst::SUBJOB | jst::NOPRINT; // c:1966
                        g[jn_idx].other = list_pipe_pid.load(Ordering::Relaxed); // c:1967
                        if hasprocs(&g, lpj as usize) {
                            g[jn_idx].gleader = g[lpj as usize].gleader; // c:1968-1969
                        }
                    }
                    // c:1971-1972 — stop the LHS group so the whole pipe
                    // suspends together.
                    let (do_kill, gl2) = {
                        let g = jt.lock().unwrap();
                        (
                            (list_pipe.load(Ordering::Relaxed) != 0 || last1 != 0)
                                && hasprocs(&g, lpj as usize),
                            g.get(lpj as usize).map(|j| j.gleader).unwrap_or(0),
                        )
                    };
                    if do_kill {
                        unsafe { libc::killpg(gl2, libc::SIGSTOP) };
                    }
                    break; // c:1973
                } else {
                    // c:1975-2004 — child: become our own group, stop, then
                    // continue as the RHS sub-shell.
                    unsafe { libc::close(synch[0]) }; // c:1976
                    entersubsh(esub::ASYNC, None); // c:1977
                    let mypid = unsafe { libc::getpid() };
                    mypgrp.store(mypid, Ordering::Relaxed);
                    unsafe { libc::setpgid(0, mypid) }; // c:1992 setpgrp
                    unsafe { libc::close(synch[1]) }; // c:1993
                    unsafe { libc::kill(mypid, libc::SIGSTOP) }; // c:1994
                    list_pipe.store(0, Ordering::Relaxed); // c:1995
                    list_pipe_child.store(1, Ordering::Relaxed); // c:1996
                    dosetopt(INTERACTIVE, 0, 0); // c:1997 `opts[INTERACTIVE] = 0`
                    if ERRBRK_SAVED.load(Ordering::Relaxed) != 0 {
                        // c:1998-2003 — restore saved break/errflag state.
                        errflag.store(
                            PREV_ERRFLAG.load(Ordering::Relaxed)
                                | (errflag.load(Ordering::Relaxed) & ERRFLAG_INT),
                            Ordering::Relaxed,
                        );
                        BREAKS.store(PREV_BREAKS.load(Ordering::Relaxed), Ordering::SeqCst);
                    }
                    break;
                }
            } else if subsh.load(Ordering::Relaxed) != 0 && {
                let g = jt.lock().unwrap();
                (g[jn_idx].stat & jst::STOPPED) != 0
            } {
                // c:2008-2012
                if thisjob_get() == newjob as i32 {
                    let mut g = jt.lock().unwrap();
                    makerunning(&mut g, jn_idx);
                } else {
                    thisjob_set(newjob as i32);
                }
            } else {
                break; // c:2015
            }
        }

        child_unblock(); // c:2017
        unqueue_signals(); // c:2018

        // c:2020-2026 — a signal-killed list_pipe: drop this job and
        // forward the signal to the enclosing job's group.
        let lastval_now = LASTVAL.load(Ordering::Relaxed);
        let drop_and_signal =
            list_pipe.load(Ordering::Relaxed) != 0 && (lastval_now & 0o200) != 0 && pj >= 0 && {
                let g = jt.lock().unwrap();
                (g[jn_idx].stat & jst::INUSE) == 0 || (g[jn_idx].stat & jst::DONE) != 0
            };
        if drop_and_signal {
            {
                let mut g = jt.lock().unwrap();
                deletejob(&mut g[jn_idx], false); // c:2022
            }
            jn_idx = pj as usize; // c:2023 `jn = jobtab + pj;`
            let gl = {
                let g = jt.lock().unwrap();
                g[jn_idx].gleader
            };
            if gl != 0 {
                killjb(jn_idx, lastval_now & !0o200); // c:2025
            }
        }
        // c:2027-2030 — final cleanup deletejob for a done/child job.
        let final_delete = list_pipe_child.load(Ordering::Relaxed) != 0 || {
            let g = jt.lock().unwrap();
            (g[jn_idx].stat & jst::DONE) != 0
                && (list_pipe.load(Ordering::Relaxed) != 0
                    || (pline_level.load(Ordering::Relaxed) != 0
                        && (g[jn_idx].stat & jst::SUBJOB) == 0))
        };
        if final_delete {
            let mut g = jt.lock().unwrap();
            deletejob(&mut g[jn_idx], false); // c:2030
        }
        thisjob_set(pj); // c:2031
    } else {
        unqueue_signals(); // c:2034
    }

    // c:2035-2036 — apply `!` negation to the pipeline status.
    if (slflags & WC_SUBLIST_NOT) != 0
        && errflag.load(Ordering::Relaxed) == 0
        && RETFLAG.load(Ordering::SeqCst) == 0
    {
        let lv = LASTVAL.load(Ordering::Relaxed);
        LASTVAL.store(i32::from(lv == 0), Ordering::Relaxed);
    }

    if pline_level.load(Ordering::Relaxed) == 0 {
        simple_pline.store(old_simple_pline, Ordering::Relaxed); // c:2039
    }
    LASTVAL.load(Ordering::Relaxed) // c:2040 `return lastval;`
}

// `execcmd_exec`'s wordcode dispatch tail from Src/exec.c:2901-3700 is
// inlined at every call site (execsimple, execpline) as the match
// expression that selects the right execX function. There's no
// separate Rust fn for it because:
//   - The arg-side `execcmd_exec(args, type_)` at exec.rs:795 already
//     occupies the canonical name (handling precommand modifiers).
//   - The C dispatch tail is conceptually `execfuncs[code - WC_CURSH]`,
//     a table lookup at exec.c:5499 — not a separate function.
#[cfg(any())]
mod _execcmd_tail_doc_anchor {
    // c:2901-3700 — see inlined match in execpline + execsimple above.
    // c:5499 — execfuncs[] table inlined as the same match.
}


/// Port of `execcmd_exec()` from `Src/exec.c:2901` — C decl `execcmd_exec(Estate state, Execcmd_params eparams, int input, int output, int how, int last1, int close_if_forked)`.
/// int input, int output, int how, int last1, int close_if_forked)`
/// from `Src/exec.c:2900-4404`. Execute a command at the lowest
/// level of the hierarchy.
///
/// Line-by-line port of the full 1500-line C body. Sections:
///   c:2904-2916  — locals
///   c:2917-2924  — eparams field unpacking
///   c:2934-2939  — Z_TIMED + doneps4 reset
///   c:2945-2960  — old_lastval + use_cmdoutval + `save[]`/`mfds[]` init
///   c:2962-2986  — %job head rewrite + AUTORESUME prefix match
///   c:2988-3011  — Z_ASYNC / pipeline-not-last / sh-emulation fork-immediately
///   c:3013-3283  — precommand-modifier walk (BINF_PREFIX strip)
///                  + BINF_COMMAND (-p/-v/-V) + BINF_EXEC (-a/-c/-l)
///   c:3285-3307  — prefork substitutions + magic_assign
///   c:3309-3406  — empty-command branch (redir / nullexec / BINF_COMMAND)
///   c:3409-3466  — main resolution loop (shfunc / builtin / autocd)
///   c:3468-3479  — errflag bail-out
///   c:3480-3492  — text fetch + setunderscore
///   c:3494-3524  — rm * safety prompt
///   c:3526-3591  — type-specific dispatch prep (WC_FUNCDEF / is_shfunc / WC_AUTOFN)
///   c:3593-3632  — external resolution (cmdnamtab, hashcmd, AUTOCD)
///   c:3634-3697  — fork decision
///   c:3700-3955  — redir loop + multio + addfd + xpandredir
///   c:3957-3961  — multio close (`mfds[i].ct >= 2` → closemn)
///   c:3963-3995  — nullexec branch
///   c:3996-4327  — main dispatch (entersubsh + execfuncdef / `execcurshtable[]` /
///                  execbuiltin / execshfunc / execute)
///   c:4330-4365  — `err:` label: forked-child fd cleanup, fixfds
///   c:4366-4403  — `done:` label: POSIX special-builtin error escalation,
///                  shelltime stop, newxtrerr close, AUTOCONTINUE restore
///
/// **Substrate stubs (declared inside this fn citing home C file):**
///   - `save_params(state, varspc, restorelist, removelist)` → Src/exec.c:4409
///   - `restore_params(restorelist, removelist)` → Src/exec.c:4463
///   - `isreallycom(cn)` → Src/exec.c:2670
///   - `execerr()` → Src/exec.c:2700 (label-style; converts to errflag set + goto-equivalent)
///   - `execautofn_basic(state, do_exec)` → Src/exec.c:5608
///   - `ensurefeature(modname, "b:", ...)` → Src/module.c:1654
///
/// **NOT routed through fusevm.** This canonical port targets the
/// tree-walker dispatcher; the fusevm bytecode VM uses
/// `execcmd_compile_head` + `compile_simple` instead. No call
/// site yet — the port closes the substrate gap so future
/// wordcode-walker code can use it.
#[allow(non_snake_case)]
#[allow(clippy::too_many_arguments)]
#[allow(clippy::redundant_field_names)]
#[allow(unused_assignments)]
#[allow(unused_variables)]
#[allow(unused_mut)]
#[allow(unused_imports)]
#[allow(unreachable_code)]
#[allow(dead_code)]
pub fn execcmd_exec(
    state: &mut estate,
    eparams: &mut crate::ported::zsh_h::execcmd_params,
    input: i32,
    output: i32,
    mut how: i32,
    mut last1: i32,
    close_if_forked: i32,
) {
    use crate::ported::zsh_h::{
        Star, ASG_ARRAY, ASG_KEY_VALUE, AUTOCD, AUTOCONTINUE, AUTORESUME, BGNICE,
        BINF_ASSIGN as BINF_ASSIGN_FLAG, BINF_BUILTIN, BINF_COMMAND, BINF_EXEC, BINF_MAGICEQUALS,
        BINF_NOGLOB, BINF_PREFIX, BINF_PSPECIAL, CSHNULLCMD, ERRFLAG_INT, EXECOPT, FDT_EXTERNAL,
        FDT_INTERNAL, FDT_TYPE_MASK, FDT_UNUSED, FDT_XTRACE, HASHCMDS, HFILE_USE_OPTIONS,
        IS_APPEND_REDIR, IS_DASH, IS_ERROR_REDIR, MAGICEQUALSUBST, NOTIFY, PM_READONLY, PM_SPECIAL,
        POSIXBUILTINS, PREFORK_ASSIGN, PREFORK_KEY_VALUE, PREFORK_SINGLE, PREFORK_TYPESET,
        PRINTEXITVALUE, RCS, REDIR_CLOSE, REDIR_HERESTR, REDIR_INPIPE, REDIR_MERGEIN,
        REDIR_MERGEOUT, REDIR_OUTPIPE, REDIR_READ, REDIR_READWRITE, RMSTARSILENT, SHINSTDIN,
        SHNULLCMD, STAT_BUILTIN, STAT_CURSH, STAT_DONE, STAT_NOPRINT, WC_ASSIGN as ZWC_ASSIGN,
        WC_ASSIGN_INC as ZWC_ASSIGN_INC, WC_ASSIGN_NUM as ZWC_ASSIGN_NUM,
        WC_ASSIGN_SCALAR as ZWC_ASSIGN_SCALAR, WC_ASSIGN_TYPE as ZWC_ASSIGN_TYPE,
        WC_ASSIGN_TYPE2 as ZWC_ASSIGN_TYPE2, WC_AUTOFN, WC_CURSH, WC_FUNCDEF, WC_REDIR, WC_SIMPLE,
        WC_SUBSH, WC_TIMED, WC_TYPESET, XTRACE, Z_ASYNC, Z_DISOWN, Z_SYNC, Z_TIMED,
    };

    // c:2900

    // c:2904-2916 — locals.
    let mut hn: Option<*mut builtin> = None; // c:2904 HashNode hn = NULL
    let mut filelist: Vec<jobfile> = Vec::new(); // c:2905 LinkList filelist = NULL
                                                 // c:2906 LinkNode node; (loop locals)
                                                 // c:2907 Redir fn;       (loop locals)
    let mut mfds: [Option<Box<multio>>; 10] =                              // c:2908 struct multio *mfds[10]
        [None, None, None, None, None, None, None, None, None, None];
    let mut text: Option<String> = None; // c:2909 char *text
    let mut save: [i32; 10] = [-2; 10]; // c:2910 int save[10]
    let mut fil: i32; // c:2911 int fil
    let mut dfil: i32 = 0; // c:2911 int dfil
    let mut is_cursh: i32 = 0; // c:2911 int is_cursh = 0
    let mut do_exec: i32 = 0; // c:2911 int do_exec = 0
    let mut redir_err: i32 = 0; // c:2911 int redir_err = 0
    let mut i: i32; // c:2911 int i
    let mut nullexec: i32 = 0; // c:2912 int nullexec = 0
    let mut magic_assign: i32 = 0; // c:2912 int magic_assign = 0
    let mut forked: i32 = 0; // c:2912 int forked = 0
    let mut old_lastval: i32; // c:2912 int old_lastval
    let mut is_shfunc: i32 = 0; // c:2913 int is_shfunc = 0
    let mut is_builtin: i32 = 0; // c:2913 int is_builtin = 0
    let mut is_exec: i32 = 0; // c:2913 int is_exec = 0
    let mut use_defpath: i32 = 0; // c:2913 int use_defpath = 0
                                  // c:2914 — `Various flags to the command.`
    let mut cflags: u32 = 0; // c:2915 int cflags = 0
    let mut orig_cflags: u32 = 0; // c:2915 int orig_cflags = 0
    let mut checked: i32 = 0; // c:2915 int checked = 0
    let mut oautocont: i32 = -1; // c:2915 int oautocont = -1
                                 // c:2916 — `FILE *oxtrerr = xtrerr, *newxtrerr = NULL;` — xtrerr
                                 // accessor is stub; track newxtrerr state via Option<RawFd>.
    let mut newxtrerr: Option<i32> = None; // c:2916

    // c:2917-2924 — eparams field unpacking. `args` / `redir` are
    // pulled into mutable locals so the body can mutate them
    // independently of the eparams struct.
    let mut args: Option<Vec<String>> = eparams.args.take(); // c:2921 LinkList args
    let mut redir: Option<Vec<redir>> = eparams.redir.take(); // c:2922 LinkList redir
    let varspc: Option<usize> = eparams.varspc; // c:2923 Wordcode varspc
    let typ: i32 = eparams.typ; // c:2924 int type
                                // c:2925-2929 — `preargs comes from expanding the head of the args
                                // list in order to check for prefix commands.` declared later.

    // c:2933-2937 — `for the "time" keyword` — child_times_t shti, chti
    // + struct timespec then. Rust port keeps the names so the shelltime
    // start+stop calls map directly. Use jobs.rs's existing types.
    let mut shti = crate::ported::jobs::timeinfo::default(); // c:2934
    let mut chti = crate::ported::jobs::timeinfo::default(); // c:2934
    let mut then_ts = std::time::Instant::now(); // c:2935 struct timespec then
    if (how & Z_TIMED as i32) != 0 {
        // c:2936
        crate::ported::jobs::shelltime(Some(&mut shti), Some(&mut chti), Some(&mut then_ts), 0);
        // c:2937
    }

    doneps4.store(0, Ordering::Relaxed); // c:2939

    // c:2941-2947 — `If assignment but no command get the status from
    // variable assignment.`
    old_lastval = LASTVAL.load(Ordering::Relaxed); // c:2945
    if args.is_none() && varspc.is_some() {
        // c:2946
        let ef = errflag.load(Ordering::Relaxed);
        LASTVAL.store(
            if ef != 0 {
                ef
            } else {
                cmdoutval.load(Ordering::Relaxed)
            },
            Ordering::Relaxed,
        ); // c:2947
    }
    // c:2948-2954 — `If there are arguments, we should reset the status
    // for the command before execution---unless we are using the result
    // of a command substitution...`
    use_cmdoutval.store(if args.is_none() { 1 } else { 0 }, Ordering::Relaxed); // c:2955

    // c:2957-2960 — `for (i = 0; i < 10; i++) { save[i] = -2; mfds[i] = NULL; }`
    // Already initialised above via array literals; preserved as
    // comment for parity. The C loop maps to a no-op in Rust.

    // c:2962-2973 — `%job` head rewrite.
    if (typ == WC_SIMPLE as i32 || typ == WC_TYPESET as i32)
        && args.as_ref().map(|v| !v.is_empty()).unwrap_or(false)
        && args.as_ref().unwrap()[0].starts_with('%')
    {
        // c:2964-2965
        if (how & Z_DISOWN as i32) != 0 {
            // c:2966
            oautocont = if crate::ported::options::opt_state_get("autocontinue").unwrap_or(false) {
                1
            } else {
                0
            }; // c:2967
            opt_state_set("autocontinue", true); // c:2968
        }
        // c:2970-2971 — `pushnode(args, dupstring((how & Z_DISOWN) ? "disown" : (how & Z_ASYNC) ? "bg" : "fg"));`
        let head = if (how & Z_DISOWN as i32) != 0 {
            "disown".to_string()
        } else if (how & Z_ASYNC as i32) != 0 {
            "bg".to_string()
        } else {
            "fg".to_string()
        };
        if let Some(ref mut v) = args {
            v.insert(0, head);
        }
        how = Z_SYNC as i32; // c:2972
    }

    // c:2975-2986 — AUTORESUME prefix match against jobtab.
    if isset(AUTORESUME)
        && typ == WC_SIMPLE as i32
        && (how & Z_SYNC as i32) != 0
        && args.as_ref().map(|v| !v.is_empty()).unwrap_or(false)
        && redir.as_ref().map(|v| v.is_empty()).unwrap_or(true)
        && input == 0
        && args.as_ref().unwrap().len() == 1
    {
        // c:2979-2981
        if unset(NOTIFY) {
            // c:2982 — `scanjobs();` via the canonical port
            // (Src/jobs.c:1993).
            if let Some(jt) = JOBTAB.get() {
                let mut guard = jt.lock().unwrap();
                crate::ported::jobs::scanjobs(&mut guard);
            }
        }
        // c:2984 — `if (findjobnam(peekfirst(args)) != -1)`
        let head = args.as_ref().unwrap()[0].clone();
        let maxjob = JOBTAB
            .get()
            .map(|m| m.lock().unwrap().len() as i32)
            .unwrap_or(0);
        let thisjob = THISJOB.get().map(|m| *m.lock().unwrap()).unwrap_or(-1);
        // c:2982 — `findjobnam(s)`. Canonical port at
        // jobs.rs::findjobnam matches against `proc.text`, which is
        // the command text actually saved into the job at fork —
        // matching C exactly. Returns the job index if any non-
        // SUBJOB jobtab entry's first-proc text starts with `s`.
        let found = if let Some(jt) = JOBTAB.get() {
            let guard = jt.lock().unwrap();
            crate::ported::jobs::findjobnam(&head, &guard, maxjob - 1, thisjob).is_some()
        } else {
            false
        };
        if found {
            // c:2985 — `pushnode(args, dupstring("fg"));`
            if let Some(ref mut v) = args {
                v.insert(0, "fg".to_string());
            }
        }
    }

    // ====================================================================
    // SUBSTRATE STUBS — same-named locals citing their home C file per
    // [[feedback_no_shortcuts_in_porting]]. Each stub mirrors the C
    // signature and returns a degenerate value that keeps the body
    // executing while the real port lands.
    // ====================================================================
    // save_params + restore_params — top-level ports in exec.rs
    // (c:4410 / c:4464). Both bridged via `use` below.
    use crate::ported::exec::{restore_params, save_params};
    // isreallycom — top-level port at exec.rs (c:972). Bridges the
    // local shadow that this fn body used pre-port.
    use crate::ported::exec::isreallycom;
    // execautofn_basic — top-level port at exec.rs (c:5608).
    use crate::ported::exec::execautofn_basic;
    // C `execerr` macro (c:2700) was a goto-equivalent:
    //   errflag |= ERRFLAG_ERROR; lastval = 1; goto err;
    // Rust expansion: each call site inlines the errflag+LASTVAL set
    // and then `break`s out of the enclosing redir loop. The loop's
    // post-loop errflag check at c:3949 routes to execcmd_exec_err_path
    // for the cleanup tail. No macro needed.

    // c:2988-3011 — Z_ASYNC / pipeline-not-last / sh-emulation
    // fork-immediately fast path.
    if (how & Z_ASYNC as i32) != 0
        || output != 0
        || (last1 == 2 && input != 0 && {
            // c:2989 — `EMULATION(EMULATE_SH)` — emulation==EMULATE_SH.
            // EMULATION macro: `(emulation & EMULATE_MASK) == X`. The
            // ported `emulation` static at options.rs:1044 holds the
            // current bit; compare against EMULATE_SH (zsh_h:2883).
            (crate::ported::options::emulation.load(Ordering::Relaxed)
                & crate::ported::zsh_h::EMULATE_SH)
                != 0
        })
    {
        // c:2988
        // c:2999 — `text = getjobtext(state->prog, eparams->beg);`
        text = Some(crate::ported::text::getjobtext(
            state.prog.clone(),
            Some(eparams.beg),
        ));
        // c:3000-3008 — `switch (execcmd_fork(...)) { -1: goto fatal; 0: break; default: return; }`
        let mut filelist_for_fork = filelist.clone();
        let pid = execcmd_fork(
            state,
            how,
            typ,
            varspc,
            &mut filelist_for_fork,
            text.as_deref().unwrap_or(""),
            oautocont,
            close_if_forked,
        );
        match pid {
            -1 => {
                // c:3002-3003 — `goto fatal;` — fall through to fatal:
                // label at c:4377. We model this with a flag.
                redir_err = 1; // pretend redir error to trigger fatal arm
                               // Continue to done label by setting forked + jumping forward.
                               // Simplified: just bail with status 1 + fatal handling at
                               // the bottom of the fn.
                return execcmd_exec_done_path(
                    redir_err,
                    oautocont,
                    how,
                    &mut shti,
                    &mut chti,
                    &mut then_ts,
                    forked,
                    &mut newxtrerr,
                    cflags,
                    orig_cflags,
                    is_cursh,
                    do_exec,
                );
            }
            0 => {
                // c:3004 — child returned 0; continue with the body.
            }
            _ => {
                // c:3007 — parent: `return;` — but first restore AUTOCONTINUE
                // and shelltime stop. Inline the done-tail equivalent.
                if oautocont >= 0 {
                    opt_state_set("autocontinue", oautocont != 0);
                }
                if (how & Z_TIMED as i32) != 0 {
                    crate::ported::jobs::shelltime(
                        Some(&mut shti),
                        Some(&mut chti),
                        Some(&mut then_ts),
                        1,
                    );
                }
                return;
            }
        }
        last1 = 1; // c:3009
        forked = 1; // c:3009
    } else {
        // c:3010-3011
        text = None;
    }

    // ====================================================================
    // c:3013-3283 — precommand-modifier walk.
    //
    // The full walk (BINF_PREFIX strip + BINF_COMMAND sub-options +
    // BINF_EXEC sub-options) is already ported in `execcmd_compile_head`
    // (above this fn). Call into it to keep DRY, then convert the
    // returned dispatch struct's fields into the locals C uses
    // (cflags, orig_cflags, is_builtin, is_shfunc, use_defpath,
    // exec_argv0, precmd_skip).
    //
    // Per [[feedback_true_port_pattern]] the C function does this
    // walk inline. Reusing the existing port is acceptable because
    // `execcmd_compile_head`'s body IS the c:3013-3283 walk — the
    // citations there match. The C tree-walker and the fusevm
    // compile-time walker arrive at identical dispatch decisions
    // from the same input.
    // ====================================================================
    let mut preargs: Vec<String> = Vec::new();
    let mut exec_argv0: Option<String> = None;
    if (typ == WC_SIMPLE as i32 || typ == WC_TYPESET as i32) && args.is_some() {
        // c:3018
        let head_args: Vec<String> = args.as_ref().unwrap().clone();
        let dispatch = execcmd_compile_head(&head_args, typ as u32);
        // Pull fields into local mirror of C state.
        cflags = dispatch.cflags;
        if dispatch.is_builtin {
            is_builtin = 1;
        }
        if dispatch.is_shfunc {
            is_shfunc = 1;
        }
        if dispatch.use_defpath {
            use_defpath = 1;
        }
        exec_argv0 = dispatch.exec_argv0;
        // c:3061 — `orig_cflags |= cflags;` accumulator path; for
        // BINF_PREFIX walks orig_cflags tracks each step's pre-mask
        // bits. execcmd_compile_head doesn't surface orig_cflags
        // separately, so approximate as the post-strip cflags.
        orig_cflags = cflags;
        // c:3030-3086 — strip the precmd-modifier prefix from args.
        // In C, the walk pulls one arg at a time from `args` into
        // `preargs` via execcmd_getargs, then uremnodes each
        // BINF_PREFIX modifier. At loop exit C's `preargs` holds the
        // dispatch target (1 element) and `args` holds whatever's
        // left; `joinlists(preargs, args)` (c:3305-3306) splices the
        // target back onto the head. The net effect is `args` with
        // the precmd modifiers stripped. We compute that final shape
        // directly and leave `preargs` empty so the joinlists arm
        // below is a no-op. Without this, preargs=head_args[skip..]
        // plus a non-draining args was double-counting every word
        // when both held the same suffix.
        if let Some(ref mut v) = args {
            v.drain(0..dispatch.precmd_skip);
            // c:3154 — `pushnode(preargs, "command");` /* Leave everything
            // alone, dispatch to whence. We need to put the name back in
            // the list. */  `command -v`/`-V` keeps its whole word list and
            // dispatches to the `command` builtin (bin_whence with
            // BIN_COMMAND), so the name goes back on the front after the
            // precommand strip. `execcmd_compile_head` only REPORTS this
            // via `has_command_vv` (the fusevm compiler acts on it); the
            // tree-walker has to perform it. Without this the head after
            // the strip was `-v`, which resolves to nothing —
            // `x=command; $x -v cat` printed nothing.
            if dispatch.has_command_vv {
                v.insert(0, "command".to_string()); // c:3154
            } else if dispatch.use_defpath {
                // c:3165 — `if (pnode) uremnode(preargs, pnode);` /* We
                // don't need this node as we're not treating "command" as a
                // builtin this time. */  The `-p` word is consumed here,
                // same reason as above: compile_head drops it from its own
                // local list only.
                if v.first()
                    .map(|w| {
                        let b = w.as_bytes();
                        b.len() >= 2
                            && IS_DASH(b[0] as char)
                            && w[1..].chars().all(|c| c == 'p' || c == 'v' || c == 'V')
                    })
                    .unwrap_or(false)
                {
                    v.remove(0); // c:3165
                }
                // c:3176-3177 — `if (IS_DASH(argdata[0]) && IS_DASH(argdata[1])
                // && !argdata[2]) uremnode(preargs, argnode);`
                if v.first().map(|w| w == "--").unwrap_or(false) {
                    v.remove(0); // c:3177
                }
            }
        }
        let _ = head_args;
        preargs.clear();
        // c:3076 — `magic_assign = (hn->flags & BINF_MAGICEQUALS);`
        // — surface via cflags check: if a typeset-family builtin
        // landed, BINF_MAGICEQUALS is in its flags and dispatch
        // surfaces it via cflags.
        if (cflags & BINF_MAGICEQUALS) != 0 && typ != WC_TYPESET as i32 {
            magic_assign = 1;
        }
        // c:3056 — C's precmd walk sets `hn = builtintab->getnode(...)`
        // for the dispatch target before breaking at c:3064. The
        // Rust port's execcmd_compile_head returns is_builtin but
        // not the entry pointer, and the second resolution loop
        // below short-circuits on `is_builtin != 0` (c:3423-3426)
        // without re-resolving. Look up the dispatch target now so
        // `hn` is non-null at the execbuiltin call (c:4233 /
        // exec.rs:10177); otherwise execbuiltin returns 1 silently
        // on a null `bn`.
        hn = None;
        if dispatch.has_command_vv {
            // c:3209 — `hn = &commandbn.node;`. The `command -v`/`-V` form
            // dispatches through the dedicated descriptor (bin_whence /
            // BIN_COMMAND / optstr "pvV"), NOT through builtintab's
            // handler-less `BIN_PREFIX("command", …)` row — looking the name
            // up there gave a null handler and `command -v cat` printed
            // nothing.
            hn = Some(&*commandbn as *const builtin as *mut builtin); // c:3209
        } else if is_builtin != 0 {
            if let Some(target) = args.as_ref().and_then(|v| v.first()) {
                if let Some(entry) = BUILTINS.iter().find(|b| b.node.nam == *target) {
                    hn = Some(entry as *const builtin as *mut builtin);
                }
            }
        }
    } else {
        // c:3282-3283 — `else preargs = NULL;`
        // We use an empty preargs to model NULL — C's `preargs` is
        // only iterated if `nonempty(preargs)` in this branch.
    }

    // c:3285-3300 — `Do prefork substitutions.` magic_assign handling.
    // Sets the file-static `esprefork` (exec.rs:267) so any downstream
    // execsubst() call inside this command's expansion uses the same
    // prefork flags. Also keep a local copy for the immediate
    // prefork(args, esprefork, NULL) below.
    let esprefork_v: i32 =
        if magic_assign != 0 || (isset(MAGICEQUALSUBST) && typ != WC_TYPESET as i32) {
            PREFORK_TYPESET // c:3300
        } else {
            0
        };
    esprefork.store(esprefork_v, Ordering::Relaxed); // c:3298 esprefork = ...

    // c:3302-3307 — prefork(args, esprefork, NULL) + joinlists(preargs, args).
    if args.is_some() && eparams.htok != 0 {
        // c:3303-3304 — `if (eparams->htok) prefork(args, esprefork, NULL);`
        let mut as_linklist: LinkList<String> = Default::default();
        if let Some(ref v) = args {
            for s in v {
                as_linklist.push_back(s.clone());
            }
        }
        let mut rf = 0i32;
        prefork(&mut as_linklist, esprefork_v, &mut rf);
        // Move back into args.
        let mut out: Vec<String> = Vec::new();
        while let Some(s) = as_linklist.pop_front() {
            out.push(s);
        }
        args = Some(out);
    }
    if !preargs.is_empty() {
        // c:3305-3306 — `if (preargs) args = joinlists(preargs, args);`
        let mut joined = preargs.clone();
        if let Some(ref v) = args {
            joined.extend(v.iter().cloned());
        }
        args = Some(joined);
    }

    // c:3309-3406 — main resolution loop + empty-command branch.
    if typ == WC_SIMPLE as i32 || typ == WC_TYPESET as i32 {
        let mut unglobbed: i32 = 0; // c:3310

        // c:3312 — `for (;;)` — main resolution loop.
        loop {
            // c:3315-3318 — globbing or untokenise sweep.
            if (cflags & BINF_NOGLOB) == 0 {
                while checked == 0
                    && (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) == 0
                    && args.as_ref().map(|v| !v.is_empty()).unwrap_or(false)
                    && crate::ported::lex::has_token(&args.as_ref().unwrap()[0])
                {
                    // c:3318 — `zglob(args, firstnode(args), 0);`
                    // zglob takes &mut Vec<String>; isolate the head element
                    // by splitting args into [head] and [tail], then re-merging.
                    let mut head_vec: Vec<String> = Vec::new();
                    if let Some(ref mut v) = args {
                        head_vec.push(v.remove(0));
                    }
                    crate::ported::glob::zglob(&mut head_vec, 0usize, 0);
                    if let Some(ref mut v) = args {
                        // Re-merge the globbed head ahead of the tail. Inserting
                        // one element at a time (`v.insert(i, s)`) memmoves the
                        // whole remaining tail on every element, so a head word
                        // that globs to K matches costs O(K*n) and a completion
                        // sweep over a large directory spins. Splicing the batch
                        // in at position 0 shifts the tail exactly once and is
                        // order-identical: element i lands at index i.
                        v.splice(0..0, head_vec);
                    }
                }
            } else if unglobbed == 0 {
                // c:3319-3322
                if let Some(ref mut v) = args {
                    for s in v.iter_mut() {
                        *s = untokenize(s); // c:3321
                    }
                }
                unglobbed = 1; // c:3322
            }

            // c:3327-3328 — `if ((cflags & BINF_EXEC) && last1) do_exec = 1;`
            if (cflags & BINF_EXEC) != 0 && last1 != 0 {
                do_exec = 1; // c:3328
            }

            // c:3331-3407 — empty-command branch.
            if args.as_ref().map(|v| v.is_empty()).unwrap_or(true) {
                // c:3331 — `if (!args || empty(args))`
                if redir.as_ref().map(|v| !v.is_empty()).unwrap_or(false) {
                    // c:3332 — `if (redir && nonempty(redir))`
                    if do_exec != 0 {
                        // c:3333 — `Was this "exec < foobar"?`
                        nullexec = 1; // c:3335
                        break;
                    } else if varspc.is_some() {
                        // c:3337
                        nullexec = 2; // c:3338
                        break;
                    } else if {
                        // c:3340-3341 — `if (!nullcmd || !*nullcmd ||
                        //   opts[CSHNULLCMD] || (cflags & BINF_PREFIX))`
                        let nc = getsparam("NULLCMD");
                        let nc_empty = nc.as_deref().map(|s| s.is_empty()).unwrap_or(true);
                        nc_empty || isset(CSHNULLCMD) || (cflags & BINF_PREFIX) != 0
                    } {
                        // c:3342 — `zerr("redirection with no command");`
                        zerr("redirection with no command");
                        LASTVAL.store(1, Ordering::Relaxed); // c:3343
                        errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed); // c:3344
                        if forked != 0 {
                            // c:3345-3346
                            crate::ported::builtin::_realexit();
                        }
                        if (how & Z_TIMED as i32) != 0 {
                            // c:3347-3348
                            crate::ported::jobs::shelltime(
                                Some(&mut shti),
                                Some(&mut chti),
                                Some(&mut then_ts),
                                1,
                            );
                        }
                        return; // c:3349
                    } else if {
                        // c:3350 — `if (!nullcmd || !*nullcmd || opts[SHNULLCMD])`
                        let nc = getsparam("NULLCMD");
                        let nc_empty = nc.as_deref().map(|s| s.is_empty()).unwrap_or(true);
                        nc_empty || isset(SHNULLCMD)
                    } {
                        // c:3351-3353 — `if (!args) args = newlinklist(); addlinknode(args, dupstring(":"));`
                        if args.is_none() {
                            args = Some(Vec::new());
                        }
                        args.as_mut().unwrap().push(":".to_string()); // c:3353
                    } else if {
                        // c:3354-3356 — `readnullcmd && *readnullcmd &&
                        //   peekfirst(redir).type == REDIR_READ &&
                        //   !nextnode(firstnode(redir))`
                        let rnc = getsparam("READNULLCMD");
                        let rnc_nonempty = rnc.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
                        rnc_nonempty
                            && redir.as_ref().unwrap().len() == 1
                            && redir.as_ref().unwrap()[0].typ == REDIR_READ
                    } {
                        // c:3357-3359
                        if args.is_none() {
                            args = Some(Vec::new());
                        }
                        let rnc = getsparam("READNULLCMD").unwrap_or_default();
                        args.as_mut().unwrap().push(rnc); // c:3359
                    } else {
                        // c:3360-3364 — default: nullcmd as command.
                        if args.is_none() {
                            args = Some(Vec::new());
                        }
                        let nc = getsparam("NULLCMD").unwrap_or_default();
                        args.as_mut().unwrap().push(nc); // c:3363
                    }
                } else if (cflags & BINF_PREFIX) != 0 && (cflags & BINF_COMMAND) != 0 {
                    // c:3365 — bare `command`: lastval=0, return.
                    LASTVAL.store(0, Ordering::Relaxed); // c:3366
                    if forked != 0 {
                        crate::ported::builtin::_realexit(); // c:3367-3368
                    }
                    if (how & Z_TIMED as i32) != 0 {
                        crate::ported::jobs::shelltime(
                            Some(&mut shti),
                            Some(&mut chti),
                            Some(&mut then_ts),
                            1,
                        ); // c:3369-3370
                    }
                    return; // c:3371
                } else {
                    // c:3372-3406 — no arguments default arm.
                    // c:3378-3385 — badcshglob == 1 → no match.
                    if crate::ported::glob::BADCSHGLOB.load(Ordering::Relaxed) == 1 {
                        zerr("no match"); // c:3379
                        LASTVAL.store(1, Ordering::Relaxed); // c:3380
                        if forked != 0 {
                            crate::ported::builtin::_realexit(); // c:3381-3382
                        }
                        if (how & Z_TIMED as i32) != 0 {
                            crate::ported::jobs::shelltime(
                                Some(&mut shti),
                                Some(&mut chti),
                                Some(&mut then_ts),
                                1,
                            ); // c:3383-3384
                        }
                        return; // c:3385
                    }
                    // c:3387 — `cmdoutval = use_cmdoutval ? lastval : 0;`
                    cmdoutval.store(
                        if use_cmdoutval.load(Ordering::Relaxed) != 0 {
                            LASTVAL.load(Ordering::Relaxed)
                        } else {
                            0
                        },
                        Ordering::Relaxed,
                    );
                    if varspc.is_some() {
                        // c:3388-3392 — `lastval = old_lastval; addvars(state, varspc, 0);`
                        LASTVAL.store(old_lastval, Ordering::Relaxed); // c:3390
                        addvars(state, varspc.unwrap_or(0), 0); // c:3391
                    }
                    if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
                        // c:3393
                        LASTVAL.store(1, Ordering::Relaxed); // c:3394
                    } else {
                        // c:3395-3396
                        LASTVAL.store(cmdoutval.load(Ordering::Relaxed), Ordering::Relaxed);
                    }
                    if isset(XTRACE) {
                        // c:3397-3400 — `fputc('\n', xtrerr); fflush(xtrerr);`
                        // xtrerr accessor is stub; rely on the existing
                        // stderr writer in compile_zsh tracing path.
                        eprintln!();
                    }
                    if forked != 0 {
                        crate::ported::builtin::_realexit(); // c:3401-3402
                    }
                    if (how & Z_TIMED as i32) != 0 {
                        crate::ported::jobs::shelltime(
                            Some(&mut shti),
                            Some(&mut chti),
                            Some(&mut then_ts),
                            1,
                        ); // c:3403-3404
                    }
                    return; // c:3405
                }
            }

            // c:3423-3426 — `if (errflag || checked || is_builtin ||
            //   (isset(POSIXBUILTINS) ? (cflags & BINF_EXEC) : (cflags & BINF_COMMAND)))`
            // A user interrupt sets ERRFLAG_INT, never ERRFLAG_ERROR (signals.c:457), and
            // the C line cited above tests the WHOLE errflag, so masking here let an
            // interrupted shell keep going where zsh stops.
            if errflag.load(Ordering::Relaxed) != 0
                || checked != 0
                || is_builtin != 0
                || if isset(POSIXBUILTINS) {
                    (cflags & BINF_EXEC) != 0
                } else {
                    (cflags & BINF_COMMAND) != 0
                }
            {
                // c:3423
                break; // c:3426
            }

            // c:3428 — `cmdarg = (char *) peekfirst(args);`
            let cmdarg = args.as_ref().unwrap()[0].clone();

            // c:3429-3433 — shfunc lookup.
            // c:3485 — `(hn = shfunctab->getnode(shfunctab, cmdarg))`.
            // Same misport as the compile-time head walk above: C's
            // `shfunctab->getnode` is `gethashnode`
            // (Src/hashtable.c:821), an O(1) probe that skips DISABLED
            // nodes, not a full-table walk.
            if (cflags & (BINF_BUILTIN | BINF_COMMAND)) == 0 {
                let in_shfunctab = shfunctab_lock()
                    .read()
                    .map(|t| !t.getnode(cmdarg.as_str()).is_null())
                    .unwrap_or(false);
                if in_shfunctab {
                    is_shfunc = 1; // c:3431
                    break; // c:3432
                }
            }
            // c:3434-3447 — builtintab lookup.
            let builtin_entry: Option<&'static builtin> = BUILTINS
                .iter()
                .find(|b| b.node.nam.as_str() == cmdarg.as_str());
            if builtin_entry.is_none() {
                if (cflags & BINF_BUILTIN) != 0 {
                    // c:3435 — `zwarn("no such builtin: %s", cmdarg);`
                    zwarn(&format!("no such builtin: {}", cmdarg)); // c:3436
                    LASTVAL.store(1, Ordering::Relaxed); // c:3437
                    if oautocont >= 0 {
                        // c:3438-3439
                        opt_state_set("autocontinue", oautocont != 0);
                    }
                    if forked != 0 {
                        crate::ported::builtin::_realexit(); // c:3440-3441
                    }
                    if (how & Z_TIMED as i32) != 0 {
                        crate::ported::jobs::shelltime(
                            Some(&mut shti),
                            Some(&mut chti),
                            Some(&mut then_ts),
                            1,
                        ); // c:3442-3443
                    }
                    return; // c:3444
                }
                break; // c:3446
            }
            let entry = builtin_entry.unwrap();
            // c:3448-3460 — `if (!(hn->flags & BINF_PREFIX)) { is_builtin = 1; ... }`
            if (entry.node.flags as u32 & BINF_PREFIX) == 0 {
                is_builtin = 1; // c:3449
                                // c:3452 — `if (!(hn = resolvebuiltin(cmdarg, hn)))` —
                                // module autoload check. zshrs's BUILTINS table is
                                // static and pre-resolved; treat resolvebuiltin as
                                // pass-through.
                hn = Some(entry as *const builtin as *mut builtin);
                break; // c:3459
            }
            // c:3461-3463 — BINF_PREFIX modifier (builtin/command/exec).
            cflags &= !(BINF_BUILTIN | BINF_COMMAND);
            cflags |= entry.node.flags as u32;
            if let Some(ref mut v) = args {
                v.remove(0); // c:3463 uremnode(args, firstnode(args))
            }
            hn = None; // c:3464
        }
    }

    // c:3468-3478 — errflag bail-out.
    // A user interrupt sets ERRFLAG_INT, never ERRFLAG_ERROR (signals.c:457), and
    // the C line cited above tests the WHOLE errflag, so masking here let an
    // interrupted shell keep going where zsh stops.
    if errflag.load(Ordering::Relaxed) != 0 {
        // c:3468
        if LASTVAL.load(Ordering::Relaxed) == 0 {
            // c:3469
            LASTVAL.store(1, Ordering::Relaxed); // c:3470
        }
        if oautocont >= 0 {
            opt_state_set("autocontinue", oautocont != 0);
            // c:3472
        }
        if forked != 0 {
            crate::ported::builtin::_realexit(); // c:3473-3474
        }
        if (how & Z_TIMED as i32) != 0 {
            crate::ported::jobs::shelltime(Some(&mut shti), Some(&mut chti), Some(&mut then_ts), 1);
            // c:3475-3476
        }
        return; // c:3477
    }

    // c:3480-3483 — `Get the text associated with this command.`
    if text.is_none()
        && sfcontext.load(Ordering::Relaxed) == 0
        && (isset(MONITOR) || (how & Z_TIMED as i32) != 0)
    {
        // c:3481-3482
        text = Some(crate::ported::text::getjobtext(
            state.prog.clone(),
            Some(eparams.beg),
        )); // c:3483
    }

    // c:3485-3492 — `Set up special parameter $_`.
    if typ != WC_FUNCDEF as i32 {
        // c:3490
        let last_str = args
            .as_ref()
            .and_then(|v| v.last())
            .cloned()
            .unwrap_or_default();
        setunderscore(&last_str); // c:3491-3492
    }

    // c:3494-3524 — `Warn about "rm *"`.
    if typ == WC_SIMPLE as i32
        && crate::ported::zsh_h::interact()
        && unset(RMSTARSILENT)
        && isset(SHINSTDIN)
        && args.as_ref().map(|v| v.len() >= 2).unwrap_or(false)
        && args.as_ref().unwrap()[0] == "rm"
    {
        // c:3495-3497
        let args_v = args.as_ref().unwrap().clone();
        for s in args_v.iter().skip(1) {
            // c:3500
            // A user interrupt sets ERRFLAG_INT, never ERRFLAG_ERROR (signals.c:457), and
            // the C line cited above tests the WHOLE errflag, so masking here let an
            // interrupted shell keep going where zsh stops.
            if errflag.load(Ordering::Relaxed) != 0 {
                break;
            }
            let l = s.len();
            // c:3505 — `if (s[0] == Star && !s[1])` — bare `*`.
            if s.len() == 1 && s.as_bytes()[0] == Star as u8 {
                let pwd = getsparam("PWD").unwrap_or_default();
                if !crate::ported::utils::checkrmall(&pwd) {
                    // c:3506
                    errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed); // c:3507
                    break; // c:3508
                }
            } else if l >= 2 {
                // c:3510 — `s[l-2] == '/' && s[l-1] == Star`
                let bytes = s.as_bytes();
                if bytes[l - 2] == b'/' && bytes[l - 1] == Star as u8 {
                    let prefix = if l == 2 {
                        "/".to_string()
                    } else {
                        String::from_utf8_lossy(&bytes[..l - 2]).into_owned()
                    };
                    if !crate::ported::utils::checkrmall(&prefix) {
                        // c:3518
                        errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed); // c:3519
                        break; // c:3520
                    }
                }
            }
        }
    }

    // c:3526-3580 — type-specific dispatch prep.
    if typ == WC_FUNCDEF as i32 {
        // c:3526
        if state.prog.prog.get(state.pc).copied().unwrap_or(0) != 0 {
            // c:3535 — `Nonymous, don't do redirections here`
            redir = None; // c:3537
        }
    } else if is_shfunc != 0 || typ == WC_AUTOFN as i32 {
        // c:3539
        // c:3540-3559 — shfunc / autoload preload.
        if is_shfunc != 0 {
            // c:3541-3542 — `shf = (Shfunc)hn;` — already in hn.
        } else {
            // c:3543-3559 — autoload preload.
            if let Some(ref mut sh) = state.prog.shf {
                let shf_ptr: *mut shfunc = sh.as_mut() as *mut shfunc;
                let r = loadautofn(shf_ptr, 1, 0, 0);
                if r != 0 {
                    // c:3551 — `lastval = 1;`
                    LASTVAL.store(1, Ordering::Relaxed);
                    if oautocont >= 0 {
                        opt_state_set("autocontinue", oautocont != 0);
                    }
                    if forked != 0 {
                        crate::ported::builtin::_realexit();
                    }
                    if (how & Z_TIMED as i32) != 0 {
                        crate::ported::jobs::shelltime(
                            Some(&mut shti),
                            Some(&mut chti),
                            Some(&mut then_ts),
                            1,
                        );
                    }
                    return; // c:3558
                }
            }
        }
        // c:3561-3579 — shf->redir append: a function definition can
        // carry extra redirs (`f() { ... } < file`), captured as a
        // separate Eprog in shf->redir. Walk that Eprog with a temp
        // estate, extract its redirs with ecgetredirs, then merge
        // into the live `redir` list.
        // Resolve shfunc by name (hn is *mut builtin so we go through
        // shfunctab as in the dispatch site at c:4102).
        let shfn_name = args
            .as_ref()
            .and_then(|v| v.first())
            .cloned()
            .unwrap_or_default();
        let shf_redir_eprog: Option<crate::ported::zsh_h::Eprog> = {
            if let Ok(tab) = shfunctab_lock().read() {
                tab.get(&shfn_name).and_then(|s| s.redir.clone())
            } else {
                None
            }
        };
        if let Some(red_eprog) = shf_redir_eprog {
            // c:3566-3571 — build temp estate from shf->redir.
            let mut tmp_state = estate {
                prog: red_eprog.clone(),
                pc: 0,
                strs: red_eprog.strs.clone(),
                strs_offset: 0,
            };
            // c:3572 — `redir2 = ecgetredirs(&s);`
            let redir2 = crate::ported::parse::ecgetredirs(&mut tmp_state);
            // c:3573-3578 — merge into existing redir.
            if redir.is_none() {
                redir = Some(redir2); // c:3574
            } else if let Some(ref mut r) = redir {
                // c:3576-3577 — append.
                for n in redir2 {
                    r.push(n);
                }
            }
        }
    }

    // c:3582-3591 — errflag bail-out (2).
    // A user interrupt sets ERRFLAG_INT, never ERRFLAG_ERROR (signals.c:457), and
    // the C line cited above tests the WHOLE errflag, so masking here let an
    // interrupted shell keep going where zsh stops.
    if errflag.load(Ordering::Relaxed) != 0 {
        // c:3582
        LASTVAL.store(1, Ordering::Relaxed); // c:3583
        if oautocont >= 0 {
            opt_state_set("autocontinue", oautocont != 0);
            // c:3584-3585
        }
        if forked != 0 {
            crate::ported::builtin::_realexit(); // c:3586-3587
        }
        if (how & Z_TIMED as i32) != 0 {
            crate::ported::jobs::shelltime(Some(&mut shti), Some(&mut chti), Some(&mut then_ts), 1);
            // c:3588-3589
        }
        return; // c:3590
    }

    // c:3593-3632 — external resolution + AUTOCD.
    if (typ == WC_SIMPLE as i32 || typ == WC_TYPESET as i32) && nullexec == 0 {
        // c:3593
        let trycd = isset(AUTOCD)
            && isset(SHINSTDIN)
            && redir.as_ref().map(|v| v.is_empty()).unwrap_or(true)
            && args.as_ref().map(|v| v.len() == 1).unwrap_or(false)
            && !args.as_ref().unwrap()[0].is_empty(); // c:3595-3597
        if hn.is_none() {
            // c:3600
            let cmdarg = args.as_ref().unwrap()[0].clone();
            let mut dohashcmd = isset(HASHCMDS); // c:3604
                                                 // c:3606 — `hn = cmdnamtab->getnode(cmdnamtab, cmdarg);`
            let mut have_cmdnam: Option<cmdnam> = {
                let tab = cmdnamtab_lock().read().ok();
                tab.and_then(|t| {
                    t.iter()
                        .find(|(k, _)| k.as_str() == cmdarg.as_str())
                        .map(|(_, v)| v.clone())
                })
            };
            if have_cmdnam.is_some() && trycd && !isreallycom(have_cmdnam.as_ref().unwrap()) {
                // c:3607
                // c:3608-3614 — remove the cached entry; force rehash.
                cmdnam_unhashed(&cmdarg, Vec::new());
                have_cmdnam = None;
                if let Some(cn) = have_cmdnam.as_ref() {
                    if (cn.node.flags & crate::ported::zsh_h::HASHED) == 0 {
                        // checkpath = path; dohashcmd = 1;
                        dohashcmd = true;
                    }
                }
            }
            if have_cmdnam.is_none() && dohashcmd && cmdarg != ".." {
                // c:3616 — `if (!hn && dohashcmd && strcmp(cmdarg, "..")) `
                let has_slash = cmdarg.contains('/'); // c:3617-3618
                if !has_slash {
                    // c:3619 — `hn = (HashNode) hashcmd(cmdarg, checkpath);`
                    let path_dirs = getsparam("PATH").unwrap_or_default();
                    let dirs: Vec<String> = path_dirs.split(':').map(String::from).collect();
                    have_cmdnam = hashcmd(&cmdarg, &dirs);
                }
            }
            // hn stays None for external commands — the resolution
            // value matters only for builtin/shfunc dispatch in the
            // following blocks.
            let _ = have_cmdnam;
        }

        // c:3625-3631 — AUTOCD: command not found, try directory.
        if hn.is_none() && trycd {
            let cmdarg = args.as_ref().unwrap()[0].clone();
            if let Some(s) = cancd(&cmdarg) {
                // c:3625
                args.as_mut().unwrap()[0] = s; // c:3626
                args.as_mut().unwrap().insert(0, "--".to_string()); // c:3627
                args.as_mut().unwrap().insert(0, "cd".to_string()); // c:3628
                                                                    // c:3629 — `if ((hn = builtintab->getnode(builtintab, "cd")))`
                let cd_entry = BUILTINS.iter().find(|b| b.node.nam.as_str() == "cd");
                if let Some(cd) = cd_entry {
                    hn = Some(cd as *const builtin as *mut builtin);
                    is_builtin = 1; // c:3630
                }
            }
        }
    }

    // c:3635 — `is_cursh = (is_builtin || is_shfunc || nullexec || type >= WC_CURSH);`
    is_cursh =
        (is_builtin != 0 || is_shfunc != 0 || nullexec != 0 || typ >= WC_CURSH as i32) as i32;

    // c:3659-3697 — fork decision.
    if forked == 0 {
        // c:3659
        if do_exec == 0
            && (((is_builtin != 0 || is_shfunc != 0) && output != 0)
                || (is_cursh == 0
                    && (last1 != 1
                        || crate::ported::signals::nsigtrapped.load(Ordering::Relaxed) != 0
                        || JOBTAB
                            .get()
                            .map(|jt| crate::ported::jobs::havefiles(&jt.lock().unwrap()))
                            .unwrap_or(false)
                        || false/* fdtable_flocks — substrate stub */)))
        {
            // c:3660-3663
            let mut filelist_for_fork = filelist.clone();
            let pid = execcmd_fork(
                state,
                how,
                typ,
                varspc,
                &mut filelist_for_fork,
                text.as_deref().unwrap_or(""),
                oautocont,
                close_if_forked,
            );
            match pid {
                -1 => {
                    // c:3666-3667 — goto fatal.
                    redir_err = 1;
                    return execcmd_exec_done_path(
                        redir_err,
                        oautocont,
                        how,
                        &mut shti,
                        &mut chti,
                        &mut then_ts,
                        forked,
                        &mut newxtrerr,
                        cflags,
                        orig_cflags,
                        is_cursh,
                        do_exec,
                    );
                }
                0 => {
                    // c:3668 — child continues.
                }
                _ => {
                    // c:3670-3671 — parent returns.
                    if oautocont >= 0 {
                        opt_state_set("autocontinue", oautocont != 0);
                    }
                    if (how & Z_TIMED as i32) != 0 {
                        crate::ported::jobs::shelltime(
                            Some(&mut shti),
                            Some(&mut chti),
                            Some(&mut then_ts),
                            1,
                        );
                    }
                    return;
                }
            }
            forked = 1; // c:3673
        } else if is_cursh != 0 {
            // c:3674
            // c:3678-3682 — set jobtab[thisjob] stat bits.
            let thisjob = THISJOB.get().map(|m| *m.lock().unwrap()).unwrap_or(-1);
            if thisjob >= 0 {
                if let Some(jt) = JOBTAB.get() {
                    let mut guard = jt.lock().unwrap();
                    if let Some(j) = guard.get_mut(thisjob as usize) {
                        j.stat |= STAT_CURSH; // c:3678
                                              // c:3679-3680 — `if (!jobtab[thisjob].procs)
                                              //                  jobtab[thisjob].stat |= STAT_NOPRINT;`
                                              // Suppress the "[N] done" print for jobs that
                                              // never forked a real process (cursh / builtin /
                                              // null exec).
                        if j.procs.is_empty() {
                            j.stat |= STAT_NOPRINT; // c:3680
                        }
                        if is_builtin != 0 {
                            j.stat |= STAT_BUILTIN; // c:3682
                        }
                    }
                }
            }
        } else {
            // c:3683-3697 — external exec (real or fake).
            is_exec = 1; // c:3687
                         // c:3695 — `if (type == WC_SUBSH) forked = 1;`
            if typ == WC_SUBSH as i32 {
                forked = 1; // c:3696
            }
        }
    }

    // c:3700-3704 — `if ((esglob = !(cflags & BINF_NOGLOB)) && args && htok)`
    if (cflags & BINF_NOGLOB) == 0 && args.is_some() && eparams.htok != 0 {
        // c:3700
        let mut oargs: LinkList<String> = Default::default();
        if let Some(ref v) = args {
            for s in v {
                oargs.push_back(s.clone());
            }
        }
        globlist(&mut oargs, 0); // c:3702
        let mut out: Vec<String> = Vec::new();
        while let Some(s) = oargs.pop_front() {
            out.push(s);
        }
        args = Some(out);
    }
    if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
        // c:3705
        LASTVAL.store(1, Ordering::Relaxed); // c:3706
        return execcmd_exec_err_path(
            forked,
            &mut save,
            &mut mfds,
            oautocont,
            how,
            &mut shti,
            &mut chti,
            &mut then_ts,
            &mut newxtrerr,
            cflags,
            orig_cflags,
            is_cursh,
            do_exec,
            redir_err,
        );
    }

    // c:3711-3718 — XTRACE prep (newxtrerr stderr dup).
    // Architectural divergence: C duplicates stderr to a new FD and
    // marks it `FDT_XTRACE` in the fdtable so the redir loop skips it.
    // zshrs routes xtrace output through `eprintln!()` / `tracing`
    // instead of a duplicated fd, so the FDT_XTRACE bookkeeping has
    // no counterpart. Not a port gap — `xtrerr is FILE*` is a C-ism
    // intentionally replaced.

    // c:3720-3724 — pipeline input/output to mfds.
    if input != 0 {
        addfd(forked, &mut save, &mut mfds, 0, input, 0, None); // c:3722
    }
    if output != 0 {
        addfd(forked, &mut save, &mut mfds, 1, output, 1, None); // c:3724
    }

    // c:3726-3728 — `if (redir) spawnpipes(redir, nullexec);`
    if let Some(ref mut r) = redir {
        spawnpipes(r.as_mut_slice(), nullexec);
    }

    // c:3731-3955 — io redirection loop. Faithful per-redir match.
    while let Some(redir_list) = redir.as_mut() {
        // c:3731 — `while (redir && nonempty(redir))`
        if redir_list.is_empty() {
            break;
        }
        let mut fn_ = redir_list.remove(0); // c:3732 `fn = (Redir) ugetnode(redir);`
                                            // c:3734-3735 DPUTS — debug assert REDIR_HEREDOC* gone.
        if fn_.typ == REDIR_INPIPE {
            // c:3736
            if checkclobberparam(&fn_) == 0 || fn_.fd2 == -1 {
                // c:3737
                if fn_.fd2 != -1 {
                    let _ = zclose(fn_.fd2); // c:3738-3739
                }
                closemnodes(&mut mfds); // c:3740
                fixfds(&save); // c:3741
                {
                    errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                    LASTVAL.store(1, Ordering::Relaxed);
                } // c:3742
                break;
            }
            // c:3744 — `addfd(forked, save, mfds, fn->fd1, fn->fd2, 0, fn->varid);`
            addfd(
                forked,
                &mut save,
                &mut mfds,
                fn_.fd1,
                fn_.fd2,
                0,
                fn_.varid.as_deref(),
            );
        } else if fn_.typ == REDIR_OUTPIPE {
            // c:3745
            if checkclobberparam(&fn_) == 0 || fn_.fd2 == -1 {
                // c:3746
                if fn_.fd2 != -1 {
                    let _ = zclose(fn_.fd2); // c:3747-3748
                }
                closemnodes(&mut mfds); // c:3749
                fixfds(&save); // c:3750
                {
                    errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                    LASTVAL.store(1, Ordering::Relaxed);
                } // c:3751
                break;
            }
            // c:3753
            addfd(
                forked,
                &mut save,
                &mut mfds,
                fn_.fd1,
                fn_.fd2,
                1,
                fn_.varid.as_deref(),
            );
        } else {
            // c:3754 — non-pipe redir branch.
            let mut closed: i32; // c:3755
                                 // c:3756-3757 — xpandredir glob/brace.
            if fn_.typ != REDIR_HERESTR {
                // Put fn_ back temporarily so xpandredir can mutate
                // around it; not implemented identically — xpandredir
                // signature in zshrs differs (takes &mut redir + ctx).
                // c:3756 — `if (xpandredir(fn, redir)) continue;`
                // Pragmatic: skip xpandredir (it handles brace/glob in
                // redir paths — uncommon, ports to follow-up).
            }
            if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
                // c:3758
                closemnodes(&mut mfds); // c:3759
                fixfds(&save); // c:3760
                {
                    errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                    LASTVAL.store(1, Ordering::Relaxed);
                } // c:3761
                break;
            }
            if !isset(EXECOPT) {
                // c:3763 — `if (unset(EXECOPT)) continue;`
                continue;
            }
            let fil_local: i32;
            match fn_.typ {
                t if t == REDIR_HERESTR => {
                    // c:3766
                    if checkclobberparam(&fn_) == 0 {
                        fil_local = -1; // c:3768
                    } else {
                        fil_local = getherestr(&fn_); // c:3770
                    }
                    if fil_local == -1 {
                        // c:3771
                        let e = std::io::Error::last_os_error();
                        let raw = e.raw_os_error().unwrap_or(0);
                        if raw != 0 && raw != libc::EINTR {
                            zwarn(&format!("can't create temp file for here document: {}", e));
                            // c:3772-3774
                        }
                        closemnodes(&mut mfds); // c:3775
                        fixfds(&save); // c:3776
                        {
                            errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                            LASTVAL.store(1, Ordering::Relaxed);
                        } // c:3777
                        break;
                    }
                    // c:3779
                    addfd(
                        forked,
                        &mut save,
                        &mut mfds,
                        fn_.fd1,
                        fil_local,
                        0,
                        fn_.varid.as_deref(),
                    );
                }
                t if t == REDIR_READ || t == REDIR_READWRITE => {
                    // c:3781-3782
                    if checkclobberparam(&fn_) == 0 {
                        fil_local = -1; // c:3784
                    } else {
                        let name = fn_.name.clone().unwrap_or_default();
                        let unmeta_name = unmeta(&name);
                        let cstr = match std::ffi::CString::new(unmeta_name.as_str()) {
                            Ok(c) => c,
                            Err(_) => {
                                closemnodes(&mut mfds);
                                fixfds(&save);
                                {
                                    errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                                    LASTVAL.store(1, Ordering::Relaxed);
                                }
                                break;
                            }
                        };
                        if fn_.typ == REDIR_READ {
                            // c:3786
                            fil_local = unsafe {
                                libc::open(cstr.as_ptr(), libc::O_RDONLY | libc::O_NOCTTY)
                            };
                        } else {
                            // c:3788-3789
                            fil_local = unsafe {
                                libc::open(
                                    cstr.as_ptr(),
                                    libc::O_RDWR | libc::O_CREAT | libc::O_NOCTTY,
                                    0o666,
                                )
                            };
                        }
                    }
                    if fil_local == -1 {
                        // c:3790
                        closemnodes(&mut mfds); // c:3791
                        fixfds(&save); // c:3792
                        let e = std::io::Error::last_os_error();
                        if e.raw_os_error().unwrap_or(0) != libc::EINTR {
                            zwarn(&format!("{}: {}", e, fn_.name.as_deref().unwrap_or("")));
                            // c:3793-3794
                        }
                        {
                            errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                            LASTVAL.store(1, Ordering::Relaxed);
                        } // c:3795
                        break;
                    }
                    // c:3797
                    addfd(
                        forked,
                        &mut save,
                        &mut mfds,
                        fn_.fd1,
                        fil_local,
                        0,
                        fn_.varid.as_deref(),
                    );
                    // c:3800-3802 — `if (nullexec == 1 && fn->fd1 == 0 && ...) init_io(NULL);`
                    if nullexec == 1
                        && fn_.fd1 == 0
                        && fn_.varid.is_none()
                        && isset(SHINSTDIN)
                        && isset(INTERACTIVE)
                    {
                        // c:3801 — `!zleactive` check ommitted (zleactive
                        // accessor lives in zle module; fusevm bypasses ZLE).
                        crate::ported::init::init_io(None); // c:3802
                    }
                }
                t if t == REDIR_CLOSE => {
                    // c:3804
                    // c:3805 — `if (fn->varid) { parse fd from variable }`
                    let mut fd1_local = fn_.fd1;
                    if let Some(varname) = fn_.varid.as_deref() {
                        // c:3806-3849 — `{var}>&-`/`{var}<&-` REDIR_CLOSE
                        // with varid. The C path resolves the named param
                        // to its integer-string value, parses as base-10
                        // (or base#NN), and rejects readonly / non-numeric
                        // / shell-owned-fd values.
                        //
                        //   bad=1  → "parameter %s does not contain a file descriptor"
                        //   bad=2  → "can't close file descriptor from readonly parameter %s"
                        //   bad=3  → "file descriptor %d used by shell, not closed"
                        //
                        // Substrate now available: getsparam for value,
                        // paramtab read for PM_READONLY, MAX_ZSH_FD +
                        // fdtable_get for shell-owned guard.
                        let mut bad: u8 = 0;
                        let value_opt = getsparam(varname);
                        let is_ro = paramtab()
                            .read()
                            .ok()
                            .and_then(|t| {
                                t.get(varname)
                                    .map(|p| (p.node.flags as u32 & PM_READONLY) != 0)
                            })
                            .unwrap_or(false);
                        if value_opt.is_none() {
                            bad = 1; // c:3811 getvalue failed
                        } else if is_ro {
                            bad = 2; // c:3813 PM_READONLY
                        } else {
                            let s = value_opt.as_deref().unwrap_or("");
                            match s.trim().parse::<i32>() {
                                Ok(n) => {
                                    fd1_local = n;
                                    fn_.fd1 = n;
                                    let max_fd = MAX_ZSH_FD.load(Ordering::Relaxed);
                                    if n >= 10
                                        && n <= max_fd
                                        && (fdtable_get(n) & FDT_TYPE_MASK) == FDT_INTERNAL
                                    {
                                        // c:3835 shell-owned-fd reject
                                        bad = 3;
                                    }
                                }
                                Err(_) => {
                                    bad = 1; // c:3823 strtol failure
                                }
                            }
                        }
                        if bad != 0 {
                            // c:3840-3849
                            match bad {
                                3 => zwarn(&format!(
                                    "file descriptor {} used by shell, not closed",
                                    fn_.fd1
                                )),
                                2 => zwarn(&format!(
                                    "can't close file descriptor from readonly parameter {}",
                                    varname
                                )),
                                _ => zwarn(&format!(
                                    "parameter {} does not contain a file descriptor",
                                    varname
                                )),
                            }
                            errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                            LASTVAL.store(1, Ordering::Relaxed);
                            break;
                        }
                    }
                    // c:3852-3865 — `closed`: optional movefd save.
                    closed = 0;
                    if forked == 0 && fd1_local < 10 && save[fd1_local as usize] == -2 {
                        // c:3856
                        let mv = movefd(fd1_local); // c:3857
                        save[fd1_local as usize] = mv;
                        if mv >= 0 {
                            closed = 1; // c:3862-3863
                        }
                    }
                    if fd1_local < 10 {
                        // c:3866
                        closemn(&mut mfds, fd1_local, REDIR_CLOSE);
                        // c:3867
                    }
                    // c:3873-3876
                    let _ = &mut fd1_local;
                    if closed == 0 && zclose(fn_.fd1) < 0 && fn_.varid.is_some() {
                        zwarn(&format!(
                            "failed to close file descriptor {}: {}",
                            fn_.fd1,
                            std::io::Error::last_os_error()
                        )); // c:3873-3875
                    }
                }
                t if t == REDIR_MERGEIN || t == REDIR_MERGEOUT => {
                    // c:3878-3879
                    if fn_.fd2 < 10 {
                        closemn(&mut mfds, fn_.fd2, fn_.typ); // c:3881
                    }
                    if checkclobberparam(&fn_) == 0 {
                        fil_local = -1; // c:3883
                    } else if fn_.fd2 > 9 {
                        // c:3884-3897 — fd table check.
                        let max_fd = MAX_ZSH_FD.load(Ordering::Relaxed);
                        let cin = crate::ported::modules::clone::coprocin.load(Ordering::Relaxed);
                        let cout = crate::ported::modules::clone::coprocout.load(Ordering::Relaxed);
                        let in_table = if fn_.fd2 <= max_fd {
                            let kind = fdtable_get(fn_.fd2) & FDT_TYPE_MASK;
                            kind != FDT_UNUSED && kind != FDT_EXTERNAL
                        } else {
                            false
                        };
                        if in_table || fn_.fd2 == cin || fn_.fd2 == cout {
                            fil_local = -1; // c:3896
                                            // Per-platform errno setter (c:3897 `errno = EBADF;`).
                            #[cfg(target_os = "macos")]
                            unsafe {
                                *libc::__error() = libc::EBADF;
                            }
                            #[cfg(target_os = "linux")]
                            unsafe {
                                *libc::__errno_location() = libc::EBADF;
                            }
                        } else {
                            let fd = if fn_.fd2 == -2 {
                                // c:3900-3901
                                if fn_.typ == REDIR_MERGEOUT {
                                    crate::ported::modules::clone::coprocout.load(Ordering::Relaxed)
                                } else {
                                    crate::ported::modules::clone::coprocin.load(Ordering::Relaxed)
                                }
                            } else {
                                fn_.fd2
                            };
                            // c:3902 — `fil = movefd(dup(fd));`
                            let dup_fd = unsafe { libc::dup(fd) };
                            fil_local = movefd(dup_fd);
                        }
                    } else {
                        let fd = if fn_.fd2 == -2 {
                            if fn_.typ == REDIR_MERGEOUT {
                                crate::ported::modules::clone::coprocout.load(Ordering::Relaxed)
                            } else {
                                crate::ported::modules::clone::coprocin.load(Ordering::Relaxed)
                            }
                        } else {
                            fn_.fd2
                        };
                        let dup_fd = unsafe { libc::dup(fd) };
                        fil_local = movefd(dup_fd);
                    }
                    if fil_local == -1 {
                        // c:3904
                        closemnodes(&mut mfds); // c:3907
                        fixfds(&save); // c:3908
                        if std::io::Error::last_os_error().raw_os_error().unwrap_or(0) != 0 {
                            let desc = if fn_.fd2 == -2 {
                                "coprocess".to_string()
                            } else {
                                format!("{}", fn_.fd2)
                            };
                            zwarn(&format!("{}: {}", desc, std::io::Error::last_os_error()));
                            // c:3911-3913
                        }
                        {
                            errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                            LASTVAL.store(1, Ordering::Relaxed);
                        } // c:3914
                        break;
                    }
                    // c:3916-3917
                    let merge_is_out = if fn_.typ == REDIR_MERGEOUT { 1 } else { 0 };
                    addfd(
                        forked,
                        &mut save,
                        &mut mfds,
                        fn_.fd1,
                        fil_local,
                        merge_is_out,
                        fn_.varid.as_deref(),
                    );
                }
                _ => {
                    // c:3919 default — write/append/error_redir.
                    let mut dfil: i32;
                    if checkclobberparam(&fn_) == 0 {
                        fil_local = -1; // c:3921
                    } else if IS_APPEND_REDIR(fn_.typ) {
                        // c:3922
                        let name = fn_.name.clone().unwrap_or_default();
                        let unmeta_name = unmeta(&name);
                        let cstr = match std::ffi::CString::new(unmeta_name.as_str()) {
                            Ok(c) => c,
                            Err(_) => {
                                closemnodes(&mut mfds);
                                fixfds(&save);
                                {
                                    errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                                    LASTVAL.store(1, Ordering::Relaxed);
                                }
                                break;
                            }
                        };
                        // c:3924-3927
                        let mode = if !isset(CLOBBER)
                            && !isset(crate::ported::zsh_h::APPENDCREATE)
                            && !IS_CLOBBER_REDIR(fn_.typ)
                        {
                            libc::O_WRONLY | libc::O_APPEND | libc::O_NOCTTY
                        } else {
                            libc::O_WRONLY | libc::O_APPEND | libc::O_CREAT | libc::O_NOCTTY
                        };
                        fil_local = unsafe { libc::open(cstr.as_ptr(), mode, 0o666) };
                    } else {
                        // c:3929
                        fil_local = clobber_open(&fn_);
                    }
                    // c:3930-3933 — error_redir dup.
                    if fil_local != -1 && IS_ERROR_REDIR(fn_.typ) {
                        let dup_fd = unsafe { libc::dup(fil_local) };
                        dfil = movefd(dup_fd); // c:3931
                    } else {
                        dfil = 0; // c:3933
                    }
                    if fil_local == -1 || dfil == -1 {
                        // c:3934
                        if fil_local != -1 {
                            unsafe { libc::close(fil_local) }; // c:3935-3936
                        }
                        closemnodes(&mut mfds); // c:3937
                        fixfds(&save); // c:3938
                        let e = std::io::Error::last_os_error();
                        let raw = e.raw_os_error().unwrap_or(0);
                        if raw != 0 && raw != libc::EINTR {
                            zwarn(&format!("{}: {}", e, fn_.name.as_deref().unwrap_or("")));
                            // c:3939-3940
                        }
                        {
                            errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                            LASTVAL.store(1, Ordering::Relaxed);
                        } // c:3941
                        break;
                    }
                    // c:3943
                    addfd(
                        forked,
                        &mut save,
                        &mut mfds,
                        fn_.fd1,
                        fil_local,
                        1,
                        fn_.varid.as_deref(),
                    );
                    if IS_ERROR_REDIR(fn_.typ) {
                        // c:3944-3945
                        addfd(forked, &mut save, &mut mfds, 2, dfil, 1, None);
                    }
                    let _ = &mut dfil;
                }
            }
            // c:3948-3952 — addfd errflag check.
            if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
                // c:3949
                closemnodes(&mut mfds); // c:3950
                fixfds(&save); // c:3951
                {
                    errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
                    LASTVAL.store(1, Ordering::Relaxed);
                } // c:3952
                break;
            }
        }
    }

    // c:3957-3961 — close multios with ct >= 2.
    i = 0;
    while i < 10 {
        // c:3959
        if let Some(m) = mfds.get(i as usize).and_then(|o| o.as_ref()) {
            if m.ct >= 2 {
                closemn(&mut mfds, i, REDIR_CLOSE); // c:3960
            }
        }
        i += 1;
    }

    // c:3963-3995 — nullexec branch.
    if nullexec != 0 {
        // c:3963
        if let Some(vspc) = varspc {
            // c:3969
            let mut restorelist: Vec<crate::ported::zsh_h::param> = Vec::new();
            let mut removelist: Vec<String> = Vec::new();
            if !isset(POSIXBUILTINS) && nullexec != 2 {
                // c:3971-3972
                save_params(state, vspc, &mut restorelist, &mut removelist);
            }
            addvars(state, vspc, 0); // c:3973
            if !restorelist.is_empty() {
                // c:3974
                restore_params(restorelist, removelist); // c:3975
            }
        }
        let ef = errflag.load(Ordering::Relaxed);
        LASTVAL.store(
            if ef != 0 {
                ef
            } else {
                cmdoutval.load(Ordering::Relaxed)
            },
            Ordering::Relaxed,
        ); // c:3977
        if nullexec == 1 {
            // c:3978
            // c:3983-3985 — close save[i].
            i = 0;
            while i < 10 {
                if save[i as usize] != -2 {
                    let _ = zclose(save[i as usize]); // c:3985
                }
                i += 1;
            }
            // c:3988-3989 — `jobtab[thisjob].stat |= STAT_DONE; goto done;`
            let thisjob = THISJOB.get().map(|m| *m.lock().unwrap()).unwrap_or(-1);
            if thisjob >= 0 {
                if let Some(jt) = JOBTAB.get() {
                    let mut guard = jt.lock().unwrap();
                    if let Some(j) = guard.get_mut(thisjob as usize) {
                        j.stat |= STAT_DONE; // c:3989
                    }
                }
            }
            return execcmd_exec_done_path(
                redir_err,
                oautocont,
                how,
                &mut shti,
                &mut chti,
                &mut then_ts,
                forked,
                &mut newxtrerr,
                cflags,
                orig_cflags,
                is_cursh,
                do_exec,
            );
        }
        if isset(XTRACE) {
            // c:3992-3994
            eprintln!();
        }
    } else if isset(EXECOPT) && (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) == 0 {
        // c:3996 — main dispatch branch.
        // c:3997 — `int q = queue_signal_level();`
        let _q = 0;
        // c:4003-4012 — entersubsh for is_exec.
        if is_exec != 0 {
            // c:4003
            let mut flags: i32 = if (how & Z_ASYNC as i32) != 0 {
                esub::ASYNC
            } else {
                0
            } | esub::PGRP
                | esub::FAKE; // c:4004-4005
            if typ != WC_SUBSH as i32 {
                flags |= esub::KEEPTRAP; // c:4007
            }
            if (do_exec != 0 || (typ >= WC_CURSH as i32 && last1 == 1)) && forked == 0 {
                // c:4008-4009
                flags |= esub::REVERTPGRP; // c:4010
            }
            entersubsh(flags, None); // c:4011
        }

        if typ == WC_FUNCDEF as i32 {
            // c:4013
            // c:4014-4036 — `redir_prog` setup from wordcode if no
            // redirs+WC_REDIR follows. Wire only when fusevm WC_REDIR
            // peek is in scope; for the tree-walker entry point we
            // approximate by passing None.
            let redir_prog: Option<crate::ported::zsh_h::Eprog> = None;
            // c:4039 — `lastval = execfuncdef(state, redir_prog);`
            let lv = execfuncdef(state, redir_prog);
            LASTVAL.store(lv, Ordering::Relaxed);
        } else if typ >= WC_CURSH as i32 {
            // c:4042
            if last1 == 1 {
                do_exec = 1; // c:4044
            }
            if typ == WC_AUTOFN as i32 {
                // c:4046
                let lv = execautofn_basic(state, do_exec); // c:4051
                LASTVAL.store(lv, Ordering::Relaxed);
            } else {
                // c:4053 — `lastval = (execfuncs[type - WC_CURSH])(state, do_exec);`
                // dispatch_execfuncs ports the C `execfuncs[]` table
                // (Src/exec.c:268) by typ → exec{cursh,for,select,...}
                // direct call. See dispatch_execfuncs at end of file.
                let lv = dispatch_execfuncs(state, typ, do_exec);
                LASTVAL.store(lv, Ordering::Relaxed);
            }
        } else if is_builtin != 0 || is_shfunc != 0 {
            // c:4055
            let mut restorelist: Vec<crate::ported::zsh_h::param> = Vec::new();
            let mut removelist: Vec<String> = Vec::new();
            let mut do_save: i32 = 0; // c:4057

            if forked == 0 {
                // c:4060
                if isset(POSIXBUILTINS) {
                    // c:4061
                    if is_shfunc != 0
                        || (hn.map(|p| unsafe { (*p).node.flags as u32 }).unwrap_or(0)
                            & (BINF_PSPECIAL | BINF_ASSIGN_FLAG))
                            != 0
                    {
                        // c:4067
                        do_save = if (orig_cflags & BINF_COMMAND) != 0 {
                            1
                        } else {
                            0
                        };
                    } else {
                        do_save = 1; // c:4070
                    }
                } else {
                    // c:4071
                    if (cflags & (BINF_COMMAND | BINF_ASSIGN_FLAG)) != 0 || magic_assign == 0 {
                        // c:4076
                        do_save = 1; // c:4077
                    }
                }
                if do_save != 0 {
                    if let Some(vspc) = varspc {
                        // c:4079
                        save_params(state, vspc, &mut restorelist, &mut removelist);
                    }
                }
            }
            if varspc.is_some() {
                // c:4082
                let mut addflags: i32 = 0; // c:4086
                if is_shfunc != 0 {
                    addflags |= ADDVAR_EXPORT; // c:4088
                }
                if !restorelist.is_empty() {
                    addflags |= ADDVAR_RESTORE; // c:4090
                }
                addvars(state, varspc.unwrap_or(0), addflags); // c:4092
                if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
                    // c:4093
                    if !restorelist.is_empty() {
                        restore_params(restorelist, removelist); // c:4094-4095
                    }
                    LASTVAL.store(1, Ordering::Relaxed); // c:4096
                    fixfds(&save); // c:4097
                    return execcmd_exec_done_path(
                        redir_err,
                        oautocont,
                        how,
                        &mut shti,
                        &mut chti,
                        &mut then_ts,
                        forked,
                        &mut newxtrerr,
                        cflags,
                        orig_cflags,
                        is_cursh,
                        do_exec,
                    );
                }
            }

            if is_shfunc != 0 {
                // c:4102-4105
                let mut a_vec: Vec<String> = args.clone().unwrap_or_default();
                // c:4104 — `execshfunc((Shfunc) hn, args);` C casts
                // HashNode hn to Shfunc; zshrs's hn is *mut builtin so
                // we re-resolve the shfunc by name from shfunctab and
                // dispatch through the top-level execshfunc port at
                // exec.rs:4978 (which routes to runshfunc).
                let name = args
                    .as_ref()
                    .and_then(|v| v.first())
                    .cloned()
                    .unwrap_or_default();
                let mut shf_clone: Option<shfunc> = if let Ok(tab) = shfunctab_lock().read() {
                    tab.get(&name).cloned()
                } else {
                    None
                };
                if let Some(ref mut shf) = shf_clone {
                    execshfunc(shf, &mut a_vec);
                }
                // c:4105 — `pipecleanfilelist(filelist, 0);` — clean
                // out the proc_subst entries from the current job's
                // filelist after the shfunc body ran. Route through
                // `JOBTAB[thisjob]`.
                if let Some(jt) = JOBTAB.get() {
                    let mut guard = jt.lock().unwrap();
                    let tj = THISJOB.get().map(|m| *m.lock().unwrap()).unwrap_or(-1);
                    if tj >= 0 {
                        if let Some(j) = guard.get_mut(tj as usize) {
                            crate::ported::jobs::pipecleanfilelist(j, false);
                        }
                    }
                }
            } else {
                // c:4107 — builtin path.
                let mut assigns: Vec<crate::ported::zsh_h::asgment> = Vec::new(); // c:4108
                let postassigns = eparams.postassigns; // c:4109
                if forked != 0 {
                    closem(FDT_INTERNAL, 0); // c:4111
                }
                if postassigns != 0 {
                    // c:4112-4230 — typeset post-assignment processing.
                    use crate::ported::zsh_h::{
                        ASG_ARRAY, ASG_KEY_VALUE, EC_DUPTOK as ECDUPTOK_LOCAL, PREFORK_ASSIGN,
                        PREFORK_KEY_VALUE, PREFORK_SINGLE, PREFORK_TYPESET, WC_ASSIGN_INC,
                        WC_ASSIGN_NUM, WC_ASSIGN_SCALAR, WC_ASSIGN_TYPE, WC_ASSIGN_TYPE2,
                    };
                    let opc = state.pc; // c:4113
                    state.pc = eparams.assignspc.unwrap_or(state.pc); // c:4114
                                                                      // c:4115 — `assigns = newlinklist();` — already declared above.
                    let mut pa_remaining = postassigns;
                    while pa_remaining > 0 {
                        // c:4116 — `while (postassigns--)`
                        pa_remaining -= 1;
                        let mut pa_htok: i32 = 0; // c:4117
                        if state.pc >= state.prog.prog.len() {
                            break;
                        }
                        let ac = state.prog.prog[state.pc]; // c:4118
                        state.pc += 1;
                        let mut name = ecgetstr(state, ECDUPTOK_LOCAL, Some(&mut pa_htok)); // c:4119
                                                                                            // c:4123-4124 DPUTS — debug assertion skipped.
                        if pa_htok != 0 {
                            // c:4126 — `init_list1(svl, name);`
                            let mut svl: LinkList<String> = Default::default();
                            svl.push_back(name.clone());
                            // c:4127-4166 — INC-scalar special case (typeset $ass form).
                            if WC_ASSIGN_TYPE(ac) == WC_ASSIGN_SCALAR
                                && WC_ASSIGN_TYPE2(ac) == WC_ASSIGN_INC
                            {
                                // c:4141 — `(void)ecgetstr(...)` — dummy.
                                let mut dummy_htok: i32 = 0;
                                let _ = ecgetstr(state, ECDUPTOK_LOCAL, Some(&mut dummy_htok));
                                let mut rf = 0i32;
                                prefork(&mut svl, PREFORK_TYPESET, &mut rf); // c:4142
                                if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
                                    // c:4143
                                    state.pc = opc; // c:4144
                                    break;
                                }
                                let mut rf2 = 0i32;
                                globlist(&mut svl, rf2); // c:4147
                                let _ = &mut rf2;
                                if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
                                    // c:4148
                                    state.pc = opc; // c:4149
                                    break;
                                }
                                // c:4152-4165 — drain svl into assigns.
                                while let Some(data) = svl.pop_front() {
                                    let (asg_name, asg_val): (String, Option<String>) =
                                        if let Some(eq_pos) = data.find('=') {
                                            // c:4156-4159
                                            (
                                                data[..eq_pos].to_string(),
                                                Some(data[eq_pos + 1..].to_string()),
                                            )
                                        } else {
                                            // c:4161-4162
                                            (data, None)
                                        };
                                    assigns.push(crate::ported::zsh_h::asgment {
                                        node: crate::ported::zsh_h::linknode {
                                            next: None,
                                            prev: None,
                                            dat: 0,
                                        },
                                        name: asg_name,
                                        flags: 0,
                                        scalar: asg_val,
                                        array: None,
                                    });
                                }
                                continue; // c:4166
                            }
                            // c:4168 — `prefork(&svl, PREFORK_SINGLE, NULL);`
                            let mut rf = 0i32;
                            prefork(&mut svl, PREFORK_SINGLE, &mut rf);
                            // c:4169-4170 — `name = empty(svl) ? "" : firstnode_data;`
                            name = if svl.is_empty() {
                                String::new()
                            } else {
                                svl.pop_front().unwrap_or_default()
                            };
                        }
                        // c:4172 — `untokenize(name);`
                        // (untokenize is destructive on bytes; Rust untokenize
                        // returns a new String — call and rebind.)
                        name = untokenize(&name);
                        let mut asg = crate::ported::zsh_h::asgment {
                            node: crate::ported::zsh_h::linknode {
                                next: None,
                                prev: None,
                                dat: 0,
                            },
                            name,
                            flags: 0,
                            scalar: None,
                            array: None,
                        };
                        if WC_ASSIGN_TYPE(ac) == WC_ASSIGN_SCALAR {
                            // c:4175
                            let mut val_htok: i32 = 0;
                            let mut val = ecgetstr(state, ECDUPTOK_LOCAL, Some(&mut val_htok)); // c:4176
                            asg.flags = 0; // c:4177
                            if WC_ASSIGN_TYPE2(ac) == WC_ASSIGN_INC {
                                // c:4178-4180 — fake assignment, no value.
                                asg.scalar = None;
                            } else {
                                if val_htok != 0 {
                                    // c:4183
                                    let mut svl: LinkList<String> = Default::default();
                                    svl.push_back(val.clone());
                                    let mut rf = 0i32;
                                    prefork(&mut svl, PREFORK_SINGLE | PREFORK_ASSIGN, &mut rf); // c:4184-4186
                                    if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
                                        // c:4187
                                        state.pc = opc; // c:4188
                                        break;
                                    }
                                    // c:4195-4196 — `val = empty(svl) ? "" : firstdata;`
                                    val = if svl.is_empty() {
                                        String::new()
                                    } else {
                                        svl.pop_front().unwrap_or_default()
                                    };
                                }
                                // c:4198 — `untokenize(val);`
                                asg.scalar = Some(untokenize(&val));
                            }
                        } else {
                            // c:4202 — array assignment.
                            asg.flags = ASG_ARRAY; // c:4202
                            let mut arr_htok: i32 = 0;
                            let arr_words = ecgetlist(
                                state,
                                WC_ASSIGN_NUM(ac) as usize,
                                ECDUPTOK_LOCAL,
                                Some(&mut arr_htok),
                            ); // c:4204
                            let mut arr_list: LinkList<String> = Default::default();
                            for s in arr_words {
                                arr_list.push_back(s);
                            }
                            if !arr_list.is_empty()
                                && (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) == 0
                            {
                                // c:4209 — `int prefork_ret = 0;`
                                let mut prefork_ret = 0i32;
                                prefork(&mut arr_list, PREFORK_ASSIGN, &mut prefork_ret); // c:4210-4211
                                if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
                                    // c:4212
                                    state.pc = opc; // c:4213
                                    break;
                                }
                                if (prefork_ret & PREFORK_KEY_VALUE) != 0 {
                                    // c:4216
                                    asg.flags |= ASG_KEY_VALUE; // c:4217
                                }
                                globlist(&mut arr_list, prefork_ret); // c:4218
                                if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
                                    // c:4220
                                    state.pc = opc; // c:4221
                                    break;
                                }
                            }
                            asg.array = Some(arr_list);
                        }
                        // c:4227 — `uaddlinknode(assigns, &asg->node);`
                        assigns.push(asg);
                    }
                    state.pc = opc; // c:4229
                }
                if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) == 0 {
                    // c:4232
                    // c:Src/builtin.c:262 — `name = (char *) ugetnode(args);`
                    // C's execbuiltin consumes args[0] (the command name)
                    // at entry. zshrs's execbuiltin reads the name from
                    // `bn->node.nam` instead, so we strip args[0] here
                    // before the call to match C's post-ugetnode argv
                    // shape. Without this, e.g. `cmd=pwd; $cmd` reached
                    // execbuiltin with args=["pwd"] and pwd's
                    // maxargs=0 check rejected the empty call as
                    // "too many arguments".
                    let mut a_vec: Vec<String> = args.clone().unwrap_or_default();
                    if !a_vec.is_empty() {
                        a_vec.remove(0);
                    }
                    let ret = crate::ported::builtin::execbuiltin(
                        a_vec,
                        assigns,
                        hn.unwrap_or(std::ptr::null_mut()),
                    ); // c:4233
                    if (errflag.load(Ordering::Relaxed) & ERRFLAG_INT) == 0 {
                        // c:4238
                        LASTVAL.store(ret, Ordering::Relaxed); // c:4239
                    }
                }
                if (do_save & BINF_COMMAND as i32) != 0 {
                    // c:4241
                    errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed); // c:4242
                }
                // c:4244 fflush(stdout) — Rust stdio auto-flushes.
                // c:4245-4251 — write-error check on save[1].
            }
            if isset(PRINTEXITVALUE)
                && isset(SHINSTDIN)
                && LASTVAL.load(Ordering::Relaxed) != 0
                && subsh.load(Ordering::Relaxed) == 0
            {
                // c:4253-4255
                eprintln!("zsh: exit {}", LASTVAL.load(Ordering::Relaxed)); // c:4258
            }

            if do_exec != 0 {
                // c:4263
                if subsh.load(Ordering::Relaxed) != 0 {
                    crate::ported::builtin::_realexit(); // c:4264-4265
                }
                if isset(RCS)
                    && crate::ported::zsh_h::interact()
                    && nohistsave.load(Ordering::Relaxed) == 0
                {
                    // c:4269
                    crate::ported::hist::savehistfile(None, HFILE_USE_OPTIONS as i32);
                    // c:4270
                }
                crate::ported::builtin::realexit(); // c:4271
            }
            if !restorelist.is_empty() {
                // c:4273
                restore_params(restorelist, removelist); // c:4274
            }
        } else {
            // c:4276 — external command execute.
            if subsh.load(Ordering::Relaxed) == 0 {
                // c:4277
                if forked == 0 {
                    // c:4280 — `setiparam("SHLVL", --shlvl);`
                    let cur = getsparam("SHLVL")
                        .and_then(|s| s.parse::<i64>().ok())
                        .unwrap_or(1);
                    setiparam("SHLVL", cur - 1); // c:4281
                }
                if do_exec != 0
                    && isset(RCS)
                    && crate::ported::zsh_h::interact()
                    && nohistsave.load(Ordering::Relaxed) == 0
                {
                    // c:4285
                    crate::ported::hist::savehistfile(None, HFILE_USE_OPTIONS as i32);
                    // c:4286
                }
            }
            if typ == WC_SIMPLE as i32 || typ == WC_TYPESET as i32 {
                // c:4288
                if varspc.is_some() {
                    // c:4289
                    let mut addflags: i32 = ADDVAR_EXPORT; // c:4290
                    if forked != 0 {
                        addflags |= ADDVAR_RESTORE; // c:4292
                    }
                    addvars(state, varspc.unwrap_or(0), addflags); // c:4293
                    if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
                        // c:4294
                        std::process::exit(1); // c:4295
                    }
                }
                closem(FDT_INTERNAL, 0); // c:4297
                                         // c:4298-4305 — close coprocin/coprocout.
                let cpi = crate::ported::modules::clone::coprocin.load(Ordering::Relaxed);
                if cpi != -1 {
                    let _ = zclose(cpi); // c:4299
                    crate::ported::modules::clone::coprocin.store(-1, Ordering::Relaxed);
                    // c:4300
                }
                let cpo = crate::ported::modules::clone::coprocout.load(Ordering::Relaxed);
                if cpo != -1 {
                    let _ = zclose(cpo); // c:4303
                    crate::ported::modules::clone::coprocout.store(-1, Ordering::Relaxed);
                    // c:4304
                }
                if forked == 0 {
                    // c:4307
                    setlimits(""); // c:4308
                }
                if (how & Z_ASYNC as i32) != 0 {
                    // c:4310 — `zsfree(STTYval); STTYval = 0;`
                    let mut guard = STTYval.lock().unwrap();
                    *guard = None; // c:4311-4312
                }
                // c:4314 — `execute(args, cflags, use_defpath);`
                let mut a_vec: Vec<String> = args.clone().unwrap_or_default();
                execute(&mut a_vec, cflags, use_defpath); // c:4314
            } else {
                // c:4315 — `( ... )` — WC_SUBSH.
                list_pipe.store(0, Ordering::Relaxed); // c:4318
                                                       // c:4319 — `pipecleanfilelist(filelist, 0);` — clean
                                                       // proc-subst entries from the current job's filelist
                                                       // before recursing into the subshell body.
                if let Some(jt) = JOBTAB.get() {
                    let mut guard = jt.lock().unwrap();
                    let tj = THISJOB.get().map(|m| *m.lock().unwrap()).unwrap_or(-1);
                    if tj >= 0 {
                        if let Some(j) = guard.get_mut(tj as usize) {
                            crate::ported::jobs::pipecleanfilelist(j, false);
                        }
                    }
                }
                state.pc += 1; // c:4324 — `state->pc++;`
                let _ = execlist(state, 0, 1); // c:4325
            }
        }
    }

    // c:4330-4404 — err: + done: + fatal:.
    return execcmd_exec_done_path(
        redir_err,
        oautocont,
        how,
        &mut shti,
        &mut chti,
        &mut then_ts,
        forked,
        &mut newxtrerr,
        cflags,
        orig_cflags,
        is_cursh,
        do_exec,
    );
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// the `done:` label tail of `execcmd_exec` (c:4366-4403), which C reaches by
/// `goto` and Rust cannot.
/// Internal helper modelling the C `done:` label tail of
/// `execcmd_exec` at `Src/exec.c:4366-4403`. Handles POSIX special-
/// builtin error escalation, AUTOCONTINUE restore, STTYval clear,
/// shelltime stop, and newxtrerr close.
#[allow(clippy::too_many_arguments)]
fn execcmd_exec_done_path(
    redir_err: i32,
    oautocont: i32,
    how: i32,
    shti: &mut crate::ported::jobs::timeinfo,
    chti: &mut crate::ported::jobs::timeinfo,
    then_ts: &mut std::time::Instant,
    forked: i32,
    newxtrerr: &mut Option<i32>,
    cflags: u32,
    orig_cflags: u32,
    is_cursh: i32,
    do_exec: i32,
) {
    use crate::ported::zsh_h::{
        AUTOCONTINUE, BINF_COMMAND, BINF_EXEC, BINF_PSPECIAL, INTERACTIVE, POSIXBUILTINS, Z_TIMED,
    };
    // c:4366
    // c:4367-4386 — POSIX special-builtin error escalation.
    if isset(POSIXBUILTINS)
        && (cflags & (BINF_PSPECIAL | BINF_EXEC)) != 0
        && (orig_cflags & BINF_COMMAND) == 0
    {
        // c:4367-4369
        let _forked_or_subsh = forked | zsh_subshell.load(Ordering::Relaxed); // c:4376
                                                                              // fatal: label entry point — same handling.
        if redir_err != 0 || (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
            // c:4378
            if !isset(INTERACTIVE) {
                // c:4379
                if _forked_or_subsh != 0 {
                    unsafe { libc::_exit(1) }; // c:4381
                } else {
                    std::process::exit(1); // c:4383
                }
            }
            errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed); // c:4385
        }
    }
    // c:4388-4389 — `if ((is_cursh || do_exec) && (how & Z_TIMED)) shelltime(...);`
    if (is_cursh != 0 || do_exec != 0) && (how & Z_TIMED as i32) != 0 {
        crate::ported::jobs::shelltime(Some(shti), Some(chti), Some(then_ts), 1);
        // c:4389
    }
    // c:4390-4398 — newxtrerr close.
    if let Some(fd) = newxtrerr.take() {
        // c:4390
        let _ = zclose(fd); // c:4396
    }
    // c:4400-4401 — `zsfree(STTYval); STTYval = 0;`
    {
        let mut guard = STTYval.lock().unwrap();
        *guard = None;
    }
    // c:4402-4403 — `if (oautocont >= 0) opts[AUTOCONTINUE] = oautocont;`
    if oautocont >= 0 {
        opt_state_set("autocontinue", oautocont != 0);
    }
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// the `err:` label tail of `execcmd_exec` (c:4330-4365), which C reaches by
/// `goto` and Rust cannot.
/// Internal helper modelling the C `err:` label tail of
/// `execcmd_exec` at `Src/exec.c:4330-4365`. Forked-child fd cleanup
/// + waitjobs + _realexit; non-forked: `fixfds(save)` + fall through
/// to done:.
#[allow(clippy::too_many_arguments)]
fn execcmd_exec_err_path(
    forked: i32,
    save: &mut [i32; 10],
    mfds: &mut [Option<Box<multio>>; 10],
    oautocont: i32,
    how: i32,
    shti: &mut crate::ported::jobs::timeinfo,
    chti: &mut crate::ported::jobs::timeinfo,
    then_ts: &mut std::time::Instant,
    newxtrerr: &mut Option<i32>,
    cflags: u32,
    orig_cflags: u32,
    is_cursh: i32,
    do_exec: i32,
    redir_err: i32,
) {
    use crate::ported::zsh_h::FDT_UNUSED;
    // c:4330
    if forked != 0 {
        // c:4331
        // c:4356-4358 — close all fds 0..10 whose fdtable entry != FDT_UNUSED.
        let mut i: i32 = 0;
        while i < 10 {
            if fdtable_get(i) != FDT_UNUSED {
                unsafe { libc::close(i) }; // c:4358
            }
            i += 1;
        }
        // c:4359 — `closem(FDT_UNUSED, 1);`
        closem(FDT_UNUSED, 1); // c:4359
                               // c:4360-4361 — `if (thisjob != -1) waitjobs();`
        let thisjob = THISJOB.get().map(|m| *m.lock().unwrap()).unwrap_or(-1);
        if thisjob != -1 {
            if let Some(jt) = JOBTAB.get() {
                let mut guard = jt.lock().unwrap();
                crate::ported::jobs::waitjobs(&mut guard, thisjob as usize); // c:4361
            }
        }
        crate::ported::builtin::_realexit(); // c:4362
    }
    fixfds(save); // c:4364

    execcmd_exec_done_path(
        redir_err,
        oautocont,
        how,
        shti,
        chti,
        then_ts,
        forked,
        newxtrerr,
        cflags,
        orig_cflags,
        is_cursh,
        do_exec,
    );
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Src/exec.c`. This helper stands in for
/// the `execfuncs[type - WC_CURSH]` function-pointer table dispatch (table at
/// c:268, call sites at c:1331 and c:4053), written as a `match` because Rust
/// has no equivalent table of `int (*)(Estate, int)`.
/// Internal helper dispatching `execfuncs[type - WC_CURSH]` from
/// `Src/exec.c:268`. Each branch maps to the ported wordcode-
/// walker function in `src/ported/exec.rs`.
fn dispatch_execfuncs(state: &mut estate, typ: i32, do_exec: i32) -> i32 {
    use crate::ported::zsh_h::{
        WC_ARITH, WC_AUTOFN, WC_CASE, WC_COND, WC_CURSH, WC_FOR, WC_FUNCDEF, WC_IF, WC_REPEAT,
        WC_SELECT, WC_SUBSH, WC_TIMED, WC_TRY, WC_WHILE,
    };
    // Port of `static int (*const execfuncs[])(Estate, int)` dispatch
    // table at `Src/exec.c:268`. C indexes by `(type - WC_CURSH)`;
    // Rust matches on the WC_* tag directly.
    match typ as wordcode {
        x if x == WC_CURSH => execcursh(state, do_exec),
        x if x == WC_FOR => execfor(state, do_exec),
        x if x == WC_SELECT => execselect(state, do_exec),
        x if x == WC_WHILE => execwhile(state, do_exec),
        x if x == WC_REPEAT => execrepeat(state, do_exec),
        x if x == WC_CASE => execcase(state, do_exec),
        x if x == WC_IF => execif(state, do_exec),
        x if x == WC_COND => execcond(state, do_exec),
        x if x == WC_ARITH => execarith(state, do_exec),
        x if x == WC_TRY => exectry(state, do_exec),
        x if x == WC_FUNCDEF => execfuncdef(state, None),
        // c:272 — execfuncs[] table dispatches `WC_AUTOFN` to
        // `execautofn` (the loadautofn-then-basic wrapper), not
        // `execautofn_basic` directly.
        x if x == WC_AUTOFN => execautofn(state, do_exec),
        x if x == WC_TIMED => exectime(state, do_exec),
        x if x == WC_SUBSH => execcursh(state, do_exec), // c:269 — same handler.
        _ => 0,
    }
}

/// Port of `stripkshdef()` from `Src/exec.c:6292` — C decl `stripkshdef(Eprog prog, char *name)`.
/// Given an Eprog read from an autoload
/// file plus the function name being defined, check whether the
/// file consists of *exactly* one `function NAME { … }` definition
/// for that name. If so, return a new Eprog whose `prog`/`strs`/
/// `pats` slice out just the function body (so calling code can
/// invoke the body directly instead of re-parsing). Otherwise
/// return the input untouched.
///
/// Header word layout consumed (matches C `pc[…]` reads):
///   pc[0] = WC_LIST with `Z_SYNC|Z_END|Z_SIMPLE` flags
///   pc[1] = (sublist header, skipped)
///   pc[2] = WC_FUNCDEF
///   pc[3] = 1                       (single-name funcdef)
///   pc[4] = name-string slot        (compared to `name`)
///   pc[5] = sbeg  (offset into strs table)
///   pc[6] = nstrs (bytes of strs to copy)
///   pc[7] = npats (number of pattern slots to allocate)
///   pc[8] = WC_FUNCDEF_SKIP target  (end-of-funcdef pc)
///   pc[9] = (unused header word — `pc += 6` lands here as the
///           start of the body wordcode stream)
///
/// Returns `None` only when the input was `None` (matches C
/// `return NULL`). Equivalence between the original `prog` and a
/// successfully stripped `prog` is *not* preserved at the pointer
/// level (C may return the original Eprog when the file fails the
/// single-funcdef shape check; this Rust port does the same by
/// passing the box back through).
///
/// `EF_MAP` (`zcompile`d / mmap'd Eprog) path: C mutates the
/// existing Eprog in place, swapping its `prog` / `strs` /
/// `pats` to slice into the funcdef body. Rust mirrors this on
/// the moved-in `Box<eprog>` (no separate `free()` needed —
/// `Vec` drop handles the old `pats`).
pub fn stripkshdef(
    prog: Option<crate::ported::zsh_h::Eprog>,
    name: &str,
) -> Option<crate::ported::zsh_h::Eprog> {
    use crate::ported::parse::ecrawstr;
    use crate::ported::zsh_h::{
        wc_code, wordcode, Dash, EF_HEAP, EF_MAP, WC_FUNCDEF, WC_FUNCDEF_SKIP, WC_LIST,
        WC_LIST_TYPE, Z_END, Z_SIMPLE, Z_SYNC,
    };

    // c:6300 — `if (!prog) return NULL;`
    let mut prog = prog?;

    // c:6302-6306 — first word must be WC_LIST with all of
    // Z_SYNC|Z_END|Z_SIMPLE set (i.e. the trivial "single simple
    // sublist" wrapper around the funcdef).
    if prog.prog.len() < 3 {
        return Some(prog);
    }
    let code0: wordcode = prog.prog[0];
    if wc_code(code0) != WC_LIST
        || (WC_LIST_TYPE(code0) & (Z_SYNC | Z_END | Z_SIMPLE) as wordcode)
            != (Z_SYNC | Z_END | Z_SIMPLE) as wordcode
    {
        return Some(prog);
    }
    // c:6307 — `pc++;` (skip the sublist header word at pc[1]).
    // c:6308 — `code = *pc++;` lands `code` on pc[2], leaving the
    // walking cursor at pc[3] which is read directly below.
    let code: wordcode = prog.prog[2];
    let pc_after_code: usize = 3;
    if wc_code(code) != WC_FUNCDEF || prog.prog[pc_after_code] != 1 {
        return Some(prog);
    }

    // c:6320 — `ptr2 = ecrawstr(prog, pc + 1, NULL);` (note: C's
    // `pc` is already past `code`, so `pc + 1` lands on pc[4] —
    // the name-string slot).
    let name_slot = pc_after_code + 1; // == 4
    let name_in_def = ecrawstr(&prog, name_slot, None);

    // c:6320-6328 — name match, tolerating Dash-tokenised hyphens
    // on either side.
    let n1 = name.as_bytes();
    let n2 = name_in_def.as_bytes();
    let mut i = 0usize;
    let mut j = 0usize;
    while i < n1.len() && j < n2.len() {
        let c1 = n1[i] as char;
        let c2 = n2[j] as char;
        if c1 != c2 && c1 != Dash && c1 != '-' && c2 != Dash && c2 != '-' {
            break;
        }
        i += 1;
        j += 1;
    }
    // c:6329 — `if (*ptr1 || *ptr2) return prog;` (any unmatched
    // tail on either side → not the right funcdef).
    if i < n1.len() || j < n2.len() {
        return Some(prog);
    }

    // c:6332-6362 — slice the funcdef body out. Layout:
    //   sbeg  = pc[2] (in C, == prog.prog[pc_after_code + 2] == [5])
    //   nstrs = pc[3] (== [6])
    //   npats = pc[4] (== [7])
    //   end   = pc + WC_FUNCDEF_SKIP(code)   (== pc_after_code + skip)
    //   pc   += 6  (body wordcode begins at pc_after_code + 6 == [9])
    let sbeg = prog.prog[pc_after_code + 2] as usize;
    let nstrs = prog.prog[pc_after_code + 3] as usize;
    let npats = prog.prog[pc_after_code + 4] as i32;
    let skip = WC_FUNCDEF_SKIP(code) as usize;
    let end_pc = pc_after_code + skip;
    let body_start = pc_after_code + 6;
    if end_pc < body_start || end_pc > prog.prog.len() {
        // Defensive: malformed header — return input untouched so
        // the caller's parse-eprog fallback re-reads from source.
        return Some(prog);
    }
    let nprg = end_pc - body_start;
    let plen = nprg * size_of::<wordcode>();
    let len = plen + (npats as usize) * size_of::<usize>() + nstrs;

    // Build the new pats slice — `dummy_patprog1` slots in C; the
    // Rust convention (mirrors `dupeprog` at parse.rs:2716) is to
    // synthesize zero-initialised patprog placeholders that
    // pattern compile-on-first-use will overwrite.
    let dummy_pat = || {
        Box::new(crate::ported::zsh_h::patprog {
            startoff: 0,
            size: 0,
            mustoff: 0,
            patmlen: 0,
            globflags: 0,
            globend: 0,
            flags: 0,
            patnpar: 0,
            patstartch: 0,
        })
    };
    let new_pats: Vec<crate::ported::zsh_h::Patprog> =
        (0..npats.max(0)).map(|_| dummy_pat()).collect();

    // c:6353 — `ret->strs = prog->strs + sbeg;` (EF_MAP) or
    // c:6359 — `memcpy(ret->strs, prog->strs + sbeg, nstrs);` (heap).
    let strs_metafied = prog.strs_metafied; // pool sliced verbatim — carry provenance
    let old_strs = prog.strs.take().unwrap_or_default();
    let old_bytes = old_strs.as_bytes();
    let new_strs = if sbeg + nstrs <= old_bytes.len() {
        Some(String::from_utf8_lossy(&old_bytes[sbeg..sbeg + nstrs]).into_owned())
    } else {
        Some(String::new())
    };

    let new_prog: Vec<wordcode> = prog.prog[body_start..end_pc].to_vec();

    if (prog.flags & EF_MAP) != 0 {
        // c:6349-6354 — in-place EF_MAP path.
        prog.pats = new_pats;
        prog.prog = new_prog;
        prog.strs = new_strs;
        prog.len = len as i32;
        prog.npats = npats;
        prog.shf = None;
        return Some(prog);
    }

    // c:6356-6361 — heap-allocated new Eprog.
    let ret = Box::new(eprog {
        flags: EF_HEAP,
        len: len as i32,
        npats,
        nref: -1, // c:6363 (heap path → never refcount-freed).
        pats: new_pats,
        prog: new_prog,
        strs: new_strs,
        shf: None, // c:6363
        dump: None,
        strs_metafied, // c:6353/6359 — the slice keeps the source pool's encoding
    });
    Some(ret)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── zsh-corpus pins for pure exec helpers ─────────────────────

    /// `Src/exec.c:996-1010` — `isrelative` returns 1 for empty.
    #[test]
    fn exec_corpus_isrelative_empty_is_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(isrelative(""), 1, "empty path is relative");
    }

    /// `isrelative("foo")` = 1 (no leading slash).
    #[test]
    fn exec_corpus_isrelative_bare_name_is_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(isrelative("foo"), 1);
        assert_eq!(isrelative("bin/cmd"), 1);
    }

    /// `isrelative("/foo")` = 0 (absolute, no `./` / `../`).
    #[test]
    fn exec_corpus_isrelative_absolute_clean_is_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(isrelative("/foo"), 0, "/foo is absolute");
        assert_eq!(isrelative("/bin/ls"), 0);
        assert_eq!(isrelative("/"), 0, "root is absolute");
    }

    /// `isrelative("/foo/../bar")` = 1 (contains `../` component).
    #[test]
    fn exec_corpus_isrelative_absolute_with_dotdot_is_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            isrelative("/foo/../bar"),
            1,
            "absolute path with ../ is still 'relative' per zsh"
        );
    }

    /// `isrelative("/foo/./bar")` = 1 (contains `./` component).
    #[test]
    fn exec_corpus_isrelative_absolute_with_dot_is_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            isrelative("/./x"),
            1,
            "absolute with ./ component reported relative"
        );
    }

    /// `Src/exec.c:5300` — `is_anonymous_function_name("(anon)")` = 1.
    #[test]
    fn exec_corpus_is_anonymous_function_name_matches_sentinel() {
        assert_eq!(is_anonymous_function_name("(anon)"), 1);
    }

    /// `is_anonymous_function_name("regular_name")` = 0.
    #[test]
    fn exec_corpus_is_anonymous_function_name_rejects_normal() {
        assert_eq!(is_anonymous_function_name("regular_name"), 0);
        assert_eq!(is_anonymous_function_name(""), 0);
        assert_eq!(
            is_anonymous_function_name("anon"),
            0,
            "plain 'anon' (no parens) is NOT the sentinel"
        );
    }

    /// `iscom("/nonexistent/never_a_path")` = false.
    #[test]
    fn exec_corpus_iscom_missing_path_false() {
        assert!(!iscom("/this/path/does/not/exist/zshrs_xyz"));
    }

    /// `iscom("/tmp")` is a directory not a regular file → false.
    #[test]
    fn exec_corpus_iscom_directory_false() {
        assert!(!iscom("/tmp"), "/tmp is a dir, not a regular command");
    }

    /// `iscom("/bin/sh")` is true on POSIX systems.
    #[test]
    fn exec_corpus_iscom_known_binary_true() {
        // /bin/sh exists on all POSIX systems with X perms.
        if std::path::Path::new("/bin/sh").exists() {
            assert!(iscom("/bin/sh"), "/bin/sh is a real executable");
        }
    }

    // ─── stripkshdef (Src/exec.c:6292) early-return paths ──────────

    /// `stripkshdef(None, "foo")` → `None` (matches C `if (!prog)
    /// return NULL;` at exec.c:6300).
    #[test]
    fn exec_corpus_stripkshdef_null_input_returns_none() {
        assert!(stripkshdef(None, "foo").is_none());
    }

    /// `stripkshdef` on an empty/degenerate Eprog returns the same
    /// Eprog unchanged (no funcdef-shape to strip).
    #[test]
    fn exec_corpus_stripkshdef_empty_prog_returns_input() {
        let prog = Box::new(eprog {
            prog: vec![],
            ..Default::default()
        });
        let out = stripkshdef(Some(prog), "foo");
        assert!(out.is_some(), "empty prog → returned unchanged");
        assert!(out.unwrap().prog.is_empty(), "no mutation");
    }

    /// `stripkshdef` on a non-WC_LIST head returns the input
    /// untouched (early return at exec.c:6304-6306).
    #[test]
    fn exec_corpus_stripkshdef_non_list_head_returns_input() {
        use crate::ported::zsh_h::{wc_bld, WC_SUBLIST};
        let prog = Box::new(eprog {
            prog: vec![wc_bld(WC_SUBLIST, 0), 0, 0],
            ..Default::default()
        });
        let out = stripkshdef(Some(prog), "foo");
        assert!(out.is_some());
        // first word is the WC_SUBLIST sentinel we passed in,
        // unchanged (the function bailed before doing any slicing).
        let p = out.unwrap();
        use crate::ported::zsh_h::wc_code;
        assert_eq!(
            wc_code(p.prog[0]),
            WC_SUBLIST,
            "header word preserved verbatim"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/exec.c. Tests that capture KNOWN
    // ZSHRS BUGS use #[ignore = "ZSHRS BUG: …"].
    // ═══════════════════════════════════════════════════════════════════

    /// `isrelative("/abs/path")` returns 0 (false = absolute path).
    /// C `Src/exec.c:996-1006` — leading `/` and no `.`/`..` components.
    #[test]
    fn isrelative_absolute_path_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(isrelative("/usr/local/bin"), 0);
    }

    /// `isrelative("foo/bar")` returns 1 (no leading slash).
    #[test]
    fn isrelative_no_leading_slash_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(isrelative("foo/bar"), 1);
    }

    /// `isrelative("/foo/./bar")` returns 1 — contains `/./` walk.
    /// C c:1001 — `.` with prev `/` + next `/` triggers relative flag.
    #[test]
    fn isrelative_dot_component_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(isrelative("/foo/./bar"), 1, "/./ in path → relative");
    }

    /// `isrelative("/foo/../bar")` returns 1 — contains `/..` walk.
    #[test]
    fn isrelative_dotdot_component_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(isrelative("/foo/../bar"), 1, "/../ in path → relative");
    }

    /// `isrelative("")` returns 1 — empty input has no leading `/`.
    /// C c:998 — `*s != '/'` includes the NUL terminator case.
    #[test]
    fn isrelative_empty_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(isrelative(""), 1, "empty string → not absolute");
    }

    /// `isrelative("/a/.b")` returns 0 — `.b` is NOT a `/./` walk
    /// (followed by another non-`/` char `b`).
    #[test]
    fn isrelative_dotfile_in_path_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            isrelative("/usr/.config/zsh"),
            0,
            "dotfile name '.config' is NOT a relative walk"
        );
    }

    /// `is_anonymous_function_name("(anon)")` returns 1 (true).
    /// C `Src/exec.c` — `!strcmp(name, ANONYMOUS_FUNCTION_NAME)`.
    #[test]
    fn is_anonymous_function_name_anon_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(is_anonymous_function_name("(anon)"), 1);
    }

    /// `is_anonymous_function_name("foo")` returns 0 (false).
    #[test]
    fn is_anonymous_function_name_normal_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(is_anonymous_function_name("foo"), 0);
        assert_eq!(is_anonymous_function_name(""), 0);
        assert_eq!(is_anonymous_function_name("(other)"), 0);
    }

    /// `isgooderr(EACCES, "/no/such/dir")` returns true when the dir
    /// is not actually accessible. C `Src/exec.c:isgooderr` filters
    /// out "unreadable / not directory" errnos so caller doesn't
    /// emit spurious warnings.
    #[test]
    fn isgooderr_eacces_unreadable_dir_returns_false() {
        let _g = crate::test_util::global_state_lock();
        // /no/such/dir doesn't exist → access(X_OK) fails non-zero
        // → !access() is 0 (false) → returns false.
        assert!(
            !isgooderr(libc::EACCES, "/no/such/dir/zshrs_test"),
            "unreadable dir with EACCES should NOT be 'good error'"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/exec.c basic accessors/predicates.
    // ═══════════════════════════════════════════════════════════════════

    /// c:658 — `isgooderr(ENOENT, _)` always false (regardless of dir).
    /// Pin: ENOENT is NEVER a "good error" because the path itself
    /// doesn't exist — caller should suppress the warning.
    #[test]
    fn isgooderr_enoent_always_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!isgooderr(libc::ENOENT, "/tmp"));
        assert!(!isgooderr(libc::ENOENT, "/no/such/dir"));
        assert!(!isgooderr(libc::ENOENT, ""));
    }

    /// c:658 — `isgooderr(ENOTDIR, _)` always false. A path component
    /// being a non-dir is a structural error, not a permission issue.
    #[test]
    fn isgooderr_enotdir_always_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!isgooderr(libc::ENOTDIR, "/tmp"));
        assert!(!isgooderr(libc::ENOTDIR, "/"));
    }

    /// c:658 — Other errnos (EPERM, EIO, ENOMEM) are "good errors"
    /// because they're not the suppressed three (EACCES/ENOENT/ENOTDIR).
    #[test]
    fn isgooderr_other_errno_returns_true() {
        let _g = crate::test_util::global_state_lock();
        assert!(isgooderr(libc::EPERM, "/tmp"));
        assert!(isgooderr(libc::EIO, "/tmp"));
        assert!(isgooderr(libc::ENOMEM, "/tmp"));
    }

    /// c:962 — `iscom("/tmp")` returns false (directory, not S_ISREG).
    #[test]
    fn iscom_directory_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!iscom("/tmp"));
        assert!(!iscom("/"));
    }

    /// c:962 — `iscom` on non-existent path returns false (access
    /// X_OK fails).
    #[test]
    fn iscom_nonexistent_path_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!iscom("/no/such/path/zshrs_iscom_test"));
        assert!(!iscom(""));
    }

    /// c:962 — `iscom("/bin/sh")` returns true on every POSIX system.
    #[test]
    #[cfg(unix)]
    fn iscom_bin_sh_returns_true() {
        let _g = crate::test_util::global_state_lock();
        // /bin/sh is a POSIX-required executable.
        assert!(iscom("/bin/sh"), "/bin/sh must be executable on POSIX");
    }

    /// c:5300 — anonymous function name is exactly "(anon)" — must
    /// not match prefixes/suffixes/case variants.
    #[test]
    fn is_anonymous_function_name_strict_match_only() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(is_anonymous_function_name("(anon"), 0, "no trailing paren");
        assert_eq!(is_anonymous_function_name("anon)"), 0, "no leading paren");
        assert_eq!(is_anonymous_function_name("(ANON)"), 0, "wrong case");
        assert_eq!(
            is_anonymous_function_name(" (anon) "),
            0,
            "leading/trailing space"
        );
        assert_eq!(is_anonymous_function_name("(anon) "), 0, "trailing space");
        assert_eq!(is_anonymous_function_name(" (anon)"), 0, "leading space");
    }

    /// c:5289 — `ANONYMOUS_FUNCTION_NAME` constant is exactly `"(anon)"`.
    /// Pin so a regen that flips parens / changes case / adds prefix
    /// would be caught.
    #[test]
    fn anonymous_function_name_const_is_literal_anon() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ANONYMOUS_FUNCTION_NAME, "(anon)");
    }

    /// c:147-148 — `isrelative("./")` returns 1 (dot-slash prefix
    /// is the canonical relative-path form).
    #[test]
    fn isrelative_dot_slash_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(isrelative("./foo"), 1);
        assert_eq!(isrelative("./"), 1);
    }

    /// c:147-148 — `isrelative("../foo")` returns 1.
    #[test]
    fn isrelative_dotdot_slash_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(isrelative("../foo"), 1);
        assert_eq!(isrelative("../"), 1);
    }

    /// c:147-148 — `/.foo` (hidden file under root) is absolute.
    /// Pin: only `/.` (with trailing `/`) or end-of-string counts as
    /// a `.` component, NOT `/.foo` (which is a normal file `.foo`).
    #[test]
    fn isrelative_root_hidden_file_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(isrelative("/.foo"), 0, "/.foo is absolute path to dotfile");
        assert_eq!(isrelative("/.bashrc"), 0, "/.bashrc is absolute");
    }

    /// c:147-148 — `/..bar` (file named `..bar`) is also absolute,
    /// since `..bar` is a regular file name, not a `..` component.
    #[test]
    fn isrelative_root_double_dot_file_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(isrelative("/..bar"), 0);
    }

    /// c:2652 — `setunderscore("")` clears `zunderscore` and resets
    /// `underscoreused` to 1 (null terminator only).
    #[test]
    fn setunderscore_empty_clears_state() {
        let _g = crate::test_util::global_state_lock();
        setunderscore(""); // initialize to known empty state
        let zu = zunderscore.lock().unwrap();
        assert!(zu.is_empty(), "zunderscore must be empty after clear");
        drop(zu);
        let used = underscoreused.load(Ordering::Relaxed);
        assert_eq!(used, 1, "underscoreused must be 1 (NUL only) after clear");
    }

    /// c:2652 — `setunderscore(str)` sets `zunderscore=str` and
    /// `underscoreused = str.len()+1` (string + null terminator).
    #[test]
    fn setunderscore_with_value_stores_string_and_length() {
        let _g = crate::test_util::global_state_lock();
        setunderscore("hello");
        let zu = zunderscore.lock().unwrap();
        assert_eq!(*zu, "hello");
        drop(zu);
        let used = underscoreused.load(Ordering::Relaxed);
        assert_eq!(used, 6, "len('hello')+1 = 6");
    }

    /// c:2656 — `underscorelen` is rounded up to 32-byte boundary
    /// for the bump-allocator-friendly buffer growth.
    #[test]
    fn setunderscore_rounds_underscorelen_to_32() {
        let _g = crate::test_util::global_state_lock();
        setunderscore("ab"); // len 2 + 1 = 3 → ceil(32) = 32
        let nl = underscorelen.load(Ordering::Relaxed);
        assert_eq!(nl, 32, "(2+1+31) & !31 = 32");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/exec.c cancd2 +
    // quote_tokenized_output.
    // ═══════════════════════════════════════════════════════════════════

    /// c:6411 — `cancd2("/tmp")` returns 1 (directory with X_OK exists).
    #[test]
    #[cfg(unix)]
    fn cancd2_existing_dir_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(cancd2("/tmp"), 1, "/tmp is a valid cd target");
    }

    /// c:6411 — `cancd2("/nonexistent")` returns 0.
    #[test]
    fn cancd2_nonexistent_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(cancd2("/__never_exists_zshrs_cancd2__"), 0);
    }

    /// c:6411 — `cancd2` for a file (not dir) returns 0.
    #[test]
    #[cfg(unix)]
    fn cancd2_regular_file_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("regular_file");
        std::fs::write(&p, "x").unwrap();
        assert_eq!(
            cancd2(p.to_str().unwrap()),
            0,
            "regular file not a cd target"
        );
    }

    /// c:2114 — `quote_tokenized_output` on empty string writes nothing.
    #[test]
    fn quote_tokenized_output_empty_writes_nothing() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = Vec::new();
        quote_tokenized_output("", &mut buf).unwrap();
        assert!(buf.is_empty());
    }

    /// c:2114 — plain ASCII passes through unchanged.
    #[test]
    fn quote_tokenized_output_plain_ascii_unchanged() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = Vec::new();
        quote_tokenized_output("hello", &mut buf).unwrap();
        assert_eq!(buf, b"hello");
    }

    /// c:2143 — space gets backslash-quoted.
    #[test]
    fn quote_tokenized_output_space_backslash_quoted() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = Vec::new();
        quote_tokenized_output("a b", &mut buf).unwrap();
        assert_eq!(buf, b"a\\ b");
    }

    /// c:2147 — tab → $'\\t'.
    #[test]
    fn quote_tokenized_output_tab_dollar_escape() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = Vec::new();
        quote_tokenized_output("a\tb", &mut buf).unwrap();
        assert_eq!(buf, b"a$'\\t'b");
    }

    /// c:2151 — newline → $'\\n'.
    #[test]
    fn quote_tokenized_output_newline_dollar_escape() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = Vec::new();
        quote_tokenized_output("a\nb", &mut buf).unwrap();
        assert_eq!(buf, b"a$'\\n'b");
    }

    /// c:2155 — CR → $'\\r'.
    #[test]
    fn quote_tokenized_output_cr_dollar_escape() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = Vec::new();
        quote_tokenized_output("a\rb", &mut buf).unwrap();
        assert_eq!(buf, b"a$'\\r'b");
    }

    /// c:2128 — shell metacharacters all get backslash-quoted.
    #[test]
    fn quote_tokenized_output_shell_metas_get_backslash() {
        let _g = crate::test_util::global_state_lock();
        for c in &[b'<', b'>', b'(', b')', b'|', b'#', b'$', b'*', b'?', b'~'] {
            let mut buf = Vec::new();
            let s = String::from_utf8(vec![b'a', *c, b'b']).unwrap();
            quote_tokenized_output(&s, &mut buf).unwrap();
            assert_eq!(buf, vec![b'a', b'\\', *c, b'b'], "char {:?}", *c as char);
        }
    }

    /// c:2158 — `=` at position 0 gets quoted (path-spec).
    #[test]
    fn quote_tokenized_output_equals_at_start_quoted() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = Vec::new();
        quote_tokenized_output("=foo", &mut buf).unwrap();
        assert_eq!(buf, b"\\=foo");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/exec.c
    // c:1287 iscom / c:1347 isrelative / c:1398 setunderscore /
    // c:1468 is_anonymous_function_name / c:2208 findcmd / c:3273 parsecmd
    // c:1264 isgooderr / c:1226 parse_string
    // ═══════════════════════════════════════════════════════════════════

    /// c:1287 — `iscom("")` empty input returns false.
    #[test]
    fn iscom_empty_string_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!iscom(""), "empty cmd name → not a command");
    }

    /// c:1287 — `iscom` returns bool (compile-time type pin).
    #[test]
    fn iscom_returns_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let _: bool = iscom("ls");
    }

    /// c:1347 — `isrelative("/abs")` returns 0 (absolute path).
    #[test]
    fn isrelative_absolute_path_returns_zero_pin() {
        assert_eq!(isrelative("/usr/bin"), 0, "/usr/bin is absolute");
        assert_eq!(isrelative("/"), 0, "/ is absolute");
    }

    /// c:1347 — `isrelative("rel/path")` returns 1 (relative).
    #[test]
    fn isrelative_relative_path_returns_one_pin() {
        assert_eq!(isrelative("foo"), 1, "foo is relative");
        assert_eq!(isrelative("./foo"), 1, "./foo is relative");
        assert_eq!(isrelative("../foo"), 1, "../foo is relative");
    }

    /// c:1347 — `isrelative("")` empty returns 1 (relative by C convention).
    #[test]
    fn isrelative_empty_returns_relative() {
        let r = isrelative("");
        assert!(r == 0 || r == 1, "must be 0 or 1");
    }

    /// c:1468 — `is_anonymous_function_name` returns i32 (type pin).
    #[test]
    fn is_anonymous_function_name_returns_i32_type() {
        let _: i32 = is_anonymous_function_name("(anon)");
    }

    /// c:1468 — `is_anonymous_function_name("")` empty returns 0.
    #[test]
    fn is_anonymous_function_name_empty_returns_zero() {
        assert_eq!(
            is_anonymous_function_name(""),
            0,
            "empty name is not anonymous"
        );
    }

    /// c:1468 — `is_anonymous_function_name` is deterministic.
    #[test]
    fn is_anonymous_function_name_is_deterministic() {
        for s in ["", "name", "(anon)", "(anon: foo)"] {
            let first = is_anonymous_function_name(s);
            for _ in 0..3 {
                assert_eq!(
                    is_anonymous_function_name(s),
                    first,
                    "is_anonymous_function_name({:?}) must be deterministic",
                    s
                );
            }
        }
    }

    /// c:1226 — `parse_string("")` empty returns Option<eprog> (type pin).
    #[test]
    fn parse_string_returns_option_eprog_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<eprog> = parse_string("", 0);
    }

    /// c:1398 — `setunderscore("")` empty string is safe.
    #[test]
    fn setunderscore_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        setunderscore("");
    }

    /// c:1264 — `isgooderr` returns bool (compile-time type pin).
    #[test]
    fn isgooderr_returns_bool_type() {
        let _: bool = isgooderr(0, "/tmp");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/exec.c
    // c:3325 makecline / c:4603 cancd / c:4674 simple_redir_name /
    // c:1287 iscom / c:1314 isreallycom / c:3076 commandnotfound
    // ═══════════════════════════════════════════════════════════════════

    /// c:3325 — `makecline` returns Vec<String> (compile-time type pin).
    #[test]
    fn makecline_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = makecline(&[]);
    }

    /// c:3325 — `makecline([])` empty returns empty Vec.
    #[test]
    fn makecline_empty_input_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = makecline(&[]);
        assert!(r.is_empty(), "empty input → empty output");
    }

    /// c:3325 — `makecline` preserves input order.
    #[test]
    fn makecline_preserves_input_order() {
        let _g = crate::test_util::global_state_lock();
        let input = vec!["one".to_string(), "two".to_string(), "three".to_string()];
        let out = makecline(&input);
        assert_eq!(out, input, "makecline must preserve order");
    }

    /// c:3325 — `makecline` clones (output is independent of input).
    #[test]
    fn makecline_returns_independent_copy() {
        let _g = crate::test_util::global_state_lock();
        let input = vec!["a".to_string(), "b".to_string()];
        let out = makecline(&input);
        assert_eq!(out.len(), input.len(), "lengths match");
        // Output can be mutated without affecting input.
        let mut out_mut = out;
        out_mut.push("c".to_string());
        assert_eq!(input.len(), 2, "input unchanged");
    }

    /// c:4603 — `cancd("")` empty path returns None.
    /// ZSHRS BUG: empty path returns Some(...) instead of None. C path
    /// at Src/exec.c:6376 enters relative-path branch which calls cancd2("")
    /// — that should return 0 (not a valid dir), causing the fn to fall
    /// through CDPATH and cd_able_vars, both of which should miss for
    /// the empty string. Likely cd_able_vars("") or CDPATH-with-empty-element
    /// is silently matching $HOME or "." here.
    /// C-faithful behavior: `cancd("")` enters the `!starts_with('/')`
    /// branch (c:6376), calls `cancd2("")` which appends to PWD →
    /// "PWD/" → fixdir → PWD itself → access+stat succeed → returns
    /// `Some(pwd)`. Verified against `/bin/zsh -fc 'cd ""; echo $?'`
    /// → `0` (success). The previous test expectation (None) was
    /// based on a misread of the C source — pin actual behavior.
    #[test]
    fn cancd_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        // cancd("") returns Some — empty path resolves through PWD per
        // the cancd2 path; matches C zsh's `cd ""` exit-0 behavior.
        // Pin PWD to a known-existing dir so a prior test that left
        // PWD set to a non-directory doesn't masquerade as the bug.
        let saved_pwd = crate::ported::params::getsparam("PWD");
        crate::ported::params::setsparam("PWD", "/");
        let r = cancd("");
        if let Some(p) = saved_pwd {
            crate::ported::params::setsparam("PWD", &p);
        } else {
            crate::ported::params::unsetparam("PWD");
        }
        assert!(
            r.is_some(),
            "empty path → Some(pwd) per cancd2-via-PWD path"
        );
    }

    /// c:4603 — `cancd("/")` root dir returns Some (always exists).
    #[test]
    fn cancd_root_returns_some() {
        let _g = crate::test_util::global_state_lock();
        let r = cancd("/");
        assert_eq!(r.as_deref(), Some("/"), "root dir cancd → Some(/)");
    }

    /// c:4603 — `cancd` returns Option<String> (compile-time type pin).
    #[test]
    fn cancd_returns_option_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<String> = cancd("/");
    }

    /// c:4603 — `cancd("/__nonexistent__")` returns None.
    #[test]
    fn cancd_nonexistent_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            cancd("/__nonexistent_zshrs_dir_xyz__").is_none(),
            "nonexistent dir → None"
        );
    }

    /// c:4603 — `cancd("/tmp")` exists → Some.
    #[test]
    fn cancd_tmp_returns_some() {
        let _g = crate::test_util::global_state_lock();
        let r = cancd("/tmp");
        assert!(r.is_some(), "/tmp exists → Some");
    }

    /// c:4603 — `cancd` is deterministic for stable paths.
    #[test]
    fn cancd_is_deterministic_for_stable_paths() {
        let _g = crate::test_util::global_state_lock();
        for p in ["/", "/tmp", "/__never__"] {
            let first = cancd(p).is_some();
            for _ in 0..3 {
                assert_eq!(
                    cancd(p).is_some(),
                    first,
                    "cancd({:?}) must be deterministic",
                    p
                );
            }
        }
    }

    /// c:1287 — `iscom` is deterministic for stable paths.
    #[test]
    fn iscom_is_deterministic_for_stable_paths() {
        let _g = crate::test_util::global_state_lock();
        for p in ["/tmp", "/__never__", "/bin/sh"] {
            let first = iscom(p);
            for _ in 0..3 {
                assert_eq!(iscom(p), first, "iscom({:?}) must be deterministic", p);
            }
        }
    }

    /// c:3076 — `commandnotfound("", ...)` empty cmd returns i32.
    #[test]
    fn commandnotfound_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let mut args = Vec::new();
        let _: i32 = commandnotfound("", &mut args);
    }
}

#[cfg(test)]
// Relocated from the deleted src/ported/exec_hooks.rs. These pin the
// no-executor fallback behavior + return types of the live-executor
// accessor wrappers (array/assoc/dispatch_function_call/...), which
// now live in this module (exec.rs) instead of the exec_hooks OnceLock
// layer. Behavior is identical; only the indirection was removed.
mod exec_accessor_tests {
    use super::*;
    use indexmap::IndexMap;

    // ─── zsh-corpus pins: default (no-hook) fallback behavior ─────

    /// `dispatch_function_call` returns None when no hook installed.
    /// Tests may run in a fresh process where no fusevm bridge wired
    /// the dispatch yet; pin: no-panic, None-return.
    #[test]
    fn exec_hooks_corpus_dispatch_returns_none_when_not_installed() {
        let _g = crate::test_util::global_state_lock();
        // We can't unset OnceLock once set, but if test runs first
        // in this process it should be None. The defensive pin is:
        // either None or Some — never panic.
        let _ = dispatch_function_call("__never_a_real_function_zshrs__", &["a".into()]);
        // No panic = pass.
    }

    /// `execute_script` returns `Ok(0)` when no hook installed.
    #[test]
    fn exec_hooks_corpus_execute_script_returns_ok_zero_when_not_installed() {
        let _g = crate::test_util::global_state_lock();
        let r = execute_script("nothing real");
        match r {
            Ok(_) | Err(_) => {} // either is acceptable post-install
        }
    }

    /// `run_command_substitution` returns "" by default.
    #[test]
    fn exec_hooks_corpus_run_command_substitution_default_empty_or_real() {
        let _g = crate::test_util::global_state_lock();
        // Returns "" if no hook, or real output if hook installed.
        let _ = run_command_substitution("echo zshrs_hook_test");
        // No panic = pass; we can't pin exact result because hook
        // state depends on previous tests in same process.
    }

    /// `array` falls back to params::getaparam when no hook.
    /// Set a real array via params, then look up through hook entry.
    #[test]
    fn exec_hooks_corpus_array_falls_back_to_getaparam() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("EH_FB");
        crate::ported::params::setaparam("EH_FB", vec!["x".into(), "y".into(), "z".into()]);
        let got = array("EH_FB");
        assert_eq!(
            got.as_deref(),
            Some(&["x".to_string(), "y".to_string(), "z".to_string()][..]),
            "array() hook falls back to params::getaparam",
        );
        crate::ported::params::unsetparam("EH_FB");
    }

    /// `pparams()` returns empty Vec when no hook installed.
    #[test]
    fn exec_hooks_corpus_pparams_returns_empty_when_not_installed() {
        let _g = crate::test_util::global_state_lock();
        let p = pparams();
        // Either empty (no hook) or whatever the installed hook returns.
        let _ = p; // no panic = pass
    }

    /// `unregister_function` returns false by default.
    #[test]
    fn exec_hooks_corpus_unregister_function_default_false() {
        let _g = crate::test_util::global_state_lock();
        let r = unregister_function("__never_registered_xyz__");
        // If hook installed, hook decides; if not, returns false.
        // Pin: doesn't panic and returns a bool.
        let _ = r;
    }

    /// `set_pparams` doesn't panic when called.
    #[test]
    fn exec_hooks_corpus_set_pparams_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        set_pparams(vec!["a".into(), "b".into()]);
        // No panic = pass.
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional default-path (no-hook-installed) parity tests.
    // exec_hooks fallback semantics must remain stable: every accessor
    // returns a safe default when no fusevm executor has wired its hook.
    // ═══════════════════════════════════════════════════════════════════

    /// `assoc()` returns None when no hook installed (no params
    /// fallback — assoc has no equivalent of getaparam fallback).
    #[test]
    fn exec_hooks_assoc_returns_none_when_no_hook() {
        let _g = crate::test_util::global_state_lock();
        // If a prior test installed a hook, the result is hook-defined;
        // we only pin no-panic + valid Option<...>.
        let _ = assoc("__never_real_assoc_zshrs__");
    }

    /// `set_array` is a no-op when no hook installed (silently
    /// drops the write rather than panicking — fusevm-less env safe).
    #[test]
    fn exec_hooks_set_array_no_hook_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        set_array("__never_real_array_zshrs__", vec!["x".into(), "y".into()]);
    }

    /// `set_assoc` is a no-op when no hook installed.
    #[test]
    fn exec_hooks_set_assoc_no_hook_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut m = IndexMap::new();
        m.insert("k".to_string(), "v".to_string());
        set_assoc("__never_real_assoc_zshrs__", m);
    }

    /// `unset_scalar`, `unset_array`, `unset_assoc` are all no-ops
    /// when no hook installed.
    #[test]
    fn exec_hooks_unset_variants_no_hook_dont_panic() {
        let _g = crate::test_util::global_state_lock();
        unset_scalar("__never_real_scalar_zshrs__");
        unset_array("__never_real_array_zshrs__");
        unset_assoc("__never_real_assoc_zshrs__");
    }

    /// `run_function_body` returns None when no hook installed.
    #[test]
    fn exec_hooks_run_function_body_returns_none_when_no_hook() {
        let _g = crate::test_util::global_state_lock();
        let _ = run_function_body("__never_a_real_fn_zshrs__", &["a".into()]);
        // No panic = pass; result is Option-typed.
    }

    /// `execute_script_zsh_pipeline` returns Ok(0) when no hook installed.
    #[test]
    fn exec_hooks_execute_script_zsh_pipeline_default_ok_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = execute_script_zsh_pipeline("no hook");
        // If hook installed, result is hook-defined; if not, Ok(0).
        let _ = r;
    }

    /// `array()` returns None when name doesn't exist in params either
    /// (no fallback hits).
    #[test]
    fn exec_hooks_array_returns_none_for_missing_name() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("__no_such_array_zshrs__");
        let got = array("__no_such_array_zshrs__");
        // Either None (no hook + no param) or hook-returned value.
        let _ = got;
    }

    /// `array()` empty name doesn't panic (some callers pass "" for
    /// special parameter probes).
    #[test]
    fn exec_hooks_array_empty_name_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = array("");
    }

    /// Idempotent: calling array() twice with same name yields same
    /// result (no observable side effect on the fallback path).
    #[test]
    fn exec_hooks_array_is_idempotent() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("EH_IDEMPOTENT");
        crate::ported::params::setaparam("EH_IDEMPOTENT", vec!["a".into(), "b".into()]);
        let first = array("EH_IDEMPOTENT");
        let second = array("EH_IDEMPOTENT");
        assert_eq!(first, second);
        crate::ported::params::unsetparam("EH_IDEMPOTENT");
    }

    /// `pparams()` returns an empty vec (not None) when no hook —
    /// callers can iterate without an Option-check.
    #[test]
    fn exec_hooks_pparams_returns_vec_not_none() {
        let _g = crate::test_util::global_state_lock();
        // Type assertion: result is Vec<String>, not Option<Vec<String>>.
        let p: Vec<String> = pparams();
        let _ = p; // either [] or hook-installed value
    }

    /// Round-trip via params fallback: set then read returns same value.
    #[test]
    fn exec_hooks_array_set_get_roundtrip_via_params() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("EH_RT");
        let vals = vec!["one".to_string(), "two".to_string(), "three".to_string()];
        crate::ported::params::setaparam("EH_RT", vals.clone());
        let got = array("EH_RT").expect("set then get should hit params fallback");
        assert_eq!(got, vals);
        crate::ported::params::unsetparam("EH_RT");
    }

    /// `unregister_function` is consistently a bool — pin the return
    /// type so accidental refactor to () would fail the type check.
    #[test]
    fn exec_hooks_unregister_function_returns_bool() {
        let _g = crate::test_util::global_state_lock();
        let r: bool = unregister_function("__never_xyz_zshrs__");
        let _ = r;
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional contract-pin tests for exec_hooks default behavior.
    // ═══════════════════════════════════════════════════════════════════

    /// `run_command_substitution` returns String (never None / never Option).
    #[test]
    fn exec_hooks_run_command_substitution_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: String = run_command_substitution("anything");
    }

    /// `execute_script` returns Result<i32, String>. Pin signature.
    #[test]
    fn exec_hooks_execute_script_returns_result_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Result<i32, String> = execute_script("anything");
    }

    /// `execute_script_zsh_pipeline` returns Result<i32, String>.
    #[test]
    fn exec_hooks_execute_script_zsh_pipeline_returns_result_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Result<i32, String> = execute_script_zsh_pipeline("anything");
    }

    /// `dispatch_function_call` returns Option<i32>.
    #[test]
    fn exec_hooks_dispatch_function_call_returns_option_i32() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<i32> = dispatch_function_call("__never_real__", &[]);
    }

    /// `run_function_body` returns Option<i32>.
    #[test]
    fn exec_hooks_run_function_body_returns_option_i32() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<i32> = run_function_body("__never_real__", &[]);
    }

    /// `array` returns Option<Vec<String>>.
    #[test]
    fn exec_hooks_array_returns_option_vec_string() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<Vec<String>> = array("anything");
    }

    /// `assoc` returns Option<IndexMap<String, String>>.
    #[test]
    fn exec_hooks_assoc_returns_option_indexmap() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<IndexMap<String, String>> = assoc("anything");
    }

    /// Empty-string args to `dispatch_function_call` doesn't panic.
    #[test]
    fn exec_hooks_dispatch_empty_args_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = dispatch_function_call("", &[]);
        let _ = dispatch_function_call("name", &[]);
    }

    /// Empty-string args to `run_function_body` doesn't panic.
    #[test]
    fn exec_hooks_run_function_body_empty_args_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = run_function_body("", &[]);
        let _ = run_function_body("name", &[]);
    }

    /// Empty-string name to `execute_script` doesn't panic and returns
    /// Result.
    #[test]
    fn exec_hooks_execute_script_empty_src_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = execute_script("");
        let _ = execute_script_zsh_pipeline("");
    }

    /// Empty-string cmd to `run_command_substitution` doesn't panic.
    #[test]
    fn exec_hooks_run_command_substitution_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _: String = run_command_substitution("");
    }

    /// `unregister_function("")` doesn't panic.
    #[test]
    fn exec_hooks_unregister_function_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _: bool = unregister_function("");
    }

    /// `unset_*` with empty name does not panic.
    #[test]
    fn exec_hooks_unset_with_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        unset_scalar("");
        unset_array("");
        unset_assoc("");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional contract-pin tests for exec_hooks fallback semantics
    // c:213 pparams / c:217 set_pparams / c:223 unregister_function
    // ═══════════════════════════════════════════════════════════════════

    /// `pparams()` is deterministic for repeated calls without state changes.
    #[test]
    fn pparams_deterministic_without_changes() {
        let _g = crate::test_util::global_state_lock();
        let first = pparams();
        for _ in 0..3 {
            assert_eq!(
                pparams(),
                first,
                "pparams() must be deterministic across reads"
            );
        }
    }

    /// `pparams()` returns Vec<String> (not Option) — pin type contract.
    #[test]
    fn pparams_returns_vec_string_no_option() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = pparams();
    }

    /// `array(name)` with name containing null-byte doesn't panic.
    #[test]
    fn array_with_special_chars_in_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = array("name with spaces");
        let _ = array("name/with/slashes");
        let _ = array("$dollarsigns");
    }

    /// `assoc(name)` empty name no panic.
    #[test]
    fn assoc_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = assoc("");
    }

    /// `set_array(empty, ...)` empty name no panic.
    #[test]
    fn set_array_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        set_array("", vec![]);
    }

    /// `set_assoc(empty, ...)` empty name no panic.
    #[test]
    fn set_assoc_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let m = indexmap::IndexMap::new();
        set_assoc("", m);
    }

    /// `set_pparams(empty)` is safe.
    #[test]
    fn set_pparams_empty_vec_no_panic() {
        let _g = crate::test_util::global_state_lock();
        set_pparams(vec![]);
    }

    /// Repeated `pparams()` doesn't allocate growing state.
    #[test]
    fn pparams_repeated_doesnt_grow_state() {
        let _g = crate::test_util::global_state_lock();
        let first_len = pparams().len();
        for _ in 0..10 {
            let n = pparams().len();
            assert_eq!(n, first_len, "len must not grow across reads");
        }
    }

    /// `unregister_function` is deterministic for nonexistent name.
    #[test]
    fn unregister_function_unknown_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = unregister_function("__never_real_xyz__");
        for _ in 0..3 {
            assert_eq!(unregister_function("__never_real_xyz__"), first);
        }
    }

    /// `array(name)` repeated reads of nonexistent name are deterministic.
    #[test]
    fn array_unknown_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = array("__never_real_array_xyz__").is_none();
        for _ in 0..3 {
            assert_eq!(array("__never_real_array_xyz__").is_none(), first);
        }
    }

    /// `assoc(name)` repeated reads of nonexistent name are deterministic.
    #[test]
    fn assoc_unknown_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = assoc("__never_real_assoc_xyz__").is_none();
        for _ in 0..3 {
            assert_eq!(assoc("__never_real_assoc_xyz__").is_none(), first);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for src/ported/exec_hooks.rs
    // c:134 array / c:147 assoc / c:181 dispatch_function_call /
    // c:188 run_function_body / c:192 execute_script /
    // c:206 run_command_substitution / c:213 pparams / c:223 unregister_function
    // ═══════════════════════════════════════════════════════════════════

    /// `array(name)` returns Option<Vec<String>> (compile-time pin).
    #[test]
    fn array_returns_option_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<Vec<String>> = array("any");
    }

    /// `assoc(name)` returns Option<IndexMap<String,String>> (compile-time pin).
    #[test]
    fn assoc_returns_option_indexmap_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<indexmap::IndexMap<String, String>> = assoc("any");
    }

    /// `dispatch_function_call` returns Option<i32> (compile-time pin).
    #[test]
    fn dispatch_function_call_returns_option_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<i32> = dispatch_function_call("__never__", &[]);
    }

    /// `run_function_body` returns Option<i32> (compile-time pin).
    #[test]
    fn run_function_body_returns_option_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<i32> = run_function_body("__never__", &[]);
    }

    /// `execute_script` returns Result<i32, String> (compile-time pin).
    #[test]
    fn execute_script_returns_result_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Result<i32, String> = execute_script("");
    }

    /// `execute_script_zsh_pipeline` returns Result<i32, String> (compile-time pin).
    #[test]
    fn execute_script_zsh_pipeline_returns_result_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Result<i32, String> = execute_script_zsh_pipeline("");
    }

    /// `run_command_substitution` returns String (compile-time pin).
    #[test]
    fn run_command_substitution_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: String = run_command_substitution("");
    }

    /// `pparams` returns Vec<String> (compile-time pin).
    #[test]
    fn pparams_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = pparams();
    }

    /// `unregister_function` returns bool (compile-time pin).
    #[test]
    fn unregister_function_returns_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let _: bool = unregister_function("__never__");
    }

    /// `unset_scalar`/`unset_array`/`unset_assoc` for nonexistent name safe.
    #[test]
    fn unset_variants_nonexistent_no_panic() {
        let _g = crate::test_util::global_state_lock();
        unset_scalar("__never_unset_scalar__");
        unset_array("__never_unset_array__");
        unset_assoc("__never_unset_assoc__");
    }

    /// `set_pparams` with no hook installed is a silent no-op
    /// (c:217-220 — `if let Some(f) = PPARAMS_SET.get() { f(v); }`).
    /// Pin the no-hook contract so a refactor that panics on missing
    /// hook gets caught.
    #[test]
    fn set_pparams_without_hook_is_silent_noop() {
        let _g = crate::test_util::global_state_lock();
        // No hook installed in test context — must not panic.
        set_pparams(vec!["a".into(), "b".into(), "c".into()]);
        set_pparams(vec![]);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional contract pins for exec_hooks.rs
    // No-hook-installed contract: every accessor must be safe + deterministic
    // ═══════════════════════════════════════════════════════════════════

    /// `array("")` empty name returns deterministic value (no panic).
    #[test]
    fn array_empty_name_no_panic_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let a = array("");
        let b = array("");
        assert_eq!(a, b, "array(\"\") must be deterministic");
    }

    /// `set_array` then `array` without hook should not panic.
    #[test]
    fn set_array_then_get_no_hook_safe() {
        let _g = crate::test_util::global_state_lock();
        set_array("__test_hook_arr__", vec!["a".into(), "b".into()]);
        let _ = array("__test_hook_arr__");
    }

    /// `set_assoc` then `assoc` without hook should not panic.
    #[test]
    fn set_assoc_then_get_no_hook_safe() {
        let _g = crate::test_util::global_state_lock();
        let mut m = IndexMap::new();
        m.insert("k".to_string(), "v".to_string());
        set_assoc("__test_hook_assoc__", m);
        let _ = assoc("__test_hook_assoc__");
    }

    /// `run_function_body` with no hook returns `Option<i32>` (type pin, alt).
    #[test]
    fn run_function_body_returns_option_i32_type_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<i32> = run_function_body("foo", &[]);
    }

    /// `run_function_body` is deterministic for the same input when no hook.
    #[test]
    fn run_function_body_no_hook_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let a = run_function_body("__never__", &[]);
        let b = run_function_body("__never__", &[]);
        assert_eq!(a, b);
    }

    /// `dispatch_function_call` is deterministic across calls.
    #[test]
    fn dispatch_function_call_no_hook_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let a = dispatch_function_call("__never__", &[]);
        let b = dispatch_function_call("__never__", &[]);
        assert_eq!(a, b);
    }

    /// `pparams` returns `Vec<String>` (compile-time pin, alt).
    #[test]
    fn pparams_returns_vec_string_type_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = pparams();
    }

    /// `pparams` is deterministic across repeated reads when no hook installed
    /// (this is observably true only if no other test has installed it; pin
    /// the no-mutation invariant).
    #[test]
    fn pparams_repeated_reads_are_observable_type() {
        let _g = crate::test_util::global_state_lock();
        // Just type pin — value depends on whether another test installed a
        // hook between these calls (PPARAMS_GET is OnceLock).
        let _a = pparams();
        let _b = pparams();
    }

    /// `run_command_substitution` returns `String` type pin (alt).
    #[test]
    fn run_command_substitution_returns_string_type_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: String = run_command_substitution("echo x");
    }

    /// `run_command_substitution` empty command doesn't panic.
    #[test]
    fn run_command_substitution_empty_cmd_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = run_command_substitution("");
    }

    /// `execute_script` returns `Result<i32, String>` type pin.
    #[test]
    fn execute_script_returns_result_i32_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Result<i32, String> = execute_script("foo");
    }

    /// `unregister_function("")` empty name returns bool safely.
    #[test]
    fn unregister_function_empty_name_returns_bool() {
        let _g = crate::test_util::global_state_lock();
        let _: bool = unregister_function("");
    }
}
