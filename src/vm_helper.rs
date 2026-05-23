//! Shell executor state for zshrs.
//!
//! **Not a port of Src/exec.c.** C zsh runs compiled programs on the native
//! **wordcode walker** in `Src/exec.c` (`execlist` / `execpline` / `execcmd`).
//! zshrs uses fusevm bytecode instead; the bridge lives in `src/fusevm_bridge.rs`.
//! This file holds:
//! - `ShellExecutor` — the runtime state struct that the VM and
//!   every ported builtin/utility threads through
//! - VM-adjacent helpers that read/write that state
//! - drift extension scaffolding still being moved out
//!
//! Path-wise this file lives at the crate root (`src/vm_helper`) rather
//! than in `src/ported/` because nothing here corresponds 1:1 to a
//! `Src/*.c` source file. `crate::ported::exec` is kept as a
//! re-export alias so existing call-sites continue to compile.

use crate::history::HistoryEngine;
// MathState is private to math.rs (per math.c — no public state struct);
// math API surface is matheval/mathevali/mnumber.
use crate::options::ZSH_OPTIONS_SET;
// TcpSessions struct deleted — see modules/tcp.rs ZTCP_SESSIONS thread_local.
// `Profiler`/`ProfileEntry` deleted in the zprof.rs strict-rules
// rewrite — zprof state now lives in module-level statics
// (`CALLS`/`NCALLS`/`ARCS`/`NARCS`/`STACK`/`ZPROF_MODULE`) matching
// the C file-statics at zprof.c:66-71.
use crate::ported::builtin::RETFLAG;
use crate::ported::builtin::{BREAKS, CONTFLAG, LOOPS};
use crate::ported::math::mathevali;
use crate::ported::modules::parameter::*;
use crate::ported::parse::ecgetstr_wordcode;
use crate::ported::parse::ecgetstr_wordcode as ecgetstr;
use crate::ported::parse::ECBUF;
use crate::ported::subst::singsub;
use crate::ported::utils::{errflag, ERRFLAG_ERROR};
use crate::ported::zle::zle_thingy::{getwidgettarget, listwidgets};
use crate::ported::zsh_h::PM_UNDEFINED;
use crate::ported::zsh_h::WC_PIPE;
use crate::ported::zsh_h::WC_REPEAT_SKIP;
use crate::ported::zsh_h::WC_SUBLIST;
use crate::ported::zsh_h::{options, MAX_OPS};
use crate::ported::zsh_h::{wc_code, wc_data, WC_END, WC_LIST};
use crate::ported::zsh_h::{
    PM_ARRAY, PM_EFLOAT, PM_EXPORTED, PM_FFLOAT, PM_HASHED, PM_INTEGER, PM_LEFT, PM_LOWER,
    PM_READONLY, PM_RIGHT_B, PM_RIGHT_Z, PM_UPPER,
};
use crate::ported::zsh_h::{
    WC_ARITH, WC_CASE, WC_COND, WC_CURSH, WC_FOR, WC_FUNCDEF, WC_IF, WC_REPEAT, WC_SELECT,
    WC_SIMPLE, WC_SUBSH, WC_TIMED, WC_TRY, WC_WHILE,
};
use crate::ported::zsh_h::{WC_CASE_SKIP, WC_CASE_TYPE};
use crate::ported::zsh_h::{WC_FOR_LIST, WC_FOR_SKIP, WC_FOR_TYPE};
use crate::ported::zsh_h::{WC_IF_SKIP, WC_IF_TYPE};
use crate::ported::zsh_h::{WC_WHILE_SKIP, WC_WHILE_TYPE};
use compsys::cache::CompsysCache;
use compsys::CompInitResult;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::ffi::CStr;
use std::ffi::CString;
use std::fs;
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::FileTypeExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::FromRawFd;
use std::sync::atomic::AtomicI32;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};
use walkdir::WalkDir;

// Backward-compat re-exports for free fns recently relocated to their
// canonical-C-file Rust modules. Existing call-sites in this file (and
// elsewhere) still reference these unqualified.
#[allow(unused_imports)]
#[allow(unused_imports)]
// drift imports removed: apply_subst_modifier, slice_scalar, strip_match_op
#[allow(unused_imports)]
pub(crate) use crate::func_body_fmt::FuncBodyFmt;
#[allow(unused_imports)]
#[allow(unused_imports)]
pub(crate) use crate::ported::glob::expand_glob_alternation;
#[allow(unused_imports)]
pub(crate) use crate::ported::hist::bufferwords as bufferwords_z_tuple;
#[allow(unused_imports)]
pub(crate) use crate::ported::math::{parse_assign, parse_compound, parse_pre_inc};
#[allow(unused_imports)]
pub use crate::ported::params::convbase as format_int_in_base;
pub use crate::ported::params::convbase_underscore;
#[allow(unused_imports)]
pub(crate) use crate::ported::params::getarrvalue;
#[allow(unused_imports)]
pub(crate) use crate::ported::utils::base64_decode;
#[allow(unused_imports)]
pub(crate) use crate::ported::utils::{ispwd, printprompt4, quotedzputs};

pub(crate) use crate::intercepts::intercept_matches;
/// AOP advice type — before, after, or around.
pub use crate::intercepts::{AdviceKind, Intercept};

/// Result from background compinit thread.
pub use crate::compinit_bg::CompInitBgResult;
use std::io::Write;
use std::sync::LazyLock;

/// State snapshot for plugin delta computation.
pub(crate) use crate::plugin_cache::PluginSnapshot;

/// Cached compiled regexes for hot paths
pub(crate) static REGEX_CACHE: LazyLock<Mutex<HashMap<String, Regex>>> =
    LazyLock::new(|| Mutex::new(HashMap::with_capacity(64)));

// `TRAP_STATE`, `TRAP_RETURN`, and `FORKLEVEL` (ports of the `int
// trap_state;` / `int trap_return;` / `int forklevel;` file-static
// globals from `Src/exec.c:134 / :155 / :1052`) moved to their
// canonical port file at `src/ported/exec.rs`. Reference them as
// `crate::ported::exec::{TRAP_STATE, TRAP_RETURN, FORKLEVEL}` from
// other modules.

// ───────────────────────────────────────────────────────────────────────────
// fusevm VM bridge (extension; not a port of Src/exec.c) lives in
// src/fusevm_bridge.rs. The bridge re-exports the symbols that the
// rest of the codebase imports as `crate::ported::exec::X`.
// ───────────────────────────────────────────────────────────────────────────
pub(crate) use crate::fusevm_bridge::ExecutorContext;
pub use crate::fusevm_bridge::*;

/// `ZSH_VERSION` / `ZSH_PATCHLEVEL` / `ZSH_VERSION_DATE` consts
/// generated by `build.rs` from `src/zsh/Config/version.mk`. Use
/// `zsh_version::ZSH_VERSION` etc. at call sites so version bumps
/// pick up automatically.
pub mod zsh_version {
    include!(concat!(env!("OUT_DIR"), "/zsh_version.rs"));
}

// `gethere` (port of `Src/exec.c:4573`) and `getoutput` (port of
// `Src/exec.c:4712`) moved to their canonical port file at
// `src/ported/exec.rs`. Reference them as
// `crate::ported::exec::{gethere, getoutput}`.

/// Match an intercept pattern against a command name or full command string.
/// Supports: exact match, glob ("git *", "_*", "*"), or "all".


/// O(1) builtin-name lookup set derived from the canonical
/// `BUILTINS` table (`src/ported/builtin.rs:122`, the 1:1 port of
/// `static struct builtin builtins[]` at `Src/builtin.c:40-137`).
/// Earlier incarnation hardcoded a separate 130-entry list which
/// drifted whenever new builtins landed in the canonical table — and
/// shadowed the `fusevm::shell_builtins::BUILTIN_SET` u16 opcode
/// constant. Renaming to `BUILTIN_NAMES` removes the shadow; the
/// initialiser walks `BUILTINS` so the set stays in sync.
///
/// The hardcoded entries inside `LazyLock::new` below are kept as
/// the union of: (1) names from `BUILTINS` (walked at first access),
/// (2) zshrs daemon-side builtins from `ZSHRS_BUILTIN_NAMES`. Both
/// arms run once at static init.
pub(crate) static BUILTIN_NAMES: LazyLock<HashSet<String>> = LazyLock::new(|| {
    let mut s: HashSet<String> = HashSet::new();
    // Walk the canonical `BUILTINS` table — the 1:1 port of
    // `static struct builtin builtins[]` at `Src/builtin.c:40-137`
    // (ported at `src/ported/builtin.rs:122`). Every name in there is
    // a real zsh builtin; the set stays in sync as new ports land.
    for b in crate::ported::builtin::BUILTINS.iter() {
        s.insert(b.node.nam.clone());
    }
    // Daemon-side (zshrs-specific extensions).
    for &n in crate::daemon::builtins::ZSHRS_BUILTIN_NAMES.iter() {
        s.insert(n.to_string());
    }
    s
});



use crate::exec_jobs::{JobState, JobTable};
use crate::parse::{Redirect, RedirectOp, ShellCommand, ShellWord, VarModifier, ZshParamFlag};
use crate::zwc::ZwcFile;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::env;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

// Drift structs moved to their canonical-C-file modules
// (src/ported/zle/computil.rs, modules/{zutil,zpty,zprof,socket}.rs,
// builtins/sched.rs). Re-exported here so existing call-sites that
// reference `crate::ported::exec::<Name>` keep compiling.
pub use crate::bash_complete::{CompGroup, CompMatch, CompSpec, CompState};
pub use crate::ported::modules::zutil::zstyle_entry;
// `ProfileEntry` re-export deleted — was unused outside
// `ShellExecutor::profile_data` (which itself is now removed).
// `ScheduledCommand` (Rust-only) deleted; use `crate::builtins::sched::schedcmd`
// (port of `struct schedcmd` from Src/Builtins/sched.c:43) for live state.
pub use crate::ported::builtin::AutoloadFlags;

// `LoopSignal` enum deleted — it was a zshrs-invented dup of the
// canonical BREAKS/CONTFLAG file-statics in src/ported/builtin.rs
// (port of `breaks`/`contflag` from Src/loop.c:46/41). The docstring
// claim that it "mirrors the LF_* set" was false — no such enum
// exists in C zsh. Cross-VM break/continue now writes to BREAKS/
// CONTFLAG directly via BUILTIN_SET_BREAK / BUILTIN_SET_CONTINUE,
// matching bin_break's writes at Src/builtin.c::bin_break c:5836+.

/// Snapshot of subshell-isolated state. Captured at `(` entry, restored at
/// `)` exit. zsh subshell semantics: assignments inside `(…)` don't leak to
/// the outer scope — and that includes `export`. zsh forks a child for the
/// subshell so the child's env::set_var dies with the child; without a fork
/// (zshrs runs subshells in-process for perf), we snapshot+restore the OS
/// env table around the subshell. Otherwise `(export y=v)` would leak `y`
/// to the parent shell, breaking every script that uses a subshell to
/// scope an env override.
/// Snapshot of mutable executor state across a subshell
/// boundary.
/// Port of the `entersubsh()` save/restore Src/exec.c does at
/// line 1084 — captures everything that must be replaced when a
/// `(...)` group fires.
pub struct SubshellSnapshot {
    /// Snapshot of `paramtab` (the C-canonical parameter store) at
    /// subshell entry. Step 1 of the unification mirrors writes to
    /// paramtab, so subshell-scoped assignments now show up there
    /// too — without this snapshot, restoring only `variables` /
    /// `arrays` / `assoc_arrays` leaks the subshell's writes to the
    /// parent via paramtab (e.g. `x=outer; (x=inner); echo $x` returned
    /// `inner` because paramsubst reads through paramtab).
    pub paramtab: HashMap<String, crate::ported::zsh_h::Param>,
    pub paramtab_hashed_storage: HashMap<String, IndexMap<String, String>>,
    pub positional_params: Vec<String>,
    pub env_vars: HashMap<String, String>,
    /// Process working directory at subshell entry. `cd` inside the
    /// subshell shouldn't leak to the parent; we restore on End.
    pub cwd: Option<PathBuf>,
    /// File-creation mask at subshell entry. zsh forks for `(...)` so
    /// `umask` set inside dies with the child; we run subshells in
    /// process so we must restore the mask on End. Otherwise
    /// `umask 022; (umask 077); umask` shows 077 in the parent.
    pub umask: u32,
    /// Parent's traps at subshell entry. zsh's `(trap "echo X" EXIT;
    /// true)` runs the trap when the subshell exits — BEFORE the parent
    /// continues. Without this snapshot, the trap inherited from parent
    /// would fire, OR a trap set inside the subshell would leak to the
    /// parent's process exit. Restored on subshell_end after the
    /// subshell's own EXIT trap (if any) has fired.
    pub traps: HashMap<String, String>,
}

