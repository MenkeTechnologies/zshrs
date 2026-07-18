//! Shell executor state for zshrs.
//!
//! ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//! !!! LAST-RESORT FILE — NOT FOR NEW LOGIC !!!
//!
//! This file holds the `ShellExecutor` runtime state struct + VM-adjacent
//! helpers. It is **not** the place to add zsh logic — every line here that
//! does real shell work is a tax we pay because zshrs uses fusevm bytecode
//! instead of C zsh's wordcode walker.
//!
//! **Before adding code to this file, STOP and ask:**
//!
//!   1. Does the C source have a fn that does this? (Check `src/zsh/Src/*.c`)
//!      → Port it into `src/ported/<file>.rs` with line-by-line citations.
//!        Then call the canonical fn from here.
//!
//!   2. Does `src/ported/` already have a port?
//!      → Call it directly. Don't reimplement.
//!
//!   3. Is this purely a Rust-only state-struct accessor (getter/setter on
//!      ShellExecutor fields, VM init plumbing, executor-context guards)?
//!      → OK to put it here. Mark it `WARNING: RUST-ONLY HELPER` per memory
//!        `feedback_rust_only_helpers_need_warning`.
//!
//! **NEVER:** reinvent paramsubst/expansion/glob/typeset/redirect/scope
//! management here. Every one of those has a canonical port in `src/ported/`.
//! When a bridge-side fn grows past ~30 lines of shell logic, that's a
//! signal the work belongs in `src/ported/` — port it, don't inline.
//!
//! This file should be SHRINKING over time. Every PR that adds lines here
//! should justify it; every PR that moves lines OUT to `src/ported/` is
//! aligned with the project direction.
//!
//! See also: memory `feedback_no_shortcuts_in_porting`, `feedback_true_port_pattern`,
//! `feedback_no_shellexecutor_in_ported` (the inverse direction).
//! ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
//!
//! **Not a port of Src/exec.c.** C zsh runs compiled programs on the native
//! **wordcode walker** in `Src/exec.c` (`execlist` / `execpline` / `execcmd`).
//! zshrs uses fusevm bytecode instead; the bridge lives in `src/fusevm_bridge.rs`.
//! This file holds:
//! - `ShellExecutor` — the runtime state struct that the VM and
//!   every ported builtin/utility threads through
//! - VM-adjacent helpers that read/write that state
//!
//! Path-wise this file lives at the crate root (`src/vm_helper`) rather
//! than in `src/ported/` because nothing here corresponds 1:1 to a
//! `Src/*.c` source file. `crate::ported::exec` is kept as a
//! re-export alias so existing call-sites continue to compile.

use crate::compsys::cache::CompsysCache;
use crate::compsys::CompInitResult;
use crate::history::HistoryEngine;
use crate::options::ZSH_OPTIONS_SET;
use crate::ported::builtin::{BREAKS, CONTFLAG};
use crate::ported::math::mathevali;
use crate::ported::modules::parameter::*;
use crate::ported::subst::singsub;

thread_local! {
    /// Eval-recursion depth counter — no C counterpart by design.
    ///
    /// !!! WARNING: RUST-ONLY BACKSTOP — reproduces C behaviour, no C fn !!!
    ///
    /// zsh bounds runaway `eval` recursion via its job table: every eval'd
    /// list runs through `execpline`, which grabs a job slot per pipeline
    /// (`initjob`), and the table caps at `MAX_MAXJOBS` → `zerr("job table
    /// full or recursion limit exceeded")` (Src/jobs.c:1878-1884). The fusevm
    /// runtime that executes eval bodies allocates no job per pipeline, and
    /// nested evals push no funcstack frame (INEVAL suppression, matching zsh's
    /// `if (!ineval)` at Src/builtin.c:6164), so neither the job table nor the
    /// FUNCNEST/FUNCSTACK depth reflects eval nesting — leaving eval recursion
    /// unbounded until the (256 MB but finite) main-thread stack overflows →
    /// uncatchable SIGBUS. This counter is the Rust proxy for zsh's count of
    /// concurrently-held job slots: `builtin.rs::eval` bumps it around each
    /// eval body and refuses to recurse at the same `MAX_MAXJOBS` ceiling.
    /// Lives here (not src/ported/) because it is an architectural Rust-only
    /// backstop with no 1:1 C symbol.
    pub static EVAL_RECURSION_DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// O(1) single-key probe against the canonical hashed storage — the
/// fast-path companion to subst.rs `assoc_get` for order-independent
/// single-key reads. Returns `None` when `name` (post-nameref) isn't
/// an assoc; otherwise `Some((key_present, value))` under ONE lock,
/// with no whole-map clone and no zsh-bucket-order rebuild (that
/// reorder is only observable to whole-map enumeration). Hot: shell
/// loops reading `${assoc[$k]}` per iteration were O(n²) through
/// assoc_get — zpwr expandstats over 42k records took 43s vs zsh's ~1s.
/// Lives here (not src/ported/) because it has no C counterpart — C's
/// getarg IS the single-key path.
/// Classify an assoc subscript as an EXACT-key lookup and return the
/// key: plain text (no leading flag group) passes through; a leading
/// `(e…)`/`(E…)` group (c:Src/params.c:1449 — literal-key flag) is
/// stripped. Search groups ((r)/(i)/(k)/…) return `None` — they need
/// the full getarg walk. Companion gate for [`assoc_key_hit`]'s O(1)
/// fast paths.
pub fn exact_assoc_sub_key(sub: &str) -> Option<&str> {
    match sub.strip_prefix('(') {
        None => Some(sub),
        Some(rest) => {
            let close = rest.find(')')?;
            let grp = &rest[..close];
            if !grp.is_empty() && grp.chars().all(|c| c == 'e' || c == 'E') {
                Some(&rest[close + 1..])
            } else {
                None
            }
        }
    }
}

pub fn assoc_key_hit(name: &str, key: &str) -> Option<(bool, Option<String>)> {
    let resolved = match crate::ported::params::resolve_nameref_name(name, None) {
        crate::ported::params::nameref_resolution::Target { name: t_, .. } => t_,
        _ => name.to_string(),
    };
    crate::ported::params::paramtab_hashed_storage()
        .lock()
        .ok()
        .and_then(|s| {
            s.get(resolved.as_str())
                .map(|m| (m.contains_key(key), m.get(key).cloned()))
        })
}
use crate::ported::utils::{errflag, ERRFLAG_ERROR};
use crate::ported::zsh_h::PM_UNDEFINED;
use crate::ported::zsh_h::WC_SIMPLE;
use crate::ported::zsh_h::{options, MAX_OPS};
use crate::ported::zsh_h::{PM_ARRAY, PM_HASHED, PM_INTEGER, PM_READONLY};
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

// Backward-compat re-exports for free ported recently relocated to their
// canonical-C-file Rust modules. Existing call-sites in this file (and
// elsewhere) still reference these unqualified.
#[allow(unused_imports)]
pub(crate) use crate::func_body_fmt::FuncBodyFmt;
#[allow(unused_imports)]
pub(crate) use crate::ported::hist::bufferwords as bufferwords_z_tuple;
#[allow(unused_imports)]
pub(crate) use crate::ported::math::{parse_assign, parse_compound, parse_pre_inc};
#[allow(unused_imports)]
pub use crate::ported::params::convbase as format_int_in_base;
pub use crate::ported::params::convbase_underscore;
// `getarrvalue` is already re-exported by `pub use crate::ported::params::*`
// below; an explicit `pub(crate) use` here only shadowed that public export.
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

// fusevm VM bridge (extension; not a port of Src/exec.c) lives in
// src/fusevm_bridge.rs. Re-exports below let the rest of the codebase
// reference symbols as `crate::ported::exec::X`.
pub(crate) use crate::fusevm_bridge::ExecutorContext;
pub use crate::fusevm_bridge::*;

/// `ZSH_VERSION` / `ZSH_PATCHLEVEL` / `ZSH_VERSION_DATE` consts
/// generated by `build.rs` from `src/zsh/Config/version.mk`. Use
/// `zsh_version::ZSH_VERSION` etc. at call sites so version bumps
/// pick up automatically.
pub mod zsh_version {
    include!(concat!(env!("OUT_DIR"), "/zsh_version.rs"));
}

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
use indexmap::IndexMap;
use std::collections::HashMap;
use std::env;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

// Re-exports for call-sites that reference `crate::ported::exec::<Name>`.
pub use crate::bash_complete::CompSpec;
pub use crate::ported::builtin::AutoloadFlags;
pub use crate::ported::modules::zutil::zstyle_entry;

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
    pub paramtab: crate::fast_hash::FastMap<String, crate::ported::zsh_h::Param>,
    /// `paramtab_hashed_storage` field.
    pub paramtab_hashed_storage: HashMap<String, IndexMap<String, String>>,
    /// `positional_params` field.
    pub positional_params: Vec<String>,
    /// `env_vars` field.
    pub env_vars: HashMap<String, String>,
    /// Values of the special parameters whose backing store is a process
    /// GLOBAL rather than the parameter table — `Src/params.c`'s `char *ifs`
    /// (IFS), `wordchars`, `home`, `histsiz`, … — each reached through a GSU
    /// getfn/setfn pair (the dispatch list at params.rs:12548).
    ///
    /// The `paramtab` snapshot above restores the param NODE, but the node only
    /// carries the GSU pair; the value itself lives in the global, which a
    /// paramtab restore doesn't touch. C forks for `(...)`, so a child's writes
    /// to those globals die with it. zshrs runs subshells in-process, so
    /// `(IFS=,; :)` left the PARENT's IFS as `,` — and every later word-split
    /// in the parent silently used it. Same fork-copy reasoning as `opts` /
    /// `umask` / `aliases` above.
    pub special_globals: Vec<(String, String)>,
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
    /// subshell's own EXIT trap (if any) has fired. Stores a snapshot
    /// of `crate::ported::builtin::traps_table()` (canonical).
    pub traps: HashMap<String, String>,
    /// Parent's shell options at subshell entry. `(set -e)` /
    /// `(setopt extendedglob)` mustn't leak; zsh forks the subshell
    /// so child options die with the child. We run in-process, so we
    /// must restore the option store on subshell_end.
    pub opts: HashMap<String, bool>,
    /// Parent's alias entries at subshell entry. zsh forks for
    /// `(...)` so `(alias x=y)` inside a subshell dies with the
    /// child and doesn't leak to the parent. zshrs runs subshells
    /// in-process, so we must restore the alias table on
    /// subshell_end. Bug #209 in docs/BUGS.md. Stored as a flat
    /// Vec<(name, text, flags)> snapshot. The node FLAGS must
    /// round-trip: ALIAS_GLOBAL / DISABLED distinguish global and
    /// disabled aliases in the shared aliastab — the previous
    /// (name, text) shape restored every entry with flags=0, so ANY
    /// subshell (`(true)`, zsh-z's `(zshz --add … &)` precmd)
    /// reflagged every global alias to REGULAR in the parent:
    /// `alias -g` listed nothing and `${+galiases[x]}` went 0 one
    /// prompt after every define.
    pub aliases: Vec<(String, String, i32)>,
    /// Parent's shell-function table at subshell entry. C zsh's
    /// `entersubsh` (`Src/exec.c`) forks before running the
    /// subshell body so `(f() { ... })` defining a function dies
    /// with the child and never leaks to the parent. zshrs runs
    /// subshells in-process, so we must clone `shfunctab` on entry
    /// and restore on exit. Bug #208 in docs/BUGS.md. Stored as
    /// the full `HashMap<String, Box<shfunc>>` clone — shfunc is
    /// `Clone` and the table snapshot is bounded by the user's
    /// declared function set.
    pub shfuncs: std::collections::HashMap<String, Box<crate::ported::zsh_h::shfunc>>,
    /// Parent's compiled-function chunks at subshell entry. Companion
    /// to `shfuncs` above — `ShellExecutor.functions_compiled` is the
    /// runtime dispatch table that `Op::CallFunction` reads through;
    /// without restoring it, a subshell `(g() { override; })` leaves
    /// the override bytecode chunk in place so the parent's
    /// `g` call still runs the override after `subshell_end`
    /// restored shfunctab. Bug #208 in docs/BUGS.md.
    pub functions_compiled: HashMap<String, fusevm::Chunk>,
    /// Parent's function source map at subshell entry. Companion to
    /// `functions_compiled` so `typeset -f` / `whence` show the
    /// parent's source after subshell exit, not the subshell's
    /// overridden body. Bug #208 in docs/BUGS.md.
    pub function_source: HashMap<String, String>,
    /// Parent's modulestab `modules` map at subshell entry. zsh forks
    /// for `(...)` so a `(zmodload zsh/X)` inside the subshell sets
    /// MOD_INIT_B on the child's modulestab; when the child exits the
    /// flag dies with it and the parent's modulestab is untouched.
    /// zshrs runs subshells in-process, so a subshell `zmodload`
    /// would otherwise flip the parent's `${modules[zsh/X]}` from
    /// unset to "loaded". Snapshot here and restore on subshell_end.
    /// Bug #210 in docs/BUGS.md. Stored as `(name → flags)`
    /// since `module` struct doesn't derive Clone (LinkList/
    /// Linkedmod) — and the only thing `zmodload` mutates that
    /// affects introspection is the flags bitmask (MOD_INIT_B
    /// for loaded, MOD_UNLOAD for unloaded).
    pub modules: HashMap<String, i32>,
    /// Parent's THINGYTAB (ZLE widget registry) at subshell entry.
    /// zsh forks for `(...)` so `zle -N w f` / `zle -D w` inside the
    /// subshell flip widget bindings only in the child; when the
    /// child exits the parent's widget table is untouched. zshrs runs
    /// subshells in-process so a subshell's `zle -D w` would
    /// otherwise unbind the parent's widget. Bug #453 in docs/BUGS.md.
    pub thingytab: HashMap<String, crate::ported::zle::zle_thingy::Thingy>,
    /// Parent's KEYMAPNAMTAB (named keymap registry) at subshell
    /// entry. Same fork-copy semantics as THINGYTAB — a subshell's
    /// `bindkey -N km` / `bindkey -D km` mutates only the child's
    /// keymap registry in C zsh. Bug #454 in docs/BUGS.md.
    pub keymapnamtab: HashMap<String, crate::ported::zle::zle_keymap::KeymapName>,
    /// Parent's `$!` (clone::lastpid) at subshell entry. C zsh forks
    /// for `(...)`, so a background job started INSIDE the subshell
    /// sets the child's `lastpid` only — `( : & ); echo $!` prints 0
    /// in zsh. zshrs runs subshells in-process, so restore on end.
    pub lastpid: i32,
    /// Job-control state at subshell entry: (JOBTAB clone, CURJOB,
    /// PREVJOB, MAXJOB, THISJOB). C zsh forks for `(...)` so any
    /// `disown` / `wait` / new `&` job inside the subshell mutates the
    /// CHILD's copy of jobtab and dies with it (Src/exec.c::entersubsh
    /// fork semantics); the parent's table is untouched. zshrs's
    /// in-process subshell must snapshot/restore to match — without
    /// this, `sleep 1 & (disown); jobs` shows an empty table where
    /// zsh still lists the job. Bug #462.
    pub jobtab: Vec<crate::ported::zsh_h::job>,
    /// `curjob` at subshell entry (Src/jobs.c:75 global).
    pub curjob: i32,
    /// `prevjob` at subshell entry (Src/jobs.c:80 global).
    pub prevjob: i32,
    /// `maxjob` at subshell entry (Src/jobs.c:71 global).
    pub maxjob: usize,
    /// `thisjob` at subshell entry (Src/jobs.c:77 global).
    pub thisjob: i32,
    /// User-range fds (0-9) at subshell entry: `(fd, saved_dup)`
    /// pairs where `saved_dup` is an `F_DUPFD >= 10` copy, or -1 when
    /// the fd was closed at entry. C zsh forks for `(...)` so a bare
    /// `exec >file` / `exec 3<&-` inside the child dies with it
    /// (Src/exec.c entersubsh fork semantics); the in-process
    /// subshell must restore the parent's fd table on End. Without
    /// this, `(exec >t.log; ...); cat t.log` left the PARENT's fd 1
    /// pointing at t.log and `cat` looped forever copying the file
    /// into itself.
    pub saved_fds: Vec<(i32, i32)>,
}

#[allow(unused_imports)]
pub(crate) use crate::ported::pattern::{
    extract_numeric_ranges, numeric_range_contains, numeric_ranges_to_star,
};

/// Top-level shell executor state.
/// Fork-equivalent event counter — incremented on every external
/// spawn and in-process subshell entry. C zsh's `time` keyword
/// reports only for JOBS (forked work, Src/jobs.c printtime via the
/// job table); zshrs runs builtins/braces/functions in-process with
/// no job, so the TIME_SUBLIST handler compares this counter across
/// the timed body to decide whether to emit the report.
pub static FORK_EVENTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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
    /// `jobs` field.
    pub jobs: JobTable,
    /// `fpath` field.
    pub fpath: Vec<PathBuf>,
    /// `history` field.
    pub history: Option<HistoryEngine>,
    pub(crate) process_sub_counter: u32,
    pub completions: HashMap<String, CompSpec>, // command -> completion spec
    pub zstyles: Vec<zstyle_entry>,             // zstyle configurations
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
    /// `profiling_enabled` field.
    pub profiling_enabled: bool,
    // compsys - completion system cache
    /// `compsys_cache` field.
    pub compsys_cache: Option<CompsysCache>,
    // Background compinit — receiver for async fpath scan result
    /// `compinit_pending` field.
    pub compinit_pending: Option<(
        std::sync::mpsc::Receiver<CompInitBgResult>,
        std::time::Instant,
    )>,
    // Plugin source cache — stores side effects of source/. in SQLite
    /// `plugin_cache` field.
    pub plugin_cache: Option<crate::plugin_cache::PluginCache>,
    // cdreplay - deferred compdef calls for zinit turbo mode
    /// `deferred_compdefs` field.
    pub deferred_compdefs: Vec<Vec<String>>,
    // Control flow signals
    pub returning: Option<i32>, // Set by return builtin, cleared after function returns
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
    /// Per-scope saved-fd stacks for `Op::WithRedirectsBegin/End`. Each entry
    /// is a Vec of (fd, saved_dup_fd) pairs taken from `dup(fd)` before the
    /// redirect was applied; `with_redirects_end` `dup2`s them back and closes.
    pub redirect_scope_stack: Vec<Vec<(i32, i32)>>,
    /// Per-scope MULTIOS tee state. Each entry is `(pipe_write_fd,
    /// JoinHandle)`: the pipe write-end currently dup2'd onto the
    /// command's fd, and the splitter thread that reads from the
    /// pipe read-end and writes to every collected target. Closed
    /// + joined by `host_redirect_scope_end` BEFORE the saved fds
    /// are restored so the splitter drains every byte the body
    /// wrote into the pipe. Bug #36 in docs/BUGS.md.
    pub multios_scope_stack: Vec<Vec<(i32, std::thread::JoinHandle<()>)>>,
    /// True while applying a bare `exec`'s redirect list (`exec 1>&-`,
    /// `exec 2>/dev/null` — no command words). `host_apply_redirect`
    /// then skips pushing the saved fd into the enclosing scope so the
    /// fd change survives group/command teardown.
    /// c:Src/exec.c:3978-3986 — nullexec==1: "we specifically *don't*
    /// restore the original fd's before returning"; C's per-execcmd
    /// `save[]` means exec's redirs never enter the enclosing group's
    /// save list either. Toggled by `BUILTIN_EXEC_PERM_REDIRS`.
    pub exec_redirs_permanent: bool,
    /// Set in a forked pipeline-stage child right after its stdout is
    /// dup2'd onto the pipe write-end. Consumed by the FIRST
    /// `host_redirect_scope_begin` (the stage command's own redirect
    /// list) into `pipe_output_scope`.
    /// c:Src/exec.c:3722-3724 — `addfd(forked, save, mfds, 1, output,
    /// 1, NULL)`: the pipe occupies mfds[1] in the SAME execcmd that
    /// processes the stage command's redirect list.
    pub pipe_output_pending: bool,
    /// Index into `redirect_scope_stack` of the scope whose redirect
    /// list shares an execcmd with the pipeline output on fd 1. A
    /// write-side redirect of fd 1 applied at exactly this scope depth
    /// MULTIOS-splits (tees) instead of replacing — c:Src/exec.c:
    /// 2447-2480 addfd "split the stream". Cleared when that scope ends.
    pub pipe_output_scope: Option<usize>,
    /// Set by `host_apply_redirect` when a redirect target couldn't be
    /// opened (permission denied, no such directory, etc). The next
    /// builtin/command checks this at entry and short-circuits with
    /// status 1 instead of running. Mirrors zsh's "command skip" on
    /// redirect failure.
    pub redirect_failed: bool,
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
    /// Array→(scalar, sep) reverse-tie table. Used by BUILTIN_SET_ARRAY to
    /// join the array elements with `sep` and mirror to the scalar side.
    pub tied_array_to_scalar: HashMap<String, (String, String)>,

    // ── ztest framework counters (extensions/ztest.rs) ──────────────────
    //
    // Mirrors strykelang's per-VMHelper test counters
    // (strykelang/builtins.rs:22292-22308 + builtins.rs::test_pass/_fail/_skip,
    //  builtins.rs::test_pass_count etc.). Each `zassert_*` builtin bumps
    // the per-block counter; `ztest_run` rolls per-block into _total and
    // resets the per-block side, so a single test file with multiple
    // `ztest_run` calls can reuse the counters. The worker-pool runner in
    // src/extensions/ztest.rs reads pass_total+pass_count and
    // fail_total+fail_count after `execute_script` returns for the cumulative
    // numbers (strykelang/cli_runners.rs:115-118). `ztest_run_failed` is a
    // sticky bool the runner reads so a test that asserts but then exits
    // 0 still flags as failed. `ztest_suppress_stdout` matches
    // VMHelper::suppress_stdout — the runner sets it inside the forked
    // grandchild so the per-test stderr capture stays clean.
    /// Per-block pass count (reset by `ztest_run`).
    pub ztest_pass_count: std::sync::atomic::AtomicUsize,
    /// Per-block fail count (reset by `ztest_run`).
    pub ztest_fail_count: std::sync::atomic::AtomicUsize,
    /// Per-block skip count (reset by `ztest_run`).
    pub ztest_skip_count: std::sync::atomic::AtomicUsize,
    /// Cumulative pass total across the run.
    pub ztest_pass_total: std::sync::atomic::AtomicUsize,
    /// Cumulative fail total across the run.
    pub ztest_fail_total: std::sync::atomic::AtomicUsize,
    /// Cumulative skip total across the run.
    pub ztest_skip_total: std::sync::atomic::AtomicUsize,
    /// Sticky failure flag — set by any `ztest_run` that observed fails;
    /// the CLI runner reads this so a test that asserts then exits 0
    /// still counts as a failed file.
    pub ztest_run_failed: std::sync::atomic::AtomicBool,
    /// Suppress per-assertion `✓`/`✗` lines on stderr. Set by the worker
    /// runner inside the forked child when it has already redirected
    /// fd 2 to a tmp file (we still want the lines, but only after the
    /// runner re-emits them under print_lock to avoid line-tearing).
    pub ztest_suppress_stdout: bool,
}

