//! Shell executor state for zshrs.
//!
//! **Not a port of Src/exec.c.** zshrs replaces zsh's tree-walking
//! interpreter (`execlist` / `execpline` / `execcmd`) with a fusevm
//! bytecode VM; the actual VM bridge lives in `src/fusevm_bridge.rs`.
//! This file holds:
//! - `ShellExecutor` — the runtime state struct that the VM and
//!   every ported builtin/utility threads through
//! - VM-adjacent helpers that read/write that state
//! - drift extension scaffolding still being moved out
//!
//! Path-wise this file lives at the crate root (`src/exec.rs`) rather
//! than in `src/ported/` because nothing here corresponds 1:1 to a
//! `Src/*.c` source file. `crate::ported::exec` is kept as a
//! re-export alias so existing call-sites continue to compile.

use crate::history::HistoryEngine;
use crate::math::MathEval;
use crate::options::ZSH_OPTIONS_SET;
use crate::pcre::PcreState;
use crate::prompt::{expand_prompt, PromptContext};
use crate::tcp::TcpSessions;
use crate::zftp::Zftp;
use crate::zprof::Profiler;
use crate::zutil::StyleTable;
use compsys::cache::CompsysCache;
use compsys::CompInitResult;
use parking_lot::Mutex;
use std::collections::HashSet;

// Backward-compat re-exports for free fns recently relocated to their
// canonical-C-file Rust modules. Existing call-sites in this file (and
// elsewhere) still reference these unqualified.
#[allow(unused_imports)]
#[allow(unused_imports)]
pub(crate) use crate::ported::glob::{expand_glob_alternation, find_top_level_tilde};
#[allow(unused_imports)]
pub use crate::ported::math::convbase as format_int_in_base;
pub use crate::ported::params::convbase_underscore;
#[allow(unused_imports)]
pub(crate) use crate::ported::math::{
    parse_subscript_arith_assign, parse_subscript_arith_compound, parse_subscript_arith_pre_inc,
};
#[allow(unused_imports)]
pub(crate) use crate::ported::params::getarrvalue;
#[allow(unused_imports)]
pub(crate) use crate::ported::pattern::{
    approximate_match, ksh_extglob_body_to_regex, parse_numeric_range,
    parse_pattern_flags_full,
};
#[allow(unused_imports)]
// drift imports removed: apply_subst_modifier, slice_scalar, strip_match_op
#[allow(unused_imports)]
pub(crate) use crate::ported::text::format_function_body_zsh;
#[allow(unused_imports)]
pub(crate) use crate::ported::utils::base64_decode;
#[allow(unused_imports)]
pub(crate) use crate::ported::utils::{
    ispwd, printprompt4, quotedzputs,
    shell_quote_value, zsh_split_z,
};

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
pub(crate) static REGEX_CACHE: LazyLock<Mutex<std::collections::HashMap<String, regex::Regex>>> =
    LazyLock::new(|| Mutex::new(std::collections::HashMap::with_capacity(64)));

// ───────────────────────────────────────────────────────────────────────────
// fusevm VM bridge (extension; not a port of Src/exec.c) lives in
// src/fusevm_bridge.rs. The bridge re-exports the symbols that the
// rest of the codebase imports as `crate::ported::exec::X`.
// ───────────────────────────────────────────────────────────────────────────
pub use crate::fusevm_bridge::*;
pub(crate) use crate::fusevm_bridge::{try_with_executor, with_executor, ExecutorContext};

/// Match an intercept pattern against a command name or full command string.
/// Supports: exact match, glob ("git *", "_*", "*"), or "all".

/// Get or compile a regex, caching the result
pub(crate) fn cached_regex(pattern: &str) -> Option<regex::Regex> {
    let mut cache = REGEX_CACHE.lock();
    if let Some(re) = cache.get(pattern) {
        return Some(re.clone());
    }
    match regex::Regex::new(pattern) {
        Ok(re) => {
            cache.insert(pattern.to_string(), re.clone());
            Some(re)
        }
        Err(_) => None,
    }
}

/// HashSet of all zsh options for O(1) lookup
/// O(1) builtin lookup — replaces the 130+ arm matches! macro in is_builtin()
pub(crate) static BUILTIN_SET: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "cd",
        "chdir",
        "pwd",
        "echo",
        "export",
        "unset",
        "source",
        "exit",
        "return",
        "bye",
        "logout",
        "log",
        "true",
        "false",
        "test",
        "local",
        "private",
        "declare",
        "typeset",
        "read",
        "shift",
        "eval",
        "jobs",
        "fg",
        "bg",
        "kill",
        "disown",
        "wait",
        "autoload",
        "history",
        "fc",
        "trap",
        "suspend",
        "alias",
        "unalias",
        "set",
        "setopt",
        "unsetopt",
        "getopts",
        "type",
        "hash",
        "command",
        "builtin",
        "let",
        "pushd",
        "popd",
        "dirs",
        "printf",
        "break",
        "continue",
        "disable",
        "enable",
        "emulate",
        "exec",
        "float",
        "integer",
        "functions",
        "print",
        "whence",
        "where",
        "which",
        "ulimit",
        "limit",
        "unlimit",
        "umask",
        "rehash",
        "unhash",
        "times",
        "zmodload",
        "r",
        "ttyctl",
        "noglob",
        "zstat",
        "stat",
        "output_strftime",
        "zsleep",
        "zselect",
        "zln",
        "zmv",
        "zcp",
        "coproc",
        "zparseopts",
        "readonly",
        "unfunction",
        "getln",
        "pushln",
        "bindkey",
        "zle",
        "sched",
        "zformat",
        "zcompile",
        "vared",
        "echotc",
        "echoti",
        "zpty",
        "zprof",
        "zsocket",
        "ztcp",
        "zregexparse",
        "clone",
        "comparguments",
        "compcall",
        "compctl",
        "compdef",
        "compdescribe",
        "compfiles",
        "compgroups",
        "compinit",
        "compquote",
        "comptags",
        "comptry",
        "compvalues",
        "cdreplay",
        "cap",
        "getcap",
        "setcap",
        "zftp",
        "zcurses",
        "bin_sysread",
        "bin_syswrite",
        "bin_syserror",
        "bin_sysopen",
        "bin_sysseek",
        "private",
        "zgetattr",
        "zsetattr",
        "zdelattr",
        "zlistattr",
        "[",
        ".",
        ":",
        "compgen",
        "complete",
    ]
    .into_iter()
    .chain(crate::daemon::builtins::ZSHRS_BUILTIN_NAMES.iter().copied())
    .collect()
});

