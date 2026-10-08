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

/// `extras` key holding the `zwhere` catalog: every folded row with its
/// definition site, so the daemon can answer `zwhere alias gst` from the
/// shard alone. Keys are `subsystem` [`ARGV_SEP`] `name`; values are the
/// JSON-encoded value, file, line and shell id joined by [`ARGV_SEP`],
/// with an empty field for a missing file or line.
pub const CATALOG_EXTRA: &str = "catalog_rows";

/// One `zwhere` catalog row: name, JSON-encoded value, file, line.
pub type CatalogRow = (String, String, Option<String>, Option<u32>);

/// The folded bundle as the daemon's canonical rows, subsystem by
/// subsystem. The single source for both `recorder_ingest` (rows over
/// IPC) and [`CATALOG_EXTRA`] (rows in the shard), so the two cannot
/// disagree about what `zwhere` shows.
pub fn catalog_rows(f: &Folded) -> Vec<(&'static str, Vec<CatalogRow>)> {
    let keyed = |m: &HashMap<String, AttrRow>| -> Vec<CatalogRow> {
        m.iter()
            .map(|(k, (v, file, line))| (k.clone(), json_string(v), file.clone(), *line))
            .collect()
    };
    let positional = |v: &[(String, AttrRow)]| -> Vec<CatalogRow> {
        v.iter()
            .enumerate()
            .map(|(i, (p, (_v, file, line)))| (i.to_string(), json_string(p), file.clone(), *line))
            .collect()
    };
    let zstyle = f
        .zstyle
        .iter()
        .enumerate()
        .map(|(i, (p, (r, file, line)))| (format!("{i}:{p}"), json_string(r), file.clone(), *line))
        .collect();
    let zmodload = f
        .zmodload
        .iter()
        .map(|(m, (_v, file, line))| (m.clone(), json_string(""), file.clone(), *line))
        .collect();
    let setopt = f
        .setopts
        .iter()
        .map(|(o, (_v, file, line))| (o.clone(), "\"on\"".to_string(), file.clone(), *line))
        .chain(
            f.unsetopts
                .iter()
                .map(|(o, (_v, file, line))| (o.clone(), "\"off\"".to_string(), file.clone(), *line)),
        )
        .collect();
    // params_typed values are already JSON; they pass through verbatim so
    // the row keeps the structured payload (attrs + value + value_array +
    // value_assoc).
    let params_typed = f
        .params_typed
        .iter()
        .map(|(k, (v, file, line))| (k.clone(), v.clone(), file.clone(), *line))
        .collect();
    vec![
        ("alias", keyed(&f.aliases)),
        ("galias", keyed(&f.galias)),
        ("salias", keyed(&f.salias)),
        ("function", keyed(&f.functions)),
        ("env", keyed(&f.env_exports)),
        ("params", keyed(&f.params)),
        ("bindkey", keyed(&f.bindkeys)),
        ("compdef", keyed(&f.compdef)),
        ("named_dir", keyed(&f.named_dirs)),
        ("zstyle", zstyle),
        ("zmodload", zmodload),
        ("setopt", setopt),
        ("trap", keyed(&f.traps)),
        ("sched", keyed(&f.sched)),
        ("zle", keyed(&f.zle_widgets)),
        ("completion", keyed(&f.completions)),
        ("params_typed", params_typed),
        ("source", positional(&f.sourced)),
        ("path", positional(&f.path)),
        ("fpath", positional(&f.fpath)),
    ]
}

/// [`catalog_rows`] encoded as the [`CATALOG_EXTRA`] bucket.
fn encode_catalog(f: &Folded, shell_id: &str) -> HashMap<String, String> {
    let sep = ARGV_SEP.to_string();
    let mut out = HashMap::new();
    for (sub, rows) in catalog_rows(f) {
        for (name, value, file, line) in rows {
            let line = line.map(|l| l.to_string()).unwrap_or_default();
            let file = file.unwrap_or_default();
            out.insert(
                [sub, name.as_str()].join(&sep),
                [value.as_str(), file.as_str(), line.as_str(), shell_id].join(&sep),
            );
        }
    }
    out
}