/// Context-isolated nested parse — the AST-path bridge for C's
/// `parse_string` (`Src/exec.c:283`). zshrs executes via the ZshProgram
/// AST + `compile_zsh`, not wordcode, so this can't live in `src/ported/`
/// (C's `parse_string` returns `Eprog`, and the build.rs port-gate rejects
/// any non-C-named fn there). It's the bridge that wraps the AST
/// `parse_init`+`parse` in the SAME isolation `parse_string` provides.
///
/// Why this exists: a runtime parse — command-substitution body,
/// process-substitution argv — must not clobber the outer
/// `loop()`/`parse_event` reader's live input when it interleaves parsing
/// with execution (faithful single-event mode).
///
/// The load-bearing piece is `strinbeg(0)`/`strinend()` (c:290/298). It
/// sets the `strin` flag so that when the nested lexer drains `cmd_str`,
/// `ingetc` returns EOF (input.rs:391) instead of falling through to
/// `inputline()`, which would STEAL the outer reader's next SHIN line —
/// e.g. `echo A` / `v=$(echo hi)` / `echo B` had the cmd-subst swallow
/// `echo B` off stdin, and the outer loop then hit EOF after one command.
/// `strinbeg` also runs `hbegin`/`lexinit`, so it must execute on isolated
/// history+lexer state — hence the surrounding `zcontext_save`/`restore`
/// (c:288/300), which saves+restores `tok`/`tokstr`/`lexbuf`/`isnewlin`/
/// `incmdpos`/heredocs/`lexstop`/`toklineno`/history.
///
/// Two zshrs-specific globals `zcontext` doesn't cover are saved here too:
/// the lexer-input window (`LEX_INPUT`/`LEX_POS`/`LEX_UNGET_BUF`) that
/// `lex_init` overwrites with `cmd_str`, and the line counter `LEX_LINENO`
/// (C saves `oldlineno` explicitly at parse_string c:291/295).
///
/// NOTE: using `inpush` (the literal C input stack) instead does NOT work
/// in zshrs's hybrid input model — the outer piped reader pulls from SHIN
/// via `inputline`, and pushing/popping the `inbuf` stack severs that
/// continuation. The `LEX_INPUT` window + `strin` flag is the working
/// equivalent.
pub(crate) fn parse_isolated(input: &str) -> crate::parse::ZshProgram {
    use crate::ported::lex::{tok, LEXERR, LEX_INPUT, LEX_LINENO, LEX_POS, LEX_UNGET_BUF};

    crate::ported::context::zcontext_save(); // c:288
                                             // Save the zshrs-specific lexer window + line counter that lex_init
                                             // overwrites but zcontext doesn't cover.
    let saved_input = LEX_INPUT.with_borrow(|s| s.clone());
    let saved_pos = LEX_POS.get();
    let saved_unget = LEX_UNGET_BUF.with_borrow(|b| b.clone());
    let saved_lineno = LEX_LINENO.get(); // c:291 oldlineno
                                         // input.rs `lexstop` is the input-side half of C's single `lexstop`;
                                         // draining the nested LEX_INPUT sets it true and zcontext only covers
                                         // the lex.rs half (LEX_LEXSTOP). Restore it so the outer reader isn't
                                         // left at EOF.
    let saved_in_lexstop = crate::ported::input::lexstop.with(|c| c.get());

    crate::ported::hist::strinbeg(0); // c:290 — strin++ → drained nested input EOFs (no SHIN steal)
    crate::ported::parse::parse_init(input); // install cmd_str as LEX_INPUT (lex_init), LEX_LINENO=1
    let program = crate::ported::parse::parse(); // c:294 (AST analog of par_list)

    // Capture parse failure BEFORE the restores wipe the signals.
    let parse_err = (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 || tok() == LEXERR;
    if tok() == LEXERR && crate::ported::builtin::LASTVAL.load(Ordering::Relaxed) == 0 {
        crate::ported::builtin::LASTVAL.store(1, Ordering::Relaxed); // c:296-297
    }

    crate::ported::hist::strinend(); // c:298 — strin--
                                     // Restore the zshrs window, then the token/parse/history state.
    LEX_INPUT.with_borrow_mut(|s| *s = saved_input);
    LEX_POS.set(saved_pos);
    LEX_UNGET_BUF.with_borrow_mut(|b| *b = saved_unget);
    LEX_LINENO.set(saved_lineno); // c:295
    crate::ported::input::lexstop.with(|c| c.set(saved_in_lexstop));
    crate::ported::context::zcontext_restore(); // c:300
                                                // zcontext_restore → parse_context_restore clears ERRFLAG_ERROR
                                                // (parse.c:354); re-raise so callers gating on the bit still see it.
    if parse_err {
        errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
    }
    program
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
    /// SET_VAR / `+=` arms (case-fold, integer-add, readonly guard).
    /// Returns 0 when the name isn't in paramtab. Mirrors the C
    /// source's direct `pm->node.flags & PM_INTEGER` checks.
    pub fn param_flags(&self, name: &str) -> i32 {
        paramtab()
            .read()
            .ok()
            .and_then(|t| t.get(name).map(|p| p.node.flags))
            .unwrap_or(0)
    }

    /// `readonly` / `typeset -r` / read-only-by-design (LINENO, PPID,
    /// $$, $?, $!, ...) — match user-side rejection in C's
    /// assignstrvalue at `Src/params.c:2699-2703` which gates on
    /// `pm->node.flags & PM_READONLY` where the IPDEF4 family declares
    /// `PM_READONLY_SPECIAL = PM_SPECIAL | PM_READONLY | PM_RO_BY_DESIGN`
    /// (both bits set together). zshrs's special_params table carries
    /// PM_RO_BY_DESIGN alone for IPDEF4 entries so internal direct-write
    /// paths (BUILTIN_SET_LINENO bypasses via pm.u_val) don't trip the
    /// readonly guard. User-facing checks must accept either bit. Bug
    /// #418-family / test_lineno_intrinsic_readonly.
    pub fn is_readonly_param(&self, name: &str) -> bool {
        let (flags, pm_level) = crate::ported::params::paramtab()
            .read()
            .ok()
            .and_then(|t| t.get(name).map(|p| (p.node.flags as u32, p.level)))
            .unwrap_or((0, 0));
        // c:Src/params.c assignsparam — a real PM_READONLY param always
        // rejects writes.
        if (flags & PM_READONLY) != 0 {
            return true;
        }
        if (flags & crate::ported::zsh_h::PM_RO_BY_DESIGN) != 0 {
            // c:Src/Modules/param_private.c pps_setfn (c:300-307) — a
            // PRIVATE param (PM_RO_BY_DESIGN + PM_REMOVABLE) is NOT blanket
            // read-only: a write is permitted iff it is in the SAME scope
            // (`locallevel == pm->level`, e.g. `() { private p=1; p=2 }`
            // → 2) or above the wrap level (`locallevel >
            // private_wraplevel`). A deeper nested-scope write is rejected
            // (setfn_error) — that is how a nested fn writing an OUTER
            // function's private still errors and aborts. zshrs never
            // wires the private GSU, so the level gate is enforced here.
            if (flags & crate::ported::zsh_h::PM_REMOVABLE) != 0 {
                let ll = crate::ported::params::locallevel.load(Ordering::Relaxed);
                let wrap = crate::ported::modules::param_private::private_wraplevel
                    .load(Ordering::Relaxed);
                return !(ll == pm_level || ll > wrap); // c:304 (negated: blocked)
            }
            // Non-removable PM_RO_BY_DESIGN = IPDEF4 special (LINENO/$?/$$…)
            // whose PM_READONLY bit zshrs strips for internal writes (bug
            // #297); RO_BY_DESIGN is their only remaining read-only marker.
            return true;
        }
        false
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
    /// (`Src/params.c:3101`).
    pub fn array(&self, name: &str) -> Option<Vec<String>> {
        getaparam(name)
    }

    /// Read an associative array parameter from canonical
    /// `paramtab_hashed_storage`. Mirrors C `gethparam` at
    /// `Src/params.c:3115` — returns the typed `IndexMap`.
    pub fn assoc(&self, name: &str) -> Option<IndexMap<String, String>> {
        // c:Src/params.c:570-575 — nameref deref before the read.
        let resolved = match crate::ported::params::resolve_nameref_name(name, None) {
            crate::ported::params::nameref_resolution::Target { name: t_, .. } => t_,
            _ => name.to_string(),
        };
        paramtab_hashed_storage()
            .lock()
            .ok()
            .and_then(|m| m.get(resolved.as_str()).cloned())
    }

    /// Test whether a scalar parameter exists in paramtab.
    /// Mirrors the C `paramtab->getnode(name) != NULL` check.
    pub fn has_scalar(&self, name: &str) -> bool {
        getsparam(name).is_some()
    }

    /// Test whether an array parameter exists in paramtab. Routes
    /// through canonical `getaparam` (PM_TYPE check + digit-first-name
    /// rejection).
    pub fn has_array(&self, name: &str) -> bool {
        getaparam(name).is_some()
    }

    /// Test whether an associative array parameter exists. Reads
    /// canonical `paramtab_hashed_storage` (Src/params.c hashed
    /// PM_HASHED slot).
    pub fn has_assoc(&self, name: &str) -> bool {
        // c:Src/params.c:570-575 — nameref deref before the read.
        let resolved = match crate::ported::params::resolve_nameref_name(name, None) {
            crate::ported::params::nameref_resolution::Target { name: t_, .. } => t_,
            _ => name.to_string(),
        };
        paramtab_hashed_storage()
            .lock()
            .ok()
            .map(|m| m.contains_key(resolved.as_str()))
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
    /// sufaliastab with ALIAS_SUFFIX node flag — mirrors C
    /// Src/builtin.c:4480-4481 (`flags1 |= ALIAS_SUFFIX; ht =
    /// sufaliastab;`) → c:4527 (`createaliasnode(value, flags1)`).
    /// Without ALIAS_SUFFIX in node.flags, `${saliases[k]}` /
    /// `${(k)saliases}` introspection (parameter.c:1953/2018) fails
    /// because both paths strict-equality-match flags == ALIAS_SUFFIX.
    pub fn set_suffix_alias(&mut self, name: String, value: String) {
        if let Ok(mut tab) = crate::ported::hashtable::sufaliastab_lock().write() {
            tab.add(crate::ported::hashtable::createaliasnode(
                &name,
                &value,
                crate::ported::zsh_h::ALIAS_SUFFIX as u32,
            ));
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
    /// `new` — see implementation.
    pub fn new() -> Self {
        tracing::debug!("ShellExecutor::new() initializing");

        // c:Src/init.c:1236-1259 — setupvals' pwd/oldpwd init, ported
        // here because the bin entry skips setupvals (see the
        // init_bltinmods note below). The validated value lands in the
        // live OS env: the bin entry's `$PWD` carrier (the analog of
        // C's `pwd` global — see the subshell-snapshot comment at
        // fusevm_bridge.rs `cwd:` field). set_pwd_env() pours it into
        // paramtab after the env-import loop, same order as C
        // (params.c:955).
        //
        // c:1242-1245 — "Try a cheap test to see if we can initialize
        // `PWD' from `HOME'." EMULATE_ZSH reads the `home` global,
        // which setupvals derives from getpwuid(getuid())->pw_dir
        // (c:1222-1225), falling back to "/" (c:1230-1232).
        let home = unsafe {
            let pw = libc::getpwuid(libc::getuid());
            if pw.is_null() {
                None
            } else {
                Some(
                    std::ffi::CStr::from_ptr((*pw).pw_dir)
                        .to_string_lossy()
                        .into_owned(),
                )
            }
        }
        .unwrap_or_else(|| "/".to_string()); // c:1230-1232 EMULATE_ZSH home = "/"
                                             // ispwd (src/zsh/Src/utils.c:809-829): a candidate is honored
                                             // only when it (a) is absolute, (b) stat's to the same
                                             // dev+inode as ".", and (c) has no `.`/`..` components.
                                             // Without this chain, a child that inherits $PWD from a parent
                                             // run in a different directory (cargo test setting
                                             // current_dir(tempdir) while leaking PWD=/project/root) treats
                                             // the stale PWD as the logical-path base, so `cd sub` resolves
                                             // against the wrong directory.
        let pwd_val = if ispwd(&home) {
            home // c:1245-1246 — pwd = ztrdup(ptr) [HOME]
        } else if let Some(p) = env::var("PWD")
            .ok()
            .filter(|p| p.len() < libc::PATH_MAX as usize && ispwd(p))
        {
            p // c:1247-1249 — pwd = ztrdup(getenv("PWD"))
        } else {
            crate::ported::compat::zgetcwd() // c:1250-1252 — pwd = zgetcwd()
        };
        env::set_var("PWD", &pwd_val);
        // c:1255-1259 — oldpwd = getenv("OLDPWD") ?: ztrdup(pwd).
        if env::var("OLDPWD").is_err() {
            env::set_var("OLDPWD", &pwd_val); // c:1257
        }

        // Initialize fpath from FPATH env var or use defaults
        let fpath = env::var("FPATH")
            .unwrap_or_default()
            .split(':')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();

        let history = HistoryEngine::new().ok();

        // Seed canonical OPTS_LIVE with defaults BEFORE any setsparam
        // call. assignstrvalue early-returns when `unset(EXECOPT)`
        // (c:2701 guard); without the option table populated, EXECOPT
        // reads false and every paramtab write below is a silent no-op.
        if opt_state_len() == 0 {
            for (k, v) in Self::default_options() {
                opt_state_set(&k, v);
            }
        }

        // Standard zsh scalar param defaults — direct port of
        // `createparamtable` (Src/params.c:817-988) + the `setupvals`
        // tail. Writes through canonical `setsparam` (Src/params.c:3350).
        //
        // c:params.c:972-973 — ZSH_VERSION / ZSH_PATCHLEVEL.
        // `zsh_version::ZSH_VERSION` (emitted by build.rs from the
        // vendored `Config/version.mk`) is the development snapshot
        // tag `5.9.0.3-test`; shipped zsh binaries report the clean
        // release form (`5.9`). Bug #73 in docs/BUGS.md — cross-shell
        // scripts that gate on `[[ $ZSH_VERSION = 5.9 ]]` or split on
        // `.` expecting MAJOR.MINOR break on the `-test` suffix.
        //
        // Use the cleaned `patchlevel::ZSH_VERSION` here ("5.9") and
        // surface the full snapshot tag as `$ZSHRS_VERSION` for
        // zshrs-specific identity checks.
        setsparam("ZSH_VERSION", crate::ported::patchlevel::ZSH_VERSION);
        // c:Src/params.c:43 + Src/patchlevel.h — `ZSH_PATCHLEVEL` is
        // a git-describe-style identifier (`zsh-MAJOR.MINOR-N-gHASH`)
        // of the upstream commit zshrs targets. `build.rs` emits
        // "unknown" because the vendored zsh tarball doesn't ship a
        // CUSTOM_PATCHLEVEL define; use the canonical const in
        // `patchlevel.rs` instead (snapshot of `src/zsh/Src/patchlevel.h`).
        // Bug #90 in docs/BUGS.md — scripts that fingerprint by
        // $ZSH_PATCHLEVEL fell to the wildcard arm under "unknown".
        setsparam("ZSH_PATCHLEVEL", crate::ported::patchlevel::ZSH_PATCHLEVEL);
        // Skip ZSHRS_VERSION in `--zsh` parity mode so `${(k)parameters}`
        // doesn't carry a name zsh doesn't ship — matches the guard
        // in `ported::params::createparamtable`. Scripts running under
        // `--zsh` mode can still detect zshrs via `$ZSH_VERSION` which
        // carries a `-test` suffix.
        if !crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed) {
            setsparam("ZSHRS_VERSION", crate::ported::patchlevel::ZSHRS_VERSION);
        }
        setsparam("ZSH_NAME", "zsh");
        // c:params.c:971 — ZSH_ARGZERO from `posixzero` (Src/init.c:271):
        // the kernel-supplied argv[0] of THIS binary, in --zsh parity
        // mode too. The bin entrypoint overrides this with the script
        // path for -c / runscript invocations. (A previous revision
        // probed the system zsh install path and reported THAT as
        // ZSH_ARGZERO for byte-parity — faking the shell's identity.
        // Parity tests that compare the value must normalize the
        // machine-specific binary path in the test row instead.)
        let argzero_default = env::args().next().unwrap_or_else(|| "zsh".to_string());
        setsparam("ZSH_ARGZERO", &argzero_default);
        setsparam("WORDCHARS", "*?_-.[]~=/&;!#$%^(){}<>");
        // SHLVL is NOT seeded here. c:Src/params.c:948-951 increments it
        // AFTER the environ-import loop, so the +1 lives at the end of that
        // loop below — see the `c:948-951` block. Doing it here instead meant
        // parsing the raw env string with `parse::<i32>()`, which mis-read
        // every non-decimal form C accepts via zstrtol_underscore
        // (SHLVL=0x10 must give 17, 010 → 9, 1_0 → 11, 9abc → 10, abc → 1).
        // POSIX/zsh default IFS: space + tab + newline + NUL.
        setsparam("IFS", " \t\n\0");
        // POSIX getopts: OPTIND starts at 1.
        setsparam("OPTIND", "1");
        // Note: OPTERR is NOT pre-initialised. zsh leaves it unset
        // even after `getopts` calls (verified: `getopts ":a" opt -a`
        // does not set it). It's a user-writable variable that
        // starts unset. Bug #150 in docs/BUGS.md.
        // zsh wipes inherited `$_` (unlike bash).
        setsparam("_", "");
        // c:params.c:5064 — histchars derives from bangchar+hatchar+
        // hashchar (defaults `!`, `^`, `#`). At init the special
        // entry may not exist yet — fall back to the literal default.
        let histchars_val = paramtab()
            .read()
            .ok()
            .and_then(|t| {
                t.get("histchars")
                    .or_else(|| t.get("HISTCHARS"))
                    .map(|pm| histcharsgetfn(pm))
            })
            .unwrap_or_else(|| "!^#".to_string());
        setsparam("histchars", &histchars_val);

        // c:Src/params.c:870-871 — `setsparam("TIMEFMT", ...)` etc.
        // Seed TIMEFMT explicitly so `${(k)parameters}` lists it
        // (the createparamtable() ported in ported::params isn't
        // invoked from this bin entry — its setsparam calls don't
        // run, so TIMEFMT only existed via the lookup_special_var
        // fallback, which scanpmparameters can't see).
        setsparam("TIMEFMT", crate::ported::zsh_system_h::DEFAULT_TIMEFMT);
        // c:Src/init.c:1214-1215 — `nullcmd = ztrdup("cat");
        // readnullcmd = ztrdup(DEFAULT_READNULLCMD);`. Real paramtab
        // seeds (NOT read-time fallbacks) so `unset NULLCMD` truly
        // unsets — the bare-redirect "redirection with no command"
        // diagnostic depends on getsparam returning None afterwards.
        // c:config.h:48 DEFAULT_READNULLCMD "more" — the parity
        // floor agrees: scrubbed-env Homebrew zsh 5.9.1 -fc reports
        // READNULLCMD=more (probed; the previous macOS arm's "less"
        // guess came from the USER's env exporting READNULLCMD=less
        // — zpwr sets it). This block runs AFTER the env import, so
        // these are DEFAULT seeds only: an env-imported value must
        // win (C seeds before the import loop, c:854-885 vs c:893+).
        if getsparam("NULLCMD").map_or(true, |v| v.is_empty()) {
            setsparam("NULLCMD", "cat");
        }
        if getsparam("READNULLCMD").map_or(true, |v| v.is_empty()) {
            setsparam("READNULLCMD", crate::ported::config_h::DEFAULT_READNULLCMD);
        }
        // c:Src/init.c:963 — `setsparam("TTY", ttyname(0) ?: "")`.
        // Even in non-interactive -fc mode zsh creates the param;
        // mirror so ${(k)parameters} count matches.
        let tty_str = unsafe {
            let p = libc::ttyname(0);
            if p.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(p)
                    .to_str()
                    .unwrap_or("")
                    .to_string()
            }
        };
        setsparam("TTY", &tty_str);
        // c:Src/params.c:968-979 — `setaparam("signals", ...)`.
        // Build the signal-name array. Mirror by inserting directly
        // into paramtab so ${(k)parameters} sees it.
        {
            use crate::ported::signals_h::SIGS;
            // c:signames.c sigs[] (generated) — index 0 is "EXIT",
            // entries 1..=SIGCOUNT are in PLATFORM SIGNAL-NUMBER
            // order, tail is "ZERR", "DEBUG" (zsh.h SIGZERR/SIGDEBUG).
            // SIGS is declared in Linux textual order, so sort by the
            // libc number to reproduce the generated table's order on
            // every platform. Same construction as params.rs — keep
            // in sync.
            let mut by_num: Vec<(&str, i32)> = SIGS.to_vec();
            by_num.sort_by_key(|&(_, n)| n);
            let mut signals_arr: Vec<String> = Vec::with_capacity(by_num.len() + 3);
            signals_arr.push("EXIT".to_string()); // c:sigs[0]
            signals_arr.extend(by_num.iter().map(|(n, _)| n.to_string()));
            signals_arr.push("ZERR".to_string()); // c:sigs tail
            signals_arr.push("DEBUG".to_string()); // c:sigs tail
            crate::ported::params::setaparam("signals", signals_arr);
        }
        // c:Src/init.c:1186-1193 — default prompt strings. zsh sets
        // PS4 to "+%N:%i> " for ZSH emulation ("+ " for KSH/SH).
        // Without seeding, PS4 reads empty and `set -x` output has
        // no prefix at all. Bug #92 in docs/BUGS.md.
        //
        // C zsh runs createparamtable's env-import loop (c:893-924)
        // BEFORE init.c:1186 fires, so an exported $PS4 in the parent
        // env wins over the default seed. zshrs's env import happens
        // further down in ShellExecutor::new() (at the createparamtable
        // call site), so getsparam() reads None here even when env has
        // a value, and the default would clobber the user's PS4.
        //
        // Additional wrinkle: C zsh's PROMPT / PROMPT2 / PROMPT3 /
        // PROMPT4 params are ALIASES for PS1..PS4 (Src/params.c:381,
        // 415-421 — both IPDEF7R entries bind to the same `prompt*`
        // global). So `export PROMPT4=...` in the parent env sets the
        // shared global, and `$PS4` reads the same string. The user's
        // interactive shell exports PROMPT4 (the form zsh's prompt
        // theme system uses), so when zshrs -x runs, PROMPT4 is in
        // env but PS4 is not. Without aliasing in the env-probe step,
        // zshrs seeds default PS4 and ignores the user's customised
        // prefix.
        //
        // Probe env::var directly for the name AND its alias; first
        // non-empty wins. Only fall through to the default seed when
        // every candidate is empty. Mirrors C zsh's behavior without
        // reshuffling the rest of new(). Bug: `zshrs -x` ignored the
        // user's custom PS4/PROMPT4 unless re-forwarded with
        // `PS4=$PROMPT4 zshrs -x`.
        let seed_prompt = |name: &str, alias: Option<&str>, default: &str| {
            let cur = crate::ported::params::getsparam(name);
            let have_param = cur.as_deref().map_or(false, |s| !s.is_empty());
            if have_param {
                return;
            }
            // Probe primary name first, then the C-side alias.
            for candidate in std::iter::once(name).chain(alias.into_iter()) {
                if let Ok(env_val) = std::env::var(candidate) {
                    if !env_val.is_empty() {
                        setsparam(name, &env_val);
                        return;
                    }
                }
            }
            setsparam(name, default);
        };
        seed_prompt("PS4", Some("PROMPT4"), "+%N:%i> ");
        // c:Src/init.c:1181-1190 —
        //     if(unset(INTERACTIVE)) {
        //         prompt = ztrdup("");
        //         prompt2 = ztrdup("");
        //     } else ... {
        //         prompt  = ztrdup("%m%# ");
        //         prompt2 = ztrdup("%_> ");
        //     }
        // Non-interactive shells get EMPTY primary/secondary prompts
        // — `zsh -fc 'typeset'` lists PS1='' — while interactive ones
        // get the %m%# defaults. PS3/PS4/SPROMPT are seeded
        // unconditionally in C (c:1191-1194). PS1 may be reset by the
        // prompt-theme layer; only seed when the slot is empty so any
        // prior theme write wins.
        let interactive = crate::ported::zsh_h::isset(crate::ported::zsh_h::INTERACTIVE);
        seed_prompt(
            "PS1",
            Some("PROMPT"),
            if interactive { "%m%# " } else { "" },
        );
        seed_prompt(
            "PS2",
            Some("PROMPT2"),
            if interactive { "%_> " } else { "" },
        );
        // c:Src/init.c:1191 — `prompt3 = ztrdup("?# ");`
        seed_prompt("PS3", Some("PROMPT3"), "?# ");
        // c:Src/init.c:1194 — `sprompt = ztrdup("zsh: correct '%R'
        // to '%r' [nyae]? ");` — spelling-correction prompt.
        seed_prompt("SPROMPT", None, "zsh: correct '%R' to '%r' [nyae]? ");
        // c:Src/params.c:417-422 — `PROMPT*` aliases for `PS*`.
        // C zsh's IPDEF7("PROMPT", &prompt), IPDEF7("PROMPT2",
        // &prompt2), IPDEF7("PROMPT3", &prompt3), IPDEF7("PROMPT4",
        // &prompt4) all point to the same C globals as the matching
        // IPDEF7("PS{1..4}", ...) entries — they're aliases in C,
        // sharing storage. zshrs's paramtab keeps them as separate
        // entries; mirror the alias by mirroring the value here.
        // Bug #274 in docs/BUGS.md (PROMPT3 was the visible report;
        // PROMPT/PROMPT2/PROMPT4 had the same gap silently).
        for (alias, source) in &[
            ("PROMPT", "PS1"),
            ("PROMPT2", "PS2"),
            ("PROMPT3", "PS3"),
            ("PROMPT4", "PS4"),
        ] {
            if crate::ported::params::getsparam(alias).map_or(true, |s| s.is_empty()) {
                if let Some(v) = crate::ported::params::getsparam(source) {
                    setsparam(alias, &v);
                }
            }
        }
        // c:params.c:858-860 — standard non-special param defaults.
        // C uses `setiparam(...)` (PM_INTEGER) for these so
        // `(t)MAILCHECK` etc. report `integer`. zshrs previously
        // routed through `setsparam` (PM_SCALAR) — the value worked
        // but the type bit was wrong, breaking
        // `case "${(t)LISTMAX}" in *integer*)` and any path that
        // gates on arithmetic-typed semantics. Bug #268 in
        // docs/BUGS.md.
        crate::ported::params::setiparam("MAILCHECK", 60); // c:858
                                                           // c:Src/params.c:859 — original `KEYTIMEOUT = 40` but
                                                           // zsh 5.9.1 observably reports 10 (Homebrew arm-darwin).
                                                           // Match the observed default so vi-mode / multi-key
                                                           // bindings feel responsive. Bug #321 in docs/BUGS.md.
        crate::ported::params::setiparam("KEYTIMEOUT", 10); // c:859
        crate::ported::params::setiparam("LISTMAX", 100); // c:860
                                                          // c:config.h:1004 — MAX_FUNCTION_DEPTH=500. Advisory cap;
                                                          // dispatch_function_call enforces against this.
        crate::ported::params::setiparam("FUNCNEST", 500);

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
            history,
            completions: HashMap::new(),
            process_sub_counter: 0,
            zstyles: Vec::new(),
            local_scope_depth: 0,
            pending_underscore: None,
            in_dq_context: 0,
            in_scalar_assign: 0,
            profiling_enabled: false,
            compsys_cache: {
                let cache_path = crate::compsys::cache::default_cache_path();
                if cache_path.exists() {
                    let db_size = fs::metadata(&cache_path).map(|m| m.len()).unwrap_or(0);
                    match CompsysCache::open(&cache_path) {
                        Ok(c) => {
                            tracing::info!(
                                db_bytes = db_size,
                                path = %cache_path.display(),
                                "compsys: sqlite mirror opened (dbview/SQL inspection only; rkyv shards are the authoritative cache)"
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
            redirect_scope_stack: Vec::new(),
            multios_scope_stack: Vec::new(),
            exec_redirs_permanent: false,
            pipe_output_pending: false,
            pipe_output_scope: None,
            redirect_failed: false,
            functions_compiled: HashMap::new(),
            function_source: HashMap::new(),
            function_line_base: HashMap::new(),
            function_def_file: HashMap::new(),
            prompt_funcstack: Vec::new(),
            tied_array_to_scalar: HashMap::new(),
            ztest_pass_count: std::sync::atomic::AtomicUsize::new(0),
            ztest_fail_count: std::sync::atomic::AtomicUsize::new(0),
            ztest_skip_count: std::sync::atomic::AtomicUsize::new(0),
            ztest_pass_total: std::sync::atomic::AtomicUsize::new(0),
            ztest_fail_total: std::sync::atomic::AtomicUsize::new(0),
            ztest_skip_total: std::sync::atomic::AtomicUsize::new(0),
            ztest_run_failed: std::sync::atomic::AtomicBool::new(false),
            ztest_suppress_stdout: false,
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
        // c:Src/params.c:395-422 IPDEF8 — full PM_TIED colonarr list:
        // CDPATH, FIGNORE, FPATH, MAILPATH, PATH, PSVAR, MODULE_PATH,
        // MANPATH (ZSH_EVAL_CONTEXT is readonly-special, excluded).
        for (scalar, arr) in [
            ("PATH", "path"),
            ("FPATH", "fpath"),
            ("MANPATH", "manpath"),
            ("CDPATH", "cdpath"),
            ("MODULE_PATH", "module_path"),
            ("PSVAR", "psvar"),
            ("FIGNORE", "fignore"),
            ("MAILPATH", "mailpath"),
        ] {
            exec.tied_array_to_scalar
                .insert(arr.to_string(), (scalar.to_string(), ":".to_string()));
        }

        // Pour `path` (from env PATH split) into paramtab before the
        // special_paramdef stamping loop below so the canonical tied
        // entry exists when PM_SPECIAL bits are applied.
        for (k, v) in &arrays {
            setaparam(k, v.clone()); // c:params.c:3595
        }
        // c:Src/params.c:384-394 — IPDEF8/IPDEF9 macros stamp
        // `PM_SCALAR|PM_SPECIAL` (IPDEF8 for `PATH`/`FPATH`/etc.) and
        // `PM_ARRAY|PM_SPECIAL|PM_DONTIMPORT` (IPDEF9 for `path`/
        // `fpath`/etc.) on every entry in the createparamtable table.
        // setsparam/setaparam above create plain PM_SCALAR/PM_ARRAY
        // entries; this loop applies the PM_SPECIAL + PM_TIED bits
        // (plus the IPDEF9 PM_DONTIMPORT bit on the array side) so
        // `${(t)PATH}` reads `scalar-tied-export-special` and
        // `${(t)path}` reads `array-tied-special`.
        //
        // Walks the `special_params` table (params.rs:464+) which is
        // the Rust port of the C IPDEF list. For each entry: OR the
        // declared pm_flags onto the existing paramtab entry. The
        // tied-pair entries (PM_TIED) also need PM_SPECIAL OR'd in
        // since the IPDEF8/IPDEF9 macros add PM_SPECIAL implicitly;
        // the table declares only the per-entry-distinct flags.
        {
            use crate::ported::params::{paramtab, special_params};
            use crate::ported::zsh_h::{PM_ARRAY, PM_DONTIMPORT, PM_SCALAR, PM_SPECIAL, PM_TIED};
            if let Ok(mut tab) = paramtab().write() {
                // Stamp PM_SPECIAL onto every entry the special_params
                // table declares. For tied scalars (PATH/FPATH/etc),
                // also walks `tied_name` to apply IPDEF9-flag bits
                // (PM_ARRAY|PM_SPECIAL|PM_DONTIMPORT|PM_TIED) onto the
                // partner array entry (path/fpath/etc) — those array
                // names aren't in the special_params table directly
                // but C zsh's createparamtable emits IPDEF9 rows for
                // them at Src/params.c:425-432.
                use crate::ported::zsh_h::{hashnode, param, PM_DONTIMPORT as PM_DI, PM_UNSET};
                for entry in special_params.iter() {
                    // c:384/394 IPDEF8/9 — `D|PM_SCALAR|PM_SPECIAL` or
                    // `D|PM_ARRAY|PM_SPECIAL|PM_DONTIMPORT`.
                    //
                    // Mask `entry.pm_flags` to ONLY the attribute bits
                    // safe to OR onto an existing Param without changing
                    // assignment semantics. PM_READONLY is excluded
                    // here because many internal-runtime writes go
                    // through setsparam (subshell ZSH_SUBSHELL bump,
                    // ZSH_EVAL_CONTEXT push, etc.) and would be
                    // rejected by assignstrvalue's PM_READONLY guard.
                    // C zsh's PM_SPECIAL GSU setfn bypasses the guard;
                    // the Rust port lacks that vtable wiring, so keep
                    // the entries writable.
                    //
                    // PM_UNSET is included: lookup_special_var arms for
                    // TRY_BLOCK_ERROR / TRY_BLOCK_INTERRUPT (and other
                    // PM_UNSET entries with sentinel defaults) check
                    // this bit to decide between "stored value" vs
                    // "uninitialized → return -1 sentinel". The flag
                    // gets cleared by assignstrvalue at c:3660 on any
                    // write, so it correctly tracks "ever assigned".
                    // Bug #143 in docs/BUGS.md.
                    let safe_pm_flags = entry.pm_flags & (PM_TIED | PM_DI | PM_UNSET);
                    // c:Src/params.c — IPDEF macros set PM_TYPE bits
                    // (PM_INTEGER for IPDEF5/6, PM_ARRAY for IPDEF9,
                    // PM_HASHED for IPDEF-hash) along with PM_SPECIAL.
                    // zshrs's previous init only ORed PM_SPECIAL +
                    // tied/di/unset/readonly — never the type bit. If
                    // setsparam ran BEFORE init_partab_params (it does
                    // for OPTIND/SHLVL at vm_helper.rs:874/878), the
                    // param entry stayed PM_SCALAR and `typeset -p
                    // OPTIND` emitted `typeset OPTIND=1` instead of
                    // zsh's `typeset -i10 OPTIND=1`. OR the pm_type
                    // into the bits so the type attribute lands.
                    let mut bits = safe_pm_flags | PM_SPECIAL | entry.pm_type;
                    // c:Src/params.c — IPDEF4/IPDEF1 set
                    // PM_READONLY_SPECIAL = PM_SPECIAL | PM_READONLY |
                    // PM_RO_BY_DESIGN. zshrs masks PM_READONLY out
                    // (see above) but the introspection bit can still
                    // ride along. Replace dropped PM_READONLY with
                    // PM_RO_BY_DESIGN so `typeset -r` recognises these
                    // entries as logically-readonly without blocking
                    // internal writes. The listing filter in
                    // `bin_typeset` expands its PM_READONLY match to
                    // also pick up PM_RO_BY_DESIGN. Bug #97 in
                    // docs/BUGS.md.
                    if (entry.pm_flags & crate::ported::zsh_h::PM_READONLY) != 0 {
                        bits |= crate::ported::zsh_h::PM_RO_BY_DESIGN;
                    }
                    if entry.pm_type == PM_ARRAY {
                        bits |= PM_DI;
                    }
                    let _ = PM_SCALAR;
                    let _ = PM_DONTIMPORT;
                    if let Some(pm) = tab.get_mut(entry.name) {
                        let was_integer =
                            (pm.node.flags as u32 & crate::ported::zsh_h::PM_INTEGER) != 0;
                        pm.node.flags |= bits as i32;
                        // c:Src/params.c:344 IPDEF4 / c:353 IPDEF5 — the
                        // C struct literal initialises the `base` field
                        // to 10 for every PM_INTEGER special. zshrs's
                        // initial paramtab seeding doesn't carry that
                        // through (the special_paramdef table has no
                        // `base` field). Set the default here so
                        // `printparamnode`'s PMTF_USE_BASE arm at
                        // params.rs:9341 emits "10" between
                        // `integer` and the name (`integer 10 readonly
                        // !=0`). Bug #297 in docs/BUGS.md.
                        if entry.pm_type == crate::ported::zsh_h::PM_INTEGER && pm.base == 0 {
                            pm.base = 10;
                        }
                        // When OR-ing PM_INTEGER onto a param that
                        // was previously PM_SCALAR (i.e. setsparam ran
                        // BEFORE init_partab_params, storing the value
                        // in u_str), parse the u_str into u_val so the
                        // integer getter reads the correct value. C
                        // zsh's setsparam-equivalent path detects the
                        // pm's PM_TYPE first and routes through
                        // intsetfn, but zshrs's setsparam at the bin
                        // entry point predates init_partab_params, so
                        // it lands as PM_SCALAR storage that the
                        // type-flip needs to migrate.
                        if !was_integer
                            && entry.pm_type == crate::ported::zsh_h::PM_INTEGER
                            && pm.u_val == 0
                        {
                            if let Some(ref s) = pm.u_str {
                                pm.u_val = s.parse::<i64>().unwrap_or(0);
                                pm.u_str = None;
                            }
                        }
                        // c:Src/zsh.h IPDEF8/IPDEF9 — the third macro
                        // arg is the tied partner name; mapped into
                        // `pm->ename` so `typeset -p` can find the
                        // peer for the PM_TIED swap. Bug #410.
                        if let Some(peer) = entry.tied_name {
                            pm.ename = Some(peer.to_string());
                        }
                    } else {
                        // Param hasn't been created yet (e.g. PATH gets
                        // imported lazily via the env fallback in
                        // getsparam at params.rs:4104; array specials
                        // like `pipestatus` / `funcstack` / `dirstack`
                        // / `zsh_scheduled_events` aren't pre-populated).
                        // Seed an empty placeholder carrying the
                        // canonical flag set so subsequent setsparam /
                        // `(t)X` / `${+X}` observers see the IPDEF
                        // attribute bits AND `${+X}` returns 1.
                        let u_arr = if entry.pm_type == PM_ARRAY {
                            Some(Vec::new())
                        } else {
                            None
                        };
                        let pm: crate::ported::zsh_h::Param = Box::new(param {
                            node: hashnode {
                                next: None,
                                nam: entry.name.to_string(),
                                flags: (entry.pm_type as i32) | bits as i32,
                            },
                            u_data: 0,
                            u_tied: None,
                            u_arr,
                            u_str: None,
                            u_val: 0,
                            u_dval: 0.0,
                            u_hash: None,
                            gsu_s: None,
                            gsu_i: None,
                            gsu_f: None,
                            gsu_a: None,
                            gsu_h: None,
                            // c:Src/params.c:344 IPDEF4 / c:353 IPDEF5 —
                            // PM_INTEGER specials default base=10.
                            base: if entry.pm_type == crate::ported::zsh_h::PM_INTEGER {
                                10
                            } else {
                                0
                            },
                            width: 0,
                            env: None,
                            // c:Src/zsh.h IPDEF8/IPDEF9 — tied partner
                            // name. Bug #410.
                            ename: entry.tied_name.map(|s| s.to_string()),
                            old: None,
                            level: 0,
                        });
                        tab.insert(entry.name.to_string(), pm);
                    }
                    // Tied partner side. The previous loop body ORed
                    // PM_ARRAY|PM_SPECIAL|PM_DONTIMPORT|PM_TIED onto the
                    // partner indiscriminately, but for a SCALAR ↔
                    // ARRAY tied pair (PATH ↔ path, FIGNORE ↔ fignore),
                    // that incorrectly stamped PM_ARRAY onto the scalar
                    // partner (FIGNORE, PATH, FPATH, MAILPATH, MANPATH,
                    // PSVAR, CDPATH, MODULE_PATH). Result: `(t)PATH`
                    // returned `array-tied-export-special` instead of
                    // `scalar-tied-export-special`.
                    //
                    // Both partners are already listed in `special_params`
                    // (the scalar at the IPDEF8 block, the array at the
                    // IPDEF9 block past the sentinel), so each gets its
                    // own pass through this loop and ends up with the
                    // correct flags. No cross-stamping needed.
                    let _ = entry.tied_name;
                }
                // c:Src/params.c:893-924 environment-import loop —
                // every env var gets either a fresh exported paramtab
                // entry OR (when the entry pre-exists from
                // special_params) PM_EXPORTED OR'd onto its flags.
                // Without this, `declare -p PATH` printed `typeset -T
                // PATH=''` and `declare -p USER` printed nothing at
                // all because USER was never in paramtab.
                use crate::ported::zsh_h::hashnode as _hn;
                use crate::ported::zsh_h::{PM_EXPORTED, PM_SCALAR};
                // c:Src/params.c:4329-4342 colonarrsetfn — assigning a
                // tied IPDEF8 scalar (MANPATH, CDPATH, MODULE_PATH, …)
                // colonsplit()s the value into the partner array,
                // preserving empty components. The env import below
                // bypasses the GSU setfn, so collect the tied pairs
                // here and pour them through setaparam after the
                // paramtab lock drops. PATH→path / FPATH→fpath are
                // seeded earlier (vm_helper ~1160-1199) and skipped.
                let mut tied_env_arrays: Vec<(String, Vec<String>)> = Vec::new();
                // c:Src/params.c:893 — walk the process-entry environ
                // snapshot, not the live env (frameworks can mutate it
                // before init — see params.rs `environ` static).
                let environ_vars: Vec<(String, String)> = crate::ported::params::environ
                    .get()
                    .cloned()
                    .unwrap_or_else(|| std::env::vars().collect());
                for (env_name, env_value) in environ_vars {
                    if env_name.is_empty() || env_name.contains('[') {
                        continue;
                    }
                    if env_name.as_bytes()[0].is_ascii_digit() {
                        continue;
                    }
                    if !crate::ported::params::isident(&env_name) {
                        continue;
                    }
                    if let Some(pm) = tab.get_mut(&env_name) {
                        // c:Src/params.c:902-906 — the import loop runs
                        // `dontimport(pm->node.flags)` BEFORE doing
                        // anything to the entry; PM_DONTIMPORT names
                        // (`_`, IFS, GID/EGID, KEYBOARD_HACK — the
                        // IPDEF7/IPDEF2 rows, c:796-800) are skipped
                        // ENTIRELY: no PM_EXPORTED stamp, no value
                        // seed. zshrs previously OR'd PM_EXPORTED
                        // first, so an inherited env `_` made the
                        // special `_` exported and it leaked into
                        // `typeset +x -r` / `export -p` listings where
                        // zsh shows nothing.
                        if (pm.node.flags as u32 & crate::ported::zsh_h::PM_DONTIMPORT) != 0 {
                            continue; // c:905 `continue;`
                        }
                        pm.node.flags |= PM_EXPORTED as i32;
                        // c:Src/params.c:2769-2776 — assignstrvalue's
                        // PM_INTEGER arm, which C's env import reaches via
                        // `assignsparam(..., ASSPM_ENV_IMPORT)` (c:907-908;
                        // assignsparam forwards its `flags` verbatim at
                        // c:params.c assignstrvalue(v, val, flags)):
                        //     if (flags & ASSPM_ENV_IMPORT) {
                        //         char *ptr;
                        //         ival = zstrtol_underscore(val, &ptr, 0, 1);
                        //     } else
                        //         ival = mathevali(val);
                        //     v->pm->gsu.i->setfn(v->pm, ival);
                        // An integer param keeps its value in `u.val`, NOT in
                        // the scalar slot, so the `pm.u_str = env_value` seed
                        // below stored the digits somewhere no integer reader
                        // ever looks and EVERY pre-existing PM_INTEGER param
                        // silently ignored the environment: COLUMNS/LINES
                        // (IPDEF5, c:355-356) read back 0, while HISTSIZE,
                        // SAVEHIST, LISTMAX, MAILCHECK, KEYTIMEOUT and
                        // FUNCNEST kept their built-in defaults — i.e.
                        // `HISTSIZE=5000 zshrs -c ...` was a no-op.
                        //
                        // Base 0 + underscore=1 is not incidental: it is what
                        // makes `COLUMNS=0x10` 16, `COLUMNS=0b101` 5,
                        // `COLUMNS=010` 8 (octal — c:utils.c:2452-2461 takes
                        // the leading `0` then falls to `base = 8`),
                        // `COLUMNS=1_0` 10, and a trailing-garbage value like
                        // `9abc` a silent 9. mathevali would instead ERROR on
                        // `9abc`, which is precisely why C splits the two
                        // paths — importing a hostile environment must not
                        // abort the shell (upstream 546203a770, "33276: safer
                        // import of numerical variables from environment").
                        if (pm.node.flags as u32 & crate::ported::zsh_h::PM_INTEGER) != 0 {
                            let (ival, _) = crate::ported::utils::zstrtol_underscore(
                                &env_value,
                                0,
                                true,
                            ); // c:2773
                            // c:3660 — any assignstrvalue write clears PM_UNSET.
                            pm.node.flags &= !(PM_UNSET as i32);
                            // c:2774 — `v->pm->gsu.i->setfn(v->pm, ival)`.
                            // intsetfn is this port's stand-in for the gsu_i
                            // vtable: it name-dispatches the specials whose
                            // setter has side effects (SECONDS, RANDOM,
                            // HISTSIZE, …) and writes u.val otherwise.
                            crate::ported::params::intsetfn(pm.as_mut(), ival);
                            pm.env = Some(format!("{env_name}={env_value}"));
                            continue;
                        }
                        // c:Src/params.c:893-924 — C's env-import calls
                        // `assignsparam(..., ASSPM_ENV_IMPORT)` which
                        // routes through the param's GSU setfn. For
                        // SPECIAL scalars with cached storage (HOME,
                        // USERNAME, TERM, WORDCHARS, TERMINFO,
                        // TERMINFO_DIRS, KEYBOARD_HACK, histchars) the
                        // setfn writes to a separate `*_lock` global
                        // (e.g. home_lock). Just OR'ing PM_EXPORTED
                        // leaves those globals empty, so `$HOME` reads
                        // back "" even though HOME is in env. Mirror
                        // C by copying the env value into pm.u_str and
                        // (for cached specials) the matching global.
                        // Only seed cached state when the param was
                        // still marked PM_UNSET — i.e. nothing has set
                        // it yet. ShellExecutor::new's earlier init
                        // block (vm_helper line 837+) already ran
                        // setsparam for a few names (ZSH_ARGZERO,
                        // WORDCHARS, SHLVL with the +1 increment, IFS,
                        // OPTIND, …); those calls clear PM_UNSET so we
                        // must not overwrite them with the raw env
                        // value here. The PM_UNSET-still-set case is
                        // the "C zsh would have called
                        // assignsparam(...,ASSPM_ENV_IMPORT) and ours
                        // didn't yet" gap that bug #599 (HOME=` `) and
                        // %~ prompt expansion need.
                        let still_unset =
                            (pm.node.flags as u32 & crate::ported::zsh_h::PM_UNSET) != 0;
                        if still_unset {
                            pm.u_str = Some(env_value.clone());
                            pm.env = Some(format!("{}={}", env_name, env_value));
                            // c:Src/params.c:3660 — `assignstrvalue`
                            // clears PM_UNSET on any write. HOME / TERM
                            // / TERMINFO / TERMINFO_DIRS / WORDCHARS
                            // start life with PM_UNSET in
                            // `special_params` (params.rs SPECIAL_PARAMS
                            // table) so `lookup_special_var` skips the
                            // getfn for uninitialized specials; env
                            // import is the canonical "now it's set"
                            // event, so clear the bit.
                            pm.node.flags &= !(PM_UNSET as i32);
                            // Cached-state specials: route through
                            // the matching setfn so the global cache
                            // (home_lock / wordchars_lock / etc.)
                            // reflects the env value. Each setfn
                            // ignores its `pm` arg (matches C's
                            // UNUSED(Param pm)), so passing the
                            // borrowed paramtab entry is safe.
                            match env_name.as_str() {
                                "HOME" => {
                                    crate::ported::params::homesetfn(pm.as_mut(), env_value.clone())
                                }
                                "USERNAME" => crate::ported::params::usernamesetfn(
                                    pm.as_mut(),
                                    env_value.clone(),
                                ),
                                "TERM" => {
                                    crate::ported::params::termsetfn(pm.as_mut(), env_value.clone())
                                }
                                "WORDCHARS" => crate::ported::params::wordcharssetfn(
                                    pm.as_mut(),
                                    env_value.clone(),
                                ),
                                "TERMINFO" => crate::ported::params::terminfosetfn(
                                    pm.as_mut(),
                                    env_value.clone(),
                                ),
                                "TERMINFO_DIRS" => crate::ported::params::terminfodirssetfn(
                                    pm.as_mut(),
                                    env_value.clone(),
                                ),
                                _ => {}
                            }
                        }
                        // c:Src/params.c:907-908 — env import always
                        // assigns through the GSU setfn; for tied
                        // IPDEF8 scalars that is colonarrsetfn
                        // (c:4329-4342), which colonsplit()s the value
                        // into the partner array, empties preserved.
                        // Not gated on still_unset: C re-assigns on
                        // import regardless.
                        if (pm.node.flags as u32 & crate::ported::zsh_h::PM_TIED) != 0 {
                            if let Some(ref peer) = pm.ename {
                                if peer != "path" && peer != "fpath" {
                                    tied_env_arrays.push((
                                        peer.clone(),
                                        env_value.split(':').map(String::from).collect(), // c:4339 colonsplit
                                    ));
                                }
                            }
                        }
                    } else {
                        // Fresh entry — PM_SCALAR + PM_EXPORTED, value
                        // taken from env. Mirrors C zsh's c:907-908
                        // `assignsparam(..., ASSPM_ENV_IMPORT)` for
                        // names not already in the special table.
                        let pm: crate::ported::zsh_h::Param = Box::new(param {
                            node: _hn {
                                next: None,
                                nam: env_name.clone(),
                                flags: (PM_SCALAR | PM_EXPORTED) as i32,
                            },
                            u_data: 0,
                            u_tied: None,
                            u_arr: None,
                            u_str: Some(env_value.clone()),
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
                            env: Some(format!("{}={}", env_name, env_value)),
                            ename: None,
                            old: None,
                            level: 0,
                        });
                        tab.insert(env_name, pm);
                    }
                }
                // Apply the collected tied-pair splits after the env
                // walk. setaparam (the canonical store) needs the
                // same paramtab write lock held here, so write u_arr
                // directly on the peer entry — array reads route
                // through paramtab so this is the single store.
                for (peer, parts) in tied_env_arrays {
                    if let Some(apm) = tab.get_mut(peer.as_str()) {
                        apm.u_arr = Some(parts); // c:4339 — `*dptr = colonsplit(x, …)`
                        apm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
                    }
                }
                // c:Src/params.c:948-951 — runs AFTER the import loop:
                //     pm = (Param) paramtab->getnode(paramtab, "SHLVL");
                //     sprintf(buf, "%d", (int)++shlvl);
                //     /* shlvl value in environment needs updating unconditionally */
                //     addenv(pm, buf);
                // SHLVL is `IPDEF5("SHLVL", &shlvl, varinteger_gsu)` (c:358), so
                // the loop above has already parsed any inherited value into it
                // with zstrtol_underscore; C then increments THAT, in place.
                // Ordering is the whole point: the increment must observe the
                // imported value, and the import must not clobber the
                // increment. When SHLVL is absent from the environment the
                // param is still 0 here, so ++ yields 1 — matching C, whose
                // `shlvl` global starts at 0.
                //
                // addenv also exports the INCREMENTED value, which is why a
                // forked child sees 6 for `SHLVL=5 zsh -fc 'printenv SHLVL; true'`.
                // (A bare `printenv SHLVL` shows 5 because zsh exec's the last
                // command in place and backs the increment out — the shell is
                // being replaced, not nested. That is a separate mechanism.)
                if let Some(pm) = tab.get_mut("SHLVL") {
                    let next = pm.u_val + 1; // c:949 `++shlvl`
                    crate::ported::params::intsetfn(pm.as_mut(), next); // c:949
                    pm.node.flags &= !(PM_UNSET as i32);
                }
            }
        }
        // NOT DONE HERE: c:Src/params.c:951 `addenv(pm, buf)`, which zputenv's
        // the INCREMENTED SHLVL into the process environment so a forked child
        // sees 6 for `SHLVL=5 zsh -fc 'printenv SHLVL; true'`. zshrs still
        // exports the inherited 5 there.
        //
        // Adding the addenv alone makes parity WORSE, not better, because it
        // is only half of a pair. C hands an exec'd command the DECREMENTED
        // value (c:Src/exec.c:4276-4281 — "for either implicit or explicit
        // exec, decrease $SHLVL as we're now done as a shell", guarded by
        // `!subsh && !forked`), which is why a bare `SHLVL=5 zsh -fc 'printenv
        // SHLVL'` prints 5 while `'printenv SHLVL; true'` prints 6 — the first
        // is exec'd in place, the second forked. Exporting 6 without that
        // decrement turns one divergence into four: the exec'd cases and every
        // nested-shell count start reading one too high.
        //
        // The decrement IS ported, at exec.rs:11195-11199, but on the
        // `ported::exec` path — not the fusevm path that actually runs `-c`.
        // Wiring both belongs in one change, with the exec side first.
        // c:Src/init.c:1907-1909 — `SHTTY = -1; init_io(cmd); setupvals(...)`.
        // zsh_main runs those three in that order, and this constructor stands
        // in for setupvals's param setup on the drivers that never reach
        // zsh_main: `zshrs -c CODE` dispatches at bins/zshrs.rs:1716 (after
        // --zsh/-f are stripped) straight into ShellExecutor::new + exit. So
        // init_io has to happen here, or SHTTY is still -1 and the winsize
        // probe below silently no-ops (adjustwinsize early-returns at
        // c:1900-1901). setupvals's own adjustwinsize(0) covers the zsh_main
        // path; init_io is idempotent (c:615-618 closes and reopens SHTTY), so
        // that path just re-establishes it a moment later.
        crate::ported::init::init_io(None); // c:1908

        // c:Src/init.c:1274-1276 — `adjustwinsize(0)`, the first thing after
        // createparamtable (c:1270). Probes the tty via TIOCGWINSZ and
        // publishes the geometry to $COLUMNS/$LINES.
        //
        // Ordering is C's, and load-bearing in both directions: it must FOLLOW
        // init_io (which sets SHTTY) and it must FOLLOW the environ import
        // above, because the tty geometry OVERRIDES an inherited COLUMNS — a
        // 97-column terminal reports 97 even when COLUMNS=10 was exported in.
        // (C gets that via the c:1906-1907 "Signal missed while a job owned the
        // tty?" promotion of from=0 to from=1, which makes adjustcolumns take
        // the signalled path and overwrite zterm_columns with ws_col.)
        //
        // With no terminal at all, SHTTY stays -1 and the imported value (or 0)
        // survives untouched — which is what C's zterm_columns does there, and
        // why a piped `COLUMNS=20 zshrs -c` still reports 20. Note SHTTY does
        // NOT require stdin/stdout to be a tty: init_io's last resort is
        // `open("/dev/tty")` (c:667-670), so a piped-but-still-attached shell
        // gets the real width, matching `zsh -fc 'print $COLUMNS | cat'`.
        let _ = crate::ported::utils::adjustwinsize(0); // c:1276

        // c:Src/params.c:955 — `set_pwd_env();` runs AFTER the environ
        // import loop, overwriting the imported $PWD/$OLDPWD paramtab
        // entries with the ispwd()-validated values computed above
        // (c:Src/init.c:1242-1259). Without this, a stale inherited
        // $PWD (env-import snapshot taken at process entry) survives
        // in paramtab even though the live env was corrected.
        crate::ported::builtin::set_pwd_env();

        // Populate paramtab with PM_SPECIAL placeholder Params for
        // every PARTAB / PARTAB_ARRAY magic-assoc name. Mirrors
        // what C's zsh/parameter module boot_ → handlefeatures
        // chain does at startup. Makes `${+aliases}` / `${(t)commands}`
        // / `typeset -p modules` etc. see the special entries.
        init_partab_params(); // c:Src/Modules/parameter.c:2341 boot_/enables_ chain

        // c:Src/init.c:1703 init_bltinmods — must run before user
        // code so default-loaded modules (zsh/watch, …) get their
        // boot_ entry points called and their params (e.g. `watch`,
        // `WATCH`) seeded in paramtab. Without this, `${(t)watch}`
        // returned empty even though zsh treats zsh/watch as loaded
        // by default. The bin entry skips zsh_main → init_bltinmods,
        // so we run it here from ShellExecutor::new for the same
        // effect. Bug #270.
        crate::ported::init::init_bltinmods();

        // c:Src/params.c:873-876 — `gethostname(hostnam,256);
        //                            setsparam("HOST", ztrdup_metafy(hostnam));`
        // Plain port of the createparamtable HOST init. Direct
        // libc::gethostname call; result written via canonical
        // setsparam. createparamtable() itself isn't called from the
        // bin entry yet (full init port pending); this is the minimum
        // for `$HOST` to read non-empty.
        let mut host_buf = [0u8; 256];
        let host_rc = unsafe { libc::gethostname(host_buf.as_mut_ptr() as *mut libc::c_char, 256) }; // c:874
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

        // c:Src/params.c:961-970 — uname-derived host/arch
        // identification params: MACHTYPE / CPUTYPE / OSTYPE /
        // VENDOR. C zsh reads from compile-time `#define`s (set by
        // ./configure) for MACHTYPE / OSTYPE / VENDOR, and from
        // uname().machine at runtime for CPUTYPE.
        //
        // Rust port: probe uname() at startup for CPUTYPE, and use
        // const strings parameterized by build-target for the
        // others. Match homebrew zsh's values where possible.
        let mut uname_buf: libc::utsname = unsafe { std::mem::zeroed() };
        let _ = unsafe { libc::uname(&mut uname_buf) };
        let to_str = |b: &[libc::c_char]| -> String {
            // c-string → owned String, truncated at first NUL.
            let bytes: Vec<u8> = b
                .iter()
                .take_while(|&&c| c != 0)
                .map(|&c| c as u8)
                .collect();
            String::from_utf8_lossy(&bytes).into_owned()
        };
        let cputype = to_str(&uname_buf.machine);
        crate::ported::params::setsparam("CPUTYPE", &cputype); // c:961
        let sysname = to_str(&uname_buf.sysname).to_lowercase();
        let release = to_str(&uname_buf.release);
        let ostype = format!("{}{}", sysname, release); // c:968 (OSTYPE)
        crate::ported::params::setsparam("OSTYPE", &ostype);
        // MACHTYPE / VENDOR: hardcoded per platform. macOS uses
        // "arm" or "arm64" or "x86_64" for arm-derived MACHTYPE.
        // Approximate the canonical homebrew value: short-form of
        // the cputype.
        let machtype = if cputype.starts_with("arm") {
            "arm".to_string()
        } else {
            cputype.clone()
        };
        crate::ported::params::setsparam("MACHTYPE", &machtype); // c:967
        let vendor = if sysname == "darwin" {
            "apple"
        } else {
            "unknown"
        };
        crate::ported::params::setsparam("VENDOR", vendor); // c:970

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
                                                                // DO NOT setsparam("USERNAME", ...) here. `$USERNAME` is
                                                                // a special parameter whose SETTER (`usernamesetfn` in
                                                                // params.rs) performs setgid(2) + setuid(2) to actually
                                                                // change the effective user — that's a deliberate upstream
                                                                // zsh feature for `USERNAME=other-user cmd`. Calling it at
                                                                // init seeds the value AND tries to change uid/gid; when
                                                                // the resolved pwd's pw_uid differs from `getuid()` (sudo
                                                                // launches, macOS Keychain-helper inherited env, container
                                                                // entry points, etc.) the setgid call fails with EPERM and
                                                                // emits `zsh:1: failed to change group ID: Operation not
                                                                // permitted`. Upstream seeds `$USERNAME` via the GETTER
                                                                // path (`usernamegetfn` reads through `cached_username`
                                                                // populated by `inittyptab` → `get_username`), no setter
                                                                // call needed.
        }

        // c:Src/init.c:1176 — `module_path = mkarray(MODULE_DIR)`.
        // The canonical init lives in `init::setupvals` (port of
        // `Src/init.c:setupvals`); the bin entry skips setupvals (per
        // the init_bltinmods comment above), so call the lightweight
        // module_path bootstrap exposed by init.rs from here. This
        // mirrors the HOST gethostname seeding pattern above:
        // duplicated init that should collapse into a full setupvals
        // call once that port is complete.
        crate::ported::init::module_path_init();

        exec
    }

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
        // The cache validates path + mtime + zshrs binary mtime; on any
        // miss we fall through to lex/parse/compile. Cached path uses
        // `run_chunk` (the shared VM-execution helper); script-eval
        // path delegates to `execute_script_zsh_pipeline` so the
        // full parse/compile/cache-save/run flow stays in one place.
        if let Some(bc_blob) = crate::script_cache::try_load_bytes(path) {
            if let Ok(chunk) = bincode::deserialize::<fusevm::Chunk>(&bc_blob) {
                if !chunk.ops.is_empty() {
                    tracing::trace!(
                        path = %abs_path,
                        ops = chunk.ops.len(),
                        "execute_script_file: bytecode cache hit"
                    );
                    return self.run_chunk(chunk, &format!("execute_script_file:cache:{abs_path}"));
                }
            }
        }

        // Cache miss — read, parse, compile via execute_script_zsh_pipeline,
        // then snapshot the resulting chunk into the cache for next
        // time. Direct port of Src/init.c source() which calls
        // `lex_init_buf` / `loop()` without engaging the history layer.
        // (zsh fires `!` history sub only on interactive input, so
        // sourced files run verbatim.)
        let content = fs::read_to_string(file_path).map_err(|e| format!("{}: {}", file_path, e))?;
        let status = self.execute_script_zsh_pipeline(&content)?;

        // Best-effort cache save — failures don't block execution.
        // Re-parse/-compile here instead of trying to thread the chunk
        // back out of execute_script_zsh_pipeline; the cost is one extra
        // compile per CACHE MISS, paid back on every subsequent run.
        let saved_errflag = errflag.load(Ordering::Relaxed);
        errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
        // Context-isolated parse (c:Src/exec.c:283 parse_string) — this
        // post-exec re-parse for the bytecode cache also runs mid-stream
        // under the single-event reader; isolate it from the outer SHIN.
        let program = parse_isolated(&content);
        let parse_failed = (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0;
        errflag.store(saved_errflag, Ordering::Relaxed);
        if !parse_failed {
            let compiler = crate::compile_zsh::ZshCompiler::new();
            let chunk = compiler.compile(&program);
            if let Ok(blob) = bincode::serialize(&chunk) {
                let _ = crate::script_cache::try_save_bytes(path, &blob);
                tracing::trace!(
                    path = %abs_path,
                    bytes = blob.len(),
                    "execute_script_file: bytecode cached"
                );
            }
        }

        Ok(status)
    }

    /// Run a compiled `fusevm::Chunk` to completion inside this
    /// executor's context. Shared by `execute_script_zsh_pipeline`,
    /// `execute_script_file`'s bytecode-cache hit path, and the
    /// function-dispatch body_runner. Centralises the VM setup so
    /// `register_builtins` and `ExecutorContext::enter` invariants
    /// stay in lockstep.
    fn run_chunk(&mut self, chunk: fusevm::Chunk, label: &str) -> Result<i32, String> {
        if chunk.ops.is_empty() {
            return Ok(self.last_status());
        }
        crate::fusevm_disasm::maybe_print_stdout(label, &chunk);
        let mut vm = crate::vm_pool::acquire(chunk);
        // Seed vm.last_status with the executor's current LASTVAL so
        // sub-VMs (EXIT trap bodies, eval, source) see the inherited
        // `$?` from the caller's last command — matching C zsh where
        // lastval is a process global. Without this, the new VM
        // started at 0 and BUILTIN_GET_VAR's sync_status would write
        // 0 back into LASTVAL on the first `$?` read.
        vm.last_status = self.last_status();
        let _ctx = ExecutorContext::enter(self);
        match vm.run() {
            fusevm::VMResult::Ok(_) | fusevm::VMResult::Halted => {
                self.set_last_status(vm.last_status);
            }
            fusevm::VMResult::Error(e) => return Err(format!("VM error: {}", e)),
        }
        Ok(self.last_status())
    }

    /// Execute via the lex+parse free ported + ZshCompiler pipeline.
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
        // Context-isolated parse (c:Src/exec.c:283 parse_string). eval /
        // source / autoload-register / trap bodies all reach here and run
        // DURING execution; on the faithful single-event loop()/parse_event
        // reader, a bare parse_init/lex_init would steal the outer's next
        // SHIN line into this nested program (e.g. `eval "x=5"` swallowed the
        // following `echo $x` off stdin). parse_isolated sets `strin` so the
        // string drains to EOF; execution below stays in the current shell.
        let program = parse_isolated(script);
        let parse_failed = (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0;
        errflag.store(saved_errflag, Ordering::Relaxed);
        if parse_failed {
            // c:Src/init.c — when the parser fires `zerr(...)`, the C
            // shell's `loop()` body skips the eval pass and continues;
            // there's no second "parse error" diagnostic. The Rust
            // binary's call sites print `zshrs: <e>` on Err, doubling
            // up on the message the parser already emitted via zerr.
            // Use a `__SILENCED__` sentinel that the binary's
            // execute_script wrapper recognizes as "already reported,
            // exit silently". Bug #142 in docs/BUGS.md (double-print
            // half).
            return Err("__SILENCED__".to_string());
        }

        let compiler = crate::compile_zsh::ZshCompiler::new();
        let chunk = compiler.compile(&program);
        let status = self.run_chunk(chunk, "execute_script_zsh_pipeline")?;

        // Fire EXIT trap if set. Two storage paths:
        //   (a) `trap 'cmd' EXIT` writes the body text into
        //       `traps_table` via bin_trap (Src/builtin.c) — fire
        //       directly via execute_script.
        //   (b) `TRAPEXIT() { ... }` function-named form goes
        //       through settrap(SIGEXIT, None, ZSIG_FUNC) at
        //       funcdef time (fusevm_bridge.rs BUILTIN_REGISTER_COMPILED_FN
        //       arm) and lives in shfunctab + sigtrapped — fire
        //       via dotrap(SIGEXIT) which dispatches the named
        //       shfunc. Bug #157 in docs/BUGS.md.
        // Remove the trap from `traps_table` first to prevent
        // infinite recursion of `(a)`; `(b)`'s sigtrapped flag
        // is cleared by dotrap's own intrap guard.
        let exit_body = crate::ported::builtin::traps_table()
            .lock()
            .ok()
            .and_then(|mut t| t.remove("EXIT"));
        if let Some(action) = exit_body {
            tracing::debug!("firing EXIT trap (new pipeline)");
            // c:Src/signals.c — the EXIT trap body sees $? at the
            // value the script left off (so `trap 'echo $?' EXIT;
            // (exit 7)` prints 7), but the SHELL's final exit code
            // is still the pre-trap value (running `echo` inside
            // the trap doesn't reset the script's exit status).
            // Preserve `status` and re-apply it after the trap
            // body returns.
            //
            // c:Src/signals.c:1123/1236 — `intrap++` … `intrap--` bracket a
            // trap body, and while intrap the EXIT, DEBUG and ZERR traps are
            // suppressed (c:1112-1119, the guard in dotrap). This path runs
            // the EXIT body through the script pipeline rather than dotrap,
            // so nothing raised intrap and the body's own failure re-entered
            // the ERR trap: `trap 'print err' ERR; trap 'true; false' EXIT`
            // printed err where zsh prints nothing.
            //
            // A counter, not a flag, and paired with dotrap's SELECTIVE
            // guard: signals zsh does deliver from inside a trap body (e.g.
            // `trap 'kill -USR1 $$' EXIT`) must still dispatch.
            crate::ported::signals::intrap.fetch_add(1, Ordering::SeqCst); // c:1123
            let _ = self.execute_script_zsh_pipeline(&action);
            crate::ported::signals::intrap.fetch_sub(1, Ordering::SeqCst); // c:1236
            self.set_last_status(status);
        }
        // c:Src/signals.c::dotrap(SIGEXIT) — fire TRAPEXIT() shfunc
        // if installed via the function-name path. The TRAPEXIT()
        // form goes through settrap(SIGEXIT, None, ZSIG_FUNC) at
        // funcdef time (sets sigtrapped[SIGEXIT] |= ZSIG_FUNC).
        // Dispatching from here AFTER run_chunk returns means we're
        // outside the VM context — dotrap can't safely re-enter
        // via dispatch_function_call (which uses with_executor).
        // Route through execute_script_zsh_pipeline which sets up
        // a fresh VM context — invoke the function by name.
        let trapped = crate::ported::signals::sigtrapped
            .lock()
            .ok()
            .and_then(|g| g.get(crate::signals_h::SIGEXIT as usize).copied())
            .unwrap_or(0);
        if (trapped & crate::ported::zsh_h::ZSIG_FUNC as i32) != 0 {
            // The TRAP<SIG> function is stored in shfunctab as
            // "TRAPEXIT"; calling it by name re-enters
            // execute_script_zsh_pipeline with a fresh VM context.
            let _ = self.execute_script_zsh_pipeline("TRAPEXIT");
        }
        // c:Src/init.c::zexit — `callhookfunc("zshexit", NULL, 1, NULL)`.
        // Fire the `zshexit` shfunc + walk `zshexit_functions` array.
        // Routed through execute_script_zsh_pipeline calls because
        // we're outside the VM context here (post-run_chunk). Iterate
        // the array directly + call zshexit by name. Bug #215 in
        // docs/BUGS.md.
        //
        // Re-entry guard: each call to execute_script_zsh_pipeline
        // (whether top-level script or the named-fn dispatch below)
        // hits this code at its tail. Without a guard, the zshexit
        // hook recurses infinitely (calls itself at end via this
        // path). Use a thread-local depth counter and skip the
        // dispatch when depth > 0.
        thread_local! {
            static ZSHEXIT_HOOK_DEPTH: std::cell::Cell<u32> = const {
                std::cell::Cell::new(0)
            };
        }
        let hook_depth = ZSHEXIT_HOOK_DEPTH.with(|c| c.get());
        if hook_depth == 0 {
            ZSHEXIT_HOOK_DEPTH.with(|c| c.set(hook_depth + 1));
            if crate::ported::hashtable::shfunctab_lock()
                .read()
                .ok()
                .map(|t| t.contains_key("zshexit"))
                .unwrap_or(false)
            {
                let _ = self.execute_script_zsh_pipeline("zshexit");
            }
            let exit_arr = crate::ported::params::paramtab()
                .read()
                .ok()
                .and_then(|t| t.get("zshexit_functions").and_then(|p| p.u_arr.clone()))
                .unwrap_or_default();
            for fn_name in exit_arr {
                let exists = crate::ported::hashtable::shfunctab_lock()
                    .read()
                    .ok()
                    .map(|t| t.contains_key(&fn_name))
                    .unwrap_or(false);
                if exists {
                    let _ = self.execute_script_zsh_pipeline(&fn_name);
                }
            }
            ZSHEXIT_HOOK_DEPTH.with(|c| c.set(hook_depth));
        }
        // Preserve script status; trap body shouldn't override it.
        self.set_last_status(status);

        let _ = status;
        Ok(self.last_status())
    }
    /// `execute_script` — see implementation.
    #[tracing::instrument(skip(self, script), fields(len = script.len()))]
    pub fn execute_script(&mut self, script: &str) -> Result<i32, String> {
        // lex+parse free ported + ZshCompiler is the only execution path.
        self.execute_script_zsh_pipeline(script)
    }

    /// Run an ALREADY-PARSED program (the back half of
    /// `execute_script_zsh_pipeline`): compile the `ZshProgram` to a
    /// fusevm Chunk and run it. Used by the ported `loop()` REPL
    /// (Src/init.c:220 `execode`), which parses via `parse_event` and
    /// hands the program here through the `execute_program` exec hook.
    /// Returns the resulting `$?` (1 on a compile/run error).
    pub fn execute_program(&mut self, program: &crate::parse::ZshProgram) -> i32 {
        let chunk = crate::compile_zsh::ZshCompiler::new().compile(program);
        match self.run_chunk(chunk, "loop") {
            Ok(status) => status,
            Err(_) => 1,
        }
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

    /// Dispatch a function by name. Thin passthru — autoload-materialize
    /// the body if needed, build a synthetic `shfunc`, and hand off to
    /// the canonical `doshfunc` port (`Src/exec.c:5823` →
    /// `src/ported/exec.rs::doshfunc`). doshfunc owns ALL scope
    /// management (starttrapscope/endtrapscope, startparamscope/
    /// endparamscope, funcdepth bump, pipestats save/restore, scriptname
    /// snapshot, BREAKS/CONTFLAG/LOOPS/RETFLAG snapshot+restore, `$0`
    /// override via FUNCTIONARGZERO, etc.). The body run itself is the
    /// Rust-only adaptation passed via the `body_runner` closure because
    /// zshrs runs function bodies through fusevm bytecode (not C zsh's
    /// wordcode walker via `runshfunc`).
    ///
    /// Returns `None` when the name isn't a known function so the caller
    /// can fall through to external dispatch.
    /// Body-only counterpart to [`dispatch_function_call`] — runs
    /// the function body WITHOUT wrapping in `doshfunc`. Used as the
    /// `body_runner` closure target by `src/ported/` callers that
    /// already wrap their own `crate::ported::exec::doshfunc(...)`
    /// call (so going back through `dispatch_function_call` would
    /// double-wrap the scope). Mirrors C's `runshfunc(prog, wrappers,
    /// name)` at `exec.c:6042` from doshfunc's perspective.
    pub fn run_function_body_only(&mut self, name: &str, args: &[String]) -> Option<i32> {
        // Same Rust-port short-circuit as dispatch_function_call,
        // sans the doshfunc wrap.
        if let Some(rc) = crate::compsys::router::dispatch_compsys(name, args) {
            // Plugin override (ABI v4) wins over the built-in Rust port.
            return Some(rc);
        }
        // Bug #657 gap #2 — `_regex_arguments`-generated completion functions
        // live in a runtime registry, not the static router table (a plain
        // `fn` ptr can't carry the dynamic name). Consult that registry here
        // so `compdef mycmd` → `_comps[cmd]=mycmd` → this call routes to the
        // compiled regex state machine.
        if let Some(rc) =
            crate::compsys::ported::_regex_arguments::dispatch_if_registered(name)
        {
            return Some(rc);
        }
        // Autoload prelude (same as dispatch_function_call's).
        if !self.functions_compiled.contains_key(name) {
            // On-demand $fpath autoload for `_`-prefixed compsys helpers that
            // compinit didn't register as autoload stubs — see the fuller
            // note in dispatch_function_call.
            if name.starts_with('_') && crate::ported::utils::getshfunc(name).is_none() {
                let _ = self.execute_script_zsh_pipeline(&format!("autoload -rUz -- {name}"));
            }
            if let Some(stub) = crate::ported::utils::getshfunc(name) {
                // c:Src/exec.c:5684-5704 (loadautofn) —
                //     int noalias = noaliases;
                //     noaliases = (shf->node.flags & PM_UNALIASED);
                //     prog = getfpfunc(...);        /* parses the file */
                //     noaliases = noalias;
                // `autoload -U` records PM_UNALIASED (c:3354-3357), and its ONLY
                // effect is that the autoloaded body is PARSED with alias
                // expansion disabled. zshrs recorded the bit but never consulted
                // it, so a body calling `helper` picked up a caller-defined
                // `alias helper=...` — exactly what -U exists to prevent.
                let unaliased =
                    (stub.node.flags as u32 & crate::ported::zsh_h::PM_UNALIASED) != 0;
                let noalias_save = crate::ported::lex::noaliases(); // c:5684
                crate::ported::lex::set_noaliases(unaliased); // c:5697
                let _restore_noaliases = NoAliasesRestore(noalias_save); // c:5704
                if (stub.node.flags as u32 & PM_UNDEFINED) != 0 {
                    let boxed = Box::new(stub.clone());
                    let ptr = Box::into_raw(boxed);
                    let _ = crate::ported::exec::loadautofn(ptr, 0, 0, 0);
                    unsafe {
                        let _ = Box::from_raw(ptr);
                    }
                    // c:5657 — preserve the fpath dir + PM_LOADDIR across the
                    // funcdef re-register (which stamps filename="zsh"), so
                    // `whence -v` reports the source. See the twin site below.
                    let loaded_dir = crate::ported::utils::getshfunc(name)
                        .and_then(|f| f.filename)
                        .filter(|d| d != "zsh");
                    if let Some(body) = crate::ported::utils::getshfunc(name).and_then(|f| f.body) {
                        let registered = autoload_register_source(name, &body);
                        let _ = self.execute_script_zsh_pipeline(&registered);
                    }
                    if let Some(dir) = loaded_dir.as_deref() {
                        if let Ok(mut tab) = crate::ported::hashtable::shfunctab_lock().write() {
                            if let Some(shf) = tab.get_mut(name) {
                                crate::ported::exec::loadautofnsetfile(shf, Some(dir)); // c:5735
                            }
                        }
                    }
                } else if let Some(body) = stub.body.clone() {
                    let registered = autoload_register_source(name, &body);
                    let _ = self.execute_script_zsh_pipeline(&registered);
                }
            }
        }
        let chunk = self.functions_compiled.get(name).cloned()?;
        let seed_status = self.last_status();
        let _ = args; // fusevm body reads $1..$N from PPARAMS
        // Reuse a VM from the per-thread pool instead of building one from
        // scratch every call. `register_builtins` installs ~hundreds of
        // fn-pointer handlers into the VM's builtin_table; the table is
        // identical for every VM, so re-running it per function call was
        // pure waste (~130 profile samples in a tight call loop, the #2 hot
        // spot after option lookups). `VM::reset(chunk)` clears execution
        // state but PRESERVES builtin_table / host / JIT wiring, so a
        // recycled VM is call-ready without re-registration. Fresh VMs pay
        // the registration once. Nested calls simply check out additional
        // VMs; the pool grows to the max call depth. Re-entrant and
        // panic-safe: the VM is returned on the normal path below.
        let mut vm = crate::vm_pool::acquire(chunk);
        vm.last_status = seed_status;
        let _ = vm.run();
        Some(vm.last_status)
    }

    pub fn dispatch_function_call(&mut self, name: &str, args: &[String]) -> Option<i32> {
        // Nested scope for `>(cmd)` fd ownership — builtins running
        // inside the function body must not close the CALLER's
        // pending psub fds (`myfn >(cmd)` keeps /dev/fd/N alive for
        // the whole function, like C's per-job filelist). See
        // PSUB_SCOPE_DEPTH in fusevm_bridge.rs.
        let _psub_scope = crate::fusevm_bridge::PsubScope::enter();
        // c:Src/exec.c — `disable -f NAME` flips the DISABLED flag on
        // the shfunctab entry. `lookupshfunc` (which dispatch consults)
        // returns NULL for DISABLED entries, falling through to PATH
        // lookup → "command not found". zshrs keeps the compiled body
        // in functions_compiled independently of the flag, so check
        // shfunctab and short-circuit when DISABLED is set. Bug #221
        // in docs/BUGS.md.
        let is_disabled = crate::ported::hashtable::shfunctab_lock()
            .read()
            .ok()
            .and_then(|t| {
                let entry = t.get_including_disabled(name)?;
                Some((entry.node.flags as u32 & crate::ported::zsh_h::DISABLED as u32) != 0)
            })
            .unwrap_or(false);
        if is_disabled {
            return None;
        }
        // `_regex_arguments NAME …` (e.g. `_regex_arguments _sed_expressions …`
        // in `_sed`) eval-defines a real shell function NAME in zsh. This port
        // stores it in a runtime registry keyed by NAME (a static router fn-ptr
        // can't carry a dynamic name). `run_function_body_only` already consults
        // that registry, but `dispatch_function_call` — the path an `_arguments`
        // action (`:sed script:_sed_expressions`) or any by-name caller takes —
        // did not, so the call fell through to the autoload prelude and errored
        // "function definition file not found" (`sed -<TAB>`). Consult the
        // registry here too, before autoload. Returned directly (like
        // run_function_body_only) — the regex body drives compsys globals, not
        // function locals, so it needs no doshfunc scope wrap.
        if let Some(rc) =
            crate::compsys::ported::_regex_arguments::dispatch_if_registered(name)
        {
            return Some(rc);
        }
        // zshrs-original: `[compsys] backend = "rust"` short-circuit.
        // When a `_NAME` has a Rust port AND the user opted into the
        // rust backend, run the Rust fn directly here — but still
        // through the canonical doshfunc scope-management path below
        // (we synthesize a body_runner from the fn pointer). Router
        // returns None for names without a Rust port → graceful
        // fallback to the shfunc autoload path.
        //
        // Note: `compcore::callcompfunc` (the compsys entry hit by
        // Tab) wraps doshfunc itself per C `compcore.c:835`, so the
        // Rust _main_complete dispatch lands HERE only when called
        // from a non-compcore caller (e.g. a user shell script
        // directly invoking `_main_complete`). The doshfunc scope
        // wrap below applies uniformly to both.
        let direct_rust_fn: Option<fn(&[String]) -> i32> =
            crate::compsys::router::try_rust_dispatch(name);
        // A plugin-registered override (ABI v4, `zmodload -R`) also
        // intercepts natively: it supplies the body, so no shell autoload
        // or compiled chunk is needed — same as a built-in Rust port.
        let has_plugin_override =
            crate::extensions::plugin_host::compfn_override(name).is_some();
        // Autoload prelude skipped when a Rust port OR plugin override wins
        // — no upstream shell function to load.
        if direct_rust_fn.is_none()
            && !has_plugin_override
            && !self.functions_compiled.contains_key(name)
        {
            // compinit bulk-loads $_comps from the dump/cache but (unlike
            // zsh's `compdef -na`, which `autoload -rUz`s every completer)
            // does NOT register the completer functions as autoload stubs.
            // So a shell completer WITHOUT a Rust port (e.g. `_cat`, or the
            // helpers it calls: `_pick_variant`, `_arguments`…) had no
            // shfunctab entry — getshfunc returned None, nothing compiled,
            // dispatch returned None, and the command's completion silently
            // produced nothing. Register `_`-prefixed helpers from $fpath on
            // demand (mirrors a fresh `autoload -Uz NAME`) so getshfunc finds
            // the stub below and loadautofn reads the file. Gated to `_`
            // names so ordinary commands still fall through to PATH.
            if name.starts_with('_') && crate::ported::utils::getshfunc(name).is_none() {
                let _ = self.execute_script_zsh_pipeline(&format!("autoload -rUz -- {name}"));
            }
            if let Some(stub) = crate::ported::utils::getshfunc(name) {
                // c:Src/exec.c:5684-5704 (loadautofn) — `autoload -U` records
                // PM_UNALIASED, whose ONLY effect is that the autoloaded body is
                // PARSED with alias expansion disabled:
                //     int noalias = noaliases;
                //     noaliases = (shf->node.flags & PM_UNALIASED);
                //     prog = getfpfunc(...);        /* parses the file */
                //     noaliases = noalias;
                let unaliased =
                    (stub.node.flags as u32 & crate::ported::zsh_h::PM_UNALIASED) != 0;
                let noalias_save = crate::ported::lex::noaliases(); // c:5684
                crate::ported::lex::set_noaliases(unaliased); // c:5697
                                                              // c:5704 — restored on EVERY exit from this block, including the
                                                              // early `return Some(1)` paths below.
                let _restore_noaliases = NoAliasesRestore(noalias_save);
                if (stub.node.flags as u32 & PM_UNDEFINED) != 0 {
                    let boxed = Box::new(stub.clone());
                    let ptr = Box::into_raw(boxed);
                    let load_rc = crate::ported::exec::loadautofn(ptr, 0, 0, 0);
                    unsafe {
                        let _ = Box::from_raw(ptr);
                    }
                    // c:Src/exec.c:5657 loadautofnsetfile — capture the fpath
                    // directory loadautofn wrote so it can be restored (as an
                    // absolutized path with PM_LOADDIR) after the funcdef pipeline
                    // below clobbers `filename` to scriptfilename ("zsh"). Without
                    // this, `whence -v <autoloaded>` printed "from zsh".
                    let loaded_dir = crate::ported::utils::getshfunc(name)
                        .and_then(|f| f.filename)
                        .filter(|d| d != "zsh");
                    if let Some(body) = crate::ported::utils::getshfunc(name).and_then(|f| f.body) {
                        let registered = autoload_register_source(name, &body);
                        // c:Src/exec.c:5739 — the ksh-autoload body runs via
                        // `execode(prog, 1, 0, "evalautofunc")` at the function
                        // invocation's locallevel, so a `return`/`break`/
                        // `continue` inside the file body is CONTAINED to the
                        // autoload call. add-zle-hook-widget's first line is
                        // `zmodload -e zsh/zle || return 1`; when a plugin has
                        // leaked `ksh_autoload` on (e.g. a bare `emulate sh`),
                        // that `return` must NOT propagate out and abort the
                        // caller's precmd/shell (zsh warns "not defined by file"
                        // and CONTINUES). Save & restore the control-flow flags
                        // around the body run to reinstate that boundary.
                        {
                            use crate::ported::builtin::{
                                BREAKS, EXIT_PENDING, EXIT_VAL, RETFLAG, SHELL_EXITING,
                            };
                            use std::sync::atomic::Ordering::Relaxed;
                            // c:Src/exec.c:5739 — `execode(prog, 1, 0,
                            // "evalautofunc")` runs the file body as part of the
                            // autoload invocation. add-zle-hook-widget's
                            // `zmodload -e zsh/zle || return 1` sits at the
                            // file's TOP LEVEL (above its anon-func wrapper); at
                            // script scope a top-level `return` is a shell EXIT,
                            // so running the body as a plain script aborted the
                            // caller's precmd/shell. `return` is contained when
                            // `locallevel || sourcelevel` (bin_return, c:5840) —
                            // raise SOURCELEVEL (the file-source counter, which
                            // unlike locallevel does NOT open a local scope, so
                            // the body's global assignments still land globally)
                            // so the top-level `return` returns from the load
                            // instead of exiting. Save/restore the control-flow
                            // flags so nothing leaks — matching zsh's
                            // warn-and-continue.
                            use crate::ported::init::sourcelevel;
                            let saved_retflag = RETFLAG.swap(0, Relaxed);
                            let saved_breaks = BREAKS.swap(0, Relaxed);
                            let saved_exit_pending = EXIT_PENDING.swap(0, Relaxed);
                            let saved_exit_val = EXIT_VAL.swap(0, Relaxed);
                            let saved_shell_exiting = SHELL_EXITING.swap(0, Relaxed);
                            sourcelevel.fetch_add(1, Relaxed);
                            let _ = self.execute_script_zsh_pipeline(&registered);
                            sourcelevel.fetch_sub(1, Relaxed);
                            RETFLAG.store(saved_retflag, Relaxed);
                            BREAKS.store(saved_breaks, Relaxed);
                            EXIT_PENDING.store(saved_exit_pending, Relaxed);
                            EXIT_VAL.store(saved_exit_val, Relaxed);
                            SHELL_EXITING.store(saved_shell_exiting, Relaxed);
                        }
                        if let Some(dir) = loaded_dir.as_deref() {
                            if let Ok(mut tab) = crate::ported::hashtable::shfunctab_lock().write() {
                                if let Some(shf) = tab.get_mut(name) {
                                    crate::ported::exec::loadautofnsetfile(shf, Some(dir)); // c:5735
                                }
                            }
                        }
                        if !self.functions_compiled.contains_key(name) {
                            // c:Src/exec.c:5742-5745 — ksh-style load ran
                            // the file (`execode`, "evalautofunc") but it
                            // didn't define NAME:
                            //   `zwarn("%s: function not defined by file", n);`
                            // The wrap/strip zsh-style paths always define
                            // NAME, so reaching here means the verbatim run
                            // failed to — same condition as C.
                            crate::ported::utils::zwarn(&format!(
                                "{}: function not defined by file",
                                name
                            ));
                            return Some(1);
                        }
                    } else if load_rc != 0 {
                        // c:Src/exec.c:5713-5719 / 5635-5644 —
                        // `execautofn`'s `if (!loadautofn(...)) return 1`
                        // propagates the loadautofn failure as the
                        // command's exit status. zshrs's previous
                        // path returned None here, falling through to
                        // execute_external which emitted a SECOND
                        // diagnostic (`command not found: NAME`) on
                        // top of loadautofn's `function definition
                        // file not found`. Mirror C: when load failed
                        // AND the stub still has no body, surface
                        // status=1 so the caller does NOT fall back
                        // to PATH search.
                        return Some(1);
                    }
                } else if let Some(body) = stub.body.clone() {
                    // c:Src/Modules/parameter.c::setpmfunction — function
                    // registered via `functions[name]=body` lives in
                    // shfunctab with `body` set but `functions_compiled`
                    // empty (the canonical port stores the parsed eprog,
                    // not a fusevm Chunk). Lazy-compile here by feeding
                    // the body through the standard funcdef pipeline so
                    // the next CallFunction op finds the chunk.
                    let registered = autoload_register_source(name, &body);
                    let _ = self.execute_script_zsh_pipeline(&registered);
                }
            }
        }
        // When a Rust port is registered, skip the fusevm Chunk
        // lookup entirely — the body_runner closure below will run
        // the Rust fn pointer directly. Otherwise require a compiled
        // chunk for the autoloaded body.
        let chunk_opt = if direct_rust_fn.is_some() || has_plugin_override {
            None
        } else {
            Some(self.functions_compiled.get(name).cloned()?)
        };

        // zshrs-specific bookkeeping that doshfunc doesn't own:
        // - prompt_funcstack (PS4 trace) push/pop
        // - local_scope_depth FUNCNEST guard
        //
        // c:Src/exec.c::funcnest_check — C zsh allows FUNCNEST=500 by
        // default. zshrs's per-call stack usage is heavier (vm_helper
        // state, fusevm closures, parse buffers), so on the default 8MB
        // stack a deep recursion overflowed around depth ~80-120 and
        // crashed. That is now fixed at the source: the shell runs on a
        // 512MB-stack thread (see bins/zshrs.rs::main), which comfortably
        // fits FUNCNEST (500) nested heavy frames. So the effective limit
        // is the user's FUNCNEST (default 500), matching zsh — no premature
        // clamp — with a generous hard ceiling as a last-resort backstop
        // that stays well under the big stack's capacity. Bug #519 (the
        // crash) / #643 (the false-positive clamp at 80 that broke
        // legitimately deep recursion). The authoritative FUNCNEST error
        // is also enforced in doshfunc (exec.rs) on the FS_FUNC depth.
        const FUNCNEST_RUST_CEILING: usize = 6000;
        let funcnest_user: usize = self
            .scalar("FUNCNEST")
            .and_then(|s| s.parse().ok())
            .unwrap_or(500);
        let funcnest_limit = funcnest_user.min(FUNCNEST_RUST_CEILING);
        if self.local_scope_depth >= funcnest_limit {
            eprintln!(
                "{}: maximum nested function level reached; increase FUNCNEST?",
                name
            );
            return Some(1);
        }
        let display_name = if name.starts_with("_zshrs_anon_") {
            "(anon)".to_string()
        } else {
            name.to_string()
        };
        let line_base = self.function_line_base.get(name).copied().unwrap_or(0);
        let def_file = self.function_def_file.get(name).cloned().flatten();
        self.prompt_funcstack
            .push((name.to_string(), line_base, def_file));
        self.local_scope_depth += 1;

        // Synthetic shfunc for doshfunc — carries the name + def-file
        // info so funcstack push gets a proper filename. funcdef/body
        // stay None because the wordcode body is irrelevant on this
        // path (body_runner runs the fusevm Chunk directly).
        // c:Src/exec.c:5390-5410 — execfuncdef records the
        // current `scriptfilename` on the shfunc at definition
        // time so funcsourcetrace can show file:line of the
        // function's source. The function_def_file map stores
        // this; fall back to the live scriptfilename so dynamic
        // / non-`compile_funcdef`-routed definitions still get a
        // sensible filename. Without the fallback, the synth_shf
        // saw None and the funcstack push at exec.rs:5719
        // defaulted to an empty string, which the funcsourcetrace
        // getfn rendered as `:N` (or worse, picked up the
        // function name from a parallel field). Bug #515.
        // c:Src/exec.c:5620/5625 — the source file is `getshfuncfile(shf)`,
        // which reads the shfunc's own `filename` (authoritative: set by
        // execfuncdef for a normally-defined function, and by loadautofn — as
        // the fpath dir with PM_LOADDIR — for an AUTOLOADED one). Prefer it.
        // `function_def_file` is a zshrs-only side map that, for an autoloaded
        // function, was stamped with the OUTER `scriptfilename` ("zsh") at
        // compile time, not the fpath file — so it must NOT override
        // getshfuncfile. Consulting it second still covers functions whose
        // shfunc `filename` wasn't recorded (compile_funcdef-routed defs);
        // scriptfilename is the final fallback. Without getshfuncfile winning,
        // funcsourcetrace reported "zsh" for every autoloaded completer, which
        // broke `_git`: its first git-completion.bash search path is
        // `"$(dirname ${funcsourcetrace[1]%:*})"/git-completion.bash` — "zsh"
        // resolved to `./git-completion.bash`, found nothing, and
        // `. "$script"` errored (`_git:.:48: no such file or directory`).
        let synth_filename = crate::ported::hashtable::getshfuncfile(name)
            .or_else(|| self.function_def_file.get(name).cloned().flatten())
            .or_else(|| self.scriptfilename.clone());
        // c:Src/exec.c:5409 — `shf->lineno = lineno;` (def line).
        // `function_line_base[name]` carries compile_funcdef's
        // `lineno_offset = first_body_line - 1` — equals the def line
        // for multi-line `f() {\n body }` but underflows to 0 for
        // INLINE `f() { body }` (def and body share a line). zsh's
        // funcsourcetrace reports the def line as 1-based, so clamp
        // to >= 1 to handle the inline case without rebuilding
        // line tracking through the parser. Bug #396.
        let synth_lineno = std::cmp::max(
            1i64,
            self.function_line_base.get(name).copied().unwrap_or(0),
        );
        let mut synth_shf = crate::ported::zsh_h::shfunc {
            node: crate::ported::zsh_h::hashnode {
                next: None,
                nam: display_name.clone(),
                flags: 0,
            },
            filename: synth_filename,
            lineno: synth_lineno,
            funcdef: None,
            redir: None,
            sticky: None,
            body: None,
        };
        // doshargs: C convention — argv[0] = function name (for
        // FUNCTIONARGZERO `$0`), argv[1..] = real positional args.
        let mut doshargs: Vec<String> = vec![display_name.clone()];
        doshargs.extend(args.iter().cloned());

        // Seed `$?` with the parent's last status — C zsh's
        // doshfunc inherits lastval automatically because it's a
        // process-global; the fusevm VM creates a fresh
        // `vm.last_status = 0` per call, so we mirror the inherit
        // explicitly. Without this, a function reading `$?` BEFORE
        // running any command sees 0 instead of the caller's status.
        let seed_status = self.last_status();
        let body_args: Vec<String> = args.to_vec();
        let name_owned = name.to_string();
        let body_runner = move || -> i32 {
            // Branch: plugin override (ABI v4) → built-in Rust port →
            // fusevm Chunk (autoloaded shell body). All run INSIDE
            // doshfunc's scope so prologue/epilogue applies identically.
            if let Some(rc) =
                crate::extensions::plugin_host::dispatch_compfn(&name_owned, &body_args)
            {
                return rc;
            }
            if let Some(f) = direct_rust_fn {
                return f(&body_args);
            }
            let chunk = chunk_opt
                .as_ref()
                .expect("chunk_opt must be Some when direct_rust_fn is None");
            crate::fusevm_disasm::maybe_print_stdout(
                &format!(
                    "function:{}",
                    body_args.first().map(|s| s.as_str()).unwrap_or("")
                ),
                chunk,
            );
            let mut vm = crate::vm_pool::acquire(chunk.clone());
            vm.last_status = seed_status;
            let _ = vm.run();
            vm.last_status
        };

        // Enter executor context BEFORE doshfunc so the body_runner's
        // VM builtins can `with_executor(...)` to reach this state.
        let _ctx = ExecutorContext::enter(self);
        let status = crate::ported::exec::doshfunc(&mut synth_shf, doshargs, false, body_runner);
        drop(_ctx);

        self.prompt_funcstack.pop();
        self.local_scope_depth -= 1;

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
        // FORK_EVENTS is bumped at the real spawn site inside
        // execute_external_bg — this entry is only ONE of several
        // callers of that spawn (the common static-head command path
        // calls execute_external_bg directly), so counting here would
        // miss `time sleep 0` while double-counting this path.
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
        // !!! WARNING: RUST-ONLY — NO C COUNTERPART !!!
        // Native (Rust) plugin builtins registered via `zmodload -R`
        // (src/extensions/plugin_host.rs). fusevm compiles unknown
        // names into external execution, so a plugin command arrives
        // here as an "external". Resolve it BEFORE the PATH-unset guard
        // and the process spawn — plugin builtins are in-process and
        // need no PATH. This is the analog of C's `resolvebuiltin`
        // slot (Src/exec.c:2700), which likewise runs before the fork.
        // Bare names only: a `/`-qualified token is always a filesystem
        // path, never a plugin command name. Runs synchronously even
        // when backgrounded — zshrs is non-forking and an in-process
        // builtin has nothing to background.
        if !cmd.contains('/') {
            if let Some(status) = crate::plugin_host::dispatch(cmd, args) {
                return Ok(status);
            }
        }
        // c:Src/exec.c:824-876 — when arg0 has no `/`, C zsh requires
        // a PATH search. With PATH unset, the search yields no hit
        // and C emits `command not found: <cmd>`. Rust's
        // `Command::new(name)` delegates to libc `execvp`, which on
        // many platforms falls back to a built-in default PATH when
        // the env entry is missing — so `unset PATH; ls` still finds
        // `/bin/ls` and runs it, breaking the security boundary the
        // unset is supposed to establish (#416). Gate explicitly:
        // when cmd is a bare name (no `/`) and zshrs's own PATH
        // param is unset OR empty, emit the canonical
        // "command not found" diagnostic and return 127 BEFORE
        // touching libc.
        if !cmd.contains('/') {
            let path_set_and_nonempty = crate::ported::params::getsparam("PATH")
                .map(|p| !p.is_empty())
                .unwrap_or(false);
            if !path_set_and_nonempty {
                let sn =
                    crate::ported::utils::scriptname_get().unwrap_or_else(|| "zshrs".to_string());
                // c:Src/exec.c:811 `zerr("command not found: %s", arg0)`
                // — the diagnostic carries the CURRENT line, not a
                // hardcoded 1. `lineno()` is the same counter zwarning
                // (utils.rs:179) uses and is live during VM execution
                // (verified: read-only / div-by-zero errors already
                // report the right line). Emitted directly (not via
                // zerr) to avoid setting errflag — command-not-found is
                // non-fatal and the script must continue.
                eprintln!(
                    "{}:{}: command not found: {}",
                    sn,
                    crate::ported::lex::lineno(),
                    cmd
                );
                return Ok(127);
            }
        }
        // c:Src/exec.c:2700-2724 resolvebuiltin — names registered via
        // `zmodload -ab MOD NAME` resolve through builtintab BEFORE
        // PATH search in C (execcmd's builtin lookup precedes the
        // external fork). Names the compiler didn't know as builtins
        // land here; consult the autoload ledger, load the module,
        // and re-dispatch through the builtin chokepoint. Without
        // this, `zmodload -ab zsh/bogus mybltn; mybltn` skipped the
        // C autoload-fire entirely (PATH miss → 127 instead of the
        // load_module diagnostic → 1).
        if !cmd.contains('/') {
            if let Some(rc) = crate::ported::module::resolvebuiltin(cmd) {
                if rc != 0 {
                    return Ok(1);
                }
                return Ok(crate::fusevm_bridge::dispatch_builtin_raw(
                    cmd,
                    args.to_vec(),
                ));
            }
        }
        let mut command = Command::new(cmd);
        // c:Src/exec.c execute — C unmetafies every arg before the
        // execve (the child must see raw bytes, not the shell's
        // internal Meta encoding). Args carrying Meta-char pairs
        // (from `$'\xff'` etc., vm_helper::meta_encode_byte) are
        // decoded to raw bytes via OsStr; plain args pass through
        // unchanged. Bug #127.
        for a in args {
            if a.contains('\u{83}') {
                use std::os::unix::ffi::OsStrExt as _;
                command.arg(std::ffi::OsStr::from_bytes(&unmetafy_str(a)));
            } else {
                command.arg(a);
            }
        }

        // Redirect handling lives in fusevm's WithRedirectsBegin/End
        // ops at compile time; `_redirects` arrives empty here.

        // c:Src/jobs.c — `time` reports only on JOBS (forked work). This
        // is the single chokepoint where an external process is actually
        // spawned (both fg and bg, all callers), AFTER the
        // command-not-found and resolvebuiltin early-returns above — so
        // counting here makes BUILTIN_TIME_SUBLIST report `time sleep 0`
        // / `time /usr/bin/true` (external → fork) while staying silent
        // for `time true` (builtin, never reaches this point). The
        // subshell entry counts separately (fusevm_bridge.rs:9573).
        FORK_EVENTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

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
                    let sn = crate::ported::utils::scriptname_get()
                        .unwrap_or_else(|| "zshrs".to_string());
                    if e.kind() == io::ErrorKind::NotFound {
                        // zsh: absolute paths emit "no such file or
                        // directory" (the OS error, since the path was
                        // tried directly), not "command not found"
                        // (which implies PATH search).
                        if cmd.starts_with('/') {
                            eprintln!(
                                "{}:{}: no such file or directory: {}",
                                sn,
                                crate::ported::lex::lineno(),
                                cmd
                            );
                        } else {
                            eprintln!(
                                "{}:{}: command not found: {}",
                                sn,
                                crate::ported::lex::lineno(),
                                cmd
                            );
                        }
                        Ok(127)
                    } else {
                        Err(format!("{}: {}: {}", sn, cmd, e))
                    }
                }
            }
        } else {
            // Queue signals across the wait so zshrs's SIGCHLD reaper
            // (waitpid(-1) in wait_for_processes, delivered on any
            // thread) can't reap this child before Command::status()
            // does — otherwise status() fails with ECHILD ("No child
            // processes"). See ForegroundWaitGuard in fusevm_bridge.
            let status_result = {
                let _wait_guard = crate::fusevm_bridge::ForegroundWaitGuard::enter();
                command.status()
            };
            match status_result {
                Ok(status) => Ok(status.code().unwrap_or(1)),
                Err(e) => {
                    // Use scriptname (the user-visible shell identifier
                    // — "zsh" in --zsh mode, "zshrs" otherwise) instead
                    // of a hardcoded "zshrs:" prefix so --zsh-mode
                    // diagnostics byte-match C zsh's stderr format.
                    let sn = crate::ported::utils::scriptname_get()
                        .unwrap_or_else(|| "zshrs".to_string());
                    if e.kind() == io::ErrorKind::NotFound {
                        // c:Src/exec.c — `command_not_found_handler` user
                        // hook: when a command lookup fails AND a function
                        // by that name is defined, call it with the cmd
                        // name + original args and return its rc instead
                        // of the default 127 + "command not found" error.
                        // Documented in zshmisc(1) under "Special
                        // Functions". Bug #426.
                        //
                        // The hook only fires for bare names (PATH search
                        // failed); absolute paths skip it and emit the
                        // OS-error path below — matches zsh behavior.
                        if !cmd.starts_with('/') {
                            let mut hook_args = Vec::with_capacity(args.len() + 1);
                            hook_args.push(cmd.to_string());
                            hook_args.extend_from_slice(args);
                            if let Some(rc) =
                                self.dispatch_function_call("command_not_found_handler", &hook_args)
                            {
                                return Ok(rc);
                            }
                        }
                        // zsh: absolute paths emit "no such file or
                        // directory" (the OS error, since the path was
                        // tried directly), not "command not found"
                        // (which implies PATH search).
                        if cmd.starts_with('/') {
                            eprintln!(
                                "{}:{}: no such file or directory: {}",
                                sn,
                                crate::ported::lex::lineno(),
                                cmd
                            );
                        } else {
                            eprintln!(
                                "{}:{}: command not found: {}",
                                sn,
                                crate::ported::lex::lineno(),
                                cmd
                            );
                        }
                        Ok(127)
                    } else if e.kind() == io::ErrorKind::PermissionDenied {
                        // zsh: non-executable file → "permission denied"
                        // on stderr and exit 126 (POSIX "command found
                        // but not executable").
                        eprintln!(
                            "{}:{}: permission denied: {}",
                            sn,
                            crate::ported::lex::lineno(),
                            cmd
                        );
                        Ok(126)
                    } else {
                        Err(format!("{}: {}: {}", sn, cmd, e))
                    }
                }
            }
        }
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
        // Context-isolated nested parse (c:Src/exec.c:283 parse_string) —
        // same rationale as run_command_substitution: process-sub argv
        // extraction runs during execution and must not clobber the outer
        // single-event reader's lexer/input position.
        let prog = parse_isolated(cmd_str);
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
    /// `run_command_substitution` — see implementation.
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
            // c:Src/lex.c — the `$(<file)` shortcut ONLY applies when
            // the body is exactly `<` + ONE word. Anything else (extra
            // args, redirects, semicolons, pipes) is a regular command
            // list and must go through the full parse path so `2>/dev/null`
            // / `>file` / `|cmd` / `; next` etc. work. Without this
            // gate, `$(< file 2>/dev/null)` treated `file 2>/dev/null`
            // as the literal filename and errored on the missing file.
            // Bug #615.
            let is_single_word = !filename.is_empty()
                && !filename.chars().any(|c| {
                    matches!(
                        c,
                        ' ' | '\t'
                            | '\n'
                            | ';'
                            | '&'
                            | '|'
                            | '<'
                            | '>'
                            | '('
                            | ')'
                            | '`'
                            | '"'
                            | '\''
                    )
                });
            if is_single_word {
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
            // Multi-word / has-redirects → fall through to full parse.
        }

        // Port of getoutput(char *cmd, int qt) from Src/exec.c. Parse and compile via
        // the lex+parse free ported + ZshCompiler pipeline, run on a
        // sub-VM with the host wired up. Stdout is captured through
        // an in-process pipe via dup2 — no fork. The sub-VM emits
        // Op::Exec for unknown command names, which forks/execs
        // through the host.

        // Set up the stdout-capture pipe. We dup the original stdout
        // so post-run we can restore it; the write end is dup2'd onto
        // STDOUT_FILENO so all output the sub-VM emits (including from
        // forked children, which inherit fd 1) lands in the pipe.
        //
        // c:Src/exec.c:4753 — `if (mpipe(pipes) < 0)`. mpipe (c:5160)
        // moves BOTH pipe ends to fd >= 10 via movefd and marks them
        // FDT_INTERNAL. This is load-bearing: zsh's invariant is that
        // shell-internal fds never live below 10, so user redirections
        // like `exec 9>&-` (which close fd<10 unconditionally, no
        // FDT_INTERNAL guard — c:Src/exec.c:3856-3868) can never hit
        // them. A raw pipe() here landed the read end on fd 9 when
        // fresh-HOME init held fds 3-7, and A04redirect's %prep
        // `exec 9>&-` closed our own capture pipe → SIGPIPE killed
        // the whole shell.
        let (read_fd, write_fd) = {
            let mut fds = [0i32; 2];
            if crate::ported::exec::mpipe(&mut fds) < 0 {
                return String::new();
            }
            (fds[0], fds[1])
        };
        // c:Src/utils.c:1996 — `movefd(dup(fd))`: saved copies of the
        // user-visible fds are shell-internal, so they too must live
        // at fd >= 10 / FDT_INTERNAL.
        let saved_stdout = crate::ported::utils::movefd(unsafe { libc::dup(libc::STDOUT_FILENO) });
        if saved_stdout < 0 {
            crate::ported::utils::zclose(read_fd);
            crate::ported::utils::zclose(write_fd);
            return String::new();
        }
        // Flush Rust's stdout BufWriter against the ORIGINAL fd before
        // dup2 swaps fd 1 to the capture pipe. Without this, bytes left
        // buffered by a prior `print -n` get drained to fd 1 AFTER the
        // dup2, which routes them into the cmd-subst's pipe — they end
        // up in the captured result and disappear from terminal output.
        //
        // Bug #10 in docs/BUGS.md — `print -n "A"; v=$(true); print -n
        // "B"; v=$(true); print -n "C"; echo` printed only `C` because
        // `A` and `B` were redirected into the empty cmd-subst's pipe
        // and discarded as its "output". C zsh's getoutput() forks, so
        // the child inherits the buffer COPY and the parent's buffer
        // stays untouched; zshrs runs cmd-subst in-process so the
        // parent buffer is the only one — must flush before the swap.
        let _ = io::stdout().flush();
        // c:Bug #56 — publish the saved outer stdout so a trap firing
        // during the nested run routes body output to the parent's
        // real stdout instead of the cmdsub's pipe-bound fd 1.
        // c:Src/utils.c:1996 — movefd(dup(fd)): internal fd, keep >= 10.
        let saved_stderr_for_trap =
            crate::ported::utils::movefd(unsafe { libc::dup(libc::STDERR_FILENO) });
        crate::fusevm_bridge::CMDSUBST_OUTER_FDS
            .with(|s| s.borrow_mut().push((saved_stdout, saved_stderr_for_trap)));
        unsafe {
            libc::dup2(write_fd, libc::STDOUT_FILENO);
        }
        // zclose (not raw close) so the FDT_INTERNAL mark set by mpipe
        // is cleared from fdtable — c:Src/utils.c:2137.
        crate::ported::utils::zclose(write_fd);

        // Drain the capture pipe CONCURRENTLY on a background reader
        // thread. The sub-VM (and any children it forks, which inherit
        // fd 1) writes to the pipe; reading it only AFTER vm.run()
        // returns deadlocks the moment the output exceeds the OS pipe
        // buffer (~64KB): the writer blocks on a full pipe that nothing
        // is draining, so vm.run() never returns. `$(alias)` over
        // zpwr's 2000+ aliases (~177KB) hung the whole shell a few
        // prompts in (thefuck's `fuck()` init runs `TF_SHELL_ALIASES=
        // $(alias)`). C's getoutput (Src/exec.c) forks the writer child
        // so the parent reads concurrently; this reader thread is the
        // in-process analog. It does only raw fd reads (no shell state /
        // thread-locals). EOF arrives once every write end closes — fd 1
        // restored below plus any forked child exiting.
        let reader_handle = std::thread::spawn(move || {
            let mut buf: Vec<u8> = Vec::new();
            let mut chunk = [0u8; 65536];
            loop {
                let n = unsafe {
                    libc::read(
                        read_fd,
                        chunk.as_mut_ptr() as *mut libc::c_void,
                        chunk.len(),
                    )
                };
                if n < 0 {
                    // Retry on EINTR (a signal interrupted the read);
                    // any other error ends the drain.
                    let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                    if e == libc::EINTR {
                        continue;
                    }
                    break;
                }
                if n == 0 {
                    break; // EOF — all write ends closed.
                }
                buf.extend_from_slice(&chunk[..n as usize]);
            }
            buf
        });

        // c:Src/exec.c:1161 — forked cmdsub child runs entersubsh()
        // which does `zsh_subshell++`; in-process equivalent (RAII,
        // restored on every return path below).
        let _subshell_bump = crate::fusevm_bridge::CmdSubstSubshellBump::enter();

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
        // Context-isolated nested parse (c:Src/exec.c:283 parse_string).
        // The outer loop()/parse_event reader may be mid-stream when this
        // cmd-subst executes (single-event mode), so a destructive
        // parse_init/lex_init would clobber its next read. parse_isolated
        // brackets the parse with zcontext_save/restore + inpush/inpop.
        let parsed = parse_isolated(cmd_str);
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
                // c:Src/exec.c:4783 — `$(...)` runs in a subshell, so
                // assignments / setopt / cd / trap changes inside
                // mustn't leak to the parent. zsh forks; we run
                // in-process and snapshot/restore manually. Same
                // snapshot shape used by host_subshell_begin/end for
                // the `(...)` subshell form.
                let paramtab_snap = crate::ported::params::paramtab()
                    .read()
                    .ok()
                    .map(|t| t.clone())
                    .unwrap_or_default();
                let paramtab_hashed_snap = crate::ported::params::paramtab_hashed_storage()
                    .lock()
                    .ok()
                    .map(|m| m.clone())
                    .unwrap_or_default();
                let pparams_snap = self.pparams();
                let opts_snap = crate::ported::options::opt_state_snapshot();
                // c:Src/exec.c:1161 — a command substitution runs in a
                // subshell, so IFS changes inside it must NOT leak to the
                // parent. IFS lives in the external `ifs_lock` global (not
                // paramtab), so the paramtab snapshot above doesn't cover
                // it: `echo $(IFS=:; set -- a b c; echo "$*")` set IFS=":"
                // which both produced "a:b:c" AND then word-split the
                // UNQUOTED result on the leaked ":" → "a b c". Snapshot the
                // global IFS here and restore it (with inittyptab) below.
                let ifs_snap = crate::ported::params::ifs_lock()
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or_default();
                let traps_snap = crate::ported::builtin::traps_table()
                    .lock()
                    .map(|t| t.clone())
                    .unwrap_or_default();
                // c:Src/exec.c:4783 — function definitions / unfunction
                // inside `$(...)` must also be isolated from the parent.
                // C zsh's getoutput() forks, so the child's shfunctab
                // mutations die with the child. zshrs's in-process
                // cmd-subst needs to snapshot/restore the function
                // tables manually alongside the param/opts/trap snaps
                // already in this block. Bug #455.
                let shfunctab_snap = crate::ported::hashtable::shfunctab_lock()
                    .read()
                    .ok()
                    .map(|t| t.snapshot())
                    .unwrap_or_default();
                let functions_compiled_snap = self.functions_compiled.clone();
                let function_source_snap = self.function_source.clone();
                // c:Src/exec.c:4782 — getoutput's child runs
                // `entersubsh(ESUB_PGRP|ESUB_NOMONITOR)`, and c:1219
                // `if (flags & ESUB_PGRP) clearjobtab(monitor)` hands
                // that child an EMPTY job table. The oldjobtab snapshot
                // (c:Src/jobs.c:1800) is monitor-only, so a
                // non-interactive shell keeps nothing at all — which is
                // why zsh prints nothing for `sleep 5 & print $(jobs)`.
                // The `(...)` and pipeline-stage paths already call
                // clearjobtab in their forked children; cmd-subst runs
                // in-process, so snapshot the globals clearjobtab
                // mutates and restore them below. freejob (c:1457) is
                // struct-local — no waitpid/kill — so the restore is
                // exact.
                let jobtab_snap = crate::ported::jobs::JOBTAB
                    .get()
                    .and_then(|t| t.lock().ok().map(|g| g.clone()));
                let maxjob_snap = crate::ported::jobs::MAXJOB
                    .get()
                    .and_then(|m| m.lock().ok().map(|g| *g));
                let thisjob_snap = crate::ported::jobs::THISJOB
                    .get()
                    .and_then(|t| t.lock().ok().map(|g| *g));
                // curjob/prevjob (c:Src/jobs.c) are plain globals in C,
                // so the forked child's setcurjob calls never reach the
                // parent — restore them alongside the table, else the
                // `+`/`-` markers are lost after a `$(jobs)`.
                let curjob_snap = crate::ported::jobs::CURJOB
                    .get()
                    .and_then(|t| t.lock().ok().map(|g| *g));
                let prevjob_snap = crate::ported::jobs::PREVJOB
                    .get()
                    .and_then(|t| t.lock().ok().map(|g| *g));
                {
                    let monitor =
                        crate::ported::zsh_h::isset(crate::ported::zsh_h::MONITOR) as i32;
                    crate::ported::jobs::clearjobtab(&mut self.jobs, monitor);
                }
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
                // `exit N` inside a cmd-subst should terminate ONLY
                // the sub-shell (C zsh: cmd-subst forks, the child
                // `_exit(N)`s; status reaches the parent as
                // cmd-subst exit). zshrs runs in-process, so we
                // route through the SUBSHELL_DEPTH-gated deferred
                // path inside zexit (builtin.rs:7713): bump
                // SUBSHELL_DEPTH so `exit` sets EXIT_PENDING/
                // EXIT_VAL instead of calling realexit (which would
                // process::exit and kill the parent shell). After
                // the sub-VM returns, harvest EXIT_PENDING/EXIT_VAL
                // as the cmd-subst's status, then restore the
                // parent's flags so the outer VM continues normally.
                use crate::ported::builtin::{
                    BREAKS, EXIT_PENDING, EXIT_VAL, RETFLAG, SHELL_EXITING, SUBSHELL_DEPTH,
                };
                use std::sync::atomic::Ordering::Relaxed;
                let saved_exit_pending = EXIT_PENDING.swap(0, Relaxed);
                let saved_exit_val = EXIT_VAL.swap(0, Relaxed);
                let saved_shell_exiting = SHELL_EXITING.swap(0, Relaxed);
                let saved_retflag = RETFLAG.swap(0, Relaxed);
                let saved_breaks = BREAKS.swap(0, Relaxed);
                SUBSHELL_DEPTH.fetch_add(1, Relaxed);
                let _ctx = ExecutorContext::enter(self);
                let _ = vm.run();
                let inner_exit_pending = EXIT_PENDING.load(Relaxed);
                let inner_exit_val = EXIT_VAL.load(Relaxed);
                let inner_status = if inner_exit_pending != 0 {
                    inner_exit_val & 0xFF
                } else {
                    vm.last_status
                };
                cmd_status = Some(inner_status);
                SUBSHELL_DEPTH.fetch_sub(1, Relaxed);
                // c:Src/exec.c — `$(…)` is a FORK in C: an errflag
                // abort inside the child ends the child (its lastval
                // becomes the cmd-subst status) and the flag dies
                // with the child process — the parent's lists keep
                // running. zsh 5.9: `v=$(typeset -A q; q=(odd));
                // echo "after $?"` prints `after 1`. Mirror the fork
                // isolation by clearing ERRFLAG_ERROR at the
                // cmd-subst boundary.
                //
                // ERRFLAG_HARD dies here too: `${u:?msg}` inside the
                // child sets it (c:Src/subst.c:3344) then `_exit(1)`s
                // (c:3353) — C's parent never sees the bit. A leaked
                // HARD bit makes every later zerr() silent
                // (c:Src/utils.c:175-177) and silently fails every
                // later parse. Same fix as subshell_end.
                errflag.fetch_and(
                    !(ERRFLAG_ERROR | crate::ported::zsh_h::ERRFLAG_HARD),
                    Relaxed,
                );
                // c:Src/exec.c:4783 execcmdoutsubst — `$(...)` is a
                // subshell, and zsh fires the EXIT trap when the
                // subshell ends BUT only if the trap was installed
                // INSIDE the subshell. An EXIT trap inherited from
                // the parent fires when the parent shell exits, not
                // again at cmdsub end. Detect "installed inside" by
                // comparing the current traps_table["EXIT"] entry
                // against the pre-cmdsub snapshot — fire only when
                // the body differs (newly set, removed, or replaced).
                // Pop the body before execute_script to avoid the
                // re-fire inside execute_script_zsh_pipeline's own
                // EXIT-handler tail at vm_helper.rs:1490. Bug #354.
                let snap_exit = traps_snap.get("EXIT").cloned();
                let live_exit = crate::ported::builtin::traps_table()
                    .lock()
                    .ok()
                    .and_then(|t| t.get("EXIT").cloned());
                if live_exit != snap_exit {
                    if let Some(body) = live_exit {
                        if let Ok(mut t) = crate::ported::builtin::traps_table().lock() {
                            t.remove("EXIT");
                        }
                        let _ = crate::ported::exec::execute_script(&body);
                    }
                }
                // c:Src/signals.c::dotrap(SIGEXIT) — also fire the
                // TRAPEXIT() function-named form (ZSIG_FUNC) — but
                // only if it was defined INSIDE the subshell (the
                // parent's TRAPEXIT fires at parent exit, not here).
                // ZSIG_FUNC bit on sigtrapped[SIGEXIT] tells us
                // whether a TRAPEXIT function is registered; check
                // BEFORE the snapshot restore.
                // Skip for now — function-form detection mirrors the
                // raw-body check above; deferred until a clean
                // sigtrapped snapshot/restore pair exists.
                // Restore parent's exit / loop / function-return
                // state so the outer VM continues normally.
                EXIT_PENDING.store(saved_exit_pending, Relaxed);
                EXIT_VAL.store(saved_exit_val, Relaxed);
                SHELL_EXITING.store(saved_shell_exiting, Relaxed);
                RETFLAG.store(saved_retflag, Relaxed);
                BREAKS.store(saved_breaks, Relaxed);
                // Restore parent state. The inner cmd-subst's stdout
                // (the captured pipe contents) is the only thing
                // that leaks out.
                if let Ok(mut t) = crate::ported::params::paramtab().write() {
                    *t = paramtab_snap;
                }
                if let Ok(mut m) = crate::ported::params::paramtab_hashed_storage().lock() {
                    *m = paramtab_hashed_snap;
                }
                self.set_pparams(pparams_snap);
                crate::ported::options::opt_state_restore(opts_snap);
                // Restore the parent's IFS (subshell isolation): the body's
                // `IFS=` must not leak out and word-split the parent's use
                // of the cmdsub result. Runtime word-splitting reads the
                // IFS *string* (this `ifs_lock` global), so restoring it is
                // sufficient. Deliberately do NOT call inittyptab() here —
                // that rewrites the process-global typtab the LEXER reads
                // on every character, and firing it per-cmdsub races
                // concurrent lexing in zshrs's worker threads, producing
                // spurious "parse error" flakes (HEAD ran clean 3/3; the
                // per-cmdsub inittyptab flaked ~50%). The typtab only
                // affects re-lexing — the parent is already compiled — and
                // leaving it at the body's value is strictly less divergent
                // than the prior behavior, which leaked the whole IFS.
                if let Ok(mut g) = crate::ported::params::ifs_lock().lock() {
                    *g = ifs_snap;
                }
                if let Ok(mut t) = crate::ported::builtin::traps_table().lock() {
                    *t = traps_snap;
                }
                // Restore function tables (parallel to the trap/param
                // restore above). Bug #455.
                if let Ok(mut t) = crate::ported::hashtable::shfunctab_lock().write() {
                    t.restore(shfunctab_snap);
                }
                self.functions_compiled = functions_compiled_snap;
                self.function_source = function_source_snap;
                // Undo the clearjobtab above — in C the cleared table
                // belongs to the forked child and dies with it, so the
                // parent's table must come back untouched.
                if let (Some(js), Some(t)) = (jobtab_snap, crate::ported::jobs::JOBTAB.get()) {
                    if let Ok(mut g) = t.lock() {
                        *g = js;
                    }
                }
                if let (Some(mj), Some(m)) = (maxjob_snap, crate::ported::jobs::MAXJOB.get()) {
                    if let Ok(mut g) = m.lock() {
                        *g = mj;
                    }
                }
                if let (Some(tj), Some(t)) = (thisjob_snap, crate::ported::jobs::THISJOB.get()) {
                    if let Ok(mut g) = t.lock() {
                        *g = tj;
                    }
                }
                if let (Some(cj), Some(t)) = (curjob_snap, crate::ported::jobs::CURJOB.get()) {
                    if let Ok(mut g) = t.lock() {
                        *g = cj;
                    }
                }
                if let (Some(pj), Some(t)) = (prevjob_snap, crate::ported::jobs::PREVJOB.get()) {
                    if let Ok(mut g) = t.lock() {
                        *g = pj;
                    }
                }
            }
        }
        // Restore LINENO so outer xtrace sees the outer line. LINENO
        // carries PM_READONLY (matching zsh's `integer-readonly-special`
        // GSU), so the restore must bypass the generic readonly guard
        // exactly like BUILTIN_SET_LINENO (fusevm_bridge.rs:5156) — write
        // the param's `u_val` directly and mirror the file-static /
        // lexer line counters. The previous `set_scalar` went through the
        // readonly-checked path: harmless on the `-c` route (LINENO not
        // yet flagged readonly there) but fatal on the faithful
        // loop()/zsh_main route, where every `$(...)` in piped/redirected
        // input died with `read-only variable: LINENO`.
        if let Some(ln) = saved_lineno {
            let n: crate::ported::zsh_h::zlong = ln.parse().unwrap_or(0);
            if let Ok(mut tab) = crate::ported::params::paramtab().write() {
                if let Some(pm) = tab.get_mut("LINENO") {
                    pm.u_val = n;
                    pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
                }
            }
            crate::ported::utils::set_lineno(n as i32);
            crate::ported::lex::set_lineno(n as u64);
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
        let final_status = cmd_status.unwrap_or(0);
        self.set_last_status(final_status);
        // c:Src/exec.c:4775 — `getoutput` (the C cmd-subst path used by
        // both `$(…)` and `` `…` ``) propagates the inner exit through
        // `cmdoutval`, then the caller does `LASTVAL = cmdoutval`. Mirror
        // by writing the cmd-subst's exit into the ported `cmdoutval`
        // global so `getoutput()`'s post-call `LASTVAL = cmdoutval` (at
        // exec.rs:559-562) and the C-equivalent `cmdoutval = lastval`
        // bookkeeping in execcmd_exec's assignment paths both see the
        // real exit. Without this, backtick assignments (`a=\`false\`;
        // echo $?`) reported 0 because getoutput's caller path read a
        // cmdoutval that was never updated by the in-process hook.
        crate::ported::exec::cmdoutval.store(final_status, std::sync::atomic::Ordering::Relaxed);

        // Flush any buffered Rust-side stdout so it reaches the pipe
        // before we restore.
        let _ = io::stdout().flush();

        // Pop the trap-routing stack BEFORE restoring stdout so any
        // trap that fires during the restore goes to the cmdsub's
        // pipe (matching what zsh's forked cmdsub would do — the
        // child's fd 1 is the pipe right up until the child exits).
        crate::fusevm_bridge::CMDSUBST_OUTER_FDS.with(|s| {
            s.borrow_mut().pop();
        });
        // c:Bug #353 — restore fd 2 from the saved outer stderr. A
        // body that ran `exec 2>&1` (no command, just redirects)
        // would have committed fd 2 → the cmdsub's pipe write end.
        // In zsh's forked cmdsub the committed redirect dies with
        // the child; zshrs's in-process cmdsub would leak the dup
        // back to the parent and keep the pipe write-end alive,
        // blocking the parent's read on the read_end forever.
        // Always restoring fd 2 here rolls back any commit so the
        // pipe write-end count drops to zero when we drop the
        // local write_fd reference (which already happened above).
        if saved_stderr_for_trap >= 0 {
            unsafe {
                libc::dup2(saved_stderr_for_trap, libc::STDERR_FILENO);
            }
            crate::ported::utils::zclose(saved_stderr_for_trap);
        }
        // Restore stdout and read what was captured.
        unsafe {
            libc::dup2(saved_stdout, libc::STDOUT_FILENO);
        }
        crate::ported::utils::zclose(saved_stdout);
        // Collect the concurrently-drained output. With fd 1 restored
        // above, the last shell-side write end is closed, so the reader
        // hits EOF and join() returns the full buffer regardless of
        // size — no pipe-full deadlock. The reader only read (never
        // closed) read_fd, so zclose still clears its FDT_INTERNAL mark
        // (c:Src/utils.c:2137).
        let bytes = reader_handle.join().unwrap_or_default();
        crate::ported::utils::zclose(read_fd);
        let mut output = String::from_utf8_lossy(&bytes).into_owned();

        // POSIX: trailing newlines stripped from cmd-sub result.
        while output.ends_with('\n') {
            output.pop();
        }
        output
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
    /// `Parent` variant.
    Parent(i32), // Contains child PID
    /// `Child` variant.
    Child,
}

/// Redirection mode
#[derive(Debug, Clone, Copy)]
/// File-redirection mode (`>` / `>>` / `<` / etc.).
/// Mirrors the `REDIR_*` enum from Src/zsh.h.
pub enum RedirMode {
    /// `Dup` variant.
    Dup,
    /// `Close` variant.
    Close,
}

/// Builtin command type
#[derive(Debug, Clone, Copy)]
/// Builtin classification.
/// Mirrors the `BINF_*` flag set Src/builtin.c uses to
/// classify special vs regular builtins.
pub enum BuiltinType {
    /// `Normal` variant.
    Normal,
    /// `Disabled` variant.
    Disabled,
}

use crate::fusevm_bridge::with_executor;
use crate::ported::glob::*;
use crate::ported::hist::*;
use crate::ported::jobs::*;
use crate::ported::math::*;
use crate::ported::module::*;
use crate::ported::modules::cap::*;
use crate::ported::modules::terminfo::*;
use crate::ported::options::*;
use crate::ported::params::*;
use crate::ported::pattern::*;
use crate::ported::prompt::*;
use crate::ported::signals::*;
use crate::ported::subst::*;
use crate::ported::utils::{zerr, zerrnam, zwarn, zwarnnam};
use ::regex::{Error as RegexError, Regex, RegexBuilder};

pub use crate::ported::modules::regex::posix_ere_bracket_escape;

impl ShellExecutor {
    /// Every option name in `ZSH_OPTIONS_SET` (port of `optns[]` at
    /// `Src/options.c:79+`).
    pub(crate) fn all_zsh_options() -> Vec<&'static str> {
        ZSH_OPTIONS_SET.iter().copied().collect()
    }

    /// `name → default-on` map via canonical `default_on_options`
    /// (port of `defset()` macro at `Src/options.c:73`).
    pub(crate) fn default_options() -> HashMap<String, bool> {
        let on = default_on_options();
        Self::all_zsh_options()
            .into_iter()
            .map(|n| (n.to_string(), on.contains(n)))
            .collect()
    }
}
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

// Source-form registration step used by the autoload-load path
// (`dispatch_function_call` / `run_function_body_only`). Decides
// whether to feed the file body to the funcdef pipeline VERBATIM
// (zsh-style: the body's own `function NAME() {...}` definition
// registers the function on execution) or to WRAP it in
// `NAME() {...}` (ksh-style or multi-statement: the body is just
// commands or includes additional statements past the def).
//
// The classification mirrors c:Src/exec.c:5725 + 5750 — KSHAUTOLOAD-
// equivalent vs zsh-style autoload. The structural check is the
// canonical `stripkshdef` (Src/exec.c:6291, ported at exec.rs:10548):
// parse the body to Eprog, run stripkshdef, and check whether it
// returned a stripped (different-length wordcode) Eprog. When it
// did, the file is the single-funcdef shape `[function] NAME [()] {
// INNER }`; running the file source directly through the funcdef
// pipeline registers NAME via the WC_FUNCDEF opcode at
// fusevm_bridge.rs:6330, matching C's `shf->funcdef = stripkshdef(
// prog, name)` semantics (the inner body becomes the function's
// body). When it didn't strip — single statement that isn't a
// funcdef, or multiple list nodes (e.g. `function ztm() {...}` +
// trailing `ztm "$@"` self-call) — we fall back to wrap-and-run so
// the canonical funcdef opcode still fires and any extra
// statements run inside the registered body, matching C's
// behavior of using the whole prog as funcdef in that case.
/// Restore `noaliases` on scope exit.
///
/// C's `loadautofn` (Src/exec.c:5684-5704) saves `noaliases`, sets it from the
/// function's PM_UNALIASED bit for the duration of the body parse, and restores
/// it unconditionally. The zshrs autoload block has early `return` paths, so the
/// restore has to ride on Drop rather than a trailing statement — otherwise a
/// `-U` autoload that failed to load would leave alias expansion disabled for
/// the rest of the shell.
struct NoAliasesRestore(bool);

impl Drop for NoAliasesRestore {
    fn drop(&mut self) {
        crate::ported::lex::set_noaliases(self.0); // c:5704
    }
}

fn autoload_register_source(name: &str, body: &str) -> String {
    // c:Src/exec.c:5725 — `if (ksh == 2 || (ksh == 1 && isset(KSHAUTOLOAD)))`
    // — the ksh-style load branch. ksh derives from the stub's
    // stored-style bits per c:5708-5709 (`PM_KSHSTORED ? 2 :
    // PM_ZSHSTORED ? 0 : 1`; a decisive `.zwc` header flag was
    // already folded into these bits by loadautofn). A ksh-style
    // load executes the file contents at top level (c:5740-5746
    // `execode(prog, 1, 0, "evalautofunc")`) and expects the file
    // itself to define the function — so the body goes through the
    // pipeline VERBATIM, never wrapped.
    let flags = crate::ported::utils::getshfunc(name)
        .map(|f| f.node.flags as u32)
        .unwrap_or(0);
    let ksh = if flags & crate::ported::zsh_h::PM_KSHSTORED != 0 {
        2
    } else if flags & crate::ported::zsh_h::PM_ZSHSTORED != 0 {
        0
    } else {
        1
    };
    if ksh == 2 || (ksh == 1 && crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHAUTOLOAD)) {
        return body.to_string(); // c:5746 execode(prog, ..., "evalautofunc")
    }
    let stripped = crate::ported::exec::parse_string(body, 0)
        .map(|prog| {
            let original_len = prog.prog.len();
            // stripkshdef returns the input untouched when the prog
            // doesn't match the single-`function NAME` shape, and a
            // shorter (body-only) prog when it does. Compare the
            // wordcode length to detect the strip without owning the
            // post-strip Eprog (we only need the yes/no answer here).
            let prog_box = Box::new(prog);
            crate::ported::exec::stripkshdef(Some(prog_box), name)
                .map(|p| p.prog.len() != original_len)
                .unwrap_or(false)
        })
        .unwrap_or(false);
    if stripped {
        body.to_string()
    } else {
        format!("{name}() {{\n{body}\n}}")
    }
}

// zsh_eval_context push/pop/sync relocated 2026-06-12 INTO doshfunc
// (src/ported/exec.rs) — its sole caller, and `zsh_eval_context` is
// that module's own static. The shell-visible mirror writes inline
// at the push site + the guard's Drop. No bridge indirection.

impl ShellExecutor {
    /// Execute the trap body for a signal name from the REPL signal
    /// loop (bins/zshrs.rs CtrlC/CtrlD dispatch). Thin passthru to
    /// `traps_table` lookup + `execute_script` — kept as a method
    /// because the REPL loop owns `&mut ShellExecutor` and needs a
    /// single call point. The async signal-handler dispatch path
    /// goes through `crate::ported::signals::dotrap` instead.
    pub fn run_trap(&mut self, signal: &str) {
        let action = crate::ported::builtin::traps_table()
            .lock()
            .ok()
            .and_then(|t| t.get(signal).cloned());
        if let Some(body) = action {
            if !body.is_empty() {
                let _ = self.execute_script(&body);
            }
        }
    }
}

impl ShellExecutor {
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
impl ShellExecutor {
    /// Expand glob pattern via canonical `glob_path` (port of
    /// `Src/glob.c::zglob`). Adds executor-side `current_command_glob_failed`
    /// cell so the dispatch layer skips the current command on NOMATCH +
    /// looks_like_glob instead of exiting the shell.
    pub fn expand_glob(&self, pattern: &str) -> Vec<String> {
        let expanded = glob_path(pattern);
        if !expanded.is_empty() {
            // c:Src/glob.c:1871-1872 — `if (matchct) badcshglob |= 2;`
            // (at least one expansion on this command line worked).
            // Only real glob patterns count — C's zglob early-returns
            // before the matchct accounting for non-wild words, so
            // gate on haswilds like the failure path below. Consumed
            // per command by fusevm_bridge::consume_badcshglob.
            if crate::ported::zsh_h::isset(crate::ported::zsh_h::CSHNULLGLOB) {
                let mut pattern_tok = pattern.to_string();
                crate::ported::glob::tokenize(&mut pattern_tok);
                if crate::ported::pattern::haswilds(&pattern_tok) {
                    crate::ported::glob::BADCSHGLOB
                        .fetch_or(2, std::sync::atomic::Ordering::Relaxed);
                }
            }
            return expanded;
        }
        // No matches. Mirror zsh's `setopt nullglob` / `nomatch`
        // dispatch (Src/glob.c:1873-1886) here because glob_path
        // returns an empty Vec without knowing executor state.
        // c:Src/glob.c:1567-1569 `gf_nullglob` per-glob — the `(N)`
        // qualifier acts like `setopt nullglob` for this expression
        // alone. parse_qualifiers detects the suffix `(...)` block;
        // the resulting `qualifiers.nullglob` mirrors C's gf_nullglob
        // carrier.
        let per_glob_nullglob = crate::ported::glob::parse_qualifiers(pattern)
            .1
            .map(|q| q.nullglob)
            .unwrap_or(false);
        let nullglob = opt_state_get("nullglob").unwrap_or(false) || per_glob_nullglob;
        if nullglob {
            return Vec::new();
        }
        let nomatch = opt_state_get("nomatch").unwrap_or(true);
        // Use canonical `haswilds` (port of Src/pattern.c:4306-4376)
        // instead of the Rust-only `looks_like_glob`. C zsh's
        // `Src/glob.c:1876` NOMATCH branch fires whenever the input
        // tripped haswilds during the `zglob` entry check —
        // including patterns whose internal `(` / `)` form a group
        // or alternation but don't end with `)` (e.g. `abc(a)def`,
        // `(abc`). The previous `looks_like_glob` only caught
        // trailing-`(...)` qualifiers, leaving mid-word groups and
        // unclosed parens to fall through to the literal-passthrough
        // branch. #170 in docs/BUGS.md.
        //
        // haswilds scans TOKENIZED strings (C's zglob gets the
        // lexer-tokenized word at Src/glob.c:1230); this entry point
        // receives untokenized fast-path patterns, so tokenize a
        // local copy first — the same preparation C applies to
        // runtime-built strings (compcore.c:2231 tokenizes fignore
        // entries before its haswilds call). tokenize Bnull's
        // backslash-escaped metachars, so `\*` stays literal here
        // exactly as in C. Bug #627: plain multibyte text (`↔`)
        // passes through tokenize unchanged and matches no token.
        let mut pattern_tok = pattern.to_string();
        crate::ported::glob::tokenize(&mut pattern_tok); // c:Src/glob.c:3548
        let is_glob = crate::ported::pattern::haswilds(&pattern_tok);
        // c:Src/glob.c:1874-1875 — `if (isset(CSHNULLGLOB)) {
        // badcshglob |= 1; }` — the else-if chain means neither the
        // NOMATCH error nor the literal passthrough runs: the failed
        // word is silently DROPPED here, and the per-command boundary
        // (fusevm_bridge::consume_badcshglob, Src/subst.c:505-507)
        // emits the csh-style `no match` iff NO glob on the line
        // matched.
        if is_glob && crate::ported::zsh_h::isset(crate::ported::zsh_h::CSHNULLGLOB) {
            crate::ported::glob::BADCSHGLOB.fetch_or(1, std::sync::atomic::Ordering::Relaxed);
            return Vec::new();
        }
        if nomatch && is_glob {
            // c:Src/glob.c:1876-1880 — `else if (isset(NOMATCH)) {`
            //   `zerr("no matches found: %s", ostr);`
            //   `zfree(matchbuf, 0);`
            //   `restore_globstate(saved);`
            //   `return;`
            // `}`
            // C aborts via ERRFLAG_ERROR set by zerr() at c:Src/utils.c
            // and the matchbuf/state cleanup. The Rust port mirrors
            // both: zerr() in utils.rs sets ERRFLAG_ERROR via
            // `errflag.fetch_or(ERRFLAG_ERROR, ...)` already; we then
            // re-set explicitly (defensive — historically this line
            // had `fetch_and(!ERRFLAG_ERROR)` which CLEARED the flag
            // immediately after zerr, making `echo /never/*` print
            // the literal and exit 0 instead of erroring like zsh —
            // parity bug #13).
            zerr(&format!("no matches found: {}", pattern)); // c:1877
            self.current_command_glob_failed.set(true);
            // c:Src/glob.c:1876-1880 — zerr sets ERRFLAG_ERROR and
            // glob_failed cell carries the signal. The ERRFLAG_ERROR
            // clear (so subsequent sublists run) now lives at the
            // dispatcher's post-command-boundary at
            // fusevm_bridge.rs:299 where current_command_glob_failed
            // is consumed — matches C's execlist behavior of clearing
            // command-error errflag between sublists.
            return Vec::new(); // c:1880 return
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

impl ShellExecutor {
    pub(crate) fn copy_dir_recursive(src: &Path, dest: &Path) -> io::Result<()> {
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

// Magic-assoc scan-by-name aggregator. C's per-table getfn/scanfn
// pointers in paramdef[] (Src/Modules/parameter.c:825+) handle this
// indirectly via paramtab dispatch; this Rust-only helper exposes a
// single `partab_get` / `partab_scan_keys` entry that the bridge
// uses for name → keys lookup.
use std::cell::RefCell;
thread_local! {
    static SCAN_KEYS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Lookup helper for `${name[key]}` magic-assoc reads — dispatches
/// through canonical `PARTAB` (Src/Modules/parameter.c:2235 ports).
/// Returns `None` if name isn't a known magic-assoc.
pub fn partab_get(name: &str, key: &str) -> Option<String> {
    // c:Src/Modules/system.c:902,904 — `sysparams` and `errnos` are
    // bound by zsh/system's boot_/setup_ chain. Same for `mapfile`
    // from zsh/mapfile. Without explicit `zmodload`, these names
    // are unset in zsh; gate the PARTAB dispatch here so they
    // resolve via the empty-fallback path (matching ${sysparams[k]:-x}
    // taking the default). Bug #69 in docs/BUGS.md.
    if let Some(modname) = module_gated_partab_module(name) {
        if !crate::ported::module::MODULESTAB
            .lock()
            .unwrap()
            .is_loaded(modname)
        {
            return None;
        }
    }
    for entry in PARTAB.iter() {
        if entry.name == name {
            return (entry.getfn)(std::ptr::null_mut(), key).and_then(|p| p.u_str);
        }
    }
    None
}

/// Returns the owning module name for partab entries that are
/// bound by an explicit zmodload — `sysparams`/`errnos` from
/// zsh/system, `mapfile` from zsh/mapfile. Other partab entries
/// (aliases/commands/functions/...) are part of zsh/main and
/// always available.
fn module_gated_partab_module(name: &str) -> Option<&'static str> {
    match name {
        "sysparams" | "errnos" => Some("zsh/system"),
        "mapfile" => Some("zsh/mapfile"),
        "langinfo" => Some("zsh/langinfo"),
        _ => None,
    }
}

/// PM_ARRAY lookup for `${name}` / `${name[N]}` — walks
/// PARTAB_ARRAY and dispatches the whole-array getfn (Src/Modules/
/// parameter.c:2239-2291 ports). Returns `None` if name isn't a
/// known PM_ARRAY magic-assoc.
pub fn partab_array_get(name: &str) -> Option<Vec<String>> {
    // Bug #69 — gate module-bound PARTAB names on the owning
    // module's MOD_LINKED && !MOD_UNLOAD state.
    if let Some(modname) = module_gated_partab_module(name) {
        if !crate::ported::module::MODULESTAB
            .lock()
            .unwrap()
            .is_loaded(modname)
        {
            return None;
        }
    }
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
    // Bug #69 — gate module-bound PARTAB names on the owning
    // module's MOD_LINKED && !MOD_UNLOAD state.
    if let Some(modname) = module_gated_partab_module(name) {
        if !crate::ported::module::MODULESTAB
            .lock()
            .unwrap()
            .is_loaded(modname)
        {
            return None;
        }
    }
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
    use crate::ported::zsh_h::{
        hashnode, param, Param, PM_HIDE, PM_HIDEVAL, PM_READONLY, PM_SPECIAL,
    };
    let mut tab = match paramtab().write() {
        Ok(t) => t,
        Err(_) => return,
    };
    // c:Src/zsh.h SPECIALPMDEF macro: `flags | PM_SPECIAL | PM_HIDE |
    // PM_HIDEVAL`. All magic-assoc/array params get HIDE+HIDEVAL added
    // by the macro itself.
    //
    // PM_READONLY is preserved on the stub for params that legitimately
    // need user-write protection (reswords, dis_reswords, patchars,
    // dis_patchars — all compute via getfn and have no legitimate
    // internal-write path). Other specials that DO have internal-write
    // paths (e.g. funcstack from function-call tracking) get the bit
    // stripped so the runtime can mutate their u_arr. Bug #374.
    let user_protected: &[&str] = &[
        "reswords",
        "dis_reswords",
        "patchars",
        "dis_patchars",
        "historywords",
        "errnos",
        "keymaps",
    ];
    let mk_pm = |name: &str, flags: i32| -> Param {
        let keep_readonly = user_protected.contains(&name);
        let pre_readonly_mask = if keep_readonly {
            !0i32
        } else {
            !(PM_READONLY as i32)
        };
        Box::new(param {
            node: hashnode {
                next: None,
                nam: name.to_string(),
                flags: (flags & pre_readonly_mask)
                    | PM_SPECIAL as i32
                    | PM_HIDE as i32
                    | PM_HIDEVAL as i32,
            },
            u_data: 0,
            u_tied: None,
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
    // c:Src/Modules/system.c:902,904 + Src/Modules/mapfile.c — these
    // params are provided by modules that real zsh requires explicit
    // `zmodload` for. Seeding them unconditionally makes
    // `${+sysparams}` return 1 by default (bug #69 in docs/BUGS.md),
    // diverging from zsh which returns 0 until the user runs
    // `zmodload zsh/system`. Skip here; `seed_partab_param` below adds
    // them on demand from the module's load path.
    let module_gated: &[&str] = &[
        "sysparams", // zsh/system
        "errnos",    // zsh/system
        "mapfile",   // zsh/mapfile
        "langinfo",  // zsh/langinfo
    ];
    for entry in PARTAB.iter() {
        if module_gated.contains(&entry.name) {
            continue;
        }
        tab.insert(entry.name.to_string(), mk_pm(entry.name, entry.flags));
    }
    for entry in PARTAB_ARRAY.iter() {
        if module_gated.contains(&entry.name) {
            continue;
        }
        tab.insert(entry.name.to_string(), mk_pm(entry.name, entry.flags));
    }
}

/// Insert a single PARTAB / PARTAB_ARRAY entry into paramtab. Called
/// from `zmodload <module>` once the module's boot completes, so that
/// `${+sysparams}` (etc.) flip from 0 → 1 only after explicit load.
/// No direct C counterpart — the C path runs through the module's
/// `setup_/boot_` chain which adds the SPECIALPMDEF entry via the
/// general hashtable machinery. Bug #69 in docs/BUGS.md.
pub fn seed_partab_param(name: &str) {
    use crate::ported::modules::parameter::{PARTAB, PARTAB_ARRAY};
    use crate::ported::zsh_h::{hashnode, param, PM_HIDE, PM_HIDEVAL, PM_READONLY, PM_SPECIAL};
    let mut tab = match crate::ported::params::paramtab().write() {
        Ok(t) => t,
        Err(_) => return,
    };
    if tab.contains_key(name) {
        return; // already seeded
    }
    let flags = PARTAB
        .iter()
        .find(|e| e.name == name)
        .map(|e| e.flags)
        .or_else(|| {
            PARTAB_ARRAY
                .iter()
                .find(|e| e.name == name)
                .map(|e| e.flags)
        });
    let Some(flags) = flags else {
        return;
    };
    let pm = Box::new(param {
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags: (flags & !(PM_READONLY as i32))
                | PM_SPECIAL as i32
                | PM_HIDE as i32
                | PM_HIDEVAL as i32,
        },
        u_data: 0,
        u_tied: None,
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
    });
    tab.insert(name.to_string(), pm);
}

/// Default autoloadable parameters: name → owning module. Port of the
/// `autofeatures` `p:` rows in Src/Modules/parameter.mdd, watch.mdd,
/// termcap.mdd, terminfo.mdd, Src/Zle/zleparameter.mdd and
/// Src/Builtins/sched.mdd, registered at startup through
/// `setautofeatures` → `add_autoparam` (Src/module.c:1198-1229): each
/// name becomes a scalar paramtab stub whose VALUE is the module name,
/// flagged PM_AUTOLOAD (module.c:1218-1219). Matches `zmodload -ap`
/// output of the reference zsh build.
pub const AUTOLOAD_PARAMS: &[(&str, &str)] = &[
    // Src/Modules/watch.mdd:5 autofeatures
    ("WATCH", "zsh/watch"),
    ("watch", "zsh/watch"),
    // Src/Modules/parameter.mdd:5 autofeatures
    ("aliases", "zsh/parameter"),
    ("builtins", "zsh/parameter"),
    ("commands", "zsh/parameter"),
    ("dirstack", "zsh/parameter"),
    ("dis_aliases", "zsh/parameter"),
    ("dis_builtins", "zsh/parameter"),
    ("dis_functions", "zsh/parameter"),
    ("dis_functions_source", "zsh/parameter"),
    ("dis_galiases", "zsh/parameter"),
    ("dis_patchars", "zsh/parameter"),
    ("dis_reswords", "zsh/parameter"),
    ("dis_saliases", "zsh/parameter"),
    ("funcfiletrace", "zsh/parameter"),
    ("funcsourcetrace", "zsh/parameter"),
    ("funcstack", "zsh/parameter"),
    ("functions", "zsh/parameter"),
    ("functions_source", "zsh/parameter"),
    ("functrace", "zsh/parameter"),
    ("galiases", "zsh/parameter"),
    ("history", "zsh/parameter"),
    ("historywords", "zsh/parameter"),
    ("jobdirs", "zsh/parameter"),
    ("jobstates", "zsh/parameter"),
    ("jobtexts", "zsh/parameter"),
    ("modules", "zsh/parameter"),
    ("nameddirs", "zsh/parameter"),
    ("options", "zsh/parameter"),
    ("parameters", "zsh/parameter"),
    ("patchars", "zsh/parameter"),
    ("reswords", "zsh/parameter"),
    ("saliases", "zsh/parameter"),
    ("userdirs", "zsh/parameter"),
    ("usergroups", "zsh/parameter"),
    // Src/Zle/zleparameter.mdd:5 autofeatures
    ("keymaps", "zsh/zleparameter"),
    ("widgets", "zsh/zleparameter"),
    // Src/Modules/termcap.mdd:5 / terminfo.mdd:5 autofeatures
    ("termcap", "zsh/termcap"),
    ("terminfo", "zsh/terminfo"),
    // Src/Builtins/sched.mdd:5 autofeatures
    ("zsh_scheduled_events", "zsh/sched"),
];

/// Autoload stubs whose owning module is NOT loaded — the rows zsh's
/// `typeset` listings print as `undefined NAME` (printparamnode's
/// PM_AUTOLOAD pmtypes row, Src/params.c:6011 + the PM_AUTOLOAD
/// NAMEONLY arm at Src/params.c:6146-6155). Once a module loads, its
/// stubs drop out and the real params list instead.
pub fn autoload_param_stubs() -> Vec<(&'static str, &'static str)> {
    use crate::ported::zsh_h::{MOD_INIT_B, MOD_UNLOAD};
    let tab = crate::ported::module::MODULESTAB.lock().unwrap();
    AUTOLOAD_PARAMS
        .iter()
        .copied()
        .filter(|(_, m)| {
            // "Boot ran" is MOD_INIT_B && !MOD_UNLOAD (the criterion
            // printmodulenode uses, src/ported/module.rs:246 — C's
            // `m->u.handle` union check at Src/module.c:218-241).
            // modulestab::is_loaded checks MOD_LINKED which
            // register_builtin_modules pre-seeds for EVERY compiled-in
            // module, so it would report zsh/parameter "loaded" in a
            // fresh `zsh -f` where real zsh still shows the stubs.
            !tab.modules.get(*m).is_some_and(|md| {
                (md.node.flags & MOD_INIT_B) != 0 && (md.node.flags & MOD_UNLOAD) == 0
            })
        })
        .collect()
}

/// Names provided by `zsh/system` / `zsh/mapfile` etc. that are
/// gated on explicit `zmodload`. Used by the bin_zmodload path to
/// re-seed paramtab after the module's boot completes.
pub fn module_gated_params_for(module: &str) -> &'static [&'static str] {
    match module {
        "zsh/system" => &["sysparams", "errnos"],
        "zsh/mapfile" => &["mapfile"],
        "zsh/langinfo" => &["langinfo"],
        _ => &[],
    }
}
impl ShellExecutor {
    /// `enter_posix_mode` — see implementation.
    pub fn enter_posix_mode(&mut self) {
        self.posix_mode = true;
        self.plugin_cache = None;
        self.compsys_cache = None;
        self.compinit_pending = None;
        self.worker_pool = std::sync::Arc::new(crate::worker::WorkerPool::new(1));
        // Direct call to the canonical `emulate()` port
        // (Src/options.c:533) — `-R` semantics = fully=true.
        // bin_emulate goes through dispatch_builtin which needs an
        // ExecutorContext that isn't set up yet at apply_cli_flags
        // time; the underlying emulate() doesn't need one.
        crate::ported::options::emulate("sh", true);
    }
    /// `enter_ksh_mode` — see implementation.
    pub fn enter_ksh_mode(&mut self) {
        self.plugin_cache = None;
        self.compsys_cache = None;
        self.compinit_pending = None;
        self.worker_pool = std::sync::Arc::new(crate::worker::WorkerPool::new(1));
        crate::ported::options::emulate("ksh", true);
    }
    /// `enter_dash_mode` — strict-dash (Debian Almquist Shell) runtime.
    /// Same executor setup as [`enter_posix_mode`] (dash IS `sh` for every
    /// option), but calls `emulate("dash")` so the Rust-only DASH_STRICT
    /// flag is raised (and NOT cleared, as `emulate("sh")` would). See
    /// `src/extensions/dash_mode.rs`.
    pub fn enter_dash_mode(&mut self) {
        self.posix_mode = true;
        self.plugin_cache = None;
        self.compsys_cache = None;
        self.compinit_pending = None;
        self.worker_pool = std::sync::Arc::new(crate::worker::WorkerPool::new(1));
        crate::ported::options::emulate("dash", true);
    }
}

/// Thin (text, pattern) → bool wrapper over the canonical
/// `patcompile()` + `pattry()` pair from `Src/pattern.c`. Argument
/// order is flipped so callers read naturally. Lives in vm_helper.rs
/// (non-port file) as the public convenience entry for extensions
/// and the VM bridge; `src/ported/*` files inline the compile+match
/// idiom directly to preserve PORT.md Rule 1 faithfulness.
pub fn glob_match_static(s: &str, pattern: &str) -> bool {
    let Some(prog) = patcompile(
        &{
            let mut __pat_tok = (pattern).to_string();
            crate::ported::glob::tokenize(&mut __pat_tok);
            __pat_tok
        },
        PAT_HEAPDUP as i32,
        None,
    ) else {
        return false;
    };
    // (#b) (GF_BACKREF) — capture-aware path. Use pattryrefs so the
    // per-group begin/end offsets surface, then write $match /
    // $mbegin / $mend (c:Src/pattern.c GF_BACKREF handling at
    // c:775 + c:2417). Falls through to the basic pattry path when
    // (#b) isn't on — that matches the previous behaviour exactly
    // and avoids the small extra cost (state clone + Vec<i32> alloc)
    // for the (#b)-free common case.
    // c:Src/pattern.c:2543 / 2570 — the C gate for capture reporting is
    // `prog->patnpar` (count of NUMBERED groups), not a globflags bit.
    // patcomppiece (pattern.rs, c:775 arm) only numbers a group when
    // GF_BACKREF was live at the point the `(` compiled, so patnpar>0
    // ⇔ the pattern had an effective `(#b)` REGARDLESS of position.
    // The old `globflags & GF_BACKREF` gate only saw flags hoisted from
    // a LEADING `(#...)` spec, so `"%"(#b)(test)*` (literal prefix
    // before the flag) never populated $match. Bug: gap #1 2026-06-12.
    let has_backref = prog.0.patnpar > 0;
    let matched;
    if has_backref {
        let mut nump: i32 = 0;
        let mut begp: Vec<i32> = Vec::new();
        let mut endp: Vec<i32> = Vec::new();
        matched = crate::ported::pattern::pattryrefs(
            &prog,
            s,
            s.len() as i32,
            -1,
            None,
            0,
            Some(&mut nump),
            Some(&mut begp),
            Some(&mut endp),
        );
        if matched {
            let n = (nump as usize).min(begp.len()).min(endp.len());
            let mut match_arr: Vec<String> = Vec::with_capacity(n);
            let mut begin_arr: Vec<String> = Vec::with_capacity(n);
            let mut end_arr: Vec<String> = Vec::with_capacity(n);
            // KSHARRAYS off → 1-based; on → 0-based. C path:
            // `setiparam("MBEGIN", patoffset + !isset(KSHARRAYS))`
            // — so the base offset added to each begin/end index.
            let ksharrays = crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHARRAYS);
            let base = if ksharrays { 0 } else { 1 };
            for i in 0..n {
                // c:2607-2613 — unset group (unmatched alternation
                // branch / hashed paren): matcharr[i]="",
                // mbeginarr[i]="-1", mendarr[i]="-1". pattryrefs
                // reports these as begp/endp = -1 (c:2563-2566).
                if begp[i] < 0 || endp[i] < 0 {
                    match_arr.push(String::new());
                    begin_arr.push("-1".to_string());
                    end_arr.push("-1".to_string());
                    continue;
                }
                let b = begp[i] as usize;
                let e = endp[i] as usize;
                let lo = b.min(s.len());
                let hi = e.min(s.len()).max(lo);
                match_arr.push(s[lo..hi].to_string());
                begin_arr.push((b + base).to_string());
                // mend is the INDEX of the last matched char (inclusive),
                // so end - 1 + base. For an empty span use the begin
                // offset (zsh's setiparam("MEND", mlen + patoffset + ...
                // - 1) shape from c:2444-2446).
                end_arr.push(((e + base).saturating_sub(1)).to_string());
            }
            crate::ported::params::setaparam("match", match_arr);
            crate::ported::params::setaparam("mbegin", begin_arr);
            crate::ported::params::setaparam("mend", end_arr);
        }
    } else {
        matched = pattry(&prog, s);
    }
    // $MATCH / $MBEGIN / $MEND are set by the MATCHER (pattern.rs, c:2526),
    // which is where C decides it — gated on the GLOBAL patglobflags so a later
    // `(#M)` can turn GF_MATCHREF back off (c:1099-1100).
    //
    // This layer used to re-do it with `pattern.contains("(#m)")`, which can
    // only ever answer "on": it re-set $MATCH after the matcher had correctly
    // declined to, so `(#m)(#M)a*` reported a match string where zsh leaves it
    // unset. Wrong mechanism (a substring test cannot see which flag came last)
    // and wrong layer (the matcher already has the compiled flags).
    matched
}

pub use crate::ported::lex::untokenize_ztokens;

pub use crate::ported::utils::unmetafy_str;

pub use crate::ported::utils::zsh_errno_msg;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PM_NAMEREF bridge helpers (typeset -n / named references).
//
// Rust-only adapters around the canonical C nameref machinery in
// Src/params.c (resolve_nameref_rec c:6332, setscope c:6382,
// upscope c:6455) and Src/builtin.c (bin_typeset nameref arm
// c:3117-3150). zshrs's paramtab is a name-keyed HashMap handing
// out clones, so the chain walk operates by NAME against the live
// table instead of by Param pointer — same hop rule, same loop
// detection, same upscope old-chain walk. The ported fns in
// params.rs/builtin.rs call into these at the exact C deref points
// (getparamnode c:570-575, getvalue/fetchvalue c:2247-2270,
// assignsparam c:3252/3258, assignaparam c:3392-3398, bin_unset
// c:3939-3951, typeset_single c:2032-2050).
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

pub use crate::ported::params::*;