/// Slice an array per zsh `${arr:offset[:length]}` semantics: the
/// offset is 0-based "skip N elements" (so `${arr:1:2}` returns
/// elements at indices 1,2). Negative offset counts from the end.
/// `length < 0` means "to the end".
pub(crate) fn slice_array_zero_based(arr: &[String], offset: i64, length: i64) -> Vec<String> {
    let n = arr.len() as i64;
    if n == 0 {
        return Vec::new();
    }
    let start = if offset < 0 {
        (n + offset).max(0) as usize
    } else {
        (offset as usize).min(arr.len())
    };
    let take = if length < 0 {
        arr.len().saturating_sub(start)
    } else {
        (length as usize).min(arr.len().saturating_sub(start))
    };
    arr.iter().skip(start).take(take).cloned().collect()
}

/// Same shape but for positional params (`@`/`*`). zsh treats
/// position 0 as `$0` (the script/shell name). For `${@:0}` it
/// includes `$0`; for `${@:1}` it skips `$0` and starts at `$1`.
/// Internally `positional_params[0]` is `$1`, so we prepend `$0`
/// then slice 0-based.
pub(crate) fn slice_positionals(exec: &ShellExecutor, offset: i64, length: i64) -> Vec<String> {
    let mut all: Vec<String> = Vec::with_capacity(exec.positional_params.len() + 1);
    all.push(
        exec.variables
            .get("0")
            .cloned()
            .unwrap_or_else(|| std::env::args().next().unwrap_or_default()),
    );
    for p in &exec.positional_params {
        all.push(p.clone());
    }
    slice_array_zero_based(&all, offset, length)
}

use crate::jobs::{JobState, JobTable};
use crate::parse::{Redirect, RedirectOp, ShellCommand, ShellWord, VarModifier, ZshParamFlag};
use crate::zwc::ZwcFile;
use indexmap::IndexMap;
use std::collections::HashMap;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};


// Drift structs moved to their canonical-C-file modules
// (src/ported/zle/computil.rs, modules/{zutil,zpty,zprof,socket}.rs,
// builtins/sched.rs). Re-exported here so existing call-sites that
// reference `crate::ported::exec::<Name>` keep compiling.
pub use crate::ported::zle::computil::{CompSpec, CompMatch, CompGroup, CompState};
pub use crate::ported::modules::zutil::ZStyle;
pub use crate::ported::modules::zpty::ZptyState;
pub use crate::ported::modules::zprof::ProfileEntry;
pub use crate::ported::modules::socket::UnixSocketState;
pub use crate::ported::builtins::sched::ScheduledCommand;
pub use crate::ported::builtin::AutoloadFlags;


/// Cross-VM loop-control signal. When `break`/`continue` is hit inside a body
/// that runs on a sub-VM (e.g. select's body), the inline patches mechanism
/// can't reach the outer loop — set this flag and the outer-loop builtin
/// drains it after each iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Loop control signal from a command body.
/// Mirrors the `LF_*` set Src/loop.c uses to thread
/// `break`/`continue`/`return` flags up through the executor.
pub enum LoopSignal {
    Break,
    Continue,
}

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
    pub variables: HashMap<String, String>,
    pub arrays: HashMap<String, Vec<String>>,
    pub assoc_arrays: HashMap<String, IndexMap<String, String>>,
    pub positional_params: Vec<String>,
    pub env_vars: HashMap<String, String>,
    /// Process working directory at subshell entry. `cd` inside the
    /// subshell shouldn't leak to the parent; we restore on End.
    pub cwd: Option<std::path::PathBuf>,
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
pub use crate::ported::params::{VarAttr, VarKind};


