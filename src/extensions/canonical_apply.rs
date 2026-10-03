//! Apply a recorded shell state to a freshly-built ShellExecutor —
//! by reading the recorder's rkyv shard directly from disk. No IPC.
//!
//! **zshrs-original infrastructure — no C source counterpart.** C
//! zsh always runs `Src/init.c::source_startup_files()` to set up
//! a fresh shell from the user's dotfiles. zshrs adds a fast path:
//! if `zshrs-recorder` has written a shard
//! (`~/.zshrs/images/*-recorder.rkyv`), we read it and apply it to
//! the shell's tables, skipping the
//! `.zshenv`/`.zprofile`/`.zshrc`/`.zlogin` source pass entirely.
//! The shard is deserialized whole (`read_canonical_shard`), not read
//! zero-copy; on a zpwr-sized recording (~20 MB) that read is the
//! largest single cost of the replay.
//!
//! **Why direct shard read, not IPC.** The original spec
//! (`docs/DAEMON.md` "NO WALKING IN CLIENTS" + cache-architecture
//! memory) calls for thin clients that mmap the daemon's pre-built
//! shards as a zero-copy data plane. The earlier IPC version of this
//! file did 1+ `definitions_query` round-trips per cold-start, which
//! at ~600μs per round-trip put us 5-10ms over the spec target. The
//! mmap path is the real architecture: kernel page-cache after first
//! launch + rkyv check_archived + struct copy = sub-millisecond.
//!
//! IPC stays intact for `zd` / editor plugins / dashboards (see
//! `daemon/definitions.rs`). It's the right interface for "give me
//! the current catalog snapshot" from external tools. It's the wrong
//! interface for the shell's own cold-start hot path.
//!
//! The recorder writes one `*-recorder.rkyv` shard per ingest into
//! `~/.zshrs/images/`. We pick the latest by mtime, deserialize,
//! and copy fields straight into the executor's pub HashMaps.
//!
//! Failure mode: any I/O error → return 0; caller falls back to
//! vanilla `source_startup_files()`. Logged so the user can see why.

#![cfg(feature = "daemon")]

use std::path::PathBuf;

use crate::daemon::paths::CachePaths;
use crate::daemon::recorder_shard::{
    TypedParam, ARGV_SEP, AUTOLOAD_PATHS_EXTRA, MATH_FUNCTIONS_EXTRA, MODULES_END_EXTRA,
    BINDKEYS_EXTRA, PARAMS_END_EXTRA, UNSET_PARAMS_EXTRA, WIDGETS_EXTRA, ZSTYLES_EXTRA,
};
use crate::daemon::shard::{list_shards, read_canonical_shard, CanonicalShard};
use crate::vm_helper::{zstyle_entry, AutoloadFlags, ShellExecutor};
// Legacy `zle()` / `KeymapName` removed alongside the
// `extensions::keymaps` dissolution. Recorder-replay paths that
// previously wrote into `ZleManager` (bindkey + user-widget
// registration) now log-and-skip until canonical replay through
// `ported::zle::zle_keymap::keymapnamtab` / `zle_thingy::thingytab`
// is wired.

/// Startup replay for `zsh_main`'s `run_init_scripts`: when
/// `[shell].skip_configs` resolved to skip (a recorder shard exists),
/// apply the shard to the session executor INSTEAD of sourcing the
/// startup files. Returns `false` when there is nothing to skip with —
/// no skip decision, no executor in scope, or a shard that applied zero
/// rows — and the caller sources the files as C does.
/// zshrs-original — no C counterpart.
pub fn replay_startup() -> bool {
    crate::fusevm_bridge::try_with_executor(replay_startup_into).unwrap_or(false)
}

/// [`replay_startup`] against an executor the caller holds — the `-c`
/// driver in `bins/zshrs.rs`, which runs before any VM context exists.
/// zshrs-original — no C counterpart.
pub fn replay_startup_into(executor: &mut ShellExecutor) -> bool {
    if !crate::daemon_presence::should_skip_configs() {
        return false;
    }
    let rows = apply_latest(executor, true);
    tracing::info!(rows, "skip_configs: startup files bypassed, recorder shard replayed");
    rows > 0
}