/// Variable attribute record + kind enum — moved to params.rs.

// Pattern helpers moved to src/ported/pattern.rs.
#[allow(unused_imports)]
pub(crate) use crate::ported::pattern::{
    extract_numeric_ranges, numeric_range_contains, numeric_ranges_to_star,
};

// `impl VarAttr` moved to src/ported/params.rs.

/// Top-level shell executor state.
/// Port of the file-static globals + `Estate` chain Src/exec.c
/// uses — `execlist()` (line 1349) drives every list, with
/// `execpline()` (line 1668), `execpline2()` (line 1991),
/// `execsimple()` (line 1290), and the per-`WC_*` `execfuncs[]`
/// table (line 268) feeding off it. The Rust port collapses
/// everything into one `ShellExecutor` so we don't need
/// thread-local globals.
pub struct ShellExecutor {
    /// Mirrors C zsh's file-static `scriptname` (Src/init.c). Used by
    /// PS4's `%N` and the `scriptname:line: …` prefix on error
    /// messages. Inside a function, MUTATES to the function name
    /// (Src/exec.c:5903 `scriptname = dupstring(name)`). Init sets
    /// this in `-c` mode to the binary basename per init.c:479; when
    /// sourcing a file via `source`/`bin_dot`, it becomes the
    /// resolved file path; otherwise it falls back through `$0` →
    /// `$ZSH_ARGZERO`.
    pub scriptname: Option<String>,
    /// Mirrors C zsh's `scriptfilename` global (Src/init.c). Tracks
    /// the FILE BEING READ (vs scriptname which tracks the active
    /// function name during a call). Used by PS4's `%x` and certain
    /// error-message prefixes that want the file location, NOT the
    /// function name.
    ///
    /// At -c-mode init, scriptname == scriptfilename == "zsh"
    /// (Src/init.c:479). When entering a function, ONLY scriptname
    /// updates (exec.c:5903); scriptfilename stays at the outer
    /// file path, so `%x` inside a function still shows the file
    /// the function was called from.
    pub scriptfilename: Option<String>,
    // `expanding_aliases` deleted — was a Rust-only HashSet recursion
    // guard duplicating C's `alias.inuse` field (`Src/zsh.h:1256`).
    // Callers now bump/clear `inuse` on the canonical alias node in
    // `aliastab` (`hashtable.rs:1804`), matching C's lexer behavior.
    // `loop_signal` deleted — was a Rust-only dup of the canonical
    // BREAKS/CONTFLAG atomics at src/ported/builtin.rs (port of
    // Src/loop.c:46 `breaks` + :41 `contflag`). Cross-VM break/
    // continue now writes BREAKS/CONTFLAG directly.
    /// Stack of subshell-state snapshots. Each `(…)` subshell pushes a copy
    /// of variables/arrays/assoc_arrays at entry and pops/restores at exit.
    /// Without this, `(x=inner; …); echo $x` shows `inner` instead of the
    /// outer-scope value.
    pub subshell_snapshots: Vec<SubshellSnapshot>,
    /// Stack of inline-assignment scopes — `X=foo Y=bar cmd` pushes
    /// a frame at the start, the assigns run inside it, and `cmd`
    /// returns into END_INLINE_ENV which restores both shell-vars
    /// and process-env to the pre-frame state. Each frame holds
    /// `(name, prev_var, prev_env)` per assigned name. zsh's
    /// equivalent is the parser-level "addvar" list executed under
    /// `addvars()` (Src/exec.c) right before the command exec.
    pub inline_env_stack: Vec<Vec<(String, Option<String>, Option<String>)>>,
    /// Set by `expand_glob`'s no-match arm when `nomatch` is on (zsh
    /// default) — instructs the simple-command dispatcher to skip
    /// executing the current command, set last_status=1, and continue
    /// to the next command in the script. zsh's bin_simple uses the
    /// errflag global for the same role: error printed, command
    /// suppressed, script continues. Without this we were calling
    /// `process::exit(1)` deep inside expand_glob, killing the whole
    /// shell on any unmatched glob even with multi-statement input.
    /// `Cell` because the no-match site only has a `&self` borrow.
    pub current_command_glob_failed: std::cell::Cell<bool>,
    pub jobs: JobTable,
    pub fpath: Vec<PathBuf>,
    pub zwc_cache: HashMap<PathBuf, ZwcFile>,
    pub history: Option<HistoryEngine>,
    /// Session-relative history line counter. Starts at 0; incremented
    /// when an interactive command is recorded. Used by `%h`/`%!` in
    /// prompt expansion (zsh's "current history line number"), distinct
    /// from the persistent disk history total.
    pub session_histnum: i64,
    pub(crate) process_sub_counter: u32,
    pub traps: HashMap<String, String>,
    // `options` field deleted — dup of canonical `OPTS_LIVE` in
    // `src/ported/options.rs:1112`. Callers route through
    // `opt_state_get`/`opt_state_set`/`opt_state_unset`/`opt_state_snapshot`.
    pub completions: HashMap<String, CompSpec>, // command -> completion spec
    // `dir_stack` field deleted — canonical `DIRSTACK` lives in
    // `modules/parameter.rs:398` (mirror of C `dirstack` global at
    // `Src/builtin.c:1456`). Callers go through that Mutex directly.
    // zsh completion system state
    pub comp_matches: Vec<CompMatch>, // Current completion matches
    pub comp_groups: Vec<CompGroup>,  // Completion groups
    pub comp_state: CompState,        // compstate associative array
    pub zstyles: Vec<zstyle_entry>,   // zstyle configurations
    pub comp_words: Vec<String>,      // words on command line
    pub comp_current: i32,            // current word index (1-based)
    pub comp_prefix: String,          // PREFIX parameter
    pub comp_suffix: String,          // SUFFIX parameter
    pub comp_iprefix: String,         // IPREFIX parameter
    pub comp_isuffix: String,         // ISUFFIX parameter
    // `readonly_vars` deleted — was a never-populated HashSet
    // duplicating the canonical `PM_READONLY` flag check on Param
    // (`zsh_h::PM_READONLY` bit on `Param.node.flags`). Callers go
    // through `is_readonly_param(name)`.
    // `last_subst` deleted — 0 callers. Canonical `hsubl`/`hsubr`
    // globals live in `Src/hist.c` and are ported on demand when
    // `:&` history-modifier replay arrives in zshrs.
    // `sub_flags` deleted — zero real callers; canonical lives in
    // `SUB_FLAGS` thread_local at `src/ported/subst.rs:498` (`sub_flags`
    // global in `Src/subst.c:2169`), accessed via `sub_flags_get` /
    // `sub_flags_set`.
    /// Current function scope depth for `local` tracking.
    pub local_scope_depth: usize,
    /// Last arg of the currently-running command, deferred into `$_`
    /// when the next command dispatches. zsh: `$_` reflects the LAST
    /// command's last arg, so `echo hi; echo $_` prints `hi` (not the
    /// `_` arg of `echo $_` itself). Promoted in `pop_args` and
    /// `host.exec` before the command's args are read.
    pub pending_underscore: Option<String>,
    /// True while expanding inside a double-quoted context. Set by
    /// `BUILTIN_EXPAND_TEXT` mode 1 around `expand_string` calls.
    /// Used by parameter-flag application to suppress array-only flags
    /// (`(o)`/`(O)`/`(n)`/`(i)`/`(M)`/`(u)`) — zsh's behaviour: those
    /// flags only fire in array context.
    pub in_dq_context: u32,
    // `in_paramsubst_nest` deleted — canonical lives in
    // `IN_PARAMSUBST_NEST` thread_local at `subst.rs:464` (mirrors
    // `paramsub_nest` global in `Src/subst.c`). Callers read it
    // directly via `crate::ported::subst::IN_PARAMSUBST_NEST.with(...)`.
    /// True (>0) while expanding the RHS of a scalar assignment.
    /// Direct port of zsh's `PREFORK_SINGLE` bit set by
    /// Src/exec.c::addvars line 2546 (`prefork(vl, isstr ?
    /// (PREFORK_SINGLE|PREFORK_ASSIGN) : PREFORK_ASSIGN, ...)`).
    /// Subst_port's paramsubst reads this via `ssub` and suppresses
    /// `(f)` / `(s:STR:)` / `(0)` / `(z)` split flags per
    /// Src/subst.c:1759 + 3902, so `y="${(f)x}"` preserves x's
    /// original separator (newlines) instead of re-joining with
    /// IFS-first-char (space).
    pub in_scalar_assign: u32,
    // `cmd_stack` deleted — duplicated the canonical `prompt::CMDSTACK`
    // thread_local (`Src/prompt.c:56 unsigned char *cmdstack`).
    // `BUILTIN_CMD_PUSH`/`BUILTIN_CMD_POP` now call `cmdpush`/`cmdpop`
    // on the canonical TLS only; prompt expansion reads it directly.
    /// IDs of history entries explicitly added during this session
    /// via `print -s`. `fc -l` uses this to scope listings to just
    /// the script-added entries (matches zsh's `-c` semantics where
    /// session history is the only thing visible to the script).
    pub session_history_ids: Vec<i64>,
    // `autoload_pending` deleted — dup of canonical shfunctab entries
    // with PM_UNDEFINED flag bit (port of C autoload_func stub at
    // `Src/exec.c:5215`). The -U/-z/-k/-t/-d AutoloadFlags details
    // were never consumed beyond serialization, dropped along with
    // the field.
    // `hook_functions` deleted — Rust-only side-store duplicating zsh's
    // canonical `<hook>_functions` paramtab arrays (the add-zsh-hook
    // idiom). `add_hook` / `delete_hook` now mutate those arrays
    // directly via `setaparam`.
    // `named_dirs` deleted — canonical `nameddirtab` lives in
    // `src/ported/hashnameddir.rs:36` (port of C `nameddirtab` in
    // `Src/hashnameddir.c`). Callers route through that Mutex.
    // bin_sysopen - file descriptor management
    pub open_fds: HashMap<i32, File>,
    pub next_fd: i32,
    // sched (Src/Builtins/sched.c) — schedcmds list lives in module
    // statics in the canonical port; nothing to carry on ShellExecutor.
    // zprof — profiling data lives in `crate::zprof` module statics
    // (CALLS/NCALLS/ARCS/NARCS/STACK), matching the C file-statics
    // at zprof.c:66-71. Only the user's "is profiling on?" toggle
    // stays here, set by the `profile` extension builtin.
    pub profiling_enabled: bool,
    // compsys - completion system cache
    pub compsys_cache: Option<CompsysCache>,
    // Background compinit — receiver for async fpath scan result
    pub compinit_pending: Option<(
        std::sync::mpsc::Receiver<CompInitBgResult>,
        std::time::Instant,
    )>,
    // Plugin source cache — stores side effects of source/. in SQLite
    pub plugin_cache: Option<crate::plugin_cache::PluginCache>,
    // cdreplay - deferred compdef calls for zinit turbo mode
    pub deferred_compdefs: Vec<Vec<String>>,
    // `command_hash` deleted — never-populated dup of canonical
    // `cmdnamtab` (`hashtable.rs:1780`, port of `Src/exec.c:5260`
    // findcmd's hash table). Callers route through cmdnamtab.
    // Control flow signals
    pub returning: Option<i32>, // Set by return builtin, cleared after function returns
    pub breaking: i32,          // break level (0 = not breaking, N = break N levels)
    pub continuing: i32,        // continue level
    // New module state — TcpSessions struct dissolved into the
    // thread_local ZTCP_SESSIONS in modules/tcp.rs (matches C's
    // file-static `ztcp_sessions` linked list).
    // `zftp` field deleted — 0 callers. Module-level state lives in
    // `ZFTP_STATE_INNER` (Src/Modules/zftp.c file-statics analogue).
    // `profiler: Profiler` deleted — see comment above.
    // `style_table` field deleted — 0 callers. Canonical `zstyletab`
    // lives in `src/ported/modules/zutil.rs::zstyletab` (LazyLock
    // Mutex matching C's `static HashTable zstyletab` at zutil.c:209).
    // termcap state dissolved per strict-rules audit — no Rust-only
    // Termcap struct; capability_lookup is stateless on $TERM.
    // Watch state — dissolved per PORT_PLAN Phase 2. C
    // (Src/Modules/watch.c:150-156) keeps `wtab`/`lastwatch`/
    // `lastutmpcheck`/`watch` as file-statics; zshrs mirrors them
    // as `thread_local!`s in src/ported/modules/watch.rs.
    // curses (Src/Modules/curses.c) — windows/colour-pairs/init flag
    // now live in module-static OnceLock<Mutex<…>>'s in
    // src/ported/modules/curses.rs (matching C's file-statics
    // `zcurses_windows`, `colorpairs`, `next_pair`).
    // pty_cmds moved to PTYCMDS global static in src/ported/modules/
    // zpty.rs (port of C `static struct ptycmd *ptycmds` file-static).
    // sched: scheduled commands now live in `SCHEDCMDS` static in
    // `src/ported/builtins/sched.rs` (port of `static struct schedcmd
    // *schedcmds` from Src/Builtins/sched.c:52). No state on
    // ShellExecutor.
    /// zsh compatibility mode - use .zcompdump, fpath scanning, etc.
    /// Also serves as the `--zsh` parity-test flag: caches off, daemon
    /// off, plugin_cache replay off so every `source` re-runs the file
    /// fresh per Src/builtin.c:6080-6123 bin_dot semantics.
    pub zsh_compat: bool,
    /// bash compatibility mode (`--bash`). Same parity-mode semantics
    /// as `zsh_compat` (caches/daemon/replay off) plus bash-specific
    /// behavior tweaks where bash 5.x diverges from zsh — e.g.
    /// `BASH_VERSION` / `BASH_REMATCH` exposed, `[[ =~ ]]` populates
    /// match indices the bash way, mapfile/readarray as builtins.
    pub bash_compat: bool,
    /// POSIX sh strict mode — no SQLite, no worker pool, no zsh extensions
    pub posix_mode: bool,
    /// Worker thread pool for background tasks (compinit, process subs, etc.)
    pub worker_pool: std::sync::Arc<crate::worker::WorkerPool>,
    /// AOP intercept table: command/function name → advice chain.
    /// Glob patterns supported (e.g. "git *", "*").
    pub intercepts: Vec<Intercept>,
    /// Async job handles: id → receiver for (status, stdout)
    pub async_jobs: HashMap<u32, crossbeam_channel::Receiver<(i32, String)>>,
    /// Next async job ID
    pub next_async_id: u32,
    /// Defer stack: commands to run on scope exit (LIFO).
    pub defer_stack: Vec<Vec<String>>,
    /// Per-scope saved-fd stacks for `Op::WithRedirectsBegin/End`. Each entry
    /// is a Vec of (fd, saved_dup_fd) pairs taken from `dup(fd)` before the
    /// redirect was applied; `with_redirects_end` `dup2`s them back and closes.
    pub redirect_scope_stack: Vec<Vec<(i32, i32)>>,
    /// Set by `host_apply_redirect` when a redirect target couldn't be
    /// opened (permission denied, no such directory, etc). The next
    /// builtin/command checks this at entry and short-circuits with
    /// status 1 instead of running. Mirrors zsh's "command skip" on
    /// redirect failure.
    pub redirect_failed: bool,
    /// Stdin content set by `Op::HereDoc(idx)` / `Op::HereString` for the next
    /// command/builtin in this VM. Consumed (and cleared) by the next command.
    pub pending_stdin: Option<String>,
    /// Compiled function bodies — name → fusevm::Chunk. Populated by
    /// `BUILTIN_REGISTER_FUNCTION` (from `FunctionDef` lowering) and lazily by
    /// `ZshrsHost::call_function` when only an AST exists in `self.functions`
    /// (autoloaded, sourced, etc.). `Op::CallFunction` dispatches through here.
    pub functions_compiled: HashMap<String, fusevm::Chunk>,
    /// Canonical source text for functions. Populated by autoload paths (the
    /// raw file/cache body), runtime FuncDef compile (the parsed source span),
    /// and `unfunction` removal. Used by introspection (`whence`, `which`,
    /// `typeset -f`) instead of reconstructing from a ShellCommand AST. When a
    /// function is in `functions_compiled` but not here, introspection falls
    /// back to `text::getpermtext(self.functions[name])`.
    pub function_source: HashMap<String, String>,
    /// `first_body_line - 1` per compiled function — matches inner
    /// `ZshCompiler::lineno_offset` / zsh `funcstack->flineno` combined with
    /// relative `$LINENO` for Src/prompt.c:909 `%I`.
    pub function_line_base: HashMap<String, i64>,
    /// `scriptfilename` when `BUILTIN_REGISTER_COMPILED_FN` ran — `%x` inside
    /// a function (prompt.c:931-934) reads `funcstack->filename`.
    pub function_def_file: HashMap<String, Option<String>>,
    /// Innermost-last stack of active compiled-call frames for prompt `%I` / `%x`.
    pub prompt_funcstack: Vec<(String, i64, Option<String>)>,
    /// Scalar→(array, sep) tie table set up by `typeset -T VAR var [SEP]`.
    /// Used by BUILTIN_SET_VAR to split the assigned scalar on `sep` and
    /// mirror it into `array`.
    pub tied_scalar_to_array: HashMap<String, (String, String)>,
    /// Array→(scalar, sep) reverse-tie table. Used by BUILTIN_SET_ARRAY to
    /// join the array elements with `sep` and mirror to the scalar side.
    pub tied_array_to_scalar: HashMap<String, (String, String)>,
    /// ZLE buffer stack — port of `bufstack` (zsh/Src/builtin.c:4567,
    /// `LinkList bufstack`). `print -z` (builtin.c:5039-5045) pushes
    /// joined args onto it; `read -z` and `getln` (builtin.c:6769-6770)
    /// pop the top entry as the input source. zsh treats this as a stack
    /// shared between the buffer/zle subsystem and the read path.
    pub buffer_stack: Vec<String>,
}

