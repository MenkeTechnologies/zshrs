//! Shell-side overlay enumeration for `zsync up --all`.
//!
//! **zshrs-original infrastructure — no C source counterpart.** C
//! zsh has no concept of pushing shell state to a daemon. Every
//! shell process owns its own parameter / option / alias / function
//! tables (Src/params.c, Src/options.c, Src/hashtable.c) and they
//! die with the process. zshrs adds the `zsync` builtin which
//! snapshots a running shell's mutable state into the daemon's
//! canonical store so other shells can pull it on startup.
//!
//! Snapshots every mutable executor table that has a corresponding
//! daemon-side canonical subsystem, so a single `zsync up --all`
//! call pushes the entire shell's overlay state up to the daemon
//! (where other shells can `zsync pull` it). The daemon-crate `zsync` builtin
//! invokes [`enumerate_all_overlays`] through the trampoline registered
//! at [`crate::daemon::zsync_builtin::register_overlay_enumerator`].
//!
//! Coverage today (matches `daemon/zsync_builtin.rs::ALL_SUBSYSTEMS`):
//!
//! | subsystem  | source                                      |
//! |------------|---------------------------------------------|
//! | `alias`    | `executor.aliases`                          |
//! | `galias`   | `executor.global_aliases`                   |
//! | `salias`   | `executor.suffix_aliases`                   |
//! | `setopt`   | `executor.options`                          |
//! | `params`   | `executor.{variables,arrays,assoc_arrays}`  |
//! | `env`      | `std::env::vars()`                          |
//! | `path`     | `$PATH`                                     |
//! | `manpath`  | `$MANPATH`                                  |
//! | `fpath`    | `executor.fpath`                            |
//! | `named_dir`| `executor.named_dirs`                       |
//! | `zstyle`   | `executor.zstyles`                          |
//! | `compdef`  | `executor.completions`                      |
//!
//! Skipped today (need richer wire format or aren't simply enumerable):
//! `function` (needs source-text preservation; bytecode in
//! `executor.functions_compiled` doesn't round-trip), `bindkey` (lives
//! in the global `ZleManager`), `zmodload` (no canonical
//! "currently-loaded" list).

use serde_json::{json, Value};