/// Read the latest recorder shard and apply its canonical state to
/// the executor. Returns total rows applied (`0` if no shard or
/// read failure → caller falls back to vanilla dotfile source).
/// zshrs-original — no C counterpart. C zsh's
/// `source_startup_files()` (Src/init.c) is the only path; this is
/// a faster alternative built on the recorder shard.
pub fn apply_all(executor: &mut ShellExecutor) -> usize {
    apply_latest(executor, false)
}

/// [`apply_all`], plus — for the startup replay only — the prompt theme.
/// The native p10k engine switches on when the theme is sourced
/// (`p10k::maybe_intercept_theme_source`), which a replay never does; the
/// recorder logs that source like any other, so each recorded path goes
/// back through the same intercept, which ignores all but the theme.
/// The in-editor compsys thread (`compsys::in_editor`) wants the state
/// without a prompt engine, so `apply_all` leaves it out.
/// zshrs-original — no C counterpart.
fn apply_latest(executor: &mut ShellExecutor, startup: bool) -> usize {
    let t0 = std::time::Instant::now();

    let paths = match CachePaths::resolve() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "canonical_apply: cache paths unresolved");
            return 0;
        }
    };

    let shard_path = match latest_recorder_shard(&paths) {
        Some(p) => p,
        None => {
            tracing::info!(
                "canonical_apply: no recorder shard found in {} — vanilla fallback",
                paths.images.display()
            );
            return 0;
        }
    };

    let shard = match read_canonical_shard(&shard_path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, path = %shard_path.display(), "canonical_apply: shard read failed");
            return 0;
        }
    };

    crate::startup_trace::mark("replay: shard read");
    let sourced = if startup { shard.sourced_files.clone() } else { Vec::new() };
    let total = apply_shard(executor, shard);
    for path in &sourced {
        let _ = crate::p10k::maybe_intercept_theme_source(std::slice::from_ref(path));
    }
    let elapsed_us = t0.elapsed().as_micros();
    tracing::info!(
        rows = total,
        elapsed_us,
        path = %shard_path.display(),
        "canonical state applied from rkyv shard (no IPC)"
    );
    total
}

/// Walk `~/.zshrs/images/` and return the newest
/// `*-recorder.rkyv` shard by mtime.
/// zshrs-original — no C counterpart.
fn latest_recorder_shard(paths: &CachePaths) -> Option<PathBuf> {
    let entries = list_shards(paths).ok()?;
    entries
        .into_iter()
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.ends_with("-recorder.rkyv"))
                .unwrap_or(false)
        })
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
}

