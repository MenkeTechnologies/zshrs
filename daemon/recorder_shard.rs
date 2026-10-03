//! Recorder bundle → canonical rkyv shard.
//!
//! zshrs-original — no C counterpart.
//!
//! One fold, two callers. `zshrs-recorder` calls [`write_bundle_shard`]
//! at end of run and writes `~/.zshrs/images/{hash8}-recorder.rkyv`
//! itself — no daemon in the loop, so a recording lands whether or not
//! `zshrs-daemon` is up. The daemon's `recorder_ingest` op calls
//! [`fold_bundle`] for the same buckets, because it also replaces its
//! in-memory canonical rows and hydrates the SQLite mirror from them.
//!
//! The shell reads the shard back with
//! `canonical_apply::apply_all` — an mmap-free rkyv read, no IPC.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::paths::CachePaths;
use crate::Result;
use crate::shard::{
    current_binary_identity, write_canonical_shard, CanonicalShard, ShardHeader,
};

/// One captured event, as `src/recorder/mod.rs::RecordEvent`
/// serializes it. The two share the wire format.
#[derive(Debug, Deserialize)]
pub struct BundleEvent {
    /// Monotonic capture order.
    pub order_idx: u64,
    /// Capture time, ns since the epoch.
    pub ts_ns: u64,
    /// Subsystem tag (`alias`, `function`, `path_mod`, …).
    pub kind: String,
    /// Entity name.
    pub name: String,
    /// Joined scalar value.
    pub value: Option<String>,
    /// File the mutation came from.
    pub file: Option<String>,
    /// Line in `file`.
    pub line: Option<u32>,
    /// `funcstack` chain at capture time.
    pub fn_chain: Option<String>,
    /// ParamAttrs bitset (scalar/integer/float/assoc/array/readonly/
    /// export/global/unique/tied/append) — set on assign events so
    /// replay reconstructs typed declarations without guessing.
    #[serde(default)]
    pub attrs: u16,
    /// Array payload — replay rebuilds `name=(...)` from this rather
    /// than splitting the joined `value`.
    #[serde(default)]
    pub value_array: Option<Vec<String>>,
    /// Assoc payload — replay rebuilds `name=(k1 v1 ...)` from this.
    #[serde(default)]
    pub value_assoc: Option<Vec<(String, String)>>,
}

/// A whole recorder run.
#[derive(Debug, Deserialize)]
pub struct Bundle {
    /// Run start, ns since the epoch.
    pub started_at_ns: u64,
    /// Run end, ns since the epoch. Doubles as the shard generation.
    pub finished_at_ns: u64,
    /// Recorder argv, space-joined.
    pub cmdline: Option<String>,
    /// `$ZDOTDIR` of the recorded run.
    pub zdotdir: Option<String>,
    /// `$HOME` of the recorded run.
    pub home: Option<String>,
    /// Every captured event, in capture order.
    pub events: Vec<BundleEvent>,
    /// Federated-catalog identity; `None` means "zshrs".
    #[serde(default)]
    pub shell_id: Option<String>,
    /// The shell's state once the init chain and its deferred work
    /// finished. Present on a run that got back to `main`; absent when
    /// an `exit` inside the sourced files ended it, and the fold falls
    /// back to the events.
    #[serde(default)]
    pub end_state: Option<EndState>,
}