impl ShellExecutor {
    /// Set a scalar parameter via the canonical `paramtab`
    /// (`Src/params.c:3350 setsparam`). The single store.
    pub fn set_scalar(&mut self, name: String, value: String) {
        setsparam(&name, &value); // c:params.c:3350
    }

    /// Read positional parameters from canonical `PPARAMS`
    /// `Mutex<Vec<String>>` (Src/init.c:pparams). The single store.
    pub fn pparams(&self) -> Vec<String> {
        crate::ported::builtin::PPARAMS
            .lock()
            .map(|p| p.clone())
            .unwrap_or_default()
    }

    /// Write positional parameters to canonical `PPARAMS`.
    pub fn set_pparams(&mut self, params: Vec<String>) {
        if let Ok(mut p) = crate::ported::builtin::PPARAMS.lock() {
            *p = params;
        }
    }

    /// Read PM_* type flags from the paramtab Param entry. Used by
    /// SET_VAR / `+=` arms (case-fold, integer-add, readonly guard)
    /// instead of the legacy `exec.var_attrs` HashMap. Returns 0 when
    /// the name isn't in paramtab. Mirrors the C source's direct
    /// `pm->node.flags & PM_INTEGER` checks.
    pub fn param_flags(&self, name: &str) -> i32 {
        paramtab()
            .read()
            .ok()
            .and_then(|t| t.get(name).map(|p| p.node.flags))
            .unwrap_or(0)
    }

    /// `typeset -i name` — Param has PM_INTEGER. Reads via
    /// `param_flags`.
    pub fn is_integer_param(&self, name: &str) -> bool {
        (self.param_flags(name) as u32 & PM_INTEGER) != 0
    }

    /// `readonly` / `typeset -r` — Param has PM_READONLY.
    pub fn is_readonly_param(&self, name: &str) -> bool {
        (self.param_flags(name) as u32 & PM_READONLY) != 0
    }

    /// Most-recent-command exit status. Reads canonical
    /// `builtin::LASTVAL` AtomicI32 (`Src/builtin.c:6443`).
    pub fn last_status(&self) -> i32 {
        crate::ported::builtin::LASTVAL.load(Ordering::Relaxed)
    }

    /// Write the most-recent-command exit status. The canonical
    /// store is `builtin::LASTVAL`; this is the single setter.
    /// Used everywhere `$?` / `%?` / errexit / ZERR trap read.
    pub fn set_last_status(&mut self, status: i32) {
        crate::ported::builtin::LASTVAL.store(status, Ordering::Relaxed);
    }

    /// Set an indexed array parameter via canonical paramtab
    /// (`setaparam`, `Src/params.c:3595`). The single store.
    pub fn set_array(&mut self, name: String, value: Vec<String>) {
        setaparam(&name, value); // c:params.c:3595
    }

    /// Set an associative array parameter via canonical
    /// `sethparam` (`Src/params.c:3602`). The single store.
    pub fn set_assoc(&mut self, name: String, value: IndexMap<String, String>) {
        let mut flat: Vec<String> = Vec::with_capacity(value.len() * 2);
        for (k, v) in &value {
            flat.push(k.clone());
            flat.push(v.clone());
        }
        sethparam(&name, flat); // c:params.c:3602
    }

    /// Read a scalar parameter. Mirrors C `getsparam` at
    /// `Src/params.c:3076` — reads through paramtab, falls back to
    /// special-var hooks and env.
    pub fn scalar(&self, name: &str) -> Option<String> {
        getsparam(name)
    }

    /// Read an array parameter via canonical `getaparam`
    /// (`Src/params.c:3101`). Routes through the C-faithful port
    /// that includes the PM_TYPE check + digit-first-name rejection
    /// — the inline paramtab.get(...).u_arr read was missing both.
    pub fn array(&self, name: &str) -> Option<Vec<String>> {
        getaparam(name)
    }

    /// Read an associative array parameter from canonical
    /// `paramtab_hashed_storage`. Mirrors C `gethparam` at
    /// `Src/params.c:3115` — returns the typed `IndexMap`.
    pub fn assoc(&self, name: &str) -> Option<IndexMap<String, String>> {
        paramtab_hashed_storage()
            .lock()
            .ok()
            .and_then(|m| m.get(name).cloned())
    }

    /// Test whether a scalar parameter exists in paramtab.
    /// Mirrors the C `paramtab->getnode(name) != NULL` check.
    pub fn has_scalar(&self, name: &str) -> bool {
        getsparam(name).is_some()
    }

    /// Test whether an array parameter exists in paramtab.
    pub fn has_array(&self, name: &str) -> bool {
        // Canonical: getaparam(name).is_some() — includes PM_TYPE check
        // + digit-first-name rejection. The inline u_arr.is_some()
        // shortcut returned true for PM_HASHED Params that had
        // u_arr=Some, which is structurally wrong.
        getaparam(name).is_some()
    }