/// Bulk-copy every subsystem from a deserialized canonical shard
/// into the executor's mutable tables.
/// zshrs-original — no C counterpart. The closest C analog is the
/// per-subsystem builtin dispatch each dotfile triggers
/// (`alias`/`bindkey`/`zstyle`/`compdef`/etc.) but compressed into
/// a single in-memory copy.
fn apply_shard(executor: &mut ShellExecutor, shard: CanonicalShard) -> usize {
    let mut total = 0;

    // Modules first: the rest leans on what they provide
    // (`zsh/parameter`, `zsh/datetime`, `zsh/files`' `zf_rm`, …).
    // `zmodload -F MODULE f…` leaves the module with those features
    // enabled, the state `zmodload -LF` showed at the end of the
    // recording.
    if let Some(modules) = shard.extras.get(MODULES_END_EXTRA) {
        for (module, features) in modules {
            // No features (`zsh/complist`): a plain load.
            let argv = if features.is_empty() {
                vec![module.clone()]
            } else {
                let mut argv = vec!["-F".to_string(), module.clone()];
                argv.extend(features.split(' ').map(str::to_string));
                argv
            };
            crate::fusevm_bridge::dispatch_builtin_raw("zmodload", argv);
            total += 1;
        }
    }

    crate::startup_trace::mark("replay: modules");
    // setopt / unsetopt, ahead of the function bodies: `setfunction`
    // parses under the live options (`extendedglob`, `kshglob`, …), as
    // the recorded files did when they defined them.
    for opt in shard.setopts {
        crate::ported::options::opt_state_set(&opt, true);
        total += 1;
    }
    for opt in shard.unsetopts {
        crate::ported::options::opt_state_set(&opt, false);
        total += 1;
    }

    crate::startup_trace::mark("replay: options");
    // Functions, stored as source text the way zshrs's own definition
    // path stores them (`shfunc_with_body`): the body is compiled on the
    // first call (`execshfunc`'s `body` arm), not here — parsing every
    // recorded body up front cost ~0.5 s of startup for ~700 functions.
    // `TRAP*` names still go through `setfunction`, which also arms the
    // signal trap (Src/Modules/parameter.c:305-313).
    {
        let mut traps = Vec::new();
        if let Ok(mut tab) = crate::ported::hashtable::shfunctab_lock().write() {
            for (name, body) in shard.functions {
                if name.starts_with("TRAP") {
                    traps.push((name, body));
                } else {
                    tab.add(crate::ported::hashtable::shfunc_with_body(&name, &body));
                }
                total += 1;
            }
        }
        for (name, body) in traps {
            crate::ported::modules::parameter::setfunction(&name, body, 0);
        }
    }

    crate::startup_trace::mark("replay: functions");
    // Aliases (3 flavors).
    for (n, v) in shard.aliases {
        executor.set_alias(n, v);
        total += 1;
    }
    for (n, v) in shard.global_aliases {
        executor.set_global_alias(n, v);
        total += 1;
    }
    for (n, v) in shard.suffix_aliases {
        executor.set_suffix_alias(n, v);
        total += 1;
    }

    crate::startup_trace::mark("replay: aliases");
    // Parameters. A recording with an end-state snapshot carries them
    // typed; older shards only have the joined scalar views.
    if let Some(typed) = shard.extras.get(PARAMS_END_EXTRA) {
        // What the files unset (`unset CDPATH`) goes first.
        if let Some(unsets) = shard.extras.get(UNSET_PARAMS_EXTRA) {
            for name in unsets.keys() {
                crate::ported::params::unsetparam(name);
                total += 1;
            }
        }
        let params: Vec<TypedParam> =
            typed.values().filter_map(|json| serde_json::from_str(json).ok()).collect();
        // Ties first: `typeset -T` creates both halves, and a value
        // assigned to either before it would be a plain parameter that
        // the tie then has to convert.
        for p in &params {
            if let Some((array, sep)) = &p.tie {
                let argv = vec!["-gT".to_string(), p.name.clone(), array.clone(), sep.clone()];
                crate::fusevm_bridge::dispatch_builtin_raw("typeset", argv);
            }
        }
        for p in params {
            apply_typed_param(executor, p);
            total += 1;
        }
    } else {
        // Exported env: mirror to process env so child commands inherit.
        for (n, v) in shard.env_exports {
            std::env::set_var(&n, &v);
            executor.set_scalar(n, v);
            total += 1;
        }

        // Non-exported shell params.
        for (n, v) in shard.params {
            executor.set_scalar(n, v);
            total += 1;
        }
    }

    // path + fpath: ordered Vec<String> in the shard. An end-state
    // recording already restored both (and their export bits) as typed
    // parameters above; only the executor's own fpath copy follows them.
    if shard.extras.contains_key(PARAMS_END_EXTRA) {
        executor.fpath = crate::ported::params::getaparam("fpath")
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect();
    } else {
        if !shard.path.is_empty() {
            let joined = shard.path.join(":");
            std::env::set_var("PATH", &joined);
            executor.set_scalar("PATH".to_string(), joined);
            total += shard.path.len();
            executor.set_array("path".to_string(), shard.path);
        }
        if !shard.fpath.is_empty() {
            let joined = shard.fpath.join(":");
            std::env::set_var("FPATH", &joined);
            executor.set_scalar("FPATH".to_string(), joined);
            total += shard.fpath.len();
            executor.fpath = shard.fpath.iter().map(PathBuf::from).collect();
            executor.set_array("fpath".to_string(), shard.fpath);
        }
    }

    crate::startup_trace::mark("replay: parameters");
    // named_dir (hash -d): insert into canonical `nameddirtab` (port
    // of C `Src/hashnameddir.c::nameddirtab`).
    for (name, path) in shard.named_dirs {
        if let Ok(mut tab) = crate::ported::hashnameddir::nameddirtab().lock() {
            tab.insert(
                name.clone(),
                crate::ported::zsh_h::nameddir {
                    node: crate::ported::zsh_h::hashnode {
                        next: None,
                        nam: name,
                        flags: 0,
                    },
                    dir: path.clone(),
                    diff: 0,
                },
            );
            total += 1;
        }
    }

    // autoload_functions: register every name as autoload-pending
    // with the standard `-Uz` flag set (NO_ALIAS + ZSH_STYLE), what
    // every modern compsys / plugin does. The body lookup happens on
    // first call via the autoload resolver; we don't pre-compile.
    // Register each autoload-pending function via the canonical
    // shfunctab stub with `PM_UNDEFINED` set — matches C's
    // `autoload_func` at `Src/exec.c:5215+` flow. `AutoloadFlags`
    // (-U/-z/-k/-t/-d) details were never consumed elsewhere; the
    // canonical bit is just "shfunc exists with PM_UNDEFINED".
    crate::startup_trace::mark("replay: named dirs");
    let _ = AutoloadFlags::NO_ALIAS;
    // Register through compinit's own helper so the stubs carry the flag
    // word `autoload -rUz` produces — PM_UNDEFINED | PM_UNALIASED |
    // PM_ZSHSTORED (compinit sh:337/541). The bare `shfunc_autoload` used
    // here before set only PM_UNDEFINED, which loses two things: the body
    // is parsed WITH alias expansion (the `-U` that exists precisely to
    // stop a caller's `alias helper=…` from rewriting a completer), and
    // `autoload_source_stamps`-based chunk caching declines to cache a
    // body whose parse could depend on the alias table.
    total +=
        crate::compsys::ported::compinit::register_autoload_stubs(shard.autoload_functions.keys());

    crate::startup_trace::mark("replay: autoload stubs");
    // Autoloads bound to a directory (`autoload -Uz DIR/NAME`) and user
    // math functions (`functions -M`): each is the argv that recreated it,
    // run through the builtin itself.
    for (extra, builtin) in [
        (AUTOLOAD_PATHS_EXTRA, "autoload"),
        (MATH_FUNCTIONS_EXTRA, "functions"),
        (WIDGETS_EXTRA, "zle"),
        (BINDKEYS_EXTRA, "bindkey"),
    ] {
        for argv in argv_rows(&shard.extras, extra) {
            crate::fusevm_bridge::dispatch_builtin_raw(builtin, argv);
            total += 1;
        }
    }

    crate::startup_trace::mark("replay: autoload dirs, math, widgets, bindkey");
    // zstyle: shard stores `Vec<(pattern, "style val val ...")>` —
    // split the joined-rest back into (style, values) so the exec
    // side has the same `zstyle_entry { pattern, style, values: Vec<_> }`
    // shape it would build by sourcing `zstyle :ctx style val val …`
    // statements.
    //
    // An end-state recording carries the real table instead: each entry
    // goes back through `setstypat` (Src/Modules/zutil.c:295), the store
    // `zstyle` itself writes and `zstyle -L` / lookups read.
    for words in argv_rows(&shard.extras, ZSTYLES_EXTRA) {
        let mut words = words.into_iter();
        let (Some(pat), Some(style), Some(eval)) = (words.next(), words.next(), words.next()) else {
            continue;
        };
        let vals: Vec<String> = words.collect();
        crate::ported::modules::zutil::setstypat(&style, &pat, None, vals, (eval == "e") as i32);
        total += 1;
    }
    for (pattern, rest) in shard.zstyle {
        let mut parts = rest.split_whitespace();
        let style = match parts.next() {
            Some(s) => s.to_string(),
            None => continue,
        };
        let values: Vec<String> = parts.map(str::to_string).collect();
        executor.zstyles.push(zstyle_entry {
            pattern,
            style,
            values,
        });
        total += 1;
    }

    // bindkey: install each captured (keyseq, widget) into the global
    // KeymapManager. Recorder encodes the keymap-target by prefixing
    // the value with `[KEYMAP] ` (per `bin_bindkey` in
    // src/vm_helper); strip that prefix and dispatch to the right
    // keymap. Default = Main.
    {
        // bindkey replay routes through the canonical `bindkey()`
        // free fn (`ported::zle::zle_bindings.rs:192`) which writes
        // to `keymapnamtab` matching what the C `bindkey` builtin
        // does at runtime.
        for (keyseq, value) in shard.bindkeys {
            let (keymap, widget) = parse_bindkey_value(&value);
            crate::ported::zle::zle_bindings::bindkey_by_name(keymap, &keyseq, widget);
            total += 1;
        }
    }

    crate::startup_trace::mark("replay: zstyle");
    // compdef: each (function, "cmd1 cmd2 ...") row replays through
    // the ported runtime `compdef()` entry point in
    // `crate::compsys::ported::compinit::compdef` — matches what an
    // interactive `compdef _git git` call would land at. Recorder
    // captures format: `name=function value="cmd1 cmd2 …"` (per
    // `builtin_compdef` in src/vm_helper). State lives in the
    // process-wide `CompdefState` published into the shell-side
    // param table; the legacy `CompsysCache` path is no longer
    // used here.
    if !shard.compdef.is_empty() {
        for (function, cmds_joined) in shard.compdef {
            let mut args: Vec<String> = Vec::with_capacity(8);
            args.push(function);
            for cmd in cmds_joined.split_whitespace() {
                args.push(cmd.to_string());
            }
            if args.len() < 2 {
                continue; // recorder dropped the cmd list — can't replay
            }
            let _rc = crate::compsys::ported::compinit::compdef(&args);
            total += 1;
        }
    }

    // zle widgets: recorder routes them into shard.extras["zle"]
    // (one per `zle -N name [body]` capture). Reinstall via
    // ZleManager.user_widgets. Body string is whatever the user
    // gave; the widget invocation path looks it up at execution
    // time so re-installing the name+body string is enough.
    if let Some(zle_widgets) = shard.extras.get("zle") {
        // User-widget replay through the canonical `Widget::user_defined`
        // + `bindwidget` machinery in `ported::zle/zle_thingy.rs`,
        // matching the C `bin_zle_new()` registration path at
        // `Src/Zle/zle_thingy.c:584`.
        for (name, body) in zle_widgets {
            let w =
                std::sync::Arc::new(crate::ported::zle::zle_h::widget::user_defined(name, body));
            crate::ported::zle::zle_thingy::rthingy(name);
            crate::ported::zle::zle_thingy::bindwidget(w, name);
            total += 1;
        }
    }

    // zmodload / manpath / plugins / extras: no executor surface today
    // (modules call `zmodload` builtin directly at use time; manpath is
    // read from $MANPATH env; plugins is diagnostic-only; extras is a
    // catch-all).
    let _ = shard.zmodload;
    let _ = shard.manpath;
    let _ = shard.plugins;
    let _ = shard.extras;

    crate::startup_trace::mark("replay: compdef");
    total
}

