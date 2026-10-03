//! Plugin-Framework-Agnostic State-Modification Recorder (PFA-SMR).
//!
//! Single-shot indexer. Spawns the user's shell init (or any script the
//! recorder bin was invoked with), captures every state mutation as it
//! flows through a state-mutating dispatcher, prints `Captured ...` to
//! stderr in real time, mirrors every line into the zshrs tracing log,
//! bundles the full set on shell exit, folds it into the canonical rkyv
//! shard (`~/.zshrs/images/{hash8}-recorder.rkyv`), prints summary stats,
//! then exits. No daemon is involved: the shell reads that shard back at
//! startup with `canonical_apply::apply_all`.
//!
//! Per docs/RECORDER.md: only the recorder can capture state at 100%
//! fidelity; the daemon never walks user config. New plugin installs
//! require the user to re-run `zshrs-recorder`.
//!
//! Module gated by `#![cfg(feature = "recorder")]` so the default
//! `zshrs` binary contains zero recorder code.

#![cfg(feature = "recorder")]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

/// Global on/off switch. `bins/zshrs-recorder.rs` calls `enable()` at
/// startup before the executor runs; `bins/zshrs.rs` never touches it
/// (this module doesn't exist in that build).
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Skip the end-of-run shard write. Set by `--dry-run` (alias
/// `--no-daemon`); hermetic tests (`tests/recorder_harness.rs`) use it so
/// a corpus run never replaces the user's recording.
static NO_WRITE: AtomicBool = AtomicBool::new(false);

/// Pid that installed the atexit hook. A forked subshell or `$(...)`
/// inherits the hook and the buffer; only this pid may finalize.
static OWNER_PID: AtomicU64 = AtomicU64::new(0);

/// Suppress the per-event "Captured KIND NAME ..." stderr line. Set
/// by `zshrs-recorder --quiet`. The summary footer + tracing log still
/// fire — only the live-capture firehose is muted.
static QUIET: AtomicBool = AtomicBool::new(false);

/// Emit the end-of-run summary as a single JSON line to stdout
/// instead of the multi-line human text on stderr. Set by
/// `zshrs-recorder --json`.
static JSON_SUMMARY: AtomicBool = AtomicBool::new(false);

/// Optional path to write the bundle to as a JSON file (alongside
/// the shard write, or instead of it under --dry-run). Set by
/// `zshrs-recorder -o PATH`. Lock contention is irrelevant — we set
/// it once at startup and read it once at flush time.
static OUTPUT_PATH: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

/// Optional override for the bundle's `shell_id`. Lets a test (or a
/// rebrand experiment) impersonate a different shell. None = default
/// "zshrs".
static SHELL_ID_OVERRIDE: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

/// Re-entrancy guard — set during emit so any builtin a recorder hook
/// itself triggers is not re-recorded.
static IN_RECORDER: AtomicBool = AtomicBool::new(false);

/// Monotonic per-record sequence number.
static ORDER_IDX: AtomicU64 = AtomicU64::new(0);

/// Recorder start time, used by the summary footer for `runs.started_at_ns`.
static START_NS: AtomicU64 = AtomicU64::new(0);

/// In-process buffer of every captured event. Folded into the shard
/// once at end-of-run.
static BUFFER: Lazy<Mutex<Vec<RecordEvent>>> = Lazy::new(|| Mutex::new(Vec::with_capacity(4096)));

/// Mirror of the SQLite `definitions.kind` discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefKind {
    /// `Alias` variant.
    Alias,
    /// `GAlias` variant.
    GAlias,
    /// `SAlias` variant.
    SAlias,
    /// `Function` variant.
    Function,
    /// `Assign` variant.
    Assign,
    /// `Typeset` variant.
    Typeset,
    /// `Export` variant.
    Export,
    /// `PathMod` variant.
    PathMod,
    /// `HashD` variant.
    HashD,
    /// `Zstyle` variant.
    Zstyle,
    /// `Bindkey` variant.
    Bindkey,
    /// `Compdef` variant.
    Compdef,
    /// `Zmodload` variant.
    Zmodload,
    /// `Setopt` variant.
    Setopt,
    /// `Unsetopt` variant.
    Unsetopt,
    /// `Trap` variant.
    Trap,
    /// `Sched` variant.
    Sched,
    /// `Source` variant.
    Source,
    /// Removal events — RECORDER.md "Open question 4: Should we record
    /// `unalias` / `unset` / `disable` events?". Recorded so `zwhere -l`
    /// lineage can see "this name was defined at A:N, removed at B:M,
    /// redefined at C:K". Without these the override chain is invisible.
    Unalias,
    /// `Unset` variant.
    Unset,
    /// `zle -N WIDGET [FUNC]` — define a new ZLE widget. Distinct from
    /// `bindkey` (which binds a key sequence to a widget) and from
    /// `function` (which defines the underlying handler). Tracked
    /// because zinit-report lists widgets and a `zwhere` query for
    /// widgets is the natural counterpart.
    Zle,
    /// `_completion-name` file discovered in an fpath directory. zinit-
    /// report's "Completions:" section lists these per plugin; the
    /// recorder synthesises one event per `_*` file found whenever an
    /// fpath dir is added (set or appended). Distinct from `compdef`
    /// (which BINDS a completion function to a command — runtime call)
    /// and from `function` (the autoload registration that compinit
    /// will emit when it walks fpath). value field is the absolute
    /// path of the discovered file.
    Completion,
}

impl DefKind {
    /// `as_str` — see implementation.
    pub fn as_str(self) -> &'static str {
        match self {
            DefKind::Alias => "alias",
            DefKind::GAlias => "alias -g",
            DefKind::SAlias => "alias -s",
            DefKind::Function => "function",
            DefKind::Assign => "assign",
            DefKind::Typeset => "typeset",
            DefKind::Export => "export",
            DefKind::PathMod => "path_mod",
            DefKind::HashD => "hash -d",
            DefKind::Zstyle => "zstyle",
            DefKind::Bindkey => "bindkey",
            DefKind::Compdef => "compdef",
            DefKind::Zmodload => "zmodload",
            DefKind::Setopt => "setopt",
            DefKind::Unsetopt => "unsetopt",
            DefKind::Trap => "trap",
            DefKind::Sched => "sched",
            DefKind::Source => "source",
            DefKind::Unalias => "unalias",
            DefKind::Unset => "unset",
            DefKind::Zle => "zle",
            DefKind::Completion => "completion",
        }
    }
}

/// Structured parameter-attribute bitflags. Mirrors zsh's per-param
/// flag set (params.c PM_*) so the recorder records the full attribute
/// vector rather than encoding flags in the value string. Only the
/// `typeset` family populates this; other kinds set it to 0.
///
/// Wire-serialised as `u16` for compactness (zsh has ~12 user-visible
/// attribute flags; one byte would also fit but u16 leaves room for
/// PM_HASHELEM / PM_NAMEDDIR / PM_AUTOLOAD-style additions later
/// without a wire-format bump).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamAttrs(pub u16);

impl ParamAttrs {
    /// `NONE` constant.
    pub const NONE: Self = Self(0);
    /// `SCALAR` constant.
    pub const SCALAR: u16 = 1 << 0;
    /// `INTEGER` constant.
    pub const INTEGER: u16 = 1 << 1;
    /// `FLOAT` constant.
    pub const FLOAT: u16 = 1 << 2;
    /// `ASSOC` constant.
    pub const ASSOC: u16 = 1 << 3;
    /// `ARRAY` constant.
    pub const ARRAY: u16 = 1 << 4;
    /// `READONLY` constant.
    pub const READONLY: u16 = 1 << 5;
    /// `EXPORT` constant.
    pub const EXPORT: u16 = 1 << 6;
    /// `GLOBAL` constant.
    pub const GLOBAL: u16 = 1 << 7;
    /// `UNIQUE` constant.
    pub const UNIQUE: u16 = 1 << 8;
    /// `TIED` constant.
    pub const TIED: u16 = 1 << 9;
    /// `HIDE` constant.
    pub const HIDE: u16 = 1 << 10;
    /// `HIDE_VAL` constant.
    pub const HIDE_VAL: u16 = 1 << 11;
    /// `+=` operation marker. Distinguishes `arr=(a b)` (replace) from
    /// `arr+=(c)` (extend). Replay needs this to drive the right
    /// codepath when reconstructing array state from the bundle.
    pub const APPEND: u16 = 1 << 12;
    /// `set` — see implementation.
    pub fn set(&mut self, mask: u16) {
        self.0 |= mask;
    }
    /// `has` — see implementation.
    pub fn has(self, mask: u16) -> bool {
        self.0 & mask != 0
    }