/// End-of-run snapshot of the tables a replay restores, read from the
/// live shell rather than folded from events. Events cannot say what
/// survived: `local`, `setopt localoptions`, `unalias`, `unfunction`
/// and `typeset -a x; x+=(…)` all leave events whose last value is
/// not the end state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EndState {
    /// `$aliases`
    pub aliases: HashMap<String, String>,
    /// `$galiases`
    pub global_aliases: HashMap<String, String>,
    /// `$saliases`
    pub suffix_aliases: HashMap<String, String>,
    /// Defined functions, name → body as `$functions[name]` renders it.
    pub functions: HashMap<String, String>,
    /// Autoload-pending functions (`$functions[name]` is
    /// `builtin autoload -X…`).
    pub autoloads: Vec<String>,
    /// Global parameters the recorded files assigned, typed.
    pub params: Vec<TypedParam>,
    /// Options the recorded files turned on.
    pub setopts: Vec<String>,
    /// Options the recorded files turned off.
    pub unsetopts: Vec<String>,
    /// Every loaded module, each with its enabled features
    /// (`zmodload -LF` form), replayed as `zmodload -F MODULE f…`.
    #[serde(default)]
    pub modules: Vec<(String, Vec<String>)>,
    /// Autoloads registered with a directory (`autoload -Uz DIR/NAME`,
    /// `PM_LOADDIR`), name → the `autoload` argv that recreates it. The
    /// rest are in `autoloads`, found on `$fpath` by name.
    #[serde(default)]
    pub autoload_paths: Vec<(String, Vec<String>)>,
    /// User math functions (`functions -M`), name → the `functions` argv that
    /// recreates it.
    #[serde(default)]
    pub math_functions: Vec<(String, Vec<String>)>,
    /// `zstyle` table, `zstyle -L` order: (pattern, style, values,
    /// `-e`).
    #[serde(default)]
    pub zstyles: Vec<(String, String, Vec<String>, bool)>,
    /// User widgets the files defined, as `zle` argvs (`-N NAME FUNC`,
    /// `-C NAME WIDGET FUNC`).
    #[serde(default)]
    pub widgets: Vec<Vec<String>>,
    /// Keymap changes as `bindkey` argvs, in order: new keymaps, links,
    /// then each keymap's binds and removals.
    #[serde(default)]
    pub bindkeys: Vec<Vec<String>>,
    /// Global parameters the shell started with that the files unset
    /// (`unset CDPATH`).
    #[serde(default)]
    pub unset_params: Vec<String>,
    /// Files that left the shell holding a descriptor in a parameter
    /// (`exec {fd}>lock`, `zsystem flock -f fd`): a lock, a session ID, an
    /// open file only the process that opened it owns. Such a parameter is
    /// left out of `params`, and the replay sources these files again, in
    /// this order, so each shell takes its own.
    #[serde(default)]
    pub resource_files: Vec<String>,
}

/// Stands for the recording terminal's path inside a recorded parameter
/// value. Each shell has its own `$TTY`; a value built from the
/// recorder's (`ZPWR_TTY=$TTY`, `$(tty)`) is only right for the shell
/// that reads it back, so the replay puts its own `$TTY` here. NUL
/// cannot come out of the recorded config's own words.
pub const TTY_PLACEHOLDER: &str = "\0TTY\0";

/// Stands for the recording shell's `$$` inside a recorded parameter
/// value (`ZPWR_TEMPFILE=…/.temp$$-…`); the replay puts its own `$$`
/// there, so per-shell names stay per shell.
pub const PID_PLACEHOLDER: &str = "\0PID\0";

/// `extras` key holding [`EndState::resource_files`], position → path.
pub const RESOURCE_FILES_EXTRA: &str = "resource_files";

/// `extras` key holding [`EndState::unset_params`], name → "".
pub const UNSET_PARAMS_EXTRA: &str = "unset_params";

/// `extras` key holding [`EndState::widgets`], position → argv.
pub const WIDGETS_EXTRA: &str = "widgets_end";

/// `extras` key holding [`EndState::bindkeys`], position → argv.
pub const BINDKEYS_EXTRA: &str = "bindkeys_end";

/// An ordered argv list as an `extras` bucket: keys are the position
/// zero-padded so they sort back into order, values the argv joined by
/// [`ARGV_SEP`].
pub fn ordered_argvs(argvs: &[Vec<String>]) -> HashMap<String, String> {
    argvs
        .iter()
        .enumerate()
        .map(|(i, argv)| (format!("{i:08}"), argv.join(&ARGV_SEP.to_string())))
        .collect()
}

/// `extras` key holding [`EndState::zstyles`]. Keys are the position,
/// zero-padded so they sort back into `zstyle -L` order; values are
/// `pattern`, `style`, `e` or `-`, then the values, joined by
/// [`ARGV_SEP`].
pub const ZSTYLES_EXTRA: &str = "zstyles_end";

/// `extras` key holding [`EndState::autoload_paths`], name → argv
/// joined by [`ARGV_SEP`].
pub const AUTOLOAD_PATHS_EXTRA: &str = "autoload_paths";

/// `extras` key holding [`EndState::math_functions`], name → argv
/// joined by [`ARGV_SEP`].
pub const MATH_FUNCTIONS_EXTRA: &str = "math_functions";

/// Joins an argv stored in an `extras` string value. A shell word can
/// hold any byte but NUL.
pub const ARGV_SEP: char = '\0';