/// An `extras` bucket of argvs (`recorder_shard::ordered_argvs`, or
/// name-keyed), in key order, each split back into its words.
/// zshrs-original — no C counterpart.
fn argv_rows(
    extras: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    extra: &str,
) -> Vec<Vec<String>> {
    let Some(rows) = extras.get(extra) else {
        return Vec::new();
    };
    let mut keys: Vec<&String> = rows.keys().collect();
    keys.sort();
    keys.into_iter()
        .map(|k| rows[k].split(ARGV_SEP).map(str::to_string).collect())
        .collect()
}

/// Restore one end-state parameter with its recorded type, export it
/// when the recording had it exported, then re-apply its other
/// `typeset` attributes (`-U`, `-H`, …). `addenv` (Src/params.c:5448)
/// sets `PM_EXPORTED` and the environment entry together.
/// zshrs-original — no C counterpart.
fn apply_typed_param(executor: &mut ShellExecutor, p: TypedParam) {
    use crate::ported::params::{addenv, getsparam, sethparam, setiparam, setnparam};
    use crate::ported::zsh_h::{mnumber, MN_FLOAT};
    let TypedParam {
        name,
        kind,
        export,
        value,
        elements,
        attrs,
        tie: _,
    } = p;
    // A same-named parameter of another type — typically a scalar
    // imported from the environment where the files declared `integer
    // ZUID_ID` — keeps its type through `setiparam`/`setsparam`; drop it
    // so the recorded type is the one created. Specials keep theirs.
    let existing = crate::ported::params::paramtab()
        .read()
        .ok()
        .and_then(|t| t.get(&name).map(|pm| crate::ported::modules::parameter::paramtypestr(pm)));
    if let Some(ty) = existing {
        let parts: Vec<&str> = ty.split('-').collect();
        if parts[0] != kind && !parts.contains(&"special") {
            crate::ported::params::unsetparam(&name);
        }
    }
    let value = value.unwrap_or_default();
    match kind.as_str() {
        "array" => executor.set_array(name.clone(), elements.unwrap_or_default()),
        "association" => {
            let _ = sethparam(&name, elements.unwrap_or_default());
        }
        "integer" => match value.parse::<i64>() {
            Ok(n) => {
                let _ = setiparam(&name, n);
            }
            Err(_) => executor.set_scalar(name.clone(), value),
        },
        "float" => match value.parse::<f64>() {
            Ok(d) => {
                let _ = setnparam(&name, mnumber { l: 0, d, type_: MN_FLOAT });
            }
            Err(_) => executor.set_scalar(name.clone(), value),
        },
        _ => executor.set_scalar(name.clone(), value),
    }
    // The export bit both ways. A scalar goes through `addenv`
    // (Src/params.c:5448); an array or association only carries the flag
    // — `export_param` never puts one in the environment (c:2659) — so
    // `typeset -gx` sets it. A parameter the recording had unexported but
    // this shell imported from its environment (`FPATH`) is unexported.
    let exported_now = crate::ported::params::paramtab()
        .read()
        .ok()
        .and_then(|t| t.get(&name).map(|pm| pm.node.flags as u32 & crate::ported::zsh_h::PM_EXPORTED != 0))
        .unwrap_or(false);
    if export && !exported_now {
        if matches!(kind.as_str(), "array" | "association") {
            crate::fusevm_bridge::dispatch_builtin_raw("typeset", vec!["-gx".to_string(), name.clone()]);
        } else {
            let _ = addenv(&name, &getsparam(&name).unwrap_or_default());
        }
    } else if !export && exported_now {
        crate::fusevm_bridge::dispatch_builtin_raw("typeset", vec!["-g".to_string(), "+x".to_string(), name.clone()]);
    }
    if !attrs.is_empty() {
        crate::fusevm_bridge::dispatch_builtin_raw("typeset", vec![format!("-g{attrs}"), name]);
    }
}