    /// Test whether an associative array parameter exists. Reads
    /// canonical `paramtab_hashed_storage` (Src/params.c hashed
    /// PM_HASHED slot).
    pub fn has_assoc(&self, name: &str) -> bool {
        paramtab_hashed_storage()
            .lock()
            .ok()
            .map(|m| m.contains_key(name))
            .unwrap_or(false)
    }

    /// Unset an associative array parameter via canonical
    /// `unsetparam` (Src/params.c:3819) — PM_READONLY rejection,
    /// stdunsetfn dispatch, env clear. Also clears the zshrs-side
    /// `paramtab_hashed_storage` parallel IndexMap shadow.
    pub fn unset_assoc(&mut self, name: &str) {
        unsetparam(name);
        let _ = paramtab_hashed_storage()
            .lock()
            .ok()
            .as_deref_mut()
            .map(|m| m.remove(name));
    }

    /// Read a regular (non-global) alias value. Reads canonical
    /// `aliastab` (Src/hashtable.c:1186). Filters out aliases that
    /// have the ALIAS_GLOBAL flag set so the regular-alias slot is
    /// distinct from the global-alias slot, mirroring C's two
    /// separate dispatch paths via `aliasflags` checks.
    pub fn alias(&self, name: &str) -> Option<String> {
        let tab = crate::ported::hashtable::aliastab_lock().read().ok()?;
        let a = tab.get(name)?;
        if (a.node.flags & crate::ported::zsh_h::ALIAS_GLOBAL as i32) != 0 {
            None
        } else {
            Some(a.text.clone())
        }
    }

    /// Read a global alias value (`alias -g`). Reads canonical
    /// `aliastab` and filters to entries with the ALIAS_GLOBAL flag.
    pub fn global_alias(&self, name: &str) -> Option<String> {
        let tab = crate::ported::hashtable::aliastab_lock().read().ok()?;
        let a = tab.get(name)?;
        if (a.node.flags & crate::ported::zsh_h::ALIAS_GLOBAL as i32) != 0 {
            Some(a.text.clone())
        } else {
            None
        }
    }

    /// Read a suffix alias value (`alias -s`). Reads canonical
    /// `sufaliastab` (Src/hashtable.c:1187).
    pub fn suffix_alias(&self, name: &str) -> Option<String> {
        let tab = crate::ported::hashtable::sufaliastab_lock().read().ok()?;
        Some(tab.get(name)?.text.clone())
    }

    /// Set a regular alias. Writes canonical aliastab with
    /// ALIAS_GLOBAL bit cleared.
    pub fn set_alias(&mut self, name: String, value: String) {
        if let Ok(mut tab) = crate::ported::hashtable::aliastab_lock().write() {
            tab.add(crate::ported::hashtable::createaliasnode(&name, &value, 0));
        }
    }

    /// Set a global alias (`alias -g`). Writes canonical aliastab
    /// with ALIAS_GLOBAL bit set.
    pub fn set_global_alias(&mut self, name: String, value: String) {
        if let Ok(mut tab) = crate::ported::hashtable::aliastab_lock().write() {
            tab.add(crate::ported::hashtable::createaliasnode(
                &name,
                &value,
                crate::ported::zsh_h::ALIAS_GLOBAL as u32,
            ));
        }
    }

    /// Set a suffix alias (`alias -s ext=cmd`). Writes canonical
    /// sufaliastab.
    pub fn set_suffix_alias(&mut self, name: String, value: String) {
        if let Ok(mut tab) = crate::ported::hashtable::sufaliastab_lock().write() {
            tab.add(crate::ported::hashtable::createaliasnode(&name, &value, 0));
        }
    }