    /// Parse a zsh `typeset -...` flag-letter sequence ("xrigU" etc.)
    /// into a ParamAttrs bitset. Includes letters from `typeset`,
    /// `integer`, `float`, `readonly`, `local`, `declare`, `export`.
    /// Letters not in the table are silently ignored.
    pub fn from_flag_chars(letters: &str) -> Self {
        let mut a = Self::NONE;
        for c in letters.chars() {
            match c {
                'i' => a.set(Self::INTEGER),
                'F' | 'E' => a.set(Self::FLOAT),
                'A' => a.set(Self::ASSOC),
                'a' => a.set(Self::ARRAY),
                'r' => a.set(Self::READONLY),
                'x' => a.set(Self::EXPORT),
                'g' => a.set(Self::GLOBAL),
                'U' => a.set(Self::UNIQUE),
                'T' => a.set(Self::TIED),
                'h' => a.set(Self::HIDE),
                'H' => a.set(Self::HIDE_VAL),
                _ => {}
            }
        }
        // If no concrete shape was set, mark as scalar (typeset
        // defaults to scalar string semantics).
        if a.0 & (Self::INTEGER | Self::FLOAT | Self::ASSOC | Self::ARRAY) == 0 {
            a.set(Self::SCALAR);
        }
        a
    }
}

/// One state-mutation event. Field set is the recorder's wire format
/// for the daemon `recorder_ingest` op; mirrors the SQL `definitions`
/// row in docs/RECORDER.md §Schema.
///
/// REPLAY-GRADE TYPE INFO. Every assign event carries enough structure
/// to round-trip the parameter exactly:
///
///   - `attrs`: `ParamAttrs` bitset — scalar/integer/float/assoc/array/
///     readonly/export/global/unique/tied. Populated on every emit_*
///     by inspecting the executor's current type for `name` (or the
///     declared type for typeset-family). Without this, replay can't
///     tell whether `EDITOR=vim` should restore as scalar or as a
///     previously-tied array.
///   - `value`: scalar form. Set for scalar / integer / float and as
///     a fallback for arrays/assocs (joined string).
///   - `value_array`: ORDERED element list for indexed-array events.
///     Empty for scalars/assocs. Lets replay reconstruct
///     `arr=(elem1 elem2 elem3)` exactly — the order is preserved
///     even when zsh's `typeset -U` would dedupe.
///   - `value_assoc`: ORDERED (key, value) pairs for assoc events.
///     Empty for scalars/arrays. Insertion order matters for replay
///     determinism (zsh assocs are insertion-ordered).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordEvent {
    /// `order_idx` field.
    pub order_idx: u64,
    /// `ts_ns` field.
    pub ts_ns: u64,
    /// `kind` field.
    pub kind: DefKind,
    /// `name` field.
    pub name: String,
    /// `value` field.
    pub value: Option<String>,
    /// `file` field.
    pub file: Option<String>,
    /// `line` field.
    pub line: Option<u32>,
    /// `fn_chain` field.
    pub fn_chain: Option<String>,
    /// `attrs` field.
    #[serde(default, skip_serializing_if = "is_default_attrs")]
    pub attrs: ParamAttrs,
    /// `value_array` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_array: Option<Vec<String>>,
    /// `value_assoc` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_assoc: Option<Vec<(String, String)>>,
    /// Recording-shell identity for the federated catalog (per
    /// docs/DAEMON_AS_SERVICE.md §"Third-party shell recorders" +
    /// `docs/SHELL_IDS.md`). Distinguishes records from different
    /// shells writing to the same daemon. None = inherit from the
    /// enclosing `RecorderBundle.shell_id` (defaults to "zshrs"
    /// at ingest time when neither is set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_id: Option<String>,
}

fn is_default_attrs(a: &ParamAttrs) -> bool {
    a.0 == 0
}

/// Bundle sent to the daemon at end-of-run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecorderBundle {
    /// `started_at_ns` field.
    pub started_at_ns: u64,
    /// `finished_at_ns` field.
    pub finished_at_ns: u64,
    /// `cmdline` field.
    pub cmdline: String,
    /// `zdotdir` field.
    pub zdotdir: Option<String>,
    /// `home` field.
    pub home: Option<String>,
    /// `events` field.
    pub events: Vec<RecordEvent>,
    /// Federated-catalog shell identity. Falls back to "zshrs" if not
    /// supplied. Per-event `shell_id` overrides this top-level value
    /// for individual records (lets one bundle carry mixed sources).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_id: Option<String>,
    /// End-of-run snapshot from [`capture_end_state`]; the shard fold
    /// prefers it over the events for aliases, functions, parameters and
    /// options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_state: Option<crate::daemon::recorder_shard::EndState>,
}
/// `enable` — see implementation.
#[inline]
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
    START_NS.store(now_ns(), Ordering::Relaxed);
    tracing::info!("recorder: enabled");
}
/// `is_enabled` — see implementation.
#[inline]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Distinct `(kind, name)` definitions captured so far, leaving out the
/// parameter and option churn (`assign`, `typeset`, `unset`, `setopt`,
/// `unsetopt`) that any function body produces. Distinct, so a deferred
/// loader that re-arms the same `sched` entry or trap on every pass of
/// its own scheduler adds nothing. The recorder's deferred-work drain
/// stops once passes stop adding these.
pub fn definition_count() -> usize {
    BUFFER
        .lock()
        .map(|b| {
            b.iter()
                .filter(|e| {
                    !matches!(
                        e.kind,
                        DefKind::Assign
                            | DefKind::Typeset
                            | DefKind::Unset
                            | DefKind::Setopt
                            | DefKind::Unsetopt
                    )
                })
                .map(|e| (e.kind, e.name.as_str()))
                .collect::<std::collections::HashSet<_>>()
                .len()
        })
        .unwrap_or(0)
}

/// Run every `sched` entry pending NOW, whatever its due time, through
/// the ported `checksched` walk (Src/Builtins/sched.c:93). Entries the
/// run schedules itself (`sched +1 …`) carry a future time and wait for
/// the next call. Returns how many entries were pending.
///
/// The recorder never reaches a prompt, so a `sched` deadline never
/// comes due on its own — and zinit turbo (`wait''`) hangs every
/// deferred plugin off `sched`.
pub fn run_pending_sched() -> usize {
    use crate::ported::builtins::sched::schedcmd;
    let mut list = schedcmd::subsh_save();
    let mut pending = 0;
    let mut node = list.as_deref_mut();
    while let Some(sch) = node {
        sch.time = 0;
        pending += 1;
        node = sch.next.as_deref_mut();
    }
    if pending > 0 {
        schedcmd::subsh_restore(list);
        crate::ported::builtins::sched::checksched();
    }
    pending
}

/// Shell state before the first recorded file ran; [`capture_end_state`]
/// keeps only what the files changed.
struct Baseline {
    options: std::collections::HashMap<&'static str, bool>,
    params: std::collections::HashMap<String, crate::daemon::recorder_shard::TypedParam>,
    keymaps: KeymapState,
    widgets: std::collections::HashMap<String, Vec<String>>,
}

/// Set by [`mark_baseline`], consumed by [`capture_end_state`].
static BASELINE: Lazy<Mutex<Option<Baseline>>> = Lazy::new(|| Mutex::new(None));

/// Path of the terminal `zshrs-recorder` gave the recording
/// (`attach_recording_tty`); [`capture_end_state`] marks it in parameter
/// values with `TTY_PLACEHOLDER`.
static RECORDING_TTY: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

/// Mark the values that belong to the recording process in every parameter
/// value — the recording terminal's path (`TTY_PLACEHOLDER`) and its `$$`
/// (`PID_PLACEHOLDER`) — so the replay can put its own there.
fn mark_process_values(params: &mut [crate::daemon::recorder_shard::TypedParam]) {
    use crate::daemon::recorder_shard::{PID_PLACEHOLDER, TTY_PLACEHOLDER};
    let tty = RECORDING_TTY.lock().ok().and_then(|t| t.clone());
    let pid = crate::ported::params::mypid.load(std::sync::atomic::Ordering::Relaxed);
    let pid = (pid > 0).then(|| pid.to_string());
    for p in params {
        for v in p.value.iter_mut().chain(p.elements.iter_mut().flatten()) {
            if let Some(tty) = &tty {
                if v.contains(tty.as_str()) {
                    *v = v.replace(tty.as_str(), TTY_PLACEHOLDER);
                }
            }
            if let Some(pid) = &pid {
                *v = replace_number(v, pid, PID_PLACEHOLDER);
            }
        }
    }
}

/// The files the recorder sources itself (the login chain, or `--file`).
static STARTUP_FILES: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// Record the files the recorder sources itself; see [`STARTUP_FILES`].
pub fn set_startup_files(files: Vec<String>) {
    if let Ok(mut s) = STARTUP_FILES.lock() {
        *s = files;
    }
}

/// Names the recorded files had a descriptor assigned to (`exec {name}>…`,
/// `zsystem flock -f name`, `sysopen -u name`), each with the order of the
/// assignment and the file running it (C's `scriptfilename`).
static DESCRIPTOR_PARAMS: Lazy<Mutex<std::collections::HashMap<String, (u64, Option<String>)>>> =
    Lazy::new(|| Mutex::new(std::collections::HashMap::new()));