/// One global parameter with its type, as `${(t)name}` names it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypedParam {
    /// Parameter name.
    pub name: String,
    /// Base type: `scalar`, `integer`, `float`, `array`, `association`.
    pub kind: String,
    /// Exported to the environment.
    pub export: bool,
    /// Scalar / integer / float value.
    #[serde(default)]
    pub value: Option<String>,
    /// Array elements, or an association flattened `k1 v1 k2 v2 …`.
    #[serde(default)]
    pub elements: Option<Vec<String>>,
    /// `typeset` attribute letters beyond the type and export (`U`, `H`,
    /// `h`, `l`, `u`, `t`), re-applied with `typeset -g` after the value.
    #[serde(default)]
    pub attrs: String,
    /// On the scalar half of a `typeset -T SCALAR array SEP` pair: the
    /// array's name and the separator.
    #[serde(default)]
    pub tie: Option<(String, String)>,
}

/// `extras` key holding the end-state modules, name → enabled features
/// joined by spaces (feature names never contain one).
pub const MODULES_END_EXTRA: &str = "modules_end";

/// `extras` key holding the end-state typed parameters, name → JSON
/// `TypedParam`. `canonical_apply` prefers it over `params`.
pub const PARAMS_END_EXTRA: &str = "params_end";


/// `(value, file, line)` — the value plus its definition site, carried
/// so `zwhere` can answer "where was this defined?".
pub type AttrRow = (String, Option<String>, Option<u32>);

/// The bundle folded into end-state, one bucket per subsystem. Keyed
/// buckets are latest-wins; positional ones keep capture order.
#[derive(Debug, Default)]
pub struct Folded {
    /// `alias NAME=VALUE`
    pub aliases: HashMap<String, AttrRow>,
    /// `alias -g`
    pub galias: HashMap<String, AttrRow>,
    /// `alias -s`
    pub salias: HashMap<String, AttrRow>,
    /// Function bodies.
    pub functions: HashMap<String, AttrRow>,
    /// Exported environment.
    pub env_exports: HashMap<String, AttrRow>,
    /// Shell parameters (joined scalar view).
    pub params: HashMap<String, AttrRow>,
    /// Shell parameters, JSON payload with attrs + array/assoc shape.
    pub params_typed: HashMap<String, AttrRow>,
    /// `bindkey`
    pub bindkeys: HashMap<String, AttrRow>,
    /// `compdef`
    pub compdef: HashMap<String, AttrRow>,
    /// `hash -d`
    pub named_dirs: HashMap<String, AttrRow>,
    /// `zstyle`, in order.
    pub zstyle: Vec<(String, AttrRow)>,
    /// `zmodload`, in order.
    pub zmodload: Vec<(String, AttrRow)>,
    /// `setopt`, in order.
    pub setopts: Vec<(String, AttrRow)>,
    /// `unsetopt`, in order.
    pub unsetopts: Vec<(String, AttrRow)>,
    /// `trap`
    pub traps: HashMap<String, AttrRow>,
    /// `sched`
    pub sched: HashMap<String, AttrRow>,
    /// `zle -N`
    pub zle_widgets: HashMap<String, AttrRow>,
    /// Discovered completions.
    pub completions: HashMap<String, AttrRow>,
    /// Sourced files, in order.
    pub sourced: Vec<(String, AttrRow)>,
    /// `$path` edits, in order.
    pub path: Vec<(String, AttrRow)>,
    /// `$fpath` edits, in order.
    pub fpath: Vec<(String, AttrRow)>,
}