/// Decode the [`CATALOG_EXTRA`] bucket back into per-subsystem rows plus
/// the recording's shell id. `None` when the shard predates the extra.
pub fn decode_catalog(
    extras: &HashMap<String, HashMap<String, String>>,
) -> Option<(Vec<(String, Vec<CatalogRow>)>, Option<String>)> {
    let bucket = extras.get(CATALOG_EXTRA)?;
    let mut by_sub: HashMap<String, Vec<CatalogRow>> = HashMap::new();
    let mut shell_id = None;
    for (k, v) in bucket {
        let Some((sub, name)) = k.split_once(ARGV_SEP) else {
            continue;
        };
        let mut fields = v.split(ARGV_SEP);
        let value = fields.next().unwrap_or("").to_string();
        let file = fields.next().filter(|s| !s.is_empty()).map(str::to_string);
        let line = fields.next().and_then(|s| s.parse().ok());
        if let Some(sid) = fields.next().filter(|s| !s.is_empty()) {
            shell_id = Some(sid.to_string());
        }
        by_sub.entry(sub.to_string()).or_default().push((name.to_string(), value, file, line));
    }
    Some((by_sub.into_iter().collect(), shell_id))
}

/// `extras` key holding every DEFINITION of a keyed name, in capture order:
/// the override chain behind the single row [`CATALOG_EXTRA`] keeps. Keys are
/// `subsystem` [`ARGV_SEP`] `name`; the value is the definitions joined by
/// [`DEFINITION_SEP`], each `json value` [`ARGV_SEP`] `file` [`ARGV_SEP`]
/// `line` [`ARGV_SEP`] `fn_chain` (empty for a missing field).
pub const CATALOG_HISTORY_EXTRA: &str = "catalog_history";

/// Separates the definitions inside one [`CATALOG_HISTORY_EXTRA`] value. A
/// JSON-encoded value cannot contain this raw control character.
pub const DEFINITION_SEP: char = '\u{1e}';

/// One definition of a name: JSON-encoded value, file, line, and the
/// `funcstack` chain it ran under (`outer ← inner`).
pub type Definition = (String, Option<String>, Option<u32>, Option<String>);

/// The subsystem a keyed event kind lands in (the names [`catalog_rows`] uses).
fn keyed_subsystem(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "alias" => "alias",
        "alias -g" | "galias" => "galias",
        "alias -s" | "salias" => "salias",
        "function" => "function",
        "export" => "env",
        "assign" | "typeset" => "params",
        "bindkey" => "bindkey",
        "compdef" => "compdef",
        "hash -d" | "hash_d" => "named_dir",
        "trap" => "trap",
        "sched" => "sched",
        "zle" => "zle",
        "completion" => "completion",
        _ => return None,
    })
}