/// Called where the shell stores a descriptor in a named parameter
/// (Src/exec.c:2404-2412 `addfd`'s varid arm; Src/Modules/system.c:414,
/// 765). Only these names can be descriptor-holding parameters — matching
/// values against open descriptors alone also caught every small integer
/// setting (`POWERLEVEL9K_DIR_MAX_LENGTH=40` and an open fd 40).
pub fn note_descriptor_param(name: &str) {
    if is_enabled() {
        let file = crate::ported::utils::scriptfilename_get();
        let order = ORDER_IDX.load(Ordering::Relaxed);
        if let Ok(mut s) = DESCRIPTOR_PARAMS.lock() {
            s.insert(name.to_string(), (order, file));
        }
    }
}

/// Pull out of `end.params` every parameter that holds a descriptor the
/// recorded files opened and still hold — named in [`DESCRIPTOR_PARAMS`]
/// and still open as `FDT_EXTERNAL` / `FDT_FLOCK` — and list the file that
/// last assigned each one in `end.resource_files`.
///
/// The number means nothing in another process: replayed, it named
/// whatever that shell had open at the same number, and zconvey wrote
/// `$$` into it (`echo "$$" >&${ZCONVEY_FD}`) and reused the session ID
/// it guarded. The descriptor, its lock and the ID it stands for have to
/// be taken by each shell, so the replay sources those files again.
fn take_descriptor_params(end: &mut crate::daemon::recorder_shard::EndState) {
    use crate::ported::zsh_h::{FDT_EXTERNAL, FDT_FLOCK, FDT_FLOCK_EXEC};
    let held = |fd: i32| {
        matches!(crate::ported::utils::fdtable_get(fd), k if k == FDT_EXTERNAL || k == FDT_FLOCK || k == FDT_FLOCK_EXEC)
    };
    let noted = DESCRIPTOR_PARAMS.lock().map(|s| s.clone()).unwrap_or_default();
    let (fds, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut end.params).into_iter().partition(|p| {
        noted.contains_key(&p.name)
            && p.value.as_deref().and_then(|v| v.parse::<i32>().ok()).is_some_and(|fd| fd >= 10 && held(fd))
    });
    end.params = kept;
    if fds.is_empty() {
        return;
    }
    // The file that took each descriptor, in the order they were taken.
    let mut files: Vec<(u64, String)> = fds
        .iter()
        .filter_map(|p| {
            let (order, file) = noted.get(&p.name)?;
            Some((*order, file.clone()?))
        })
        .collect();
    files.sort();
    // Never a startup file: sourcing `.zshrc` again would be the whole
    // startup the replay replaces. A descriptor taken there is dropped.
    let startup = STARTUP_FILES.lock().map(|s| s.clone()).unwrap_or_default();
    for (_, file) in files {
        if startup.contains(&file) {
            continue;
        }
        if !end.resource_files.contains(&file) {
            end.resource_files.push(file);
        }
    }
    tracing::info!(
        params = ?fds.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        files = ?end.resource_files,
        "recorder: descriptor-holding parameters; their files are sourced again at replay"
    );
}

/// Replace every occurrence of the decimal `number` in `s` that is not
/// part of a longer run of digits — `.temp4242-x` matches 4242,
/// `142429` does not.
fn replace_number(s: &str, number: &str, with: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    let mut from = 0;
    while let Some(off) = s[from..].find(number) {
        let at = from + off;
        let end = at + number.len();
        let digit_before = at > 0 && bytes[at - 1].is_ascii_digit();
        let digit_after = end < bytes.len() && bytes[end].is_ascii_digit();
        if !digit_before && !digit_after {
            out.push_str(&s[last..at]);
            out.push_str(with);
            last = end;
        }
        from = at + 1;
    }
    out.push_str(&s[last..]);
    out
}

/// Record the recording terminal's path; see [`RECORDING_TTY`].
pub fn set_recording_tty(path: Option<String>) {
    if let Ok(mut t) = RECORDING_TTY.lock() {
        *t = path;
    }
}

/// Snapshot taken by [`capture_end_state`], attached to the bundle by
/// [`flush`].
static END_STATE: Lazy<Mutex<Option<crate::daemon::recorder_shard::EndState>>> =
    Lazy::new(|| Mutex::new(None));

/// Options that describe how THIS process was started — the recorder is
/// a non-interactive script runner, the replaying shell usually an
/// interactive login one — never something a config file chose.
const UNRECORDED_OPTIONS: &[&str] = &[
    "interactive",
    "loginshell",
    "shinstdin",
    "singlecommand",
    "privileged",
    "restricted",
    "monitor",
    "zle",
    "rcs",
    "globalrcs",
];

/// Parameters whose value belongs to the running process or to the
/// command that last ran, not to the config: replaying the recorder's
/// copy would be wrong in every shell.
const UNRECORDED_PARAMS: &[&str] = &[
    "_",
    "0",
    "argv",
    "PWD",
    "OLDPWD",
    "SHLVL",
    "SECONDS",
    "RANDOM",
    "LINENO",
    "TTY",
    "TTYIDLE",
    "pipestatus",
    "ZSH_SCRIPT",
    "ZSH_ARGZERO",
    "ZSH_EXECUTION_STRING",
    "ZSH_SUBSHELL",
    "match",
    "mbegin",
    "mend",
    "MATCH",
    "MBEGIN",
    "MEND",
    "reply",
    "REPLY",
];

/// Every canonical option and its state. `OPTNS` also lists the
/// `OPT_ALIAS` rows (`login`, `dotglob`, …); an alias is skipped because
/// its canonical row carries the same state.
fn option_states() -> std::collections::HashMap<&'static str, bool> {
    use crate::ported::options::{optlookup, opt_state_get, OPTNS};
    OPTNS
        .iter()
        .copied()
        .filter(|&n| {
            let no = optlookup(n);
            no > 0 && crate::ported::zsh_h::opt_name(no) == n
        })
        .filter_map(|n| opt_state_get(n).map(|on| (n, on)))
        .collect()
}

/// One global parameter as the replay needs it, or `None` for one that
/// cannot or must not be replayed: read-only, the special hashes
/// (`$aliases`, `$functions`, `$options`, … — views of tables the
/// snapshot carries itself), or a name in [`UNRECORDED_PARAMS`].
fn read_global_param(name: &str) -> Option<crate::daemon::recorder_shard::TypedParam> {
    use crate::ported::params::{getaparam, gethparam, getsparam, paramtab};
    if UNRECORDED_PARAMS.contains(&name) {
        return None;
    }
    // `${(t)name}` (Src/Modules/parameter.c:43 `paramtypestr`), plus the
    // tie partner and separator a `typeset -T` scalar carries (`ename`,
    // `u.tied->joinchar`, Src/zsh.h:1864, 1870-1873).
    let (ty, tie) = paramtab().read().ok().and_then(|t| {
        t.get(name).map(|pm| {
            let tie = pm.ename.clone().map(|arr| {
                let sep = pm.u_tied.as_ref().map(|t| t.joinchar).unwrap_or(b':' as i32);
                (arr, char::from_u32(sep as u32).unwrap_or(':').to_string())
            });
            (crate::ported::modules::parameter::paramtypestr(pm), tie)
        })
    })?;
    let parts: Vec<&str> = ty.split('-').collect();
    let kind = parts[0];
    let special = parts.contains(&"special");
    if parts.contains(&"readonly") || (kind == "association" && special) {
        return None;
    }
    let (value, elements) = match kind {
        "array" => (None, getaparam(name)),
        // `gethparam` is the VALUES only (Src/params.c:3118
        // `SCANPM_WANTVALS`); `gethkparam` the keys, in the same scan
        // order (c:3138).
        "association" => {
            let keys = crate::ported::params::gethkparam(name).unwrap_or_default();
            let vals = gethparam(name).unwrap_or_default();
            let flat = keys.into_iter().zip(vals).flat_map(|(k, v)| [k, v]).collect();
            (None, Some(flat))
        }
        "scalar" | "integer" | "float" => (getsparam(name), None),
        _ => return None,
    };
    // The `paramtypestr` words (c:72-90) that `typeset` sets with a
    // plain letter.
    let attrs: String = [("unique", 'U'), ("hideval", 'H'), ("hide", 'h'), ("lower", 'l'), ("upper", 'u'), ("tag", 't')]
        .iter()
        .filter(|(word, _)| parts.contains(word))
        .map(|(_, c)| *c)
        .collect();
    Some(crate::daemon::recorder_shard::TypedParam {
        name: name.to_string(),
        kind: kind.to_string(),
        export: parts.contains(&"export"),
        value,
        elements,
        attrs,
        // A special pair (`PATH`/`path`) is tied in every shell already.
        tie: tie.filter(|_| kind == "scalar" && parts.contains(&"tied") && !special),
    })
}