// Pattern helpers moved to src/ported/pattern.rs.
#[allow(unused_imports)]
pub(crate) use crate::ported::pattern::{
    expand_posix_char_classes, extract_numeric_ranges, replace_numeric_ranges_with_star,
    NumericRange,
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
    /// Mirrors C zsh's file-static `scriptname` (Src/init.c) — the
    /// short name used for `%N` / `%x` prompt expansion + the
    /// `scriptname:line: …` prefix on error messages. Decoupled from
    /// `$0` (which holds the full `argzero` path). Init sets this in
    /// `-c` mode to the binary basename per Src/init.c:479
    /// (`scriptname = scriptfilename = ztrdup("zsh")`); when sourcing
    /// a file via `source`/`bin_dot`, it becomes the resolved file
    /// path; otherwise it falls back through `$0` → `$ZSH_ARGZERO`.
    pub scriptname: Option<String>,
    pub aliases: HashMap<String, String>,
    pub global_aliases: HashMap<String, String>, // alias -g: expand anywhere
    pub suffix_aliases: HashMap<String, String>, // alias -s: expand by file extension
    /// Names whose alias is currently mid-expansion. zsh's lexer disables
    /// an alias from re-expanding inside its own body (so `alias ls='ls
    /// -la'` works without infinite recursion). zshrs expands aliases
    /// at run time, so we need an explicit recursion guard. Cleared
    /// when expansion of that name finishes.
    pub expanding_aliases: std::collections::HashSet<String>,
    /// Set by `break`/`continue` keywords when no enclosing loop in the
    /// current chunk's patch lists. Outer-loop builtins (BUILTIN_RUN_SELECT)
    /// observe + clear this after each body run.
    pub loop_signal: Option<LoopSignal>,
    /// Stack of subshell-state snapshots. Each `(…)` subshell pushes a copy
    /// of variables/arrays/assoc_arrays at entry and pops/restores at exit.
    /// Without this, `(x=inner; …); echo $x` shows `inner` instead of the
    /// outer-scope value.
    pub subshell_snapshots: Vec<SubshellSnapshot>,
    pub last_status: i32,
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
    pub variables: HashMap<String, String>,
    pub arrays: HashMap<String, Vec<String>>,
    pub assoc_arrays: HashMap<String, IndexMap<String, String>>, // zsh associative arrays (insertion-ordered, mirrors zsh hashtable hnodes)
    pub jobs: JobTable,
    pub fpath: Vec<PathBuf>,
    pub zwc_cache: HashMap<PathBuf, ZwcFile>,
    pub positional_params: Vec<String>,
    pub history: Option<HistoryEngine>,
    /// Session-relative history line counter. Starts at 0; incremented
    /// when an interactive command is recorded. Used by `%h`/`%!` in
    /// prompt expansion (zsh's "current history line number"), distinct
    /// from the persistent disk history total.
    pub session_histnum: i64,
    pub(crate) process_sub_counter: u32,
    pub traps: HashMap<String, String>,
    pub options: HashMap<String, bool>,
    pub completions: HashMap<String, CompSpec>, // command -> completion spec
    pub dir_stack: Vec<PathBuf>,
    // zsh completion system state
    pub comp_matches: Vec<CompMatch>, // Current completion matches
    pub comp_groups: Vec<CompGroup>,  // Completion groups
    pub comp_state: CompState,        // compstate associative array
    pub zstyles: Vec<ZStyle>,         // zstyle configurations
    pub comp_words: Vec<String>,      // words on command line
    pub comp_current: i32,            // current word index (1-based)
    pub comp_prefix: String,          // PREFIX parameter
    pub comp_suffix: String,          // SUFFIX parameter
    pub comp_iprefix: String,         // IPREFIX parameter
    pub comp_isuffix: String,         // ISUFFIX parameter
    pub readonly_vars: std::collections::HashSet<String>, // Read-only variables
    /// Per-variable attribute table. Tracks the type/flag declared via
    /// `typeset -i / -F / -E / -L / -R / -Z / -r / -x / -A / -a` so the
    /// `(t)` parameter flag can return the canonical zsh type string
    /// (`integer`, `float`, `scalar-left`, `scalar-readonly-export`, …).
    pub var_attrs: std::collections::HashMap<String, VarAttr>,
    /// Last `:s/X/Y/` history-modifier pair, replayed by `:&`.
    /// Direct port of `hsubl` / `hsubr` globals in Src/hist.c.
    /// SubstState mirrors / commits this so all paramsubst calls
    /// share the most recent value.
    pub last_subst: Option<(String, String, u8)>,
    /// SUB_* flag bits set per paramsubst call by the (M)/(R)/(B)/
    /// (E)/(N)/(S) flag-loop arms. Read by BUILTIN_PARAM_FILTER /
    /// REPLACE / STRIP to alter their match disposition. Direct
    /// port of subst.c:2169-2199 — value matches zsh.h:1981-1996
    /// (SUB_MATCH=0x0008, SUB_REST=0x0010, etc.). Reset by the
    /// dispatch arm after consumption.
    pub sub_flags: u32,
    /// Stack for `local` variable save/restore (name, old_value).
    pub local_save_stack: Vec<(String, Option<String>)>,
    /// Parallel stack for `local arr=(...)` array save/restore.
    /// `Some(prev)` means restore on exit; `None` means the name had no
    /// outer array binding and should be removed.
    pub local_array_save_stack: Vec<(String, Option<Vec<String>>)>,
    /// Parallel stack for `local -A h=(...)` assoc save/restore. zsh
    /// shadows the outer assoc binding; without this, `typeset -A h`
    /// inside a function leaked into the parent.
    pub local_assoc_save_stack: Vec<(String, Option<IndexMap<String, String>>)>,
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
    /// Nesting depth of `${...}` (paramsubst) recursion. Bumped by
    /// every `substitute_brace` / nested-paramsubst entry in the
    /// engine. `BUILTIN_PARAM_FLAG` consults `> 1` to decide whether
    /// to collapse a split-result array back to scalar (top-level
    /// DQ) or pass through (nested — outer subscript / second-level
    /// substitution still needs the array shape). Direct port of
    /// zsh paramsubst's recursive aval threading (Src/subst.c:3245+
    /// where the inner call returns aval; outer continues without
    /// re-joining until its own emission point).
    pub in_paramsubst_nest: u32,
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
    /// Command-context stack — direct port of zsh's `cmdstack`
    /// global (Src/prompt.c:56 `unsigned char *cmdstack`). Pushed
    /// by `BUILTIN_CMD_PUSH` (compile_zsh emits around each
    /// compound command), popped by `BUILTIN_CMD_POP`. Read by
    /// `%_` in PS4 / prompt expansion to render the cumulative
    /// control-flow context labels in the xtrace prefix
    /// (`if`, `then`, `cmdand`, `cmdor`, `cmdsubst`, …).
    /// `build_prompt_context` clones this into PromptContext so
    /// the prompt expander sees the live stack.
    pub cmd_stack: Vec<crate::prompt::CmdState>,
    /// IDs of history entries explicitly added during this session
    /// via `print -s`. `fc -l` uses this to scope listings to just
    /// the script-added entries (matches zsh's `-c` semantics where
    /// session history is the only thing visible to the script).
    pub session_history_ids: Vec<i64>,
    pub autoload_pending: HashMap<String, AutoloadFlags>, // Functions marked for autoload
    // zsh hooks (precmd, preexec, chpwd, etc.)
    pub hook_functions: HashMap<String, Vec<String>>, // hook_name -> [function_names]
    // Named directories (hash -d)
    pub named_dirs: HashMap<String, PathBuf>, // name -> path
    // zpty - pseudo-terminal management
    pub zptys: HashMap<String, ZptyState>,
    // bin_sysopen - file descriptor management
    pub open_fds: HashMap<i32, std::fs::File>,
    pub next_fd: i32,
    // sched - scheduled commands
    pub scheduled_commands: Vec<ScheduledCommand>,
    // zprof - profiling data
    pub profile_data: HashMap<String, ProfileEntry>,
    pub profiling_enabled: bool,
    // zsocket - Unix domain sockets
    pub unix_sockets: HashMap<i32, UnixSocketState>,
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
    // command hash table (hash builtin)
    pub command_hash: HashMap<String, String>,
    // Control flow signals
    pub returning: Option<i32>, // Set by return builtin, cleared after function returns
    pub breaking: i32,          // break level (0 = not breaking, N = break N levels)
    pub continuing: i32,        // continue level
    // New module state
    pub pcre_state: PcreState,
    pub tcp_sessions: TcpSessions,
    pub zftp: Zftp,
    pub profiler: Profiler,
    pub style_table: StyleTable,
    /// Persistent state for the `cap`/`getcap`/`setcap` family —
    /// owned here so the canonical port at
    /// `src/ported/modules/termcap.rs` survives across `echotc`
    /// calls. `Termcap` defaults to an uninitialised handle; the
    /// canonical port lazy-initialises on first use.
    pub termcap: crate::termcap::Termcap,
    /// Persistent watch/log state — the canonical port at
    /// `src/ported/modules/watch.rs:625` (`bin_log()` from
    /// `Src/Modules/watch.c`) needs to remember the previous utmp
    /// snapshot across calls.
    pub watch_state: crate::watch::WatchState,
    /// Persistent zcurses windows/colour-pairs — the canonical
    /// port at `src/ported/modules/curses.rs:573` mutates this in
    /// place (window creation, attribute mods, etc.).
    pub curses: crate::curses::Curses,
    /// Persistent set of named zpty subprocesses — the canonical
    /// port at `src/ported/modules/zpty.rs:367` looks up names
    /// across calls (`zpty -r`, `zpty -w`, `zpty -d`).
    pub pty_cmds: crate::zpty::PtyCmds,
    /// Persistent scheduler queue used by the canonical port at
    /// `src/ported/builtins/sched.rs:291`. Distinct from the
    /// legacy `scheduled_commands` field above which predates the
    /// canonical Scheduler port.
    pub sched: crate::builtins::sched::Scheduler,
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
    /// Single-string substitution via the canonical pipeline. Snapshots
    /// the executor state into a `SubstState`, runs `singsub` from
    /// `Src/subst.c:514`, commits any side-effects (assigns inside
    /// `${var:=default}`, etc.) back to the executor.
    ///
    /// Replaces the bot-invented `expand_string` method that was deleted
    /// in the citation purge (180463e1e7). All call sites that previously
    /// did `exec.singsub(s)` now do `exec.singsub(s)` and route
    /// through the C-faithful `singsub`.
    pub fn singsub(&mut self, s: &str) -> String {
        let mut state = crate::ported::subst::SubstState::from_executor(self);
        let r = crate::ported::subst::singsub(s, &mut state);
        state.commit_to_executor(self);
        r
    }


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

        // Initialize standard zsh variables
        let mut variables = HashMap::new();
        variables.insert("ZSH_VERSION".to_string(), "5.9".to_string());
        variables.insert(
            "ZSH_PATCHLEVEL".to_string(),
            "zsh-5.9-0-g73d3173".to_string(),
        );
        variables.insert("ZSH_NAME".to_string(), "zsh".to_string());
        // $ZSH_ARGZERO mirrors `posixzero` from Src/init.c:271
        // (`argv0 = argzero = posixzero = *argv++`). Src/params.c:971
        // does the actual `setsparam("ZSH_ARGZERO", ztrdup(posixzero))`
        // at the same setup phase Rust handles here. For -c / runscript
        // invocations the bin entrypoint overrides this with the
        // script path (Src/init.c:297).
        variables.insert(
            "ZSH_ARGZERO".to_string(),
            std::env::args().next().unwrap_or_else(|| "zsh".to_string()),
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
        // `$histchars` defaults to `!^#` per zshparam(1) — bang
        // (history expand), hat (quick subst), hash (comment).
        // Initialize so script reads return the canonical 3-char
        // string instead of empty.
        variables.insert("histchars".to_string(), "!^#".to_string());

        // `$WATCHFMT` default per Src/Modules/watch.c:60 — used by
        // `log` / `watch` builtins to format login/logout events.
        // Without this, `${WATCHFMT}` returned empty even though
        // C-zsh ships the documented `%n has %a %l from %m.` format.
        variables.insert("WATCHFMT".to_string(), "%n has %a %l from %m.".to_string());

        // `$FUNCNEST` default. Real zsh defaults to 500 (Src/zsh.h
        // MAXNEST), but zshrs's bytecode-VM recursion eats ~40KB of
        // Rust stack per frame and tops out around 150 on the
        // default 8MB stack. We seed `100` here so plugin probes
        // (`${FUNCNEST:-default}`) get a realistic cap that
        // matches what `call_function` actually enforces. Users
        // who need deeper need to raise FUNCNEST explicitly AND
        // run with a larger stack (RUST_MIN_STACK).
        variables.insert("FUNCNEST".to_string(), "100".to_string());

        // Run setlocale(LC_ALL, "") so nl_langinfo() (used by the
        // `langinfo` module) returns the host's actual locale instead
        // of the C/POSIX default ("US-ASCII"). Direct port of zsh's
        // Src/init.c:1208 setlocale call. unsafe { } around libc is
        // standard for this exact use-case — setlocale is process-
        // global and must run once at startup.
        unsafe {
            libc::setlocale(libc::LC_ALL, c"".as_ptr());
        }

        let mut exec = Self {
            aliases: {
                let mut a = HashMap::new();
                // zsh ships these two aliases compiled-in; visible in
                // a fresh `zsh -f -c 'alias'`. Adding them so zshrs's
                // alias listing matches zsh's defaults.
                a.insert("run-help".to_string(), "man".to_string());
                a.insert("which-command".to_string(), "whence".to_string());
                a
            },
            scriptname: None,
            global_aliases: HashMap::new(),
            suffix_aliases: HashMap::new(),
            expanding_aliases: std::collections::HashSet::new(),
            loop_signal: None,
            subshell_snapshots: Vec::new(),
            last_status: 0,
            inline_env_stack: Vec::new(),
            current_command_glob_failed: std::cell::Cell::new(false),
            variables,
            arrays: {
                let mut a = HashMap::new();
                // $path mirrors $PATH (tied array)
                let path_dirs: Vec<String> = env::var("PATH")
                    .unwrap_or_default()
                    .split(':')
                    .map(|s| s.to_string())
                    .collect();
                a.insert("path".to_string(), path_dirs);
                a
            },
            // `terminfo` and `termcap` are NOT pre-seeded into
            // `assoc_arrays` — `magic_assoc_lookup` handles them
            // lazily via ncurses (tigetstr/tgetstr) so ANY cap name
            // resolves, not just a hardcoded common subset. Pre-
            // seeding broke uncommon caps (e.g. `${terminfo[bel]}`
            // returned "") because the `user_defined_assoc` gate at
            // line ~3110 short-circuits magic-lookup once the name
            // is already in `assoc_arrays`.
            assoc_arrays: HashMap::new(),
            jobs: JobTable::new(),
            fpath,
            zwc_cache: HashMap::new(),
            positional_params: Vec::new(),
            history,
            session_histnum: 0,
            completions: HashMap::new(),
            dir_stack: Vec::new(),
            process_sub_counter: 0,
            traps: HashMap::new(),
            options: Self::default_options(),
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
            readonly_vars: std::collections::HashSet::new(),
            var_attrs: std::collections::HashMap::new(),
            last_subst: None,
            sub_flags: 0,
            local_save_stack: Vec::new(),
            local_array_save_stack: Vec::new(),
            local_assoc_save_stack: Vec::new(),
            local_scope_depth: 0,
            pending_underscore: None,
            in_dq_context: 0,
            in_paramsubst_nest: 0,
            in_scalar_assign: 0,
            cmd_stack: Vec::new(),
            session_history_ids: Vec::new(),
            autoload_pending: HashMap::new(),
            hook_functions: HashMap::new(),
            named_dirs: HashMap::new(),
            zptys: HashMap::new(),
            open_fds: HashMap::new(),
            next_fd: 10,
            scheduled_commands: Vec::new(),
            profile_data: HashMap::new(),
            profiling_enabled: false,
            unix_sockets: HashMap::new(),
            compsys_cache: {
                let cache_path = compsys::cache::default_cache_path();
                if cache_path.exists() {
                    let db_size = std::fs::metadata(&cache_path).map(|m| m.len()).unwrap_or(0);
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
                    let _ = std::fs::create_dir_all(parent);
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
            command_hash: HashMap::new(),
            returning: None,
            breaking: 0,
            continuing: 0,
            pcre_state: PcreState::new(),
            tcp_sessions: TcpSessions::new(),
            zftp: Zftp::new(),
            profiler: Profiler::new(),
            style_table: StyleTable::new(),
            termcap: Default::default(),
            watch_state: crate::watch::WatchState::new(),
            curses: Default::default(),
            pty_cmds: Default::default(),
            sched: Default::default(),
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
            pending_stdin: None,
            functions_compiled: HashMap::new(),
            function_source: HashMap::new(),
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
            exec.arrays.insert("fpath".to_string(), fpath_arr);
        }
        if let Ok(path) = env::var("PATH") {
            let path_arr: Vec<String> = path
                .split(':')
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect();
            if !path_arr.is_empty() {
                exec.arrays.insert("path".to_string(), path_arr);
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
        exec
    }

    // enter_posix_mode / enter_ksh_mode moved to src/ported/options.rs
    // (canonical C source: Src/options.c:533 emulate()).

    // host_apply_redirect / host_redirect_scope_begin / host_redirect_scope_end /
    // host_set_pending_stdin / host_exec_external moved to src/fusevm_bridge.rs
    // (extension; not a port of Src/exec.c).

    /// Add a directory to fpath
    pub fn add_fpath(&mut self, path: PathBuf) {
        if !self.fpath.contains(&path) {
            self.fpath.insert(0, path);
        }
    }

    /// Tab expansion — direct port of `zexpandtabs` in zsh/Src/utils.c:5973.
    /// Moved to `crate::ported::utils::zexpandtabs`; re-exported below.

    /// Execute a script file with bytecode caching — skips lex+parse+compile on cache hit.
    /// Bytecode is stored in rkyv keyed by (path, mtime).
    pub fn execute_script_file(&mut self, file_path: &str) -> Result<i32, String> {
        use std::path::Path;

        let path = Path::new(file_path);
        let abs_path = path
            .canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .to_string();

        // Try bytecode cache first — rkyv shard at ~/.cache/zshrs/scripts.rkyv.
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
                    let mut vm = fusevm::VM::new(chunk);
                    register_builtins(&mut vm);
                    let _ctx = ExecutorContext::enter(self);
                    match vm.run() {
                        fusevm::VMResult::Ok(_) | fusevm::VMResult::Halted => {
                            self.last_status = vm.last_status;
                        }
                        fusevm::VMResult::Error(e) => {
                            return Err(format!("VM error: {}", e));
                        }
                    }
                    return Ok(self.last_status);
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
        let mut parser = crate::parse::ZshParser::new(&content);
        let program = parser.parse().map_err(|errs| {
            errs.first()
                .map(|e| format!("{}", e))
                .unwrap_or_else(|| "parse error".to_string())
        })?;

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
            let mut vm = fusevm::VM::new(chunk);
            register_builtins(&mut vm);
            let _ctx = ExecutorContext::enter(self);
            match vm.run() {
                fusevm::VMResult::Ok(_) | fusevm::VMResult::Halted => {
                    self.last_status = vm.last_status;
                }
                fusevm::VMResult::Error(e) => {
                    return Err(format!("VM error: {}", e));
                }
            }
        }

        Ok(self.last_status)
    }

    /// Execute via the ZshLexer + ZshParser + ZshCompiler pipeline.
    /// This is the only execution path; `execute_script` delegates here.
    pub fn execute_script_zsh_pipeline(&mut self, script: &str) -> Result<i32, String> {
        // Skip history expansion for non-interactive script execution
        // (`zsh -c '…'`, internal eval, sourced files). zsh's `!`
        // history sub only fires on the REPL command line, never on
        // a pre-parsed script body. The interactive REPL has its
        // own dedicated path that calls expand_history before
        // dispatching here.
        let mut parser = crate::parse::ZshParser::new(script);
        let program = match parser.parse() {
            Ok(p) => p,
            Err(errs) => {
                return Err(errs
                    .first()
                    .map(|e| format!("{}", e))
                    .unwrap_or_else(|| "parse error".to_string()));
            }
        };

        let compiler = crate::compile_zsh::ZshCompiler::new();
        let chunk = compiler.compile(&program);

        if chunk.ops.is_empty() {
            return Ok(self.last_status);
        }

        let mut vm = fusevm::VM::new(chunk);
        register_builtins(&mut vm);
        {
            let _ctx = ExecutorContext::enter(self);
            match vm.run() {
                fusevm::VMResult::Ok(_) | fusevm::VMResult::Halted => {
                    self.last_status = vm.last_status;
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

        Ok(self.last_status)
    }

    #[tracing::instrument(skip(self, script), fields(len = script.len()))]
    pub fn execute_script(&mut self, script: &str) -> Result<i32, String> {
        // ZshLexer + ZshParser + ZshCompiler is the only execution path.
        self.execute_script_zsh_pipeline(script)
    }

    /// Whether `name` is a known function. Checks the compiled-functions
    /// table and the autoload-pending registry — `autoload foo` should
    /// make `whence foo`/`type foo`/`functions foo` recognize `foo` as
    /// a function before it's actually loaded. Doesn't trigger autoload
    /// itself; use `maybe_autoload` first if you need to load before
    /// introspecting.
    pub fn function_exists(&self, name: &str) -> bool {
        self.functions_compiled.contains_key(name) || self.autoload_pending.contains_key(name)
    }

    /// Canonical source text for a function. Returns from `function_source`
    /// (populated by autoload paths and runtime FuncDef registration via
    /// BUILTIN_REGISTER_COMPILED_FN with body_source). Returns `None` if
    /// no canonical source is on file.
    pub fn function_definition_text(&self, name: &str) -> Option<String> {
        self.function_source.get(name).cloned()
    }

    /// Remove a function from both tables (compiled chunk + canonical
    /// source). Returns true iff at least one table held it.
    pub fn remove_function(&mut self, name: &str) -> bool {
        let a = self.functions_compiled.remove(name).is_some();
        let c = self.function_source.remove(name).is_some();
        a || c
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
        // Resolve to a Chunk via the same cascade as ZshrsHost::call_function.
        // Always trigger autoload first if pending — the stub in self.functions
        // only counts as "loaded" once it has a real Chunk in functions_compiled.
        if self.autoload_pending.contains_key(name) {
            self.maybe_autoload(name);
        }
        let chunk = if let Some(c) = self.functions_compiled.get(name) {
            c.clone()
        } else {
            if !self.function_exists(name) {
                let _ = self.autoload_function(name);
            }
            self.functions_compiled.get(name).cloned()?
        };

        // FUNCNEST guard — see `call_function` for the lower-than-
        // zsh ceiling rationale. Cap at 100 by default (matches
        // call_function's ceiling).
        let funcnest_limit: usize = self
            .variables
            .get("FUNCNEST")
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
        let saved_params = std::mem::replace(&mut self.positional_params, args.to_vec());
        let saved_local_count = self.local_save_stack.len();
        let saved_local_arr_count = self.local_array_save_stack.len();
        let saved_local_assoc_count = self.local_assoc_save_stack.len();
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
        let saved_zero = self.variables.insert("0".to_string(), display_name);
        self.local_scope_depth += 1;

        let mut vm = fusevm::VM::new(chunk);
        register_builtins(&mut vm);
        let _ctx = ExecutorContext::enter(self);
        let _ = vm.run();
        let status = vm.last_status;
        drop(_ctx);

        self.positional_params = saved_params;
        self.local_scope_depth -= 1;
        match saved_zero {
            Some(v) => {
                self.variables.insert("0".to_string(), v);
            }
            None => {
                self.variables.remove("0");
            }
        }
        while self.local_save_stack.len() > saved_local_count {
            if let Some((var_name, old_val)) = self.local_save_stack.pop() {
                match old_val {
                    Some(v) => {
                        self.variables.insert(var_name, v);
                    }
                    None => {
                        self.variables.remove(&var_name);
                    }
                }
            }
        }
        while self.local_array_save_stack.len() > saved_local_arr_count {
            if let Some((arr_name, old_arr)) = self.local_array_save_stack.pop() {
                match old_arr {
                    Some(items) => {
                        self.arrays.insert(arr_name, items);
                    }
                    None => {
                        self.arrays.remove(&arr_name);
                    }
                }
            }
        }
        while self.local_assoc_save_stack.len() > saved_local_assoc_count {
            if let Some((assoc_name, old_assoc)) = self.local_assoc_save_stack.pop() {
                match old_assoc {
                    Some(map) => {
                        self.assoc_arrays.insert(assoc_name, map);
                    }
                    None => {
                        self.assoc_arrays.remove(&assoc_name);
                    }
                }
            }
        }

        // Honor explicit `return N` from inside the function body.
        if let Some(ret) = self.returning.take() {
            self.last_status = ret;
            Some(ret)
        } else {
            self.last_status = status;
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

    /// Parse `cmd_str` via ZshParser and pull out the first Simple
    /// command's words, untokenized + variable-expanded, ready to spawn
    /// as argv. Used by process-substitution where we need raw argv to
    /// hand to `Command::new`. Returns empty vec if the cmd isn't a
    /// simple shape — pipelines / compound forms aren't process-sub
    /// friendly anyway.
    fn simple_cmd_words(&mut self, cmd_str: &str) -> Vec<String> {
        let mut parser = crate::parse::ZshParser::new(cmd_str);
        let prog = match parser.parse() {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        };
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
                    self.singsub(&untoked)
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    pub(crate) fn run_process_sub_in(&mut self, cmd_str: &str) -> String {
        use std::fs;
        use std::process::Stdio;

        // Phase 2: parse via ZshParser. Extract the first Simple cmd's
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
        use std::fs;
        use std::process::Stdio;

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
                self.singsub(filename)
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

        // Port of getoutput() from Src/exec.c. Parse and compile via
        // the ZshLexer + ZshParser + ZshCompiler pipeline, run on a
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
        self.cmd_stack.push(crate::prompt::CmdState::CmdSubst);
        // Save LINENO so the inner cmdsubst's line counter doesn't
        // leak into the outer trace — direct port of Src/exec.c:1407
        // `oldlineno = lineno;` followed by `lineno = oldlineno;`
        // restore at line 1640. Inner program parses fresh as line 1
        // and increments from there; once it returns, the outer
        // line at the `$(…)` site must read the original outer
        // lineno (so xtrace renders `+:5:> echo …` not `+:1:> …`).
        let saved_lineno = self.variables.get("LINENO").cloned();
        // Anchor the inner program's lineno to the outer's current
        // $LINENO so xtrace inside the cmdsubst renders the outer
        // line. zsh's execlist preserves lineno across the inner
        // exec — for our sub-VM (fresh compile) we use lineno_addend
        // to shift inner's line N → outer_lineno + (N - 1).
        let outer_lineno: u64 = self
            .variables
            .get("LINENO")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        let mut parser = crate::parse::ZshParser::new(cmd_str);
        let prog = parser.parse().ok();
        let mut cmd_status: Option<i32> = None;
        if let Some(prog) = prog {
            let mut compiler = crate::compile_zsh::ZshCompiler::new();
            compiler.lineno_addend = outer_lineno.saturating_sub(1);
            let chunk = compiler.compile(&prog);
            if !chunk.ops.is_empty() {
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
                vm.last_status = self.last_status;
                let _ctx = ExecutorContext::enter(self);
                let _ = vm.run();
                cmd_status = Some(vm.last_status);
            }
        }
        // Restore LINENO so outer xtrace sees the outer line.
        if let Some(ln) = saved_lineno {
            self.variables.insert("LINENO".to_string(), ln);
        }
        self.cmd_stack.pop();
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
            self.last_status = status;
        } else {
            self.last_status = 0;
        }

        // Flush any buffered Rust-side stdout so it reaches the pipe
        // before we restore.
        use std::io::Write;
        let _ = io::stdout().flush();

        // Restore stdout and read what was captured.
        unsafe {
            libc::dup2(saved_stdout, libc::STDOUT_FILENO);
            libc::close(saved_stdout);
        }
        use std::os::unix::io::FromRawFd;
        let read_file = unsafe { std::fs::File::from_raw_fd(read_fd) };
        let mut output = String::new();
        use std::io::Read;
        let _ = std::io::BufReader::new(read_file).read_to_string(&mut output);

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
        let mut exec = ShellExecutor::new();
        let status = exec.execute_script("true").unwrap();
        assert_eq!(status, 0);
    }

    #[test]
    fn test_if_true() {
        let mut exec = ShellExecutor::new();
        let status = exec.execute_script("if true; then true; fi").unwrap();
        assert_eq!(status, 0);
    }

    #[test]
    fn test_if_false() {
        let mut exec = ShellExecutor::new();
        let status = exec
            .execute_script("if false; then true; else false; fi")
            .unwrap();
        assert_eq!(status, 1);
    }

    #[test]
    fn test_for_loop() {
        let mut exec = ShellExecutor::new();
        exec.execute_script("for i in a b c; do true; done")
            .unwrap();
        assert_eq!(exec.last_status, 0);
    }

    #[test]
    fn test_and_list() {
        let mut exec = ShellExecutor::new();
        let status = exec.execute_script("true && true").unwrap();
        assert_eq!(status, 0);

        let status = exec.execute_script("true && false").unwrap();
        assert_eq!(status, 1);
    }

    #[test]
    fn test_or_list() {
        let mut exec = ShellExecutor::new();
        let status = exec.execute_script("false || true").unwrap();
        assert_eq!(status, 0);
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
fn emit_path_or_assign(
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
    // ═══════════════════════════════════════════════════════════════════
    // AOP INTERCEPT — the killer builtin
    // ═══════════════════════════════════════════════════════════════════

    // ═══════════════════════════════════════════════════════════════════
    // CONCURRENT PRIMITIVES — ship work to the worker pool from shell
    // No stryke dependency. Pure zshrs. Thin binary gets full parallelism.
    // ═══════════════════════════════════════════════════════════════════
}

/// Natural-order string compare: walks both strings in parallel, treating
/// runs of digits as integer chunks. So "file2" < "file10" < "file20".
/// Used by the `(n)` / `(on)` / `(On)` parameter flag for human-friendly
/// sort. Falls back to byte-cmp for non-digit segments.
pub(crate) fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();
    let mut ai = 0;
    let mut bi = 0;
    while ai < a_bytes.len() && bi < b_bytes.len() {
        let a_is_digit = a_bytes[ai].is_ascii_digit();
        let b_is_digit = b_bytes[bi].is_ascii_digit();
        if a_is_digit && b_is_digit {
            // Skip leading zeros for the numeric compare, but keep them
            // for the lexical tiebreaker so "01" < "1" (zsh does this).
            let a_zero_end = ai + a_bytes[ai..].iter().take_while(|c| **c == b'0').count();
            let b_zero_end = bi + b_bytes[bi..].iter().take_while(|c| **c == b'0').count();
            let a_digits_end = a_zero_end
                + a_bytes[a_zero_end..]
                    .iter()
                    .take_while(|c| c.is_ascii_digit())
                    .count();
            let b_digits_end = b_zero_end
                + b_bytes[b_zero_end..]
                    .iter()
                    .take_while(|c| c.is_ascii_digit())
                    .count();
            let a_num = &a_bytes[a_zero_end..a_digits_end];
            let b_num = &b_bytes[b_zero_end..b_digits_end];
            // Compare by length of stripped digits first (shorter = smaller),
            // then byte-by-byte if same length.
            match a_num.len().cmp(&b_num.len()) {
                Ordering::Equal => match a_num.cmp(b_num) {
                    Ordering::Equal => {
                        // Numeric values equal; tiebreak on raw zero-prefix length.
                        let a_lead = a_zero_end - ai;
                        let b_lead = b_zero_end - bi;
                        if a_lead != b_lead {
                            return a_lead.cmp(&b_lead);
                        }
                        ai = a_digits_end;
                        bi = b_digits_end;
                    }
                    ord => return ord,
                },
                ord => return ord,
            }
        } else {
            match a_bytes[ai].cmp(&b_bytes[bi]) {
                Ordering::Equal => {
                    ai += 1;
                    bi += 1;
                }
                ord => return ord,
            }
        }
    }
    a_bytes.len().cmp(&b_bytes.len())
}

impl ShellExecutor {
    // ═══════════════════════════════════════════════════════════════════════════
    // Additional zsh builtins
    // ═══════════════════════════════════════════════════════════════════════════

    /// Helper to check if name is a builtin
    /// O(1) builtin check via static HashSet — replaces 130+ arm linear match
    pub(crate) fn is_builtin(&self, name: &str) -> bool {
        BUILTIN_SET.contains(name) || name.starts_with('_')
    }

    /// Helper to find command in PATH — checks command_hash first for O(1) hit
    pub(crate) fn find_in_path(&self, name: &str) -> Option<String> {
        // O(1) hash table lookup from rehash
        if let Some(path) = self.command_hash.get(name) {
            return Some(path.clone());
        }
        // Fallback: linear PATH walk
        let path_var = env::var("PATH").unwrap_or_default();
        for dir in path_var.split(':') {
            let full_path = format!("{}/{}", dir, name);
            if std::path::Path::new(&full_path).exists() {
                return Some(full_path);
            }
        }
        None
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // zsh module builtins
    // ═══════════════════════════════════════════════════════════════════════════

    // =========================================================================
    // Process control functions - Port from exec.c
    // =========================================================================

    /// Fork a new process
    /// Port of zfork() from exec.c
    pub fn zfork(&mut self, flags: ForkFlags) -> std::io::Result<ForkResult> {
        // Check for job control
        let can_background = self.options.get("monitor").copied().unwrap_or(false);

        unsafe {
            match libc::fork() {
                -1 => Err(std::io::Error::last_os_error()),
                0 => {
                    // Child process
                    if !flags.contains(ForkFlags::NOJOB) && can_background {
                        // Set up job control
                        let pid = libc::getpid();
                        if flags.contains(ForkFlags::NEWGRP) {
                            libc::setpgid(0, 0);
                        }
                        if flags.contains(ForkFlags::FGTTY) {
                            libc::tcsetpgrp(0, pid);
                        }
                    }

                    // Reset signal handlers
                    if !flags.contains(ForkFlags::KEEPSIGS) {
                        self.reset_signals();
                    }

                    Ok(ForkResult::Child)
                }
                pid => {
                    // Parent process
                    if !flags.contains(ForkFlags::NOJOB) {
                        // Add to job table
                        self.add_child_process(pid);
                    }
                    Ok(ForkResult::Parent(pid))
                }
            }
        }
    }

    /// Add a child process to tracking
    fn add_child_process(&mut self, pid: i32) {
        // Would track in job table
        self.variables.insert("!".to_string(), pid.to_string());
    }

    /// Reset signal handlers to defaults
    fn reset_signals(&self) {
        unsafe {
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::signal(libc::SIGQUIT, libc::SIG_DFL);
            libc::signal(libc::SIGTERM, libc::SIG_DFL);
            libc::signal(libc::SIGTSTP, libc::SIG_DFL);
            libc::signal(libc::SIGTTIN, libc::SIG_DFL);
            libc::signal(libc::SIGTTOU, libc::SIG_DFL);
            libc::signal(libc::SIGCHLD, libc::SIG_DFL);
        }
    }

    /// Execute a command in the current process (exec family)
    /// Port of zexecve() from exec.c
    pub fn zexecve(&self, cmd: &str, args: &[String]) -> ! {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let c_cmd = CString::new(cmd).expect("CString::new failed");

        // Build argv
        let c_args: Vec<CString> = std::iter::once(c_cmd.clone())
            .chain(args.iter().map(|s| CString::new(s.as_str()).unwrap()))
            .collect();

        let c_argv: Vec<*const libc::c_char> = c_args
            .iter()
            .map(|s| s.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();

        // Build envp from current environment
        let env_vars: Vec<CString> = std::env::vars()
            .map(|(k, v)| CString::new(format!("{}={}", k, v)).unwrap())
            .collect();

        let c_envp: Vec<*const libc::c_char> = env_vars
            .iter()
            .map(|s| s.as_ptr())
            .chain(std::iter::once(std::ptr::null()))
            .collect();

        unsafe {
            libc::execve(c_cmd.as_ptr(), c_argv.as_ptr(), c_envp.as_ptr());
            // If we get here, exec failed
            eprintln!(
                "zshrs: exec failed: {}: {}",
                cmd,
                std::io::Error::last_os_error()
            );
            std::process::exit(127);
        }
    }

    /// Enter a subshell
    /// Port of entersubsh() from exec.c
    pub fn entersubsh(&mut self, flags: SubshellFlags) {
        // Increment subshell level
        let level = self
            .get_variable("ZSH_SUBSHELL")
            .parse::<i32>()
            .unwrap_or(0);
        self.variables
            .insert("ZSH_SUBSHELL".to_string(), (level + 1).to_string());

        // Handle job control
        if flags.contains(SubshellFlags::NOMONITOR) {
            self.options.insert("monitor".to_string(), false);
        }

        // Close unneeded fds
        if !flags.contains(SubshellFlags::KEEPFDS) {
            self.close_extra_fds();
        }

        // Reset traps
        if !flags.contains(SubshellFlags::KEEPTRAPS) {
            self.reset_traps();
        }
    }

    /// Close extra file descriptors
    fn close_extra_fds(&self) {
        // Close fds > 10 (common shell convention)
        for fd in 10..256 {
            unsafe {
                libc::close(fd);
            }
        }
    }

    /// Reset all traps
    fn reset_traps(&mut self) {
        self.traps.clear();
    }

    /// Find command in PATH
    /// Port of findcmd() from exec.c
    pub fn findcmd(&self, name: &str, do_hash: bool) -> Option<String> {
        // Direct port of src/zsh/Src/exec.c:897-953 findcmd.
        //
        // Algorithm:
        //   1. If name contains `/` and is relative-prefixed (starts
        //      with `./`/`../`) OR is an absolute path, the caller
        //      shouldn't be calling findcmd — return None per
        //      exec.c:914-919 `if (s = strchr(arg0, '/')) ... return
        //      NULL;`. Match zsh: cmds with `/` are NOT searched
        //      through PATH.
        //   2. Hash table lookup first (exec.c:909-911) — fast path
        //      for cached resolutions.
        //   3. PATH walk (exec.c:943-951) — for each dir in $PATH,
        //      try `dir/name` and check via iscom (X_OK + S_ISREG).
        if name.is_empty() {
            return None;
        }
        if name.contains('/') {
            // exec.c:914-919 — path-containing names skip PATH.
            // The caller is expected to handle absolute / relative
            // paths directly via spawn/exec.
            return None;
        }

        // exec.c:909-911 — hash table lookup. Match zsh:
        // unconditional even when do_hash=true (the option is HASHCMDS
        // in zsh, governing whether to *populate* the hash, not
        // whether to consult it).
        if do_hash {
            if let Some(path) = self.command_hash.get(name) {
                if Self::iscom_static(path) {
                    return Some(path.clone());
                }
            }
        }

        // exec.c:943-951 — walk $PATH and X_OK-test each candidate.
        if let Ok(path_var) = std::env::var("PATH") {
            for dir in path_var.split(':') {
                let full = if dir.is_empty() {
                    name.to_string()
                } else {
                    format!("{}/{}", dir, name)
                };
                if Self::iscom_static(&full) {
                    return Some(full);
                }
            }
        }

        None
    }

    /// Check if `s` is an executable regular file. Direct port of
    /// src/zsh/Src/exec.c:961-969 iscom — `access(s, X_OK) == 0 &&
    /// stat(s).S_ISREG`. Static so findcmd can call it without
    /// borrowing self.
    fn iscom_static(s: &str) -> bool {
        use std::ffi::CString;
        let c = match CString::new(s) {
            Ok(c) => c,
            Err(_) => return false,
        };
        if unsafe { libc::access(c.as_ptr(), libc::X_OK) } != 0 {
            return false;
        }
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::stat(c.as_ptr(), &mut st) } != 0 {
            return false;
        }
        (st.st_mode & libc::S_IFMT) == libc::S_IFREG
    }

    /// Hash a command (add to command hash table)
    /// Port of hashcmd() from exec.c
    pub fn hashcmd(&mut self, name: &str, path: &str) {
        self.command_hash.insert(name.to_string(), path.to_string());
    }

    /// Check if command exists and is executable
    /// Port of iscom() from exec.c
    pub fn iscom(&self, name: &str) -> bool {
        // Check if it's a builtin
        if self.is_builtin_cmd(name) {
            return true;
        }

        // Check if it's a function
        if self.function_exists(name) {
            return true;
        }

        // Check if it's an alias
        if self.aliases.contains_key(name) {
            return true;
        }

        // Check in PATH
        self.findcmd(name, true).is_some()
    }

    /// Check if name is a builtin (process control version)
    fn is_builtin_cmd(&self, name: &str) -> bool {
        BUILTIN_SET.contains(name)
    }

    /// Close all file descriptors except stdin/stdout/stderr
    /// Port of closem() from exec.c
    pub fn closem(&self, exceptions: &[i32]) {
        for fd in 3..256 {
            if !exceptions.contains(&fd) {
                unsafe {
                    libc::close(fd);
                }
            }
        }
    }

    /// Create a pipe
    /// Port of mpipe() from exec.c
    pub fn mpipe(&self) -> std::io::Result<(i32, i32)> {
        let mut fds = [0i32; 2];
        let result = unsafe { libc::pipe(fds.as_mut_ptr()) };
        if result == -1 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok((fds[0], fds[1]))
        }
    }

    /// Add a file descriptor for redirection
    /// Port of addfd() from exec.c
    pub fn addfd(&self, fd: i32, target_fd: i32, mode: RedirMode) -> std::io::Result<()> {
        match mode {
            RedirMode::Dup => {
                if fd != target_fd {
                    unsafe {
                        if libc::dup2(fd, target_fd) == -1 {
                            return Err(std::io::Error::last_os_error());
                        }
                    }
                }
            }
            RedirMode::Close => unsafe {
                libc::close(target_fd);
            },
        }
        Ok(())
    }

    /// Get heredoc content
    /// Port of gethere() from exec.c
    pub fn gethere(&mut self, terminator: &str, strip_tabs: bool) -> String {
        let mut content = String::new();

        // Would read until terminator is found
        // This is simplified - real impl reads from input

        if strip_tabs {
            content = content
                .lines()
                .map(|line| line.trim_start_matches('\t'))
                .collect::<Vec<_>>()
                .join("\n");
        }

        content
    }

    /// Get herestring content
    /// Port of getherestr() from exec.c
    pub fn getherestr(&mut self, word: &str) -> String {
        let expanded = self.singsub(word);
        format!("{}\n", expanded)
    }

    /// Resolve a builtin command
    /// Port of resolvebuiltin() from exec.c
    pub fn resolvebuiltin(&self, name: &str) -> Option<BuiltinType> {
        if self.is_builtin_cmd(name) {
            Some(BuiltinType::Normal)
        } else {
            // Check disabled_builtins if we had that field
            None
        }
    }

    /// Check if cd is possible
    /// Port of cancd() from exec.c
    pub fn cancd(&self, path_str: &str) -> bool {
        use std::os::unix::fs::PermissionsExt;

        let path = std::path::Path::new(path_str);
        if !path.is_dir() {
            return false;
        }

        if let Ok(meta) = path.metadata() {
            let mode = meta.permissions().mode();
            // Check execute permission (needed for cd)
            let uid = unsafe { libc::getuid() };
            let gid = unsafe { libc::getgid() };
            let file_uid = meta.uid();
            let file_gid = meta.gid();

            if uid == file_uid {
                return (mode & 0o100) != 0;
            } else if gid == file_gid {
                return (mode & 0o010) != 0;
            } else {
                return (mode & 0o001) != 0;
            }
        }

        false
    }

    /// Command not found handler
    /// Port of commandnotfound() from exec.c
    pub fn commandnotfound(&mut self, name: &str, args: &[String]) -> i32 {
        if self.function_exists("command_not_found_handler") {
            let mut handler_args = vec![name.to_string()];
            handler_args.extend(args.iter().cloned());
            if let Some(code) =
                self.dispatch_function_call("command_not_found_handler", &handler_args)
            {
                return code;
            }
        }

        eprintln!("zshrs:1: command not found: {}", name);
        127
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