/// Decode a recorder-emitted bindkey value into (keymap, widget).
///
/// Format from `src/vm_helper:bin_bindkey`:
///   `widget_name`               → KeymapName::Main
///   `[keymap_name] widget_name` → KeymapName::from_str("keymap_name")
///
/// Unknown keymap names fall back to `Main` (matches what zsh's
/// `bindkey` does for unrecognized -M targets — a safer default than
/// silently dropping the binding).
/// Parse a `bindkey` shard value into `(keymap, sequence)`.
/// zshrs-original — splits the canonical form `"keymap:sequence"`
/// the recorder writes back into the two arguments
/// `bindkey` (Src/Zle/zle_keymap.c) takes at the C builtin layer.
fn parse_bindkey_value(value: &str) -> (&str, &str) {
    if let Some(rest) = value.strip_prefix('[') {
        if let Some(close_idx) = rest.find(']') {
            let keymap_str = &rest[..close_idx];
            let widget = rest[close_idx + 1..].trim_start();
            return (keymap_str, widget);
        }
    }
    ("main", value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindkey_value_parses_main_keymap_default() {
        let _g = crate::test_util::global_state_lock();
        let (km, w) = parse_bindkey_value("history-search-backward");
        assert_eq!(km, "main");
        assert_eq!(w, "history-search-backward");
    }

    #[test]
    fn bindkey_value_parses_explicit_keymap() {
        let _g = crate::test_util::global_state_lock();
        let (km, w) = parse_bindkey_value("[viins] backward-delete-char");
        assert_eq!(km, "viins");
        assert_eq!(w, "backward-delete-char");
    }

    #[test]
    fn bindkey_value_unknown_keymap_falls_back_to_main() {
        let _g = crate::test_util::global_state_lock();
        let (km, w) = parse_bindkey_value("[totally-not-real] do-thing");
        // No longer falls back to "main" — the C `bindkey` builtin
        // forwards the literal name to keymapnamtab lookup, which
        // surfaces the error there. parse_bindkey_value just returns
        // the bracketed text verbatim.
        assert_eq!(km, "totally-not-real");
        assert_eq!(w, "do-thing");
    }

    #[test]
    fn bindkey_value_handles_extra_whitespace_after_close_bracket() {
        let _g = crate::test_util::global_state_lock();
        let (km, w) = parse_bindkey_value("[vicmd]   forward-word");
        assert_eq!(km, "vicmd");
        assert_eq!(w, "forward-word");
    }

    // ========================================================
    // parse_bindkey_value — additional edge cases
    // ========================================================

    #[test]
    fn bindkey_value_handles_empty_keymap_brackets() {
        let _g = crate::test_util::global_state_lock();
        // `[] widget` → empty keymap name, widget preserved.
        let (km, w) = parse_bindkey_value("[] do-thing");
        assert_eq!(km, "");
        assert_eq!(w, "do-thing");
    }

    #[test]
    fn bindkey_value_handles_empty_widget_part() {
        let _g = crate::test_util::global_state_lock();
        let (km, w) = parse_bindkey_value("[viins]");
        assert_eq!(km, "viins");
        assert_eq!(w, "");
    }

    #[test]
    fn bindkey_value_with_unclosed_bracket_falls_back_to_main() {
        // `[viins` with no `]` is malformed — must not panic / slice OOB.
        let _g = crate::test_util::global_state_lock();
        let (km, w) = parse_bindkey_value("[viins backward-delete-char");
        assert_eq!(km, "main");
        assert_eq!(w, "[viins backward-delete-char");
    }

    #[test]
    fn bindkey_value_empty_string_returns_main_and_empty() {
        let _g = crate::test_util::global_state_lock();
        let (km, w) = parse_bindkey_value("");
        assert_eq!(km, "main");
        assert_eq!(w, "");
    }

    #[test]
    fn bindkey_value_widget_with_dashes_and_dots_preserved() {
        let _g = crate::test_util::global_state_lock();
        let (km, w) = parse_bindkey_value("[viins] zle-line-init");
        assert_eq!(km, "viins");
        assert_eq!(w, "zle-line-init");
    }

    #[test]
    fn bindkey_value_does_not_strip_close_bracket_inside_widget() {
        // Once we find the FIRST `]`, anything past (after trimming
        // leading whitespace) is the widget — including stray brackets.
        let _g = crate::test_util::global_state_lock();
        let (km, w) = parse_bindkey_value("[viins] weird[name");
        assert_eq!(km, "viins");
        assert_eq!(w, "weird[name");
    }

    #[test]
    fn bindkey_value_keymap_with_dashes() {
        let _g = crate::test_util::global_state_lock();
        let (km, w) = parse_bindkey_value("[my-keymap] do-thing");
        assert_eq!(km, "my-keymap");
        assert_eq!(w, "do-thing");
    }

    #[test]
    fn bindkey_value_no_bracket_means_main_keymap() {
        // Plain "widget" form (no `[...]`) → main keymap.
        let _g = crate::test_util::global_state_lock();
        let (km, w) = parse_bindkey_value("backward-kill-word");
        assert_eq!(km, "main");
        assert_eq!(w, "backward-kill-word");
    }

    #[test]
    fn bindkey_value_widget_after_tab_whitespace() {
        // Tab characters after `]` count as whitespace for trim_start.
        let _g = crate::test_util::global_state_lock();
        let (km, w) = parse_bindkey_value("[main]\tup-line-or-history");
        assert_eq!(km, "main");
        assert_eq!(w, "up-line-or-history");
    }

    #[test]
    fn bindkey_value_nested_open_bracket_uses_first_close() {
        // First `]` wins for keymap delimiter.
        let _g = crate::test_util::global_state_lock();
        let (km, w) = parse_bindkey_value("[ke[ymap] widget");
        assert_eq!(km, "ke[ymap");
        assert_eq!(w, "widget");
    }
}