/// Every global parameter the replay could restore, keyed by name.
fn global_params() -> std::collections::HashMap<String, crate::daemon::recorder_shard::TypedParam> {
    // Names first: the readers below take `paramtab` themselves.
    let names: Vec<String> = crate::ported::params::paramtab()
        .read()
        .map(|t| t.iter().map(|(n, _)| n.clone()).collect())
        .unwrap_or_default();
    names
        .into_iter()
        .filter_map(|n| read_global_param(&n).map(|p| (n, p)))
        .collect()
}

/// Every loaded module and its enabled features — what `zmodload -LF`
/// prints (`bin_zmodload_features`, Src/module.c:3022-3024 via
/// `printmodulenode` c:218-262). Replaying `zmodload -F MODULE f…` gives a
/// module exactly these features whether the files ran a bare
/// `zmodload zsh/datetime` or `zmodload -F zsh/files b:zf_rm`.
fn loaded_module_features() -> std::collections::HashMap<String, Vec<String>> {
    use crate::ported::module::{enables_module, features_module, MODULESTAB};
    use crate::ported::zsh_h::{MOD_ALIAS, MOD_INIT_B, MOD_UNLOAD};
    let mut out = std::collections::HashMap::new();
    let Ok(mut table) = MODULESTAB.lock() else {
        return out;
    };
    let table = &mut *table;
    let names: Vec<String> = table
        .modules
        .iter()
        .filter(|(_, m)| {
            let f = m.node.flags;
            f & MOD_ALIAS == 0 && f & MOD_INIT_B != 0 && f & MOD_UNLOAD == 0
        })
        .map(|(n, _)| n.clone())
        .collect();
    for name in names {
        let mut features = Vec::new();
        if features_module(table, &name, &mut features) != 0 {
            continue;
        }
        // A module with no features (`zsh/complist`) is loaded or not;
        // an empty list replays as a plain `zmodload MODULE`.
        if features.is_empty() {
            out.insert(name, Vec::new());
            continue;
        }
        let mut enables = None;
        if enables_module(table, &name, &mut enables) != 0 {
            continue;
        }
        let enables = enables.unwrap_or_default();
        let on: Vec<String> = features
            .into_iter()
            .zip(enables)
            .filter(|(_, e)| *e != 0)
            .map(|(f, _)| f)
            .collect();
        out.insert(name, on);
    }
    out
}

/// What one key sequence does in a keymap: run a widget, or send a
/// string (`bindkey -s`).
#[derive(Clone, PartialEq)]
enum KeyAction {
    Widget(String),
    Send(String),
}

/// Every keymap, as `bindkey -lL` and `bindkey -LM` would show it.
struct KeymapState {
    /// Keymap name → the name its bindings are listed under. Names that
    /// share one keymap (`main` after `bindkey -v` is `viins`) map to
    /// that keymap's primary name (Src/Zle/zle_keymap.c:78 `primary`).
    names: std::collections::HashMap<String, String>,
    /// Primary name → its bindings.
    binds: std::collections::HashMap<String, std::collections::HashMap<Vec<u8>, KeyAction>>,
}

/// Read `keymapnamtab`.
fn keymap_state() -> KeymapState {
    use crate::ported::zle::zle_keymap::keymapnamtab;
    let mut state = KeymapState {
        names: Default::default(),
        binds: Default::default(),
    };
    let Ok(tab) = keymapnamtab().lock() else {
        return state;
    };
    let named: Vec<(String, std::sync::Arc<crate::ported::zle::zle_keymap::Keymap>)> =
        tab.iter().map(|(n, kmn)| (n.clone(), kmn.keymap.clone())).collect();
    drop(tab);
    for (name, km) in &named {
        // The primary name when the keymap has one and it is linked,
        // else the alphabetically first name sharing the keymap.
        let mut sharing: Vec<&String> = named
            .iter()
            .filter(|(_, other)| std::sync::Arc::ptr_eq(km, other))
            .map(|(n, _)| n)
            .collect();
        sharing.sort();
        let primary = km
            .primary
            .as_ref()
            .filter(|p| sharing.contains(p))
            .unwrap_or(sharing[0])
            .clone();
        state.names.insert(name.clone(), primary.clone());
        if state.binds.contains_key(&primary) {
            continue;
        }
        let mut binds = std::collections::HashMap::new();
        for (byte, t) in km.first.iter().enumerate() {
            if let Some(t) = t.as_ref().filter(|t| t.nam != "undefined-key") {
                binds.insert(vec![byte as u8], KeyAction::Widget(t.nam.clone()));
            }
        }
        for (seq, kb) in &km.multi {
            // A `multi` node with neither is only a prefix of longer
            // sequences (c:85-91 `prefixct`).
            let action = match (&kb.bind, &kb.str) {
                (Some(t), _) if t.nam != "undefined-key" => KeyAction::Widget(t.nam.clone()),
                (_, Some(s)) => KeyAction::Send(s.clone()),
                _ => continue,
            };
            binds.insert(seq.clone(), action);
        }
        state.binds.insert(primary, binds);
    }
    state
}

/// A key sequence as `\NNN` octal escapes, one per byte. `bindkey`
/// decodes both its sequence and its `-s` string with
/// `GETKEYS_BINDKEY`, which includes `GETKEY_OCTAL_ESC` (Src/zsh.h:3141,
/// 3185; Src/Zle/zle_keymap.c:1022, 1038), so any byte — NUL, `^`, `\`,
/// high-bit — round-trips without quoting.
fn key_escape(seq: &[u8]) -> String {
    seq.iter().map(|b| format!("\\{b:03o}")).collect()
}

/// The `bindkey` argvs that turn keymap state `base` into `end`: new
/// keymaps (`-N`), links (`-A`), then per-keymap binds and removals
/// (`-M KEYMAP SEQ WIDGET`, `-s`, `-r`), keys as [`key_escape`] writes them.
fn keymap_diff(base: &KeymapState, end: &KeymapState) -> Vec<Vec<String>> {
    let mut created = Vec::new();
    let mut linked = Vec::new();
    let mut names: Vec<&String> = end.names.keys().collect();
    names.sort();
    for name in names {
        let primary = &end.names[name];
        if name == primary {
            if !base.names.contains_key(name) {
                created.push(vec!["-N".to_string(), name.clone()]);
            }
        } else if base.names.get(name) != Some(primary) {
            linked.push(vec!["-A".to_string(), primary.clone(), name.clone()]);
        }
    }

    let empty = std::collections::HashMap::new();
    let mut bound = Vec::new();
    let mut primaries: Vec<&String> = end.binds.keys().collect();
    primaries.sort();
    for km in primaries {
        let before = base
            .names
            .get(km)
            .and_then(|p| base.binds.get(p))
            .unwrap_or(&empty);
        let after = &end.binds[km];
        let mut seqs: Vec<&Vec<u8>> = after.keys().chain(before.keys()).collect();
        seqs.sort();
        seqs.dedup();
        for seq in seqs {
            let mut argv = vec!["-M".to_string(), km.clone()];
            match (before.get(seq), after.get(seq)) {
                (b, Some(a)) if b == Some(a) => continue,
                (_, Some(KeyAction::Widget(w))) => argv.extend([key_escape(seq), w.clone()]),
                (_, Some(KeyAction::Send(s))) => {
                    argv.extend(["-s".to_string(), key_escape(seq), key_escape(s.as_bytes())])
                }
                (Some(_), None) => argv.extend(["-r".to_string(), key_escape(seq)]),
                (None, None) => continue,
            }
            bound.push(argv);
        }
    }
    created.into_iter().chain(linked).chain(bound).collect()
}

/// User widgets in `thingytab`, name → the `zle` argv that defines it
/// (`zle -N NAME FUNC`, `zle -C NAME WIDGET FUNC`).
fn widget_state() -> std::collections::HashMap<String, Vec<String>> {
    use crate::ported::zle::zle_h::WidgetImpl;
    let Ok(tab) = crate::ported::zle::zle_thingy::thingytab().lock() else {
        return Default::default();
    };
    tab.iter()
        .filter_map(|(name, t)| {
            let argv = match &t.widget.as_ref()?.u {
                WidgetImpl::UserFunc(f) => vec!["-N".to_string(), name.clone(), f.clone()],
                WidgetImpl::Comp { wid, func, .. } => {
                    vec!["-C".to_string(), name.clone(), wid.clone(), func.clone()]
                }
                WidgetImpl::Internal(_) => return None,
            };
            Some((name.clone(), argv))
        })
        .collect()
}

/// Bring up the line editor's tables the way `zsh_main` does for an
/// interactive shell (Src/init.c, `init.rs` `zle_load_state` block), so
/// the baseline holds the keymaps and widgets the replaying shell starts
/// with. The recorder never runs `zsh_main`.
fn init_zle_tables() {
    use crate::ported::zle::zle_keymap::{createkeymapnamtab, default_bindings, keymapnamtab};
    crate::ported::zle::zle_thingy::init_thingies();
    createkeymapnamtab();
    // `default_bindings` rebuilds every keymap; only on an empty table.
    if keymapnamtab().lock().map(|t| !t.contains_key("main")).unwrap_or(false) {
        default_bindings();
    }
}