/// Every definition of every keyed name, in capture order — the chain
/// `zwhere` lists with its file:line sites and call chains. A name defined
/// once has a chain of one.
pub fn catalog_history(bundle: &Bundle) -> Vec<(String, String, Vec<Definition>)> {
    let mut chains: Vec<((&'static str, String), Vec<Definition>)> = Vec::new();
    let mut index: HashMap<(&'static str, String), usize> = HashMap::new();
    for ev in &bundle.events {
        let Some(sub) = keyed_subsystem(&ev.kind) else {
            continue;
        };
        let key = (sub, ev.name.clone());
        let def = (
            json_string(&ev.value.clone().unwrap_or_default()),
            ev.file.clone(),
            ev.line,
            ev.fn_chain.clone(),
        );
        match index.get(&key) {
            Some(&i) => chains[i].1.push(def),
            None => {
                index.insert(key.clone(), chains.len());
                chains.push((key, vec![def]));
            }
        }
    }
    chains
        .into_iter()
        .map(|((sub, name), defs)| (sub.to_string(), name, defs))
        .collect()
}

/// [`catalog_history`] encoded as the [`CATALOG_HISTORY_EXTRA`] bucket.
fn encode_catalog_history(bundle: &Bundle) -> HashMap<String, String> {
    let sep = ARGV_SEP.to_string();
    catalog_history(bundle)
        .into_iter()
        .map(|(sub, name, defs)| {
            let value = defs
                .iter()
                .map(|(v, file, line, chain)| {
                    let line = line.map(|l| l.to_string()).unwrap_or_default();
                    [
                        v.as_str(),
                        file.as_deref().unwrap_or(""),
                        line.as_str(),
                        chain.as_deref().unwrap_or(""),
                    ]
                    .join(&sep)
                })
                .collect::<Vec<_>>()
                .join(&DEFINITION_SEP.to_string());
            ([sub.as_str(), name.as_str()].join(&sep), value)
        })
        .collect()
}

/// Decode the [`CATALOG_HISTORY_EXTRA`] bucket: `(subsystem, name)` → the
/// definitions of that name in capture order. Empty for a shard recorded
/// before the bucket existed.
pub fn decode_catalog_history(
    extras: &HashMap<String, HashMap<String, String>>,
) -> HashMap<(String, String), Vec<Definition>> {
    let mut out = HashMap::new();
    let Some(bucket) = extras.get(CATALOG_HISTORY_EXTRA) else {
        return out;
    };
    for (k, v) in bucket {
        let Some((sub, name)) = k.split_once(ARGV_SEP) else {
            continue;
        };
        let defs = v
            .split(DEFINITION_SEP)
            .map(|d| {
                let mut f = d.split(ARGV_SEP);
                let value = f.next().unwrap_or("").to_string();
                let file = f.next().filter(|s| !s.is_empty()).map(str::to_string);
                let line = f.next().and_then(|s| s.parse().ok());
                let chain = f.next().filter(|s| !s.is_empty()).map(str::to_string);
                (value, file, line, chain)
            })
            .collect();
        out.insert((sub.to_string(), name.to_string()), defs);
    }
    out
}

/// A JSON string literal for `s`, the encoding canonical rows store.
fn json_string(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
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

/// Plugins the recording loaded, as `(manager, name)` in first-seen order.
///
/// The recorder is plugin-framework-agnostic: it records state mutations and
/// does not know what a "plugin" is. A plugin is therefore recovered from WHERE
/// its files live — each framework installs into a layout of its own — so the
/// sourced files and the fpath directories are matched against those layouts.
/// A `*.plugin.zsh` outside every known layout is reported as `manual`.
pub fn detect_plugins(sourced: &[String], fpath: &[String]) -> Vec<(String, String)> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for path in sourced.iter().chain(fpath) {
        if let Some(found) = plugin_from_path(path) {
            if seen.insert(found.clone()) {
                out.push(found);
            }
        }
    }
    out
}

/// The `(manager, name)` a path belongs to, or `None`.
fn plugin_from_path(path: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    // The segment(s) after the first occurrence of `marker`, when `marker`
    // is a single path component.
    let after = |marker: &str, n: usize| -> Option<Vec<&str>> {
        let i = parts.iter().position(|p| *p == marker)?;
        let rest = parts.get(i + 1..i + 1 + n)?;
        Some(rest.to_vec())
    };
    let has = |component: &str| parts.iter().any(|p| *p == component);
    let owner_repo = |a: &str, b: &str| format!("{a}/{b}");

    // zinit / zplugin: plugins/<owner>---<repo>; `_local---<name>` is a local
    // plugin dir. A snippet is named by its directory (`OMZP::git`).
    if has(".zinit") || has("zinit") || has(".zplugin") || has("zplugin") {
        if let Some(r) = after("plugins", 1) {
            return Some(("zinit".into(), r[0].replacen("---", "/", 1)));
        }
        if let Some(r) = after("snippets", 1) {
            return Some(("zinit-snippet".into(), r[0].to_string()));
        }
    }
    // zplug: repos/<owner>/<repo>
    if has(".zplug") {
        if let Some(r) = after("repos", 2) {
            return Some(("zplug".into(), owner_repo(r[0], r[1])));
        }
    }
    // antigen: bundles/<owner>/<repo>
    if has(".antigen") || has("antigen") {
        if let Some(r) = after("bundles", 2) {
            return Some(("antigen".into(), owner_repo(r[0], r[1])));
        }
    }
    // antibody: one directory per URL, `/` encoded as `-SLASH-`.
    if has("antibody") {
        if let Some(dir) = parts.iter().find(|p| p.contains("-SLASH-")) {
            let name = dir.rsplit("-SLASH-").take(2).collect::<Vec<_>>();
            if let [repo, owner] = name[..] {
                return Some(("antibody".into(), owner_repo(owner, repo)));
            }
        }
    }
    // sheldon: repos/<host>/<owner>/<repo>
    if has("sheldon") {
        if let Some(r) = after("repos", 3) {
            return Some(("sheldon".into(), owner_repo(r[1], r[2])));
        }
    }
    // zgenom: sources/<owner>/<repo>/...; zgen: <owner>/<repo>-<branch>
    if has(".zgenom") {
        if let Some(r) = after("sources", 2) {
            return Some(("zgenom".into(), owner_repo(r[0], r[1])));
        }
    }
    if has(".zgen") {
        let i = parts.iter().position(|p| *p == ".zgen")?;
        if let (Some(owner), Some(repo)) = (parts.get(i + 1), parts.get(i + 2)) {
            let repo = repo.rsplit_once('-').map_or(*repo, |(r, _)| r);
            return Some(("zgen".into(), owner_repo(owner, repo)));
        }
    }
    // prezto / zim: modules/<name>
    if has(".zprezto") {
        if let Some(r) = after("modules", 1) {
            return Some(("prezto".into(), r[0].to_string()));
        }
    }
    if has(".zim") {
        if let Some(r) = after("modules", 1) {
            return Some(("zim".into(), r[0].to_string()));
        }
    }
    // oh-my-zsh: plugins/<name> under the install or under custom/. Checked
    // after the managers that bundle oh-my-zsh inside their own tree (antigen).
    if has(".oh-my-zsh") || has("ohmyzsh") || has("oh-my-zsh") {
        if let Some(r) = after("plugins", 1) {
            return Some(("oh-my-zsh".into(), r[0].to_string()));
        }
    }
    // Any other `<name>.plugin.zsh`.
    let file = parts.last()?;
    file.strip_suffix(".plugin.zsh").map(|n| ("manual".into(), n.to_string()))
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
        plugins: detect_plugins(&keys(&f.sourced), &keys(&f.fpath)),
        sourced_files: keys(&f.sourced),
        extras,
    };
    if let Some(end) = &bundle.end_state {
        apply_end_state(&mut shard, end);
    }
    let shell_id = bundle.shell_id.as_deref().unwrap_or("zshrs");
    shard.extras.insert(CATALOG_EXTRA.to_string(), encode_catalog(f, shell_id));
    let history = encode_catalog_history(bundle);
    if !history.is_empty() {
        shard.extras.insert(CATALOG_HISTORY_EXTRA.to_string(), history);
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

#[cfg(test)]
mod plugin_detection_tests {
    use super::*;

    fn one(path: &str) -> Option<(String, String)> {
        plugin_from_path(path)
    }

    fn pair(m: &str, n: &str) -> Option<(String, String)> {
        Some((m.to_string(), n.to_string()))
    }

    #[test]
    fn each_framework_layout_names_its_plugin() {
        let cases = [
            ("/h/.zinit/plugins/hlissner---zsh-autopair/autopair.zsh", pair("zinit", "hlissner/zsh-autopair")),
            ("/h/.zinit/plugins/_local---zinit/zinit.zsh", pair("zinit", "_local/zinit")),
            ("/h/.local/share/zinit/plugins/a---b/b.plugin.zsh", pair("zinit", "a/b")),
            ("/h/.zinit/snippets/OMZP::git/git.plugin.zsh", pair("zinit-snippet", "OMZP::git")),
            ("/h/.oh-my-zsh/plugins/git/git.plugin.zsh", pair("oh-my-zsh", "git")),
            ("/h/.oh-my-zsh/custom/plugins/zsh-z/zsh-z.plugin.zsh", pair("oh-my-zsh", "zsh-z")),
            ("/h/.zplug/repos/zsh-users/zsh-autosuggestions/x.zsh", pair("zplug", "zsh-users/zsh-autosuggestions")),
            ("/h/.antigen/bundles/robbyrussell/oh-my-zsh/plugins/git/x.zsh", pair("antigen", "robbyrussell/oh-my-zsh")),
            ("/h/.cache/antibody/https-COLON--SLASH--SLASH-github.com-SLASH-zsh-users-SLASH-zsh-syntax-highlighting/z.zsh", pair("antibody", "zsh-users/zsh-syntax-highlighting")),
            ("/h/.local/share/sheldon/repos/github.com/zsh-users/zsh-completions/c.zsh", pair("sheldon", "zsh-users/zsh-completions")),
            ("/h/.zgenom/sources/zsh-users/zsh-history-substring-search/___/x.zsh", pair("zgenom", "zsh-users/zsh-history-substring-search")),
            ("/h/.zgen/zsh-users/zsh-syntax-highlighting-master/x.zsh", pair("zgen", "zsh-users/zsh-syntax-highlighting")),
            ("/h/.zprezto/modules/git/init.zsh", pair("prezto", "git")),
            ("/h/.zim/modules/fzf/init.zsh", pair("zim", "fzf")),
            ("/opt/stuff/fzf-tab.plugin.zsh", pair("manual", "fzf-tab")),
        ];
        for (path, want) in cases {
            assert_eq!(one(path), want, "{path}");
        }
    }

    #[test]
    fn ordinary_files_are_not_plugins() {
        for path in ["/h/.zshrc", "/h/.zshenv", "/etc/zshrc", "/h/dotfiles/aliases.zsh", "/h/.zinit/bin/zinit.zsh"] {
            assert_eq!(one(path), None, "{path}");
        }
    }

    #[test]
    fn detect_plugins_dedups_in_first_seen_order() {
        let sourced = vec![
            "/h/.zinit/plugins/a---b/one.zsh".to_string(),
            "/h/.zshrc".to_string(),
            "/h/.zinit/plugins/c---d/two.zsh".to_string(),
            "/h/.zinit/plugins/a---b/three.zsh".to_string(),
        ];
        let fpath = vec!["/h/.zinit/plugins/c---d".to_string(), "/h/.oh-my-zsh/plugins/git".to_string()];
        assert_eq!(
            detect_plugins(&sourced, &fpath),
            vec![
                ("zinit".to_string(), "a/b".to_string()),
                ("zinit".to_string(), "c/d".to_string()),
                ("oh-my-zsh".to_string(), "git".to_string()),
            ]
        );
    }
}

#[cfg(test)]
mod catalog_history_tests {
    use super::*;

    fn ev(kind: &str, name: &str, value: &str, file: &str, line: u32) -> BundleEvent {
        BundleEvent {
            order_idx: 0,
            ts_ns: 0,
            kind: kind.to_string(),
            name: name.to_string(),
            value: Some(value.to_string()),
            file: Some(file.to_string()),
            line: Some(line),
            fn_chain: Some("load ← init".to_string()),
            attrs: 0,
            value_array: None,
            value_assoc: None,
        }
    }

    fn bundle(events: Vec<BundleEvent>) -> Bundle {
        Bundle {
            started_at_ns: 0,
            finished_at_ns: 1,
            cmdline: None,
            zdotdir: None,
            home: None,
            events,
            shell_id: None,
            end_state: None,
        }
    }

    #[test]
    fn every_definition_of_a_redefined_name_survives_with_its_site() {
        let b = bundle(vec![
            ev("alias", "gst", "git status", "/h/a.zsh", 3),
            ev("alias", "gco", "git checkout", "/h/a.zsh", 4),
            ev("alias", "gst", "git status -sb", "/h/b.zsh", 1),
            ev("function", "gst", "echo hi", "/h/c.zsh", 9),
            ev("alias", "gst", "git status --short", "/h/main.zsh", 3),
        ]);
        let shard = build_shard(&b, &fold_bundle(&b));
        let hist = decode_catalog_history(&shard.extras);
        let alias_chain = &hist[&("alias".to_string(), "gst".to_string())];
        let sites: Vec<_> = alias_chain.iter().map(|(_, f, l, _)| (f.clone().unwrap(), l.unwrap())).collect();
        assert_eq!(
            sites,
            vec![("/h/a.zsh".into(), 3), ("/h/b.zsh".into(), 1), ("/h/main.zsh".into(), 3)]
        );
        assert_eq!(alias_chain[1].0, "\"git status -sb\"");
        assert_eq!(alias_chain[1].3.as_deref(), Some("load ← init"));
        // A name defined once has a chain of one, and the same name in another
        // subsystem is its own chain.
        assert_eq!(hist[&("alias".to_string(), "gco".to_string())].len(), 1);
        assert_eq!(hist[&("function".to_string(), "gst".to_string())].len(), 1);
    }
}