/// Build the full overlay snapshot.
/// One entry per non-empty subsystem; empty subsystems are omitted
/// to keep the wire payload minimal. Called by daemon-crate `zsync
/// up --all` through the registered trampoline.
/// zshrs-original — no C counterpart. C zsh's nearest analog is
/// the `typeset`/`alias`/`hash`/`set` listing builtins
/// (Src/builtin.c) which dump state to stdout; this serializes the
/// same kind of data into JSON for the canonical-state daemon.
pub fn enumerate_all_overlays() -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::new();

    crate::fusevm_bridge::with_executor(|exec| {
        // Plain string maps — alias / galias / salias / setopt all
        // follow the same shape: `{key: value}` JSON object.
        if !exec.aliases.is_empty() {
            out.push(("alias".into(), map_to_json(&exec.aliases)));
        }
        if !exec.global_aliases.is_empty() {
            out.push(("galias".into(), map_to_json(&exec.global_aliases)));
        }
        if !exec.suffix_aliases.is_empty() {
            out.push(("salias".into(), map_to_json(&exec.suffix_aliases)));
        }
        if !exec.options.is_empty() {
            // Bool → string ("on"/"off") so the canonical store's
            // string-only value column doesn't have to special-case.
            let map: serde_json::Map<String, Value> = exec
                .options
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(if *v { "on" } else { "off" }.into())))
                .collect();
            out.push(("setopt".into(), Value::Object(map)));
        }

        // params: scalars, arrays, and assoc maps merged into one
        // object. Arrays serialize as JSON arrays of strings; assoc
        // as nested objects. zsync's daemon-side push handler
        // accepts the union shape.
        if !exec.variables.is_empty() || !exec.arrays.is_empty() || !exec.assoc_arrays.is_empty() {
            let mut params = serde_json::Map::new();
            for (k, v) in &exec.variables {
                params.insert(k.clone(), Value::String(v.clone()));
            }
            for (k, v) in &exec.arrays {
                params.insert(
                    k.clone(),
                    Value::Array(v.iter().map(|s| Value::String(s.clone())).collect()),
                );
            }
            for (k, v) in &exec.assoc_arrays {
                let inner: serde_json::Map<String, Value> = v
                    .iter()
                    .map(|(ik, iv)| (ik.clone(), Value::String(iv.clone())))
                    .collect();
                params.insert(k.clone(), Value::Object(inner));
            }
            out.push(("params".into(), Value::Object(params)));
        }

        // fpath: ordered Vec<PathBuf> → JSON array of strings.
        if !exec.fpath.is_empty() {
            out.push((
                "fpath".into(),
                Value::Array(
                    exec.fpath
                        .iter()
                        .map(|p| Value::String(p.display().to_string()))
                        .collect(),
                ),
            ));
        }

        // named_dir: hash -d entries.
        if !exec.named_dirs.is_empty() {
            let map: serde_json::Map<String, Value> = exec
                .named_dirs
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.display().to_string())))
                .collect();
            out.push(("named_dir".into(), Value::Object(map)));
        }

        // compdef: completion specs are richer than scalars; we ship
        // the source command-list mapping (which is what compdef
        // canonical apply consumes — see canonical_apply.rs's
        // compdef block).
        if !exec.completions.is_empty() {
            let map: serde_json::Map<String, Value> = exec
                .completions
                .iter()
                .map(|(k, v)| (k.clone(), json!(format!("{:?}", v))))
                .collect();
            out.push(("compdef".into(), Value::Object(map)));
        }

        // zstyle: ordered Vec<ZStyle>. Serialize with debug to
        // capture every field deterministically; the daemon side
        // treats the value as opaque text today.
        if !exec.zstyles.is_empty() {
            let arr: Vec<Value> = exec
                .zstyles
                .iter()
                .map(|z| json!(format!("{:?}", z)))
                .collect();
            out.push(("zstyle".into(), Value::Array(arr)));
        }
    });

    // Process-env tables. These live outside the executor (they're
    // libc env, not a Rust HashMap), so we read them after the
    // with_executor borrow drops.
    let mut env_map = serde_json::Map::new();
    for (k, v) in std::env::vars() {
        env_map.insert(k, Value::String(v));
    }
    if !env_map.is_empty() {
        out.push(("env".into(), Value::Object(env_map)));
    }

    // Filter empty path components — they confuse the daemon's
    // strict "must be a directory" validation, and zsh treats an
    // empty entry in $PATH as the cwd (a misfeature most users want
    // to avoid persisting into canonical state).
    if let Ok(p) = std::env::var("PATH") {
        let parts: Vec<Value> = p
            .split(':')
            .filter(|s| !s.is_empty())
            .map(|s| Value::String(s.into()))
            .collect();
        if !parts.is_empty() {
            out.push(("path".into(), Value::Array(parts)));
        }
    }
    if let Ok(p) = std::env::var("MANPATH") {
        let parts: Vec<Value> = p
            .split(':')
            .filter(|s| !s.is_empty())
            .map(|s| Value::String(s.into()))
            .collect();
        if !parts.is_empty() {
            out.push(("manpath".into(), Value::Array(parts)));
        }
    }

    out
}

/// Serialize a string-keyed map as a JSON object.
/// zshrs-original convenience for the wire format `enumerate_all_overlays`
/// produces. No C counterpart.
fn map_to_json<'a, I>(iter: I) -> Value
where
    I: IntoIterator<Item = (&'a String, &'a String)>,
{
    let map: serde_json::Map<String, Value> = iter
        .into_iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    Value::Object(map)
}