/// Fold every event into its subsystem bucket. The bundle is end-state
/// for the run, so a later event for the same key replaces an earlier
/// one.
pub fn fold_bundle(bundle: &Bundle) -> Folded {
    let mut f = Folded::default();
    for ev in &bundle.events {
        let attr = |val: String| -> AttrRow { (val, ev.file.clone(), ev.line) };
        let value = || ev.value.clone().unwrap_or_default();
        match ev.kind.as_str() {
            "alias" => {
                f.aliases.insert(ev.name.clone(), attr(value()));
            }
            "alias -g" | "galias" => {
                f.galias.insert(ev.name.clone(), attr(value()));
            }
            "alias -s" | "salias" => {
                f.salias.insert(ev.name.clone(), attr(value()));
            }
            "function" => {
                f.functions.insert(ev.name.clone(), attr(value()));
            }
            "export" => {
                f.env_exports.insert(ev.name.clone(), attr(value()));
            }
            "assign" | "typeset" => {
                f.params.insert(ev.name.clone(), attr(value()));
                let payload = serde_json::json!({
                    "attrs": ev.attrs,
                    "value": ev.value,
                    "value_array": ev.value_array,
                    "value_assoc": ev.value_assoc,
                });
                f.params_typed
                    .insert(ev.name.clone(), attr(payload.to_string()));
            }
            "bindkey" => {
                f.bindkeys.insert(ev.name.clone(), attr(value()));
            }
            "compdef" => {
                f.compdef.insert(ev.name.clone(), attr(value()));
            }
            "hash -d" | "hash_d" => {
                f.named_dirs.insert(ev.name.clone(), attr(value()));
            }
            "zstyle" => f.zstyle.push((ev.name.clone(), attr(value()))),
            "zmodload" => f.zmodload.push((ev.name.clone(), attr(String::new()))),
            "setopt" => f.setopts.push((ev.name.clone(), attr("on".to_string()))),
            "unsetopt" => f
                .unsetopts
                .push((ev.name.clone(), attr("off".to_string()))),
            "trap" => {
                f.traps.insert(ev.name.clone(), attr(value()));
            }
            "sched" => {
                f.sched.insert(ev.name.clone(), attr(value()));
            }
            "zle" => {
                f.zle_widgets.insert(ev.name.clone(), attr(value()));
            }
            "completion" => {
                f.completions.insert(ev.name.clone(), attr(value()));
            }
            "source" => f.sourced.push((ev.name.clone(), attr(String::new()))),
            "path_mod" => {
                let row = (ev.name.clone(), attr(String::new()));
                if ev.value.as_deref() == Some("fpath") {
                    f.fpath.push(row);
                } else {
                    f.path.push(row);
                }
            }
            other => {
                tracing::debug!(other, "recorder bundle: unknown kind, ignored");
            }
        }
    }
    f
}

/// Build the canonical shard from a folded bundle. The shard keeps
/// values only; file/line attribution lives in the daemon's canonical
/// rows.
pub fn build_shard(bundle: &Bundle, f: &Folded) -> CanonicalShard {
    let source_root = bundle
        .zdotdir
        .clone()
        .or_else(|| bundle.home.clone())
        .unwrap_or_else(|| "<recorder>".to_string());
    let (mtime_secs, mtime_nsecs, binary_len) = current_binary_identity();
    let header = ShardHeader {
        magic: 0,
        format_version: 0,
        generation: bundle.finished_at_ns,
        built_at_ns: bundle.finished_at_ns,
        slug: "recorder".to_string(),
        source_root,
        entry_count: bundle.events.len() as u32,
        binary_mtime_secs: mtime_secs,
        binary_mtime_nsecs: mtime_nsecs,
        binary_len,
    };
    let map = |m: &HashMap<String, AttrRow>| -> HashMap<String, String> {
        m.iter().map(|(k, (v, _, _))| (k.clone(), v.clone())).collect()
    };
    let pairs = |v: &[(String, AttrRow)]| -> Vec<(String, String)> {
        v.iter().map(|(k, (val, _, _))| (k.clone(), val.clone())).collect()
    };
    let keys = |v: &[(String, AttrRow)]| -> Vec<String> { v.iter().map(|(k, _)| k.clone()).collect() };

    // Subsystems added after the shard format froze ride in `extras`, so
    // each addition needs no format bump.
    let mut extras: HashMap<String, HashMap<String, String>> = HashMap::new();
    for (name, bucket) in [
        ("zle", &f.zle_widgets),
        ("completion", &f.completions),
        ("params_typed", &f.params_typed),
    ] {
        if !bucket.is_empty() {
            extras.insert(name.to_string(), map(bucket));
        }
    }

    let mut shard = CanonicalShard {
        header,
        aliases: map(&f.aliases),
        global_aliases: map(&f.galias),
        suffix_aliases: map(&f.salias),
        functions: map(&f.functions),
        autoload_functions: HashMap::new(),
        setopts: keys(&f.setopts),
        unsetopts: keys(&f.unsetopts),
        bindkeys: map(&f.bindkeys),
        named_dirs: map(&f.named_dirs),
        compdef: map(&f.compdef),
        zstyle: pairs(&f.zstyle),
        zmodload: keys(&f.zmodload),
        env_exports: map(&f.env_exports),
        params: map(&f.params),
        path: keys(&f.path),
        fpath: keys(&f.fpath),
        manpath: Vec::new(),
        plugins: Vec::new(),
        sourced_files: keys(&f.sourced),
        extras,
    };
    if let Some(end) = &bundle.end_state {
        apply_end_state(&mut shard, end);
    }
    shard
}