/// Record the option and parameter state before any file is sourced.
pub fn mark_baseline() {
    init_zle_tables();
    let base = Baseline {
        options: option_states(),
        params: global_params(),
        keymaps: keymap_state(),
        widgets: widget_state(),
    };
    if let Ok(mut b) = BASELINE.lock() {
        *b = Some(base);
    }
}

/// Snapshot what the init chain left behind — aliases, functions,
/// autoload stubs, and the global parameters and options that differ
/// from [`mark_baseline`] — for the shard fold to use instead of
/// replaying events. Read from the live tables, not the event log:
/// events miss `typeset -g x=(…)` and cannot say what `local`,
/// `localoptions`, `unalias` or `unfunction` left behind. Run after the
/// deferred-work drain, while the executor is still alive; `flush`
/// attaches it to the bundle.
pub fn capture_end_state() {
    use crate::daemon::recorder_shard::EndState;
    use crate::ported::hashtable::{aliastab_lock, shfunctab_lock, sufaliastab_lock};
    use crate::ported::zsh_h::{
        ALIAS_GLOBAL, DISABLED, MFF_STR, MFF_USERFUNC, PM_KSHSTORED, PM_LOADDIR, PM_TAGGED,
        PM_TAGGED_LOCAL, PM_UNALIASED, PM_UNDEFINED, PM_ZSHSTORED,
    };

    let mut end = EndState::default();
    if let Ok(tab) = aliastab_lock().read() {
        for (name, a) in tab.iter() {
            if a.node.flags & DISABLED != 0 {
                continue;
            }
            let bucket = if a.node.flags & ALIAS_GLOBAL != 0 {
                &mut end.global_aliases
            } else {
                &mut end.aliases
            };
            bucket.insert(name.clone(), a.text.clone());
        }
    }
    if let Ok(tab) = sufaliastab_lock().read() {
        for (name, a) in tab.iter() {
            if a.node.flags & DISABLED == 0 {
                end.suffix_aliases.insert(name.clone(), a.text.clone());
            }
        }
    }

    // A defined function's body is the source text the shell itself
    // compiles from (`shfunc.body`). The `$functions[name]` deparse is
    // the fallback only: zshrs's deparse loses quoting
    // (`'_.!~*'\''()-'` comes back as `'_.!~*'()-'`) and so does not
    // always parse again.
    let mut deparse: Vec<String> = Vec::new();
    if let Ok(tab) = shfunctab_lock().read() {
        for (name, shf) in tab.iter() {
            if shf.node.flags & DISABLED != 0 {
                continue;
            }
            let flags = shf.node.flags as u32;
            if flags & PM_UNDEFINED != 0 {
                match shf.filename.as_ref().filter(|_| flags & PM_LOADDIR != 0) {
                    // `autoload -Uz DIR/NAME` (Src/builtin.c:3288-3290 `add_autoload_function`):
                    // the function comes from DIR, not `$fpath`.
                    Some(dir) => {
                        let mut letters = String::from("-");
                        for (bit, c) in [
                            (PM_UNALIASED, 'U'),
                            (PM_ZSHSTORED, 'z'),
                            (PM_KSHSTORED, 'k'),
                            (PM_TAGGED, 't'),
                            (PM_TAGGED_LOCAL, 'T'),
                        ] {
                            if flags & bit != 0 {
                                letters.push(c);
                            }
                        }
                        let mut argv = Vec::new();
                        if letters.len() > 1 {
                            argv.push(letters);
                        }
                        argv.push(format!("{dir}/{name}"));
                        end.autoload_paths.push((name.clone(), argv));
                    }
                    None => end.autoloads.push(name.clone()),
                }
            } else if let Some(body) = shf.body.as_ref().filter(|b| !b.is_empty()) {
                end.functions.insert(name.clone(), body.clone());
            } else {
                deparse.push(name.clone());
            }
        }
    }
    end.autoloads.sort();
    end.autoload_paths.sort();

    // User math functions, as the `functions -M` argv `listusermathfunc`
    // prints for each (Src/builtin.c:3243-3277): the min/max/function
    // words appear only as far as they differ from the defaults.
    if let Ok(table) = crate::ported::module::MATHFUNCS.lock() {
        for p in table.iter().filter(|p| p.flags & MFF_USERFUNC != 0) {
            let mut showargs = if p.module.is_some() {
                3
            } else if p.maxargs != if p.minargs != 0 { p.minargs } else { -1 } {
                2
            } else if p.minargs != 0 {
                1
            } else {
                0
            };
            let mut argv = vec![
                if p.flags & MFF_STR != 0 { "-Ms" } else { "-M" }.to_string(),
                p.name.clone(),
            ];
            for word in [p.minargs.to_string(), p.maxargs.to_string(), p.module.clone().unwrap_or_default()] {
                if showargs == 0 {
                    break;
                }
                argv.push(word);
                showargs -= 1;
            }
            end.math_functions.push((p.name.clone(), argv));
        }
    }
    end.math_functions.sort();

    if let Ok(t) = crate::ported::modules::zutil::zstyletab.lock() {
        end.zstyles = t.entries();
    }

    // Src/Modules/parameter.c:388 `getfunction`, read after the table
    // guard drops (it takes the same lock).
    for name in deparse {
        let body = crate::ported::modules::parameter::getpmfunction(std::ptr::null_mut(), &name)
            .and_then(|pm| pm.u_str);
        if let Some(body) = body {
            end.functions.insert(name, body);
        }
    }

    if let Some(base) = BASELINE.lock().ok().and_then(|mut b| b.take()) {
        let now = global_params();
        // Gone since the baseline, or unset by the files outright — the
        // recorder starts from a scrubbed environment, so a variable a
        // session can carry (`unset CDPATH`) may never have been in its
        // baseline to go missing.
        let unset_by_files: Vec<String> = BUFFER
            .lock()
            .map(|b| {
                b.iter()
                    .filter(|e| e.kind == DefKind::Unset && !e.name.contains('['))
                    .map(|e| e.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let mut unset: Vec<String> = base
            .params
            .keys()
            .cloned()
            .chain(unset_by_files)
            .filter(|n| !now.contains_key(n) && !UNRECORDED_PARAMS.contains(&n.as_str()))
            .collect();
        unset.sort();
        unset.dedup();
        end.unset_params = unset;
        // Every global the files assigned, plus any other that changed. A
        // diff against the baseline alone dropped what the files set to the
        // value the recorder already inherited: zpwr exports
        // ZPWR_SEND_KEYS_PANE=-1, the recorder's parent shell had exported
        // the same, and a shell replayed from a clean environment had none
        // (`(( $ZPWR_SEND_KEYS_PANE != -1 ))`: operand expected).
        let assigned: std::collections::HashSet<String> = BUFFER
            .lock()
            .map(|b| {
                b.iter()
                    .filter(|e| matches!(e.kind, DefKind::Assign | DefKind::Typeset | DefKind::Export))
                    .map(|e| e.name.split('[').next().unwrap_or(&e.name).to_string())
                    .collect()
            })
            .unwrap_or_default();
        let mut params: Vec<_> = now
            .into_values()
            .filter(|p| assigned.contains(&p.name) || base.params.get(&p.name) != Some(p))
            .collect();
        params.sort_by(|a, b| a.name.cmp(&b.name));
        end.params = params;
        take_descriptor_params(&mut end);
        mark_process_values(&mut end.params);

        for (name, on) in option_states() {
            if UNRECORDED_OPTIONS.contains(&name) || base.options.get(name) == Some(&on) {
                continue;
            }
            if on {
                end.setopts.push(name.to_string());
            } else {
                end.unsetopts.push(name.to_string());
            }
        }
        end.setopts.sort();
        end.unsetopts.sort();

        // Every loaded module, not a diff: the recorder's baseline already
        // has modules the replaying shell may not (`zsh/complist`, whose
        // `.menu-select` a recorded `zle -C menu-select` needs).
        let mut modules: Vec<(String, Vec<String>)> = loaded_module_features().into_iter().collect();
        modules.sort();
        end.modules = modules;

        let mut widgets: Vec<(String, Vec<String>)> = widget_state()
            .into_iter()
            .filter(|(name, argv)| base.widgets.get(name) != Some(argv))
            .collect();
        widgets.sort();
        end.widgets = widgets.into_iter().map(|(_, argv)| argv).collect();
        end.bindkeys = keymap_diff(&base.keymaps, &keymap_state());
    }

    tracing::info!(
        aliases = end.aliases.len() + end.global_aliases.len() + end.suffix_aliases.len(),
        functions = end.functions.len(),
        autoloads = end.autoloads.len(),
        params = end.params.len(),
        unset_params = end.unset_params.len(),
        modules = end.modules.len(),
        autoload_paths = end.autoload_paths.len(),
        math_functions = end.math_functions.len(),
        zstyles = end.zstyles.len(),
        widgets = end.widgets.len(),
        bindkeys = end.bindkeys.len(),
        setopts = end.setopts.len(),
        unsetopts = end.unsetopts.len(),
        "recorder: end state captured"
    );
    if let Ok(mut s) = END_STATE.lock() {
        *s = Some(end);
    }
}

/// Free-fn equivalent of `ShellExecutor::recorder_ctx()` — the same
/// RecordCtx built from the shell's parameter table (`$LINENO`,
/// `$funcstack`) and C's `scriptfilename` global, so canonical free-fn
/// ports of C builtins (which don't take a `&ShellExecutor`) can emit
/// recorder events without the executor in scope. These are shell
/// parameters, not environment variables: reading them through
/// `std::env` left every event's file and line unset.
pub fn recorder_ctx_global() -> RecordCtx {
    let line = crate::ported::params::getsparam("LINENO").and_then(|s| s.parse::<u32>().ok());
    let file = crate::ported::utils::scriptfilename_get();
    let fn_chain = crate::ported::params::getaparam("funcstack").and_then(|s| {
        if s.is_empty() {
            None
        } else {
            let mut parts: Vec<&str> = s.iter().map(String::as_str).collect();
            parts.reverse();
            Some(parts.join(" > "))
        }
    });
    RecordCtx {
        file,
        line,
        fn_chain,
    }
}
/// Skip the end-of-run shard write (`--dry-run`).
#[inline]
pub fn set_no_write(v: bool) {
    NO_WRITE.store(v, Ordering::Relaxed);
}

#[inline]
fn no_write() -> bool {
    NO_WRITE.load(Ordering::Relaxed)
}
/// `set_quiet` — see implementation.
#[inline]
pub fn set_quiet(v: bool) {
    QUIET.store(v, Ordering::Relaxed);
}

#[inline]
fn quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}
/// `set_json_summary` — see implementation.
#[inline]
pub fn set_json_summary(v: bool) {
    JSON_SUMMARY.store(v, Ordering::Relaxed);
}

#[inline]
fn json_summary_enabled() -> bool {
    JSON_SUMMARY.load(Ordering::Relaxed)
}
/// `set_output_path` — see implementation.
pub fn set_output_path(p: Option<String>) {
    if let Ok(mut g) = OUTPUT_PATH.lock() {
        *g = p;
    }
}

fn output_path() -> Option<String> {
    OUTPUT_PATH.lock().ok().and_then(|g| g.clone())
}
/// `set_shell_id_override` — see implementation.
pub fn set_shell_id_override(s: Option<String>) {
    if let Ok(mut g) = SHELL_ID_OVERRIDE.lock() {
        *g = s;
    }
}

fn shell_id_override() -> Option<String> {
    SHELL_ID_OVERRIDE.lock().ok().and_then(|g| g.clone())
}

fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Per-call site context derived from the executor (current `$LINENO`,
/// current source file, current `$funcstack`).
#[derive(Debug, Clone, Default)]
pub struct RecordCtx {
    /// `file` field.
    pub file: Option<String>,
    /// `line` field.
    pub line: Option<u32>,
    /// `fn_chain` field.
    pub fn_chain: Option<String>,
}

fn loc_str(file: &Option<String>, line: Option<u32>) -> String {
    match (file.as_deref(), line) {
        (Some(f), Some(l)) => format!("{}:{}", f, l),
        (Some(f), None) => f.to_string(),
        _ => "<unknown>".to_string(),
    }
}

fn fn_chain_suffix(chain: &Option<String>) -> String {
    match chain.as_deref() {
        Some(c) if !c.is_empty() => format!(" ({})", c),
        _ => String::new(),
    }
}

/// Push a record. Real-time stderr line + tracing log line + push to
/// in-process buffer. Re-entrancy-guarded.
pub fn emit(
    kind: DefKind,
    name: impl Into<String>,
    value: Option<String>,
    file: Option<String>,
    line: Option<u32>,
    fn_chain: Option<String>,
) {
    emit_with_attrs(kind, name, value, file, line, fn_chain, ParamAttrs::NONE)
}

/// Push a record with explicit ParamAttrs. Used by typeset-family
/// dispatchers so the structured attrs ride alongside the record.
pub fn emit_with_attrs(
    kind: DefKind,
    name: impl Into<String>,
    value: Option<String>,
    file: Option<String>,
    line: Option<u32>,
    fn_chain: Option<String>,
    attrs: ParamAttrs,
) {
    emit_full(kind, name, value, file, line, fn_chain, attrs, None, None)
}

/// Full emit with replay-grade structured payload. Array and assoc
/// hooks call this directly so element ordering / key-value pairs
/// survive the wire trip to the daemon for exact reconstruction.
#[allow(clippy::too_many_arguments)]
pub fn emit_full(
    kind: DefKind,
    name: impl Into<String>,
    value: Option<String>,
    file: Option<String>,
    line: Option<u32>,
    fn_chain: Option<String>,
    attrs: ParamAttrs,
    value_array: Option<Vec<String>>,
    value_assoc: Option<Vec<(String, String)>>,
) {
    if !is_enabled() {
        return;
    }
    if IN_RECORDER.swap(true, Ordering::Acquire) {
        return;
    }
    let name = name.into();

    // Realtime "Captured ..." line, format per docs/RECORDER.md user spec.
    let value_part = match value.as_deref() {
        Some(v) => format!("={}", short_value(v)),
        None => String::new(),
    };
    let loc = loc_str(&file, line);
    let chain = fn_chain_suffix(&fn_chain);
    let kind_str = kind.as_str();
    let attrs_part = if attrs.0 == 0 {
        String::new()
    } else {
        format!(" [{}]", attrs_to_str(attrs))
    };
    if !quiet() {
        eprintln!(
            "Captured {} {}{}{}, file: {}{}",
            kind_str, name, attrs_part, value_part, loc, chain
        );
    }
    tracing::info!(
        kind = kind_str,
        %name,
        value = value.as_deref().unwrap_or(""),
        attrs = attrs.0,
        file = file.as_deref().unwrap_or(""),
        line = line.unwrap_or(0),
        fn_chain = fn_chain.as_deref().unwrap_or(""),
        "recorder: captured"
    );

    let ev = RecordEvent {
        order_idx: ORDER_IDX.fetch_add(1, Ordering::Relaxed),
        ts_ns: now_ns(),
        kind,
        name,
        value,
        file,
        line,
        fn_chain,
        attrs,
        value_array,
        value_assoc,
        // Per-event shell_id stays None — the bundle's top-level
        // shell_id at flush time tells the daemon which shell these
        // events came from. Shaving N bytes per event matters at
        // recorder scale (zpwr ingest = ~20k events).
        shell_id: None,
    };
    if let Ok(mut buf) = BUFFER.lock() {
        buf.push(ev);
    }
    IN_RECORDER.store(false, Ordering::Release);
}

/// Format ParamAttrs as a comma-joined human label for the realtime
/// stderr line (`[scalar,export,readonly]` etc.). Wire format stays
/// the raw u16.
fn attrs_to_str(a: ParamAttrs) -> String {
    let mut parts: Vec<&'static str> = Vec::new();
    if a.has(ParamAttrs::INTEGER) {
        parts.push("integer");
    }
    if a.has(ParamAttrs::FLOAT) {
        parts.push("float");
    }
    if a.has(ParamAttrs::ASSOC) {
        parts.push("assoc");
    }
    if a.has(ParamAttrs::ARRAY) {
        parts.push("array");
    }
    if a.has(ParamAttrs::SCALAR) && parts.is_empty() {
        parts.push("scalar");
    }
    if a.has(ParamAttrs::READONLY) {
        parts.push("readonly");
    }
    if a.has(ParamAttrs::EXPORT) {
        parts.push("export");
    }
    if a.has(ParamAttrs::GLOBAL) {
        parts.push("global");
    }
    if a.has(ParamAttrs::UNIQUE) {
        parts.push("unique");
    }
    if a.has(ParamAttrs::TIED) {
        parts.push("tied");
    }
    if a.has(ParamAttrs::HIDE) {
        parts.push("hide");
    }
    if a.has(ParamAttrs::HIDE_VAL) {
        parts.push("hideval");
    }
    if a.has(ParamAttrs::APPEND) {
        parts.push("append");
    }
    parts.join(",")
}

/// Truncate values for the realtime stderr line. SQL/IPC store the full
/// value; only the human readout is trimmed.
fn short_value(s: &str) -> String {
    const MAX: usize = 120;
    let single = s.replace('\n', "\\n");
    if single.chars().count() <= MAX {
        format!("\"{}\"", single)
    } else {
        let mut clipped: String = single.chars().take(MAX).collect();
        clipped.push('…');
        format!("\"{}\"", clipped)
    }
}

/// Per-builtin convenience wrappers — one per state-mutating dispatcher.
pub fn emit_alias(name: &str, value: Option<&str>, ctx: RecordCtx) {
    emit(
        DefKind::Alias,
        name,
        value.map(str::to_string),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
/// `emit_galias` — see implementation.
pub fn emit_galias(name: &str, value: Option<&str>, ctx: RecordCtx) {
    emit(
        DefKind::GAlias,
        name,
        value.map(str::to_string),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
/// `emit_salias` — see implementation.
pub fn emit_salias(name: &str, value: Option<&str>, ctx: RecordCtx) {
    emit(
        DefKind::SAlias,
        name,
        value.map(str::to_string),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
/// `emit_function` — see implementation.
pub fn emit_function(name: &str, body: Option<&str>, ctx: RecordCtx) {
    emit(
        DefKind::Function,
        name,
        body.map(str::to_string),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
/// `emit_assign` — see implementation.
pub fn emit_assign(name: &str, value: &str, ctx: RecordCtx) {
    emit(
        DefKind::Assign,
        name,
        Some(value.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}

/// Scalar-assign with structured attrs. Call sites that have already
/// inspected the executor for the parameter's declared type
/// (`var_attrs.kind`, `readonly`, `export`) pass a populated
/// `ParamAttrs` here so the recorded event tells replay exactly how
/// to declare the variable. Without populated attrs, replay would
/// have to guess scalar vs array vs integer from the value string —
/// guesses break when `EDITOR=vim` was previously declared `typeset
/// -gx EDITOR`, since plain replay loses the export/global bits.
pub fn emit_assign_typed(name: &str, value: &str, attrs: ParamAttrs, ctx: RecordCtx) {
    emit_with_attrs(
        DefKind::Assign,
        name,
        Some(value.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
        attrs,
    );
}

/// Indexed-array assign with the ordered element list preserved.
/// `is_append` controls a SET vs APPEND attribute bit so replay knows
/// whether to start the array fresh or extend an existing one.
pub fn emit_array_assign(
    name: &str,
    elements: Vec<String>,
    mut attrs: ParamAttrs,
    is_append: bool,
    ctx: RecordCtx,
) {
    attrs.set(ParamAttrs::ARRAY);
    if is_append {
        attrs.set(ParamAttrs::APPEND);
    }
    let joined = elements.join(" ");
    emit_full(
        DefKind::Assign,
        name,
        Some(joined),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
        attrs,
        Some(elements),
        None,
    );
}

/// Assoc-array assign with insertion-ordered (key, value) pairs.
/// `is_append` distinguishes `h=(...)` (replace) from `h+=(...)`
/// / `h[k]=v` element-add semantics.
pub fn emit_assoc_assign(
    name: &str,
    pairs: Vec<(String, String)>,
    mut attrs: ParamAttrs,
    is_append: bool,
    ctx: RecordCtx,
) {
    attrs.set(ParamAttrs::ASSOC);
    if is_append {
        attrs.set(ParamAttrs::APPEND);
    }
    // Joined key/value pairs as the scalar fallback for clients that
    // only read `value`.
    let joined = pairs
        .iter()
        .flat_map(|(k, v)| [k.as_str(), v.as_str()])
        .collect::<Vec<_>>()
        .join(" ");
    emit_full(
        DefKind::Assign,
        name,
        Some(joined),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
        attrs,
        None,
        Some(pairs),
    );
}
/// Backwards-compat: legacy emit_typeset used by sites that haven't
/// migrated to structured attrs yet. Parses the leading flags out of
/// `flags_value` (everything starting with `-`) and routes to the
/// attrs-aware emitter. New call sites should use `emit_typeset_attrs`.
pub fn emit_typeset(name: &str, flags_value: &str, ctx: RecordCtx) {
    let mut letters = String::new();
    let mut value_part = String::new();
    let mut iter = flags_value.split_whitespace();
    while let Some(tok) = iter.next() {
        if let Some(rest) = tok.strip_prefix('-') {
            letters.push_str(rest);
        } else if let Some(rest) = tok.strip_prefix('+') {
            // +F means "unset float attribute"; for the recorder we
            // don't currently distinguish set/unset of attrs — record
            // the letters as-is.
            letters.push_str(rest);
        } else {
            if !value_part.is_empty() {
                value_part.push(' ');
            }
            value_part.push_str(tok);
        }
    }
    let attrs = ParamAttrs::from_flag_chars(&letters);
    let value_opt = if value_part.is_empty() {
        None
    } else {
        Some(value_part)
    };
    emit_with_attrs(
        DefKind::Typeset,
        name,
        value_opt,
        ctx.file,
        ctx.line,
        ctx.fn_chain,
        attrs,
    );
}

/// Typed typeset emitter — call sites that already have attrs in hand
/// (e.g. `builtin_integer` knows it's emitting an integer) skip the
/// flag-string round-trip.
pub fn emit_typeset_attrs(name: &str, value: Option<&str>, attrs: ParamAttrs, ctx: RecordCtx) {
    emit_with_attrs(
        DefKind::Typeset,
        name,
        value.map(str::to_string),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
        attrs,
    );
}
/// `emit_export` — see implementation.
pub fn emit_export(name: &str, value: Option<&str>, ctx: RecordCtx) {
    emit(
        DefKind::Export,
        name,
        value.map(str::to_string),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
/// `emit_path_mod` — see implementation.
pub fn emit_path_mod(name: &str, op: &str, ctx: RecordCtx) {
    emit(
        DefKind::PathMod,
        name,
        Some(op.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
/// `emit_hash_d` — see implementation.
pub fn emit_hash_d(name: &str, path: &str, ctx: RecordCtx) {
    emit(
        DefKind::HashD,
        name,
        Some(path.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
/// `emit_zstyle` — see implementation.
pub fn emit_zstyle(pattern: &str, rest: &str, ctx: RecordCtx) {
    emit(
        DefKind::Zstyle,
        pattern,
        Some(rest.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
/// `emit_bindkey` — see implementation.
pub fn emit_bindkey(seq: &str, widget: &str, ctx: RecordCtx) {
    emit(
        DefKind::Bindkey,
        seq,
        Some(widget.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
/// `emit_compdef` — see implementation.
pub fn emit_compdef(func: &str, cmds: &str, ctx: RecordCtx) {
    emit(
        DefKind::Compdef,
        func,
        Some(cmds.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
/// `emit_zmodload` — see implementation.
pub fn emit_zmodload(module: &str, flags: &str, ctx: RecordCtx) {
    emit(
        DefKind::Zmodload,
        module,
        Some(flags.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
/// `emit_setopt` — see implementation.
pub fn emit_setopt(opt: &str, ctx: RecordCtx) {
    emit(DefKind::Setopt, opt, None, ctx.file, ctx.line, ctx.fn_chain);
}
/// `emit_unsetopt` — see implementation.
pub fn emit_unsetopt(opt: &str, ctx: RecordCtx) {
    emit(
        DefKind::Unsetopt,
        opt,
        None,
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
/// `emit_trap` — see implementation.
pub fn emit_trap(sig: &str, handler: &str, ctx: RecordCtx) {
    emit(
        DefKind::Trap,
        sig,
        Some(handler.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
/// `emit_sched` — see implementation.
pub fn emit_sched(when: &str, cmd: &str, ctx: RecordCtx) {
    emit(
        DefKind::Sched,
        when,
        Some(cmd.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
/// `emit_source` — see implementation.
pub fn emit_source(path: &str, ctx: RecordCtx) {
    emit(
        DefKind::Source,
        path,
        None,
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
/// `emit_unalias` — see implementation.
pub fn emit_unalias(name: &str, ctx: RecordCtx) {
    emit(
        DefKind::Unalias,
        name,
        None,
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
/// `emit_unset` — see implementation.
pub fn emit_unset(name: &str, ctx: RecordCtx) {
    emit(DefKind::Unset, name, None, ctx.file, ctx.line, ctx.fn_chain);
}
/// `emit_zle` — see implementation.
pub fn emit_zle(widget: &str, func: Option<&str>, ctx: RecordCtx) {
    emit(
        DefKind::Zle,
        widget,
        func.map(str::to_string),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
/// `emit_completion` — see implementation.
pub fn emit_completion(name: &str, abs_path: &str, ctx: RecordCtx) {
    emit(
        DefKind::Completion,
        name,
        Some(abs_path.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}

/// Walk `dir` and emit one `Completion` event per `_*` file found —
/// matches what zinit-report surfaces in its "Completions:" section.
/// Called from the path/fpath array hook whenever an fpath dir is
/// added (set or appended). Filesystem I/O — bounded to the directory
/// the user just registered, so the cost is paid once per fpath edit
/// (typically tens of files per plugin, hundreds for big completion
/// trees like zsh-more-completions).
///
/// Filename rule (matches compinit/compaudit at zsh/Src/Zle/comp1.c):
/// every entry starting with `_` and not a directory is a candidate.
/// Symlinks are dereferenced. Hidden subdirs and `.zwc` files are
/// skipped. Errors (unreadable dir, EACCES) are tracing-warn'd and
/// otherwise silent — this is a best-effort surfacing layer.
pub fn discover_completions_in_fpath_dir(dir: &str, ctx: &RecordCtx) {
    if !is_enabled() {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            tracing::warn!(?e, dir, "recorder: completion discovery skipped");
            return;
        }
    };
    for entry in entries.flatten() {
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        if !name.starts_with('_') {
            continue;
        }
        if name.ends_with(".zwc") {
            continue;
        }
        let path = entry.path();
        // file_type may need to follow a symlink to know if it's a dir.
        let is_file = match entry.file_type() {
            Ok(ft) if ft.is_symlink() => std::fs::metadata(&path)
                .map(|m| m.is_file())
                .unwrap_or(false),
            Ok(ft) => ft.is_file(),
            Err(_) => false,
        };
        if !is_file {
            continue;
        }
        let abs = path.to_string_lossy().to_string();
        emit_completion(&name, &abs, ctx.clone());
    }
}

/// End-of-run summary printed to stderr right before the IPC bundle
/// is sent. Counts per `kind` plus totals. Called from `atexit`, so
/// must avoid touching anything backed by thread-locals (tracing's
/// dispatch + once_cell::Lazy can both raise AccessError once Rust
/// starts tearing down TLS).
pub fn print_summary() {
    if !is_enabled() {
        return;
    }
    let buf = match BUFFER.try_lock() {
        Ok(b) => b,
        Err(_) => return,
    };
    let total = buf.len();
    let mut counts: std::collections::BTreeMap<&'static str, usize> =
        std::collections::BTreeMap::new();
    for ev in buf.iter() {
        *counts.entry(ev.kind.as_str()).or_insert(0) += 1;
    }
    let started = START_NS.load(Ordering::Relaxed);
    let elapsed_ms = if started > 0 {
        (now_ns().saturating_sub(started)) / 1_000_000
    } else {
        0
    };
    if json_summary_enabled() {
        // Single JSON line to stdout — pipes cleanly into jq / scripts.
        let mut counts_pairs = Vec::with_capacity(counts.len());
        for (k, v) in &counts {
            counts_pairs.push(format!("\"{}\":{}", k, v));
        }
        println!(
            "{{\"total_events\":{},\"elapsed_ms\":{},\"counts\":{{{}}}}}",
            total,
            elapsed_ms,
            counts_pairs.join(",")
        );
    } else {
        eprintln!();
        eprintln!("--- zshrs-recorder summary ---");
        eprintln!("  total events: {}", total);
        for (k, v) in &counts {
            eprintln!("  {:<10} {}", k, v);
        }
        eprintln!("  elapsed:      {} ms", elapsed_ms);
    }
}

/// Bundle every captured event, fold it into the canonical rkyv shard
/// and write `~/.zshrs/images/{hash8}-recorder.rkyv`, then clear the
/// buffer. Returns `true` when the shard was written. No daemon: the
/// recorder owns this write so a recording lands on a machine where
/// `zshrs-daemon` was never started. Called from `atexit`, so avoids
/// tracing/TLS-touching helpers (use `eprintln!` only).
///
/// `--dry-run` skips the shard write; `-o PATH` still writes the
/// bundle JSON so post-mortem inspection works either way.
#[cfg(feature = "daemon")]
pub fn flush() -> bool {
    if !is_enabled() {
        return false;
    }
    let events = match BUFFER.try_lock() {
        Ok(mut b) => std::mem::take(&mut *b),
        Err(_) => return false,
    };
    let events_empty = events.is_empty();
    let bundle = RecorderBundle {
        started_at_ns: START_NS.load(Ordering::Relaxed),
        finished_at_ns: now_ns(),
        cmdline: std::env::args().collect::<Vec<_>>().join(" "),
        zdotdir: std::env::var("ZDOTDIR").ok(),
        home: std::env::var("HOME").ok(),
        events,
        // `--shell-id ID` overrides this so a recorder run can
        // impersonate a non-zshrs source (federation testing,
        // rebrand experiments). Default = "zshrs" since this code
        // path is exclusive to the AOP-instrumented zshrs-recorder.
        shell_id: Some(shell_id_override().unwrap_or_else(|| "zshrs".to_string())),
        end_state: END_STATE.lock().ok().and_then(|mut s| s.take()),
    };

    // `-o PATH` writes the bundle to a JSON file alongside (or instead
    // of, under --dry-run) the shard. Empty bundles still write —
    // caller asked for the file, give them the file (even if it just
    // confirms "recorder ran, captured nothing" — that itself is a
    // useful diagnostic when a parse error caused zero events).
    if let Some(path) = output_path() {
        match serde_json::to_string(&bundle) {
            Ok(s) => {
                if let Err(e) = std::fs::write(&path, s.as_bytes()) {
                    eprintln!("recorder: write {path}: {e}");
                } else {
                    eprintln!("recorder: bundle written to {path}");
                }
            }
            Err(e) => eprintln!("recorder: bundle serialize for output failed: {e}"),
        }
    }

    if events_empty {
        eprintln!("recorder: no events to flush");
        return false;
    }
    if no_write() {
        return false;
    }

    let t0 = Instant::now();
    // `RecordEvent` and the shard fold's `BundleEvent` share one wire
    // format; round-tripping through serde keeps the two crates'
    // types independent.
    let folded_input = serde_json::to_value(&bundle).and_then(serde_json::from_value::<
        crate::daemon::recorder_shard::Bundle,
    >);
    let shard_bundle = match folded_input {
        Ok(b) => b,
        Err(e) => {
            eprintln!("recorder: bundle conversion failed: {e}");
            return false;
        }
    };
    let paths = match crate::daemon::paths::CachePaths::resolve() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("recorder: cache paths unresolved: {e}");
            return false;
        }
    };
    if let Err(e) = paths.ensure_dirs() {
        eprintln!("recorder: {}: {e}", paths.images.display());
        return false;
    }
    match crate::daemon::recorder_shard::write_bundle_shard(&paths, &shard_bundle) {
        Ok(path) => {
            eprintln!(
                "recorder: {} events written to {} in {} ms",
                shard_bundle.events.len(),
                path.display(),
                t0.elapsed().as_millis()
            );
            true
        }
        Err(e) => {
            eprintln!("recorder: shard write failed: {e}");
            false
        }
    }
}
/// Non-daemon builds carry no shard writer.
#[cfg(not(feature = "daemon"))]
pub fn flush() -> bool {
    if !is_enabled() {
        return false;
    }
    eprintln!("recorder: daemon feature off — shard not written");
    false
}

/// Set once the summary + shard write ran, so the natural end of
/// `main` and the atexit hook never both fire.
static FINALIZED: AtomicBool = AtomicBool::new(false);

/// Print the summary and write the shard. `bins/zshrs-recorder.rs`
/// calls this at the end of `main`, while thread-locals are still
/// alive — `write_canonical_shard` logs through `tracing`, which
/// panics with `AccessError` once `main` has returned. The atexit
/// hook covers the paths that never get back to `main` (`exit` inside
/// the sourced files).
pub fn finalize() {
    if FINALIZED.swap(true, Ordering::SeqCst) {
        return;
    }
    print_summary();
    flush();
}

extern "C" fn atexit_finalize() {
    // A forked subshell or `$(...)` inherits this hook AND a copy of the
    // buffer holding every event captured so far. Finalizing there wrote
    // a partial shard (and a partial summary) per child, and whichever
    // child exited last won — the parent's full recording was lost.
    if std::process::id() as u64 != OWNER_PID.load(Ordering::Relaxed) {
        return;
    }
    if FINALIZED.swap(true, Ordering::SeqCst) {
        return;
    }
    // libc atexit runs AFTER the Rust runtime starts tearing down
    // thread-locals. `tracing::*` and `once_cell::Lazy` both touch TLS,
    // so we must catch the AccessError they raise during destruction —
    // an unwinding panic from an `extern "C"` function aborts the
    // process and swallows the very output the user is here to see.
    // The mark lets the log sites below the call skip `tracing` entirely
    // rather than panic and be caught; see `atexit_teardown`.
    crate::atexit_teardown::mark();
    let _ = std::panic::catch_unwind(|| {
        print_summary();
    });
    let _ = std::panic::catch_unwind(|| {
        flush();
    });
}

/// Register `print_summary` + `flush` as a libc `atexit` hook so they
/// run even when the shell exits via `std::process::exit`. Only the
/// calling process finalizes; forked children skip the hook.
pub fn install_atexit() {
    OWNER_PID.store(std::process::id() as u64, Ordering::Relaxed);
    // SAFETY: `atexit_finalize` is a plain `extern "C"` function with the
    // libc-required signature.
    unsafe {
        libc::atexit(atexit_finalize);
    }
}