    /// Snapshot the alias map as a sorted `Vec<(name, value)>`,
    /// only entries WITHOUT the ALIAS_GLOBAL flag (regular aliases).
    pub fn alias_entries(&self) -> Vec<(String, String)> {
        if let Ok(tab) = crate::ported::hashtable::aliastab_lock().read() {
            tab.iter_sorted()
                .into_iter()
                .filter(|(_, a)| (a.node.flags & crate::ported::zsh_h::ALIAS_GLOBAL as i32) == 0)
                .map(|(k, a)| (k.clone(), a.text.clone()))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Snapshot the global-alias entries (ALIAS_GLOBAL flag set).
    pub fn global_alias_entries(&self) -> Vec<(String, String)> {
        if let Ok(tab) = crate::ported::hashtable::aliastab_lock().read() {
            tab.iter_sorted()
                .into_iter()
                .filter(|(_, a)| (a.node.flags & crate::ported::zsh_h::ALIAS_GLOBAL as i32) != 0)
                .map(|(k, a)| (k.clone(), a.text.clone()))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Snapshot the suffix-alias entries.
    pub fn suffix_alias_entries(&self) -> Vec<(String, String)> {
        if let Ok(tab) = crate::ported::hashtable::sufaliastab_lock().read() {
            tab.iter_sorted()
                .into_iter()
                .map(|(k, a)| (k.clone(), a.text.clone()))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Unset an array parameter. Direct port of `unsetparam_pm` for
    /// a PM_ARRAY Param. Mirrors are kept for now while the field
    /// transitions.
    /// Unset an array parameter via canonical `unsetparam`
    /// (Src/params.c:3819). Routes through the C-faithful port
    /// that runs PM_NAMEREF skip + PM_READONLY rejection via
    /// unsetparam_pm + stdunsetfn dispatch + pm.old scope restore.
    /// Inline `tab.remove(name)` skipped all four.
    pub fn unset_array(&mut self, name: &str) {
        unsetparam(name);
    }

    /// Unset a scalar parameter via canonical `unsetparam`. Same
    /// C-faithful path as `unset_array`; the C `unsetparam` itself
    /// is type-agnostic and dispatches through PM_TYPE inside.
    pub fn unset_scalar(&mut self, name: &str) {
        unsetparam(name);
    }

    /// Unset a parameter via canonical `unsetparam` (Src/params.c:
    /// 3819) — PM_NAMEREF skip + PM_READONLY rejection via
    /// unsetparam_pm + stdunsetfn dispatch + env clear for exported.
    /// Also clears the zshrs-side `paramtab_hashed_storage` parallel
    /// IndexMap shadow used for assoc-array value backing (no C
    /// counterpart — folds into Param.u_hash once that wires up).
    pub(crate) fn unset_var(&mut self, name: &str) {
        unsetparam(name);
        let _ = paramtab_hashed_storage()
            .lock()
            .ok()
            .as_deref_mut()
            .map(|m| m.remove(name));
    }

    /// Single-string substitution via the canonical pipeline. Snapshots
    /// the executor state into a `SubstState`, runs `singsub` from
    /// `Src/subst.c:514`, commits any side-effects (assigns inside
    /// `${var:=default}`, etc.) back to the executor.
    ///
    /// Replaces the bot-invented `expand_string` method that was deleted
    /// in the citation purge (180463e1e7). All call sites that previously
    /// did `exec.singsub(s)` now do `exec.singsub(s)` and route

    pub fn new() -> Self {
        tracing::debug!("ShellExecutor::new() initializing");

        // Validate the inherited $PWD against the real cwd before any
        // builtin reads it as a logical-path base. Direct port of zsh's
        // ispwd() at src/zsh/Src/utils.c:809-829: $PWD is honored only
        // when it (a) is absolute, (b) stat's to the same dev+inode as
        // ".", and (c) contains no `.`/`..` components. Otherwise zsh
        // resets it to getcwd() (init.c:1247-1253).
        //
        // Without this check, a child process that inherits $PWD from
        // a parent run in a different directory (cargo test setting
        // current_dir(/tmp) but leaking PWD=/project/root) sees the
        // stale PWD and `cd .` later snaps the real cwd to wherever
        // PWD points, escaping the parent's sandbox. ztst harnesses
        // hit this and polluted the project root with test artifacts.
        if let Ok(pwd_env) = env::var("PWD") {
            let valid = ispwd(&pwd_env);
            if !valid {
                if let Ok(real) = env::current_dir() {
                    env::set_var("PWD", &real);
                }
            }
        } else if let Ok(real) = env::current_dir() {
            env::set_var("PWD", &real);
        }

        // Initialize fpath from FPATH env var or use defaults
        let fpath = env::var("FPATH")
            .unwrap_or_default()
            .split(':')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();

        let history = HistoryEngine::new().ok();

        // Initialize standard zsh variables.
        //
        // `ZSH_VERSION` / `ZSH_PATCHLEVEL` come from the vendored zsh
        // source — build.rs parses `src/zsh/Config/version.mk` and
        // emits the constants below. Previously hardcoded `"5.9"` /
        // `"zsh-5.9-0-g73d3173"`; the latter was an invented git-hash
        // literal that didn't correspond to any real commit. The C
        // source sets these at `Src/params.c:972-973` via
        // `setsparam("ZSH_VERSION", ztrdup_metafy(ZSH_VERSION))`.
        let mut variables = HashMap::new();
        variables.insert(
            "ZSH_VERSION".to_string(),
            zsh_version::ZSH_VERSION.to_string(),
        ); // c:params.c:972
        variables.insert(
            "ZSH_PATCHLEVEL".to_string(),
            zsh_version::ZSH_PATCHLEVEL.to_string(),
        ); // c:params.c:973
        variables.insert("ZSH_NAME".to_string(), "zsh".to_string());
        // $ZSH_ARGZERO mirrors `posixzero` from Src/init.c:271
        // (`argv0 = argzero = posixzero = *argv++`). Src/params.c:971
        // does the actual `setsparam("ZSH_ARGZERO", ztrdup(posixzero))`
        // at the same setup phase Rust handles here. For -c / runscript
        // invocations the bin entrypoint overrides this with the
        // script path (Src/init.c:297).
        variables.insert(
            "ZSH_ARGZERO".to_string(),
            env::args().next().unwrap_or_else(|| "zsh".to_string()),
        );
        // ZLE word boundary chars — matches mainline zsh's default.
        variables.insert(
            "WORDCHARS".to_string(),
            "*?_-.[]~=/&;!#$%^(){}<>".to_string(),
        );
        variables.insert(
            "SHLVL".to_string(),
            env::var("SHLVL")
                .map(|v| {
                    v.parse::<i32>()
                        .map(|n| (n + 1).to_string())
                        .unwrap_or_else(|_| "1".to_string())
                })
                .unwrap_or_else(|_| "1".to_string()),
        );
        // POSIX/zsh default IFS is space, tab, newline, NUL. Splitters
        // throughout the codebase fall back to ` \t\n` when IFS is
        // missing; expose the actual default value so user code that
        // inspects $IFS sees what zsh exposes.
        variables.insert("IFS".to_string(), " \t\n\0".to_string());

        // POSIX `getopts` initial state: OPTIND starts at 1, OPTERR
        // at 1 (errors enabled). Without these, scripts that read
        // `$OPTIND` before the first `getopts` call see empty strings
        // (zsh: `1`).
        variables.insert("OPTIND".to_string(), "1".to_string());
        variables.insert("OPTERR".to_string(), "1".to_string());

        // zsh starts with `$_` empty (unlike bash which inherits the
        // OS-env value). The parent process sets `_=/path/to/binary`
        // before exec; zsh wipes that. Initialize to empty so script
        // reads of `$_` before any command runs return empty.
        variables.insert("_".to_string(), String::new());
        // `$histchars` — `Src/params.c:5064 histcharsgetfn` composes
        // bangchar+hatchar+hashchar (defaults `!`, `^`, `#` per
        // `Src/init.c:1100-1102`). Route through the C-port `histcharsgetfn`
        // so the value follows any runtime updates to the trio.
        // c:5064 — `pm->gsu.s->getfn(pm)` dispatches to histcharsgetfn.
        // Mirror via paramtab lookup; at this init point the special
        // entry may not exist yet, so fall back to default `!^#`.
        let histchars_val = paramtab()
            .read()
            .ok()
            .and_then(|t| {
                t.get("histchars")
                    .or_else(|| t.get("HISTCHARS"))
                    .map(|pm| histcharsgetfn(pm))
            })
            .unwrap_or_else(|| "!^#".to_string());
        variables.insert("histchars".to_string(), histchars_val); // c:params.c:5064

        // c:Src/params.c:858-860 standard non-special param defaults.
        // The full createparamtable() body installs special_paramdef
        // entries (LINENO/PPID/EUID/etc) as PM_READONLY which would
        // block subsequent BUILTIN_SET_LINENO writes; the readonly-
        // special bypass at setsparam isn't ported yet. Inline these
        // three setiparam-equivalent values in the meantime.
        variables.insert("MAILCHECK".to_string(), "60".to_string()); // c:858
        variables.insert("KEYTIMEOUT".to_string(), "40".to_string()); // c:859
        variables.insert("LISTMAX".to_string(), "100".to_string()); // c:860
                                                                    // `$WATCHFMT` — `Src/Modules/watch.c:137 DEFAULT_WATCHFMT`.
                                                                    // zsh's watch boot_ seeds WATCHFMT to the default when the
                                                                    // module loads. zshrs's modules are statically linked but
                                                                    // boot_ isn't wired into require_module yet, so seed the
                                                                    // default here. `print "$WATCHFMT"` prints the default
                                                                    // (diverges from `/bin/zsh -fc` which leaves it unset until
                                                                    // an explicit `zmodload zsh/watch`, but matches the
                                                                    // post-zmodload state that most plugin code expects).
        variables.insert(
            "WATCHFMT".to_string(),
            crate::ported::modules::watch::DEFAULT_WATCHFMT.to_string(),
        );

        // `$FUNCNEST` default. Real zsh defaults to 500 (compile-time
        // MAX_FUNCTION_DEPTH at configure.ac:400 / config.h:1004
        // `#define MAX_FUNCTION_DEPTH 500`, used at Src/params.c:113
        // to initialize `zsh_funcnest`). Match the canonical value so
        // `$FUNCNEST` reads identically to zsh; the cap is advisory
        // (call_function enforces against this) so users with stack
        // pressure can lower it. Bumped from 100 (zshrs-only override
        // for the Rust 8MB stack) — was a parity gap because every
        // plugin probe that reads `${FUNCNEST:-N}` saw 100 instead
        // of 500.
        variables.insert("FUNCNEST".to_string(), "500".to_string());

        // Run setlocale(LC_ALL, "") so nl_langinfo() (used by the
        // `langinfo` module) returns the host's actual locale instead
        // of the C/POSIX default ("US-ASCII"). Direct port of zsh's
        // Src/init.c:1208 setlocale call. unsafe { } around libc is
        // standard for this exact use-case — setlocale is process-
        // global and must run once at startup.
        unsafe {
            libc::setlocale(libc::LC_ALL, c"".as_ptr());
        }

        // c:hashtable.c:1206 createaliastables() — seeds aliastab with
        // the `run-help` / `which-command` defaults. Run once at shell
        // init so the canonical port owns the default-alias set; the
        // Executor's `aliases` HashMap then mirrors aliastab.
        crate::ported::hashtable::createaliastables();
        // Build the initial $path tied array as a local — fans out
        // to paramtab below; no ShellExecutor mirror anymore.
        let mut arrays: HashMap<String, Vec<String>> = HashMap::new();
        let path_dirs: Vec<String> = env::var("PATH")
            .unwrap_or_default()
            .split(':')
            .map(|s| s.to_string())
            .collect();
        arrays.insert("path".to_string(), path_dirs);
        // Seed canonical OPTS_LIVE with defaults if not already
        // populated. `default_options` builds the same name→bool map
        // we previously cloned into `exec.options`.
        if opt_state_len() == 0 {
            for (k, v) in Self::default_options() {
                opt_state_set(&k, v);
            }
        }
        let mut exec = Self {
            // c:Src/init.c:479 — `-c` mode: scriptname = scriptfilename
            // = ztrdup("zsh"). Both start at the literal "zsh".
            // dispatch_function_call overrides scriptname per c:5903;
            // scriptfilename stays at the outer file.
            scriptname: Some("zsh".to_string()),
            scriptfilename: Some("zsh".to_string()),
            subshell_snapshots: Vec::new(),
            inline_env_stack: Vec::new(),
            current_command_glob_failed: std::cell::Cell::new(false),
            jobs: JobTable::new(),
            fpath,
            zwc_cache: HashMap::new(),
            history,
            session_histnum: 0,
            completions: HashMap::new(),
            process_sub_counter: 0,
            traps: HashMap::new(),
            // zsh completion system
            comp_matches: Vec::new(),
            comp_groups: Vec::new(),
            comp_state: CompState::default(),
            zstyles: Vec::new(),
            comp_words: Vec::new(),
            comp_current: 0,
            comp_prefix: String::new(),
            comp_suffix: String::new(),
            comp_iprefix: String::new(),
            comp_isuffix: String::new(),
            local_scope_depth: 0,
            pending_underscore: None,
            in_dq_context: 0,
            in_scalar_assign: 0,
            session_history_ids: Vec::new(),
            open_fds: HashMap::new(),
            next_fd: 10,
            profiling_enabled: false,
            compsys_cache: {
                let cache_path = compsys::cache::default_cache_path();
                if cache_path.exists() {
                    let db_size = fs::metadata(&cache_path).map(|m| m.len()).unwrap_or(0);
                    match CompsysCache::open(&cache_path) {
                        Ok(c) => {
                            tracing::info!(
                                db_bytes = db_size,
                                path = %cache_path.display(),
                                "compsys: sqlite cache opened"
                            );
                            Some(c)
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "compsys: failed to open cache");
                            None
                        }
                    }
                } else {
                    tracing::debug!("compsys: no cache at {}", cache_path.display());
                    None
                }
            },
            compinit_pending: None, // (receiver, start_time)
            plugin_cache: {
                let pc_path = crate::plugin_cache::default_cache_path();
                if let Some(parent) = pc_path.parent() {
                    let _ = fs::create_dir_all(parent);
                }
                match crate::plugin_cache::PluginCache::open(&pc_path) {
                    Ok(pc) => {
                        let (plugins, functions) = pc.stats();
                        tracing::info!(
                            plugins,
                            cached_functions = functions,
                            path = %pc_path.display(),
                            "plugin_cache: sqlite opened"
                        );
                        Some(pc)
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "plugin_cache: failed to open");
                        None
                    }
                }
            },
            deferred_compdefs: Vec::new(),
            returning: None,
            breaking: 0,
            continuing: 0,
            zsh_compat: false,
            bash_compat: false,
            posix_mode: false,
            worker_pool: {
                let config = crate::config::load();
                let pool_size = crate::config::resolve_pool_size(&config.worker_pool);
                std::sync::Arc::new(crate::worker::WorkerPool::new(pool_size))
            },
            intercepts: Vec::new(),
            async_jobs: HashMap::new(),
            next_async_id: 1,
            defer_stack: Vec::new(),
            redirect_scope_stack: Vec::new(),
            redirect_failed: false,
            pending_stdin: None,
            functions_compiled: HashMap::new(),
            function_source: HashMap::new(),
            function_line_base: HashMap::new(),
            function_def_file: HashMap::new(),
            prompt_funcstack: Vec::new(),
            tied_scalar_to_array: HashMap::new(),
            tied_array_to_scalar: HashMap::new(),
            buffer_stack: Vec::new(),
        };
        // Mirror env-derived path arrays into the `arrays` table so
        // user-level `fpath` / `path` array reads see the inherited
        // entries. zsh: `fpath+=…` should append to the inherited
        // 43-entry array, not replace it. Same for `path` (PATH).
        let fpath_arr: Vec<String> = exec
            .fpath
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        if !fpath_arr.is_empty() {
            exec.set_array("fpath".to_string(), fpath_arr);
        }
        if let Ok(path) = env::var("PATH") {
            let path_arr: Vec<String> = path
                .split(':')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
            if !path_arr.is_empty() {
                exec.set_array("path".to_string(), path_arr);
            }
        }
        // Register the standard tied path-family pairs so `path+=` /
        // `fpath+=` / etc. mirror through the array→scalar sync hook
        // in BUILTIN_APPEND_ARRAY (and the SET_ARRAY tied path).
        // Direct port of the implicit ties that zsh wires up at
        // startup for PATH/path, FPATH/fpath, etc. Source-of-truth
        // for the pairs is Src/init.c's `setupvals()` PM_TIED entries.
        for (scalar, arr) in [
            ("PATH", "path"),
            ("FPATH", "fpath"),
            ("MANPATH", "manpath"),
            ("CDPATH", "cdpath"),
            ("MODULE_PATH", "module_path"),
        ] {
            exec.tied_array_to_scalar
                .insert(arr.to_string(), (scalar.to_string(), ":".to_string()));
            exec.tied_scalar_to_array
                .insert(scalar.to_string(), (arr.to_string(), ":".to_string()));
        }

        // Mirror every constructor-time `variables` / `arrays` /
        // `assoc_arrays` seed into paramtab so the C-port readers see
        // the same initial state. C does this implicitly because its
        // single `paramtab` is populated by `setupvals()` /
        // `createparam()` calls at init (Src/init.c:1014-1300). The
        // Rust port builds local HashMaps first and then constructs
        // self; this loop fans the contents out to paramtab in one
        // pass at the end of new().
        for (k, v) in &variables {
            setsparam(k, v); // c:params.c:3350
        }
        for (k, v) in &arrays {
            setaparam(k, v.clone()); // c:params.c:3595
        }
        // Populate paramtab with PM_SPECIAL placeholder Params for
        // every PARTAB / PARTAB_ARRAY magic-assoc name. Mirrors
        // what C's zsh/parameter module boot_ → handlefeatures
        // chain does at startup. Makes `${+aliases}` / `${(t)commands}`
        // / `typeset -p modules` etc. see the special entries.
        init_partab_params(); // c:Src/Modules/parameter.c:2341 boot_/enables_ chain

        // c:Src/params.c:873-876 — `gethostname(hostnam,256);
        //                            setsparam("HOST", ztrdup_metafy(hostnam));`
        // Plain port of the createparamtable HOST init. Direct
        // libc::gethostname call; result written via canonical
        // setsparam. createparamtable() itself isn't called from the
        // bin entry yet (full init port pending); this is the minimum
        // for `$HOST` to read non-empty.
        let mut host_buf = [0u8; 256];
        let host_rc = unsafe {
            libc::gethostname(host_buf.as_mut_ptr() as *mut libc::c_char, 256)
        }; // c:874
        if host_rc == 0 {
            if let Ok(c) = std::ffi::CStr::from_bytes_until_nul(&host_buf) {
                if let Ok(name) = c.to_str() {
                    crate::ported::params::setsparam("HOST", name); // c:875
                }
            }
        }
        // c:Src/init.c:479 — `-c` mode: scriptname = scriptfilename
        // = ztrdup("zsh"). Both globals start as the literal "zsh"
        // (not the binary path) so PS4's %x / %N print "zsh" not
        // "/path/to/zshrs" at the top level. Function dispatch
        // overrides scriptname per c:5903; scriptfilename stays.
        crate::ported::utils::set_scriptname(Some("zsh".to_string()));
        crate::ported::utils::set_scriptfilename(Some("zsh".to_string()));

        // c:Src/params.c:878-882 — `setsparam("LOGNAME", getlogin() ?:
        // cached_username);`. C's createparamtable also assigns
        // USERNAME from the same source (cached_username) via the
        // special_paramdefs table. Here mirror the LOGNAME +
        // USERNAME seeding so the canonical paramtab entries exist
        // (usernamegetfn at c:4655 reads through Param.u_str).
        // Same one-shot init pattern as the HOST gethostname call
        // above — full createparamtable() port is pending.
        let logname = unsafe {
            let p = libc::getlogin();
            if p.is_null() {
                None
            } else {
                Some(std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned())
            }
        }; // c:880
        if let Some(name) = logname {
            crate::ported::params::setsparam("LOGNAME", &name); // c:881
            crate::ported::params::setsparam("USERNAME", &name); // c:special_paramdefs
        }
        exec
    }

    // enter_posix_mode / enter_ksh_mode moved to src/ported/options.rs
    // (canonical C source: Src/options.c:533 emulate()).

    // host_apply_redirect / host_redirect_scope_begin / host_redirect_scope_end /
    // host_set_pending_stdin / host_exec_external moved to src/fusevm_bridge.rs
    // (extension; not a port of Src/exec.c).

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
            fs::read_to_string(file_path).map_err(|e| format!("{}: {}", file_path, e))?;
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

    /// Whether `name` is a known function. Checks the compiled-functions
    /// table and the autoload-pending registry — `autoload foo` should
    /// make `whence foo`/`type foo`/`functions foo` recognize `foo` as
    /// a function before it's actually loaded. Doesn't trigger autoload
    /// itself; use `maybe_autoload` first if you need to load before
    /// introspecting.
    pub fn function_exists(&self, name: &str) -> bool {
        // Either compiled (already loaded) or shfunctab has an
        // autoload stub with PM_UNDEFINED set (pending). Matches C's
        // `lookupshfunc(name)` semantics at `Src/exec.c:5215`.
        if self.functions_compiled.contains_key(name) {
            return true;
        }
        crate::ported::hashtable::shfunctab_lock()
            .read()
            .ok()
            .map(|t| t.get(name).is_some())
            .unwrap_or(false)
    }

    /// Canonical source text for a function. Returns from `function_source`
    /// (populated by autoload paths and runtime FuncDef registration via
    /// BUILTIN_REGISTER_COMPILED_FN with body_source). Returns `None` if
    /// no canonical source is on file.
    pub fn function_definition_text(&self, name: &str) -> Option<String> {
        self.function_source.get(name).cloned()
    }


    /// Sorted list of every known function name (union of compiled + source).
    pub fn function_names(&self) -> Vec<String> {
        let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for k in self.functions_compiled.keys() {
            set.insert(k.clone());
        }
        for k in self.function_source.keys() {
            set.insert(k.clone());
        }
        set.into_iter().collect()
    }

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
                if (stub.node.flags as u32 & PM_UNDEFINED) != 0 {
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
        let saved_zero = getsparam("0");
        self.set_scalar("0".to_string(), display_name);
        self.local_scope_depth += 1;
        // c:Src/exec.c doshfunc startparamscope(): bump canonical
        // `locallevel` so any `local`/`typeset` inside the body
        // installs Params at the correct scope. endparamscope at
        // exit decrements + restores Param.old chain.
        locallevel.fetch_add(1, Ordering::Relaxed);
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
        endparamscope();
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

    pub(crate) fn collect_until_paren(chars: &mut std::iter::Peekable<std::str::Chars>) -> String {
        let mut result = String::new();
        let mut depth = 1;

        for c in chars.by_ref() {
            if c == '(' {
                depth += 1;
                result.push(c);
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
                result.push(c);
            } else {
                result.push(c);
            }
        }

        result
    }

    pub(crate) fn collect_until_double_paren(
        chars: &mut std::iter::Peekable<std::str::Chars>,
    ) -> String {
        let mut result = String::new();
        let mut arith_depth = 1; // Tracks $(( ... )) nesting
        let mut paren_depth = 0; // Tracks ( ... ) nesting within expression

        while let Some(c) = chars.next() {
            if c == '(' {
                if paren_depth == 0 && chars.peek() == Some(&'(') {
                    // Nested $(( - but we need to see if it's really another arithmetic
                    // For simplicity, track inner parens
                    paren_depth += 1;
                    result.push(c);
                } else {
                    paren_depth += 1;
                    result.push(c);
                }
            } else if c == ')' {
                if paren_depth > 0 {
                    // Inside nested parens, just close one level
                    paren_depth -= 1;
                    result.push(c);
                } else if chars.peek() == Some(&')') {
                    // At top level and seeing )) - this closes our arithmetic
                    chars.next();
                    arith_depth -= 1;
                    if arith_depth == 0 {
                        break;
                    }
                    result.push_str("))");
                } else {
                    // Single ) at top level - shouldn't happen in valid expression
                    result.push(c);
                }
            } else {
                result.push(c);
            }
        }

        result
    }

    /// Parse `cmd_str` via parse_init+parse and pull out the first Simple
    /// command's words, untokenized + variable-expanded, ready to spawn
    /// as argv. Used by process-substitution where we need raw argv to
    /// hand to `Command::new`. Returns empty vec if the cmd isn't a
    /// simple shape — pipelines / compound forms aren't process-sub
    /// friendly anyway.
    fn simple_cmd_words(&mut self, cmd_str: &str) -> Vec<String> {
        // Mirror Src/init.c-style errflag save/clear/check around the
        // parse. Process-sub argv extraction silently bails on syntax
        // errors (matches zsh's behavior when the inner command can't
        // be parsed).
        let saved_errflag = errflag.load(Ordering::Relaxed);
        errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
        crate::ported::parse::parse_init(cmd_str);
        let prog = crate::ported::parse::parse();
        let parse_failed = (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0;
        errflag.store(saved_errflag, Ordering::Relaxed);
        if parse_failed {
            return Vec::new();
        }
        let first = match prog.lists.first() {
            Some(l) => l,
            None => return Vec::new(),
        };
        let pipe = &first.sublist.pipe;
        if let crate::parse::ZshCommand::Simple(simple) = &pipe.cmd {
            simple
                .words
                .iter()
                .map(|w| {
                    // Untokenize then variable-expand — text-based
                    // word expansion for the spawned argv.
                    let untoked = crate::lex::untokenize(w);
                    singsub(&untoked)
                })
                .collect()
        } else {
            Vec::new()
        }
    }

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
                if let Ok(fifo) = OpenOptions::new().write(true).open(&fifo_clone) {
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
                if let Ok(fifo) = File::open(&fifo_clone) {
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
                singsub(filename)
            } else {
                filename.to_string()
            };
            let resolved = resolved.to_string();
            match fs::read_to_string(&resolved) {
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
        cmdpush(crate::ported::zsh_h::CS_CMDSUBST as u8); // c:zsh.h:2799
                                                                                 // Save LINENO so the inner cmdsubst's line counter doesn't
                                                                                 // leak into the outer trace — direct port of Src/exec.c:1407
                                                                                 // `oldlineno = lineno;` followed by `lineno = oldlineno;`
                                                                                 // restore at line 1640. Inner program parses fresh as line 1
                                                                                 // and increments from there; once it returns, the outer
                                                                                 // line at the `$(…)` site must read the original outer
                                                                                 // lineno (so xtrace renders `+:5:> echo …` not `+:1:> …`).
        let saved_lineno = getsparam("LINENO");
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
        cmdpop();
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
        let read_file = unsafe { File::from_raw_fd(read_fd) };
        let mut output = String::new();
        let _ = io::BufReader::new(read_file).read_to_string(&mut output);

        // POSIX: trailing newlines stripped from cmd-sub result.
        while output.ends_with('\n') {
            output.pop();
        }
        output
    }

    // ksh_autoload_body moved to src/ported/builtin.rs
}

impl Default for ShellExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_echo() {
        let _g = crate::test_util::global_state_lock();
        let mut exec = ShellExecutor::new();
        let status = exec.execute_script("true").unwrap();
        assert_eq!(status, 0);
    }

    #[test]
    fn test_if_true() {
        let _g = crate::test_util::global_state_lock();
        let mut exec = ShellExecutor::new();
        let status = exec.execute_script("if true; then true; fi").unwrap();
        assert_eq!(status, 0);
    }

    #[test]
    fn test_if_false() {
        let _g = crate::test_util::global_state_lock();
        let mut exec = ShellExecutor::new();
        let status = exec
            .execute_script("if false; then true; else false; fi")
            .unwrap();
        assert_eq!(status, 1);
    }

    #[test]
    fn test_for_loop() {
        let _g = crate::test_util::global_state_lock();
        let mut exec = ShellExecutor::new();
        exec.execute_script("for i in a b c; do true; done")
            .unwrap();
        assert_eq!(exec.last_status(), 0);
    }

    #[test]
    fn test_and_list() {
        let _g = crate::test_util::global_state_lock();
        let mut exec = ShellExecutor::new();
        let status = exec.execute_script("true && true").unwrap();
        assert_eq!(status, 0);

        let status = exec.execute_script("true && false").unwrap();
        assert_eq!(status, 1);
    }

    #[test]
    fn test_or_list() {
        let _g = crate::test_util::global_state_lock();
        let mut exec = ShellExecutor::new();
        let status = exec.execute_script("false || true").unwrap();
        assert_eq!(status, 0);
    }

    /// Pin: `forklevel` matches the C global declared at
    /// `Src/exec.c:1052` (`int forklevel;`). Like `int` in C, the
    /// Rust port is an AtomicI32 starting at 0 (no fork has occurred
    /// at process start). Per `Src/exec.c:1221` (`forklevel =
    /// locallevel;`), every subshell entry copies `locallevel` into
    /// the global; the SIGPIPE handler at `Src/signals.c:808` reads
    /// it back to distinguish the top-level shell from a subshell.
    #[test]
    fn test_forklevel_default_zero_and_roundtrip() {
        let _g = crate::test_util::global_state_lock();
        use std::sync::atomic::Ordering;
        let prev = crate::ported::exec::FORKLEVEL.load(Ordering::Relaxed);
        // Default state at process start: zero (matches C's BSS init
        // of `int forklevel;` to 0).
        crate::ported::exec::FORKLEVEL.store(0, Ordering::Relaxed);
        assert_eq!(crate::ported::exec::FORKLEVEL.load(Ordering::Relaxed), 0);
        // Simulate the c:1221 store: `forklevel = locallevel;`.
        crate::ported::exec::FORKLEVEL.store(3, Ordering::Relaxed);
        assert_eq!(crate::ported::exec::FORKLEVEL.load(Ordering::Relaxed), 3);
        crate::ported::exec::FORKLEVEL.store(prev, Ordering::Relaxed);
    }
}

// Plugin-Framework-Agnostic State-Modification Recorder hook helpers.
/// Recorder helper: emit one record for an array/scalar mutation
/// targeting a path-family parameter (path/fpath/manpath/module_path/
/// cdpath, lower- or upper-cased), or one `assign` record for any
/// other name. Centralises the path-family list so `BUILTIN_SET_ARRAY`,
/// `BUILTIN_APPEND_ARRAY`, and `BUILTIN_APPEND_SCALAR_OR_PUSH` share
/// the same routing.
///
/// `is_append` distinguishes `arr=(...)` from `arr+=(...)` so the
/// emitted event carries the APPEND attr bit and replay can choose
/// between fresh-set and extend semantics.
///
/// `attrs` carries any pre-existing type info from
/// `recorder_attrs_for(name)` (readonly/export/global) — array shape
/// and APPEND get OR'd in by emit_array_assign.
#[cfg(feature = "recorder")]
pub(crate) fn emit_path_or_assign(
    name: &str,
    values: &[String],
    attrs: crate::recorder::ParamAttrs,
    is_append: bool,
    ctx: &crate::recorder::RecordCtx,
) {
    let lower = name.to_ascii_lowercase();
    let kind_name: Option<&'static str> = match lower.as_str() {
        "path" => Some("path"),
        "fpath" => Some("fpath"),
        "manpath" => Some("manpath"),
        "module_path" => Some("module_path"),
        "cdpath" => Some("cdpath"),
        _ => None,
    };
    match kind_name {
        Some(k) => {
            for v in values {
                crate::recorder::emit_path_mod(v, k, ctx.clone());
                // Each fpath addition also surfaces every `_completion`
                // file inside the directory — matches zinit-report's
                // per-plugin "Completions:" listing. Only fpath dirs
                // get this treatment; PATH dirs hold executables, not
                // completion functions.
                if k == "fpath" {
                    crate::recorder::discover_completions_in_fpath_dir(v, ctx);
                }
            }
        }
        None => {
            // Non-path arrays: emit ONE `assign` event with the
            // ordered element list preserved in value_array. Replay
            // reconstructs `name=(elem1 elem2 ...)` exactly without
            // having to re-split a joined string.
            crate::recorder::emit_array_assign(
                name,
                values.to_vec(),
                attrs,
                is_append,
                ctx.clone(),
            );
        }
    }
}

// Whole impl block is `#[cfg(feature = "recorder")]` so the default
// build sees no recorder symbols on `ShellExecutor`.
#[cfg(feature = "recorder")]
impl ShellExecutor {}

#[cfg(feature = "recorder")]
impl ShellExecutor {}

impl ShellExecutor {
    // `add_hook` / `delete_hook` now live in src/extensions/hooks.rs.
    // That file was orphaned (never declared as a module) and the no-op
    // stubs that previously sat here silently swallowed every
    // `add-zsh-hook` registration. Wiring hooks.rs back into lib.rs
    // restored the real paramtab-backed implementation; the empty
    // stubs were removed so dispatch resolves unambiguously.

    // ═══════════════════════════════════════════════════════════════════
    // AOP INTERCEPT — the killer builtin
    // ═══════════════════════════════════════════════════════════════════

    // ═══════════════════════════════════════════════════════════════════
    // CONCURRENT PRIMITIVES — ship work to the worker pool from shell
    // No stryke dependency. Pure zshrs. Thin binary gets full parallelism.
    // ═══════════════════════════════════════════════════════════════════
}

impl ShellExecutor {
    // ═══════════════════════════════════════════════════════════════════════════
    // Additional zsh builtins
    // ═══════════════════════════════════════════════════════════════════════════

    /// Helper to check if name is a builtin. Consults the canonical
    /// `BUILTINS` table (`src/ported/builtin.rs:122`, the 1:1 port of
    /// `static struct builtin builtins[]` at `Src/builtin.c:40-137`).
    /// Earlier implementation hardcoded a separate `BUILTIN_SET`
    /// HashSet of 130+ names — duplicated state that drifts when new
    /// builtins land in the canonical table. The cached lookup set
    /// below is built once from `BUILTINS` so the O(1) cost stays
    /// without a separate authoritative list.
    pub(crate) fn is_builtin(&self, name: &str) -> bool {
        BUILTIN_NAMES.contains(name) || name.starts_with('_')
    }

    /// Helper to find command in PATH. The fast path consults the
    /// `command_hash` table (rebuilt by `rehash` per `Src/Modules/
    /// hashed.c`); the slow path delegates to the canonical port of
    /// `findcmd()` (`Src/exec.c:5260`, ported at
    /// `src/ported/builtin.rs:4047`). Earlier inline PATH walk
    /// duplicated findcmd's logic without honoring `name.contains('/')`
    /// (the C source returns the literal path for slashed names
    /// without walking $PATH).
    pub(crate) fn find_in_path(&self, name: &str) -> Option<String> {
        // Canonical command-hash lives in `cmdnamtab`. `get_full_path`
        // returns the resolved path for HASHED entries.
        if let Some(p) = crate::ported::hashtable::cmdnamtab_lock()
            .read()
            .ok()
            .and_then(|t| t.get_full_path(name))
        {
            return Some(p.display().to_string());
        }
        crate::ported::builtin::findcmd(name, 0, 0) // c:exec.c:5260
    }



    // ═══════════════════════════════════════════════════════════════════════
    // Coreutils builtins (anti-fork) — only active when !posix_mode
    // ═══════════════════════════════════════════════════════════════════════

    // nproc-equivalent already exists via builtin_nproc.
}

use std::os::unix::fs::MetadataExt;

bitflags::bitflags! {
    /// Flags for zfork()
    #[derive(Debug, Clone, Copy, Default)]
    pub struct ForkFlags: u32 {
        const NOJOB = 1 << 0;    // Don't add to job table
        const NEWGRP = 1 << 1;   // Create new process group
        const FGTTY = 1 << 2;    // Take foreground terminal
        const KEEPSIGS = 1 << 3; // Keep signal handlers
    }
}

bitflags::bitflags! {
    /// Flags for entersubsh()
    #[derive(Debug, Clone, Copy, Default)]
    pub struct SubshellFlags: u32 {
        const NOMONITOR = 1 << 0; // Disable job control
        const KEEPFDS = 1 << 1;   // Keep file descriptors
        const KEEPTRAPS = 1 << 2; // Keep trap handlers
    }
}

/// Result of fork operation
#[derive(Debug)]
/// `fork()` outcome (parent / child / error).
/// Mirrors the integer return of `zfork()` from Src/exec.c:349.
pub enum ForkResult {
    Parent(i32), // Contains child PID
    Child,
}

/// Redirection mode
#[derive(Debug, Clone, Copy)]
/// File-redirection mode (`>` / `>>` / `<` / etc.).
/// Mirrors the `REDIR_*` enum from Src/zsh.h.
pub enum RedirMode {
    Dup,
    Close,
}

/// Builtin command type
#[derive(Debug, Clone, Copy)]
/// Builtin classification.
/// Mirrors the `BINF_*` flag set Src/builtin.c uses to
/// classify special vs regular builtins.
pub enum BuiltinType {
    Normal,
    Disabled,
}

// =====================================================================
// Builtin dispatch stubs.
//
// These methods used to live in `src/ported/builtin.rs` inside
// `impl ShellExecutor` blocks. Per user feedback ("each of those
// bin_* is fake anyways"), the impl blocks were deleted from the
// port tree. The methods are recreated here as stubs so existing
// callers (fusevm_bridge, ext_builtins, vm_helper's own dispatch loop)
// keep compiling. Each stub delegates to the canonical free-fn port
// at `crate::ported::builtin::bin_X` when one exists, or returns 0.
//
// The recorder hooks the original methods carried are preserved as
// commented snippets at the bottom of `src/ported/builtin.rs` —
// they will be re-wired here once the canonical bin_* ports are
// true to C.
// =====================================================================
use crate::fusevm_bridge::with_executor;
use crate::ported::glob::*;
use crate::ported::hist::*;
use crate::ported::jobs::*;
use crate::ported::math::*;
use crate::ported::module::*;
use crate::ported::modules::cap::*;
// bin_ztcp / bin_echotc imports deleted — zero callers since
// `ztcp` / `echotc` route through dispatch_builtin in the bridge.
use crate::ported::modules::terminfo::*;
use crate::ported::options::*;
use crate::ported::params::*;
use crate::ported::pattern::*;
use crate::ported::prompt::*;
use crate::ported::signals::*;
use crate::ported::subst::*;
use crate::ported::utils::{zerr, zerrnam, zwarn, zwarnnam};
use ::regex::{Error as RegexError, Regex, RegexBuilder};


// =====================================================================
// MOVED FROM: src/ported/options.rs
// =====================================================================

impl ShellExecutor {
    /// Returns every option name in `ZSH_OPTIONS_SET` (canonical port
    /// of `optns[]` at `Src/options.c:79+`). Replaces a 200-line
    /// hardcoded `&[...]` duplicate that drifted from upstream.
    pub(crate) fn all_zsh_options() -> Vec<&'static str> {
        ZSH_OPTIONS_SET
            .iter()
            .copied()
            .collect()
    }
    /// Build the `name → bool` default-option map. Routes through
    /// canonical `options::default_on_options` (data-driven from
    /// `ZSH_OPTIONS_SET` + `optns_flags` — port of `defset()` macro
    /// at `Src/options.c:73`). Replaces a 60-line hardcoded
    /// `defaults_on` array that drifted from upstream every time a
    /// new option landed in optns[].
    pub(crate) fn default_options() -> HashMap<String, bool> {
        let on = default_on_options();
        Self::all_zsh_options()
            .into_iter()
            .map(|n| (n.to_string(), on.contains(n)))
            .collect()
    }

}

// =====================================================================
// MOVED FROM: src/ported/params.rs
// =====================================================================

impl ShellExecutor {
    /// PURE PASSTHRU to the canonical `params::getsparam` (C port of
    /// `Src/params.c::getsparam`). Every special-name case the old
    /// 316-line body handled lives in `params::lookup_special_var` +
    /// `getsparam`'s paramtab/env walk. Returns an empty string for
    /// unset names (matching the old fn's signature; callers that
    /// need the set/unset distinction call `scalar` / `has_scalar`
    /// directly).
    pub(crate) fn get_variable(&self, name: &str) -> String {
        getsparam(name).unwrap_or_default()
    }
}

// =====================================================================
// MOVED FROM: src/ported/signals.rs
// =====================================================================

impl ShellExecutor {
    /// Execute trap handlers for a signal
    pub fn run_trap(&mut self, signal: &str) {
        if let Some(action) = self.traps.get(signal).cloned() {
            // Empty action = signal-ignore. Don't try to execute "".
            if !action.is_empty() {
                let _ = self.execute_script(&action);
            }
        }
    }
}

// =====================================================================
// MOVED FROM: src/ported/prompt.rs
// =====================================================================

impl ShellExecutor {
    /// Expand prompt escape sequences using the full prompt module.
    /// `expand_prompt` itself now reads C globals (paramtab / LASTVAL /
    /// curhist / JOBTAB / scriptname) so no per-executor sync is
    /// needed — the executor's state already mirrors those globals.
    pub(crate) fn expand_prompt_string(&self, s: &str) -> String {
        expand_prompt(s)
    }
    pub(crate) fn apply_prompt_theme(&mut self, theme: &str, preview: bool) {
        let (ps1, rps1) = match theme {
            "minimal" => ("%# ", ""),
            "off" => ("$ ", ""),
            "adam1" => (
                "%B%F{cyan}%n@%m %F{blue}%~%f%b %# ",
                "%F{yellow}%D{%H:%M}%f",
            ),
            "redhat" => ("[%n@%m %~]$ ", ""),
            _ => ("%n@%m %~ %# ", ""),
        };
        if preview {
            println!("PS1={:?}", ps1);
            println!("RPS1={:?}", rps1);
        } else {
            self.set_scalar("PS1".to_string(), ps1.to_string());
            self.set_scalar("RPS1".to_string(), rps1.to_string());
            self.set_scalar("prompt_theme".to_string(), theme.to_string());
        }
    }
}

// =====================================================================
// MOVED FROM: src/ported/glob.rs
// =====================================================================

impl ShellExecutor {
    /// Expand glob pattern to matching files
    // expand_glob — thin wrapper around the canonical `glob::glob_path`
    // (C port of `Src/glob.c::zglob`). glob_path handles every glob
    // feature: brace alternation, extendedglob `^pat` / `~` exclusion,
    // `(#i)`/`(#l)`/`(#aN)` inline flags, numeric ranges, char classes,
    // ksh `!()` negation, recursive `**`, qualifiers `(.)` `(/)` `(@)`
    // `(o…)` `(O…)` `(M)` `(T)`, NULLGLOB / GLOBDOTS / CASEGLOB option
    // gating — every one of those was duplicated in the 411-line
    // hand-roll that previously lived here.
    //
    // Executor-side state that the canonical port can't touch:
    //   - `current_command_glob_failed` cell: lets the dispatch layer
    //     (fusevm_bridge::pop_args / host_exec_external) skip the
    //     current command without exiting the shell when NOMATCH +
    //     looks_like_glob fires.
    //   - The diagnostic emission + per-list errflag-clear that
    //     C zsh's execlist interleaves between failed lists (zshrs's
    //     fusevm doesn't have an equivalent loop, so drop the bit
    //     here after zerr).
    pub fn expand_glob(&self, pattern: &str) -> Vec<String> {
        let expanded = glob_path(pattern);
        if !expanded.is_empty() {
            return expanded;
        }
        // No matches. Mirror zsh's `setopt nullglob` / `nomatch`
        // dispatch (Src/glob.c:1873-1886) here because glob_path
        // returns an empty Vec without knowing executor state.
        let nullglob = opt_state_get("nullglob").unwrap_or(false);
        if nullglob {
            return Vec::new();
        }
        let nomatch = opt_state_get("nomatch").unwrap_or(true);
        if nomatch && Self::looks_like_glob(pattern) {
            zerr(&format!("no matches found: {}", pattern));
            errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
            self.current_command_glob_failed.set(true);
            return Vec::new();
        }
        // Pattern has no glob meta — pass through literally.
        vec![pattern.to_string()]
    }
    /// True iff the literal `pattern` actually contains a glob metachar
    /// in a position that would have triggered globbing. Used to avoid
    /// spurious "no matches" errors when expand_glob is called on a
    /// plain path that happened to route through this code (e.g. some
    /// fast paths bridge unconditionally).
    pub(crate) fn looks_like_glob(pattern: &str) -> bool {
        // A trailing `(qualifier)` is itself a glob trigger — e.g.
        // `path(L+10)` should be treated as a glob even when the
        // body has no `*`/`?`/`[...]`.
        let has_qual_suffix = if let Some(open) = pattern.rfind('(') {
            pattern.ends_with(')') && open + 1 < pattern.len() - 1
        } else {
            false
        };
        // Strip trailing `(...)` qualifier so we test the pattern body.
        let body = if let Some(open) = pattern.rfind('(') {
            if pattern.ends_with(')') {
                &pattern[..open]
            } else {
                pattern
            }
        } else {
            pattern
        };
        // Walk character-by-character so escaped metachars (`\*`, `\?`,
        // `\[`) are NOT counted as glob triggers. zsh: `echo \*` prints
        // a literal `*`; without the unescaped check, looks_like_glob
        // returned true on the bare `*` and the runtime glob expansion
        // aborted with NOMATCH.
        let chars: Vec<char> = body.chars().collect();
        let mut i = 0;
        let mut has_unescaped_star = false;
        let mut has_unescaped_question = false;
        let mut has_unescaped_bracket_open: Option<usize> = None;
        while i < chars.len() {
            let c = chars[i];
            if c == '\\' && i + 1 < chars.len() {
                // Escaped char — skip both.
                i += 2;
                continue;
            }
            match c {
                '*' => has_unescaped_star = true,
                '?' => has_unescaped_question = true,
                '[' if has_unescaped_bracket_open.is_none() => {
                    has_unescaped_bracket_open = Some(i);
                }
                _ => {}
            }
            i += 1;
        }
        // `[` only counts when there's a matching `]` after it.
        let has_bracket_class = has_unescaped_bracket_open
            .map(|i| body[i + 1..].contains(']'))
            .unwrap_or(false);
        // `<N-M>` numeric range glob is also a trigger — match shape
        // `<` + optional digits + `-` + optional digits + `>` outside
        // any bracket expression.
        let has_numeric_range =
            body.contains('<') && body.contains('>') && !extract_numeric_ranges(body).is_empty();
        has_unescaped_star
            || has_unescaped_question
            || has_bracket_class
            || has_qual_suffix
            || has_numeric_range
    }
}


// =====================================================================
// MOVED FROM: src/ported/utils.rs
// =====================================================================

impl ShellExecutor {
    pub(crate) fn copy_dir_recursive(
        src: &Path,
        dest: &Path,
    ) -> io::Result<()> {
        if !dest.exists() {
            fs::create_dir_all(dest)?;
        }
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let src_path = entry.path();
            let dest_path = dest.join(entry.file_name());

            if file_type.is_dir() {
                Self::copy_dir_recursive(&src_path, &dest_path)?;
            } else {
                fs::copy(&src_path, &dest_path)?;
            }
        }
        Ok(())
    }
}


// =====================================================================
// Magic-assoc key dispatch — fusevm-bridge aggregator that fans a
// magic-assoc table NAME out into the right scanpm* port from
// src/ported/modules/parameter.rs.
// =====================================================================
//
// The C source (Src/Modules/parameter.c) doesn't have a single
// "scan-by-name" function — each magic-assoc registers its own
// per-table getfn/scanfn pointer in the paramdef[] table at
// c:825-..., and zsh's paramtab dispatch reaches them through that
// table. fusevm_bridge's magic_assoc_lookup needs name → keys
// lookup at the call site; that aggregator is THIS Rust-only
// convenience, parked outside src/ported/ per the rule that
// src/ported/ holds direct C ports only.

use std::cell::RefCell;
thread_local! {
    static SCAN_KEYS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Lookup helper for `${name[key]}` magic-assoc reads — dispatches
/// through canonical `PARTAB` (Src/Modules/parameter.c:2235 ports).
/// Returns `None` if name isn't a known magic-assoc.
pub fn partab_get(name: &str, key: &str) -> Option<String> {
    for entry in PARTAB.iter() {
        if entry.name == name {
            return (entry.getfn)(std::ptr::null_mut(), key).and_then(|p| p.u_str);
        }
    }
    None
}

/// PM_ARRAY lookup for `${name}` / `${name[N]}` — walks
/// PARTAB_ARRAY and dispatches the whole-array getfn (Src/Modules/
/// parameter.c:2239-2291 ports). Returns `None` if name isn't a
/// known PM_ARRAY magic-assoc.
pub fn partab_array_get(name: &str) -> Option<Vec<String>> {
    for entry in PARTAB_ARRAY.iter() {
        if entry.name == name {
            return Some((entry.getfn)(std::ptr::null_mut()));
        }
    }
    None
}

/// Scan helper for `${(k)name}` — enumerates keys via canonical
/// scanfn, collected into Vec via SCAN_KEYS thread-local.
pub fn partab_scan_keys(name: &str) -> Option<Vec<String>> {
    for entry in PARTAB.iter() {
        if entry.name == name {
            SCAN_KEYS.with(|k| k.borrow_mut().clear());
            fn cb(node: &crate::ported::zsh_h::HashNode, _flags: i32) {
                SCAN_KEYS.with(|k| k.borrow_mut().push(node.nam.clone()));
            }
            (entry.scanfn)(std::ptr::null_mut(), Some(cb), 0);
            return Some(SCAN_KEYS.with(|k| k.borrow().clone()));
        }
    }
    None
}

// `scan_magic_assoc_keys` deleted — its 36-line per-name match
// dispatching to canonical scanpmX is the same routing PARTAB +
// `partab_scan_keys` (above) and PARTAB_ARRAY + `partab_array_get`
// now provide directly. Zero callers after the bridge magic-assoc
// fallback was cut over to PARTAB-only dispatch in b092a5dc19.

/// Populate paramtab with PM_SPECIAL placeholder Params for every
/// PARTAB / PARTAB_ARRAY entry — Rust-only init helper, no direct
/// C counterpart (closest is `handlefeatures` walking `partab[]`
/// in `Src/Modules/parameter.c:2341` boot/enables chain).
///
/// Each magic-assoc name gets a Param with `entry.flags | PM_SPECIAL`.
/// Value reads still route through `partab_get` / `partab_array_get`;
/// having the Param in paramtab makes `paramtab.get(name)` return
/// Some(Param) so `${+name}` / `${(t)name}` / `typeset -p name` see
/// the entry. Without this, those reads returned empty for every
/// magic-assoc (aliases, commands, functions, etc.).
///
/// Called from ShellExecutor::new() since zshrs's bin entry skips
/// the canonical module-bootstrap chain.
pub fn init_partab_params() {
    use crate::ported::modules::parameter::{PARTAB, PARTAB_ARRAY};
    use crate::ported::zsh_h::{hashnode, param, Param, PM_READONLY, PM_SPECIAL};
    let mut tab = match paramtab().write() {
        Ok(t) => t,
        Err(_) => return,
    };
    // Strip PM_READONLY when seeding stubs: read-only Params block
    // INTERNAL writes from the runtime's own function-call /
    // funcstack-push paths that go through setaparam. Real reads
    // route via PARTAB getfn callbacks, not these stub Params, so
    // the readonly flag's purpose (block userspace assignment) is
    // moot for the stub anyway.
    let mk_pm = |name: &str, flags: i32| -> Param {
        Box::new(param {
            node: hashnode {
                next: None,
                nam: name.to_string(),
                flags: (flags & !(PM_READONLY as i32)) | PM_SPECIAL as i32,
            },
            u_data: 0,
            u_arr: None,
            u_str: None,
            u_val: 0,
            u_dval: 0.0,
            u_hash: None,
            gsu_s: None,
            gsu_i: None,
            gsu_f: None,
            gsu_a: None,
            gsu_h: None,
            base: 0,
            width: 0,
            env: None,
            ename: None,
            old: None,
            level: 0,
        })
    };
    for entry in PARTAB.iter() {
        tab.insert(entry.name.to_string(), mk_pm(entry.name, entry.flags));
    }
    for entry in PARTAB_ARRAY.iter() {
        tab.insert(entry.name.to_string(), mk_pm(entry.name, entry.flags));
    }
}

// =====================================================================
// SubstState bridge — DELETED per user directive ("delete SubstState").
//
// `subst_state_from_executor` and `subst_state_commit_to_executor`
// were Rust-only plumbing that snapshotted executor state into a
// `SubstState` struct, then mutated it back out. Both the struct
// and the bridge are gone. subst.rs now reads/writes canonical
// globals (`utils::errflag`, `hist::hsubl/hsubr/hsubpatopt`,
// `options::opt_state_get/set`) and executor state directly via
// `fusevm_bridge::try_with_executor`. The single piece of state
// the bridge guarded — bumping `exec.last_status` on errflag — now
// lives at the per-call site in fusevm_bridge.rs subst_port arms.
// =====================================================================

impl ShellExecutor {
    pub fn enter_posix_mode(&mut self) {
        self.posix_mode = true;
        self.plugin_cache = None;
        self.compsys_cache = None;
        self.compinit_pending = None;
        self.worker_pool = std::sync::Arc::new(crate::worker::WorkerPool::new(1));
        // Route through canonical dispatch_builtin → BUILTINS["emulate"]
        // (Src/builtin.c bin_emulate entry). execbuiltin parses the
        // `-R` flag from the "LR" optstr automatically.
        dispatch_builtin(
            "emulate",
            vec!["sh".to_string(), "-R".to_string()],
        );
    }
    pub fn enter_ksh_mode(&mut self) {
        self.plugin_cache = None;
        self.compsys_cache = None;
        self.compinit_pending = None;
        self.worker_pool = std::sync::Arc::new(crate::worker::WorkerPool::new(1));
        dispatch_builtin(
            "emulate",
            vec!["ksh".to_string(), "-R".to_string()],
        );
    }
}

// ─────────────────────────────────────────────────────────
// Static glob match — module-level free fn (no executor state).
// PURE PASSTHRU to `pattern::patmatch` (C port of
// `Src/pattern.c::patmatch`). The 535-line hand-rolled regex
// translator that previously lived here re-implemented every
// glob feature patcompile already handles — extendedglob `^pat`
// negation, `~` exclusion, `!()` kshglob negation, inline `(#i)`
// `(#I)` `(#l)` `(#aN)` flags, numeric ranges `<a-b>`, alternation
// `(a|b)`, char classes `[...]`. Route through patmatch instead.
// ─────────────────────────────────────────────────────────
pub fn glob_match_static(s: &str, pattern: &str) -> bool {
    // Argument order is reversed vs patmatch(pattern, text) — keep
    // the public (text, pattern) order so callers don't have to
    // change.
    patmatch(pattern, s)
}

// `loadautofn` (port of `Src/exec.c:5682`) and `getfpfunc` (port of
// `Src/exec.c:5260`) moved to their canonical port file at
// `src/ported/exec.rs`. Reference them as
// `crate::ported::exec::{loadautofn, getfpfunc}`.