/// Replace the event-folded alias, function, parameter and option
/// buckets with the end-of-run snapshot. The scalar `params` and
/// `env_exports` views stay filled for readers that predate the typed
/// `params_end` extra.
fn apply_end_state(shard: &mut CanonicalShard, end: &EndState) {
    shard.aliases = end.aliases.clone();
    shard.global_aliases = end.global_aliases.clone();
    shard.suffix_aliases = end.suffix_aliases.clone();
    shard.functions = end.functions.clone();
    shard.autoload_functions = end.autoloads.iter().map(|n| (n.clone(), String::new())).collect();
    shard.setopts = end.setopts.clone();
    shard.unsetopts = end.unsetopts.clone();

    shard.params.clear();
    shard.env_exports.clear();
    let mut typed = HashMap::with_capacity(end.params.len());
    for p in &end.params {
        let scalar = match (&p.value, &p.elements) {
            (Some(v), _) => v.clone(),
            (None, Some(e)) => e.join(" "),
            (None, None) => String::new(),
        };
        if p.export {
            shard.env_exports.insert(p.name.clone(), scalar.clone());
        }
        shard.params.insert(p.name.clone(), scalar);
        if let Ok(json) = serde_json::to_string(p) {
            typed.insert(p.name.clone(), json);
        }
    }
    shard.extras.remove("params_typed");
    shard.extras.insert(PARAMS_END_EXTRA.to_string(), typed);
    let modules = end.modules.iter().map(|(m, f)| (m.clone(), f.join(" "))).collect();
    shard.extras.insert(MODULES_END_EXTRA.to_string(), modules);
    shard.zmodload = end.modules.iter().map(|(m, _)| m.clone()).collect();
    let by_name = |argvs: &[(String, Vec<String>)]| -> HashMap<String, String> {
        argvs
            .iter()
            .map(|(name, argv)| (name.clone(), argv.join(&ARGV_SEP.to_string())))
            .collect()
    };
    shard.extras.insert(AUTOLOAD_PATHS_EXTRA.to_string(), by_name(&end.autoload_paths));
    shard.extras.insert(MATH_FUNCTIONS_EXTRA.to_string(), by_name(&end.math_functions));

    // The event fold's `zstyle` rows join the values with spaces, which
    // a value like `'%d (errors: %e)'` cannot survive; the snapshot
    // replaces them.
    shard.zstyle.clear();
    let zstyles: Vec<Vec<String>> = end
        .zstyles
        .iter()
        .map(|(pat, style, vals, eval)| {
            let mut words = vec![pat.clone(), style.clone(), if *eval { "e" } else { "-" }.to_string()];
            words.extend(vals.iter().cloned());
            words
        })
        .collect();
    shard.extras.insert(ZSTYLES_EXTRA.to_string(), ordered_argvs(&zstyles));

    // The event fold's bindkey rows carry no keymap (`bindkey -M vicmd`
    // and `-M viopp` binds landed in `main`) and its widget rows no
    // `-C`; the snapshot replaces both.
    shard.bindkeys.clear();
    shard.extras.remove("zle");
    let resources: Vec<Vec<String>> = end.resource_files.iter().map(|f| vec![f.clone()]).collect();
    shard.extras.insert(RESOURCE_FILES_EXTRA.to_string(), ordered_argvs(&resources));
    let unsets = end.unset_params.iter().map(|n| (n.clone(), String::new())).collect();
    shard.extras.insert(UNSET_PARAMS_EXTRA.to_string(), unsets);
    shard.extras.insert(WIDGETS_EXTRA.to_string(), ordered_argvs(&end.widgets));
    shard.extras.insert(BINDKEYS_EXTRA.to_string(), ordered_argvs(&end.bindkeys));
}

/// Fold `bundle` and write its shard into `paths.images`. Returns the
/// shard path. This is the recorder's whole persistence step.
pub fn write_bundle_shard(paths: &CachePaths, bundle: &Bundle) -> Result<PathBuf> {
    let folded = fold_bundle(bundle);
    write_canonical_shard(paths, &build_shard(bundle, &folded))
}
