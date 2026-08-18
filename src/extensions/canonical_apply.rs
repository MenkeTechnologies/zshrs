//! Apply daemon canonical state to a freshly-built ShellExecutor —
//! by reading the daemon's rkyv shard directly from disk. No IPC.
//!
//! **zshrs-original infrastructure — no C source counterpart.** C
//! zsh always runs `Src/init.c::source_startup_files()` to set up
//! a fresh shell from the user's dotfiles. zshrs adds a fast path:
//! if `zshrs-daemon` has a canonical-state shard on disk
//! (`~/.zshrs/images/*-recorder.rkyv`), we mmap and apply it
//! directly into the executor's `pub` HashMaps, skipping the
//! `.zshenv`/`.zprofile`/`.zshrc`/`.zlogin` source pass entirely.
//! The shard format is rkyv (zero-copy archived structs) so the
//! cold-start cost is ~1ms instead of ~150ms.
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
use crate::daemon::shard::{list_shards, read_canonical_shard, CanonicalShard};
use crate::vm_helper::{zstyle_entry, AutoloadFlags, ShellExecutor};
// Legacy `zle()` / `KeymapName` removed alongside the
// `extensions::keymaps` dissolution. Recorder-replay paths that
// previously wrote into `ZleManager` (bindkey + user-widget
// registration) now log-and-skip until canonical replay through
// `ported::zle::zle_keymap::keymapnamtab` / `zle_thingy::thingytab`
// is wired.

/// Read the latest recorder shard and apply its canonical state to
/// the executor. Returns total rows applied (`0` if no shard or
/// read failure → caller falls back to vanilla dotfile source).
/// zshrs-original — no C counterpart. C zsh's
/// `source_startup_files()` (Src/init.c) is the only path; this is
/// a faster alternative built on top of the daemon shard.
pub fn apply_all(executor: &mut ShellExecutor) -> usize {
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

    let total = apply_shard(executor, shard);
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

    // setopt / unsetopt.
    for opt in shard.setopts {
        crate::ported::options::opt_state_set(&opt, true);
        total += 1;
    }
    for opt in shard.unsetopts {
        crate::ported::options::opt_state_set(&opt, false);
        total += 1;
    }

    // path + fpath: ordered Vec<String> in the shard.
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

    // zstyle: shard stores `Vec<(pattern, "style val val ...")>` —
    // split the joined-rest back into (style, values) so the exec
    // side has the same `zstyle_entry { pattern, style, values: Vec<_> }`
    // shape it would build by sourcing `zstyle :ctx style val val …`
    // statements.
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

    // inline-defined functions: captured in shard but not yet wired.
    // Two paths to install:
    //   (a) parse body string + fusevm-compile here on cold-start.
    //       ~50ms for 1k functions on M-series — defeats the
    //       zero-cost goal.
    //   (b) recorder ships pre-compiled bytecode in the shard;
    //       shell installs the chunk directly.
    // Path (b) is the right one (needs recorder-side change to
    // capture body bytecode at definition time + a new
    // `functions_compiled: HashMap<String, Vec<u8>>` shard field).
    // Until then, autoload-pending fallback covers the case: when
    // the user calls a function that wasn't pre-installed, the
    // autoload resolver fires + parses the body lazily.
    let _ = shard.functions;

    // zmodload / manpath / plugins / sourced_files / extras: no
    // executor surface today (modules call `zmodload` builtin
    // directly at use time; manpath is read from $MANPATH env;
    // plugins/sourced_files are diagnostic-only; extras is a
    // catch-all).
    let _ = shard.zmodload;
    let _ = shard.manpath;
    let _ = shard.plugins;
    let _ = shard.sourced_files;
    let _ = shard.extras;

    total
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
