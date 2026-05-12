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
        let alias_e = exec.alias_entries();
        if !alias_e.is_empty() {
            out.push(("alias".into(), entries_to_json(&alias_e)));
        }
        let galias_e = exec.global_alias_entries();
        if !galias_e.is_empty() {
            out.push(("galias".into(), entries_to_json(&galias_e)));
        }
        let salias_e = exec.suffix_alias_entries();
        if !salias_e.is_empty() {
            out.push(("salias".into(), entries_to_json(&salias_e)));
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
        // Scalars from paramtab (canonical); arrays + assocs from
        // their respective stores. Iterate paramtab once for scalars
        // (entries with no u_arr — array entries set u_arr).
        let scalar_entries: Vec<(String, String)> =
            if let Ok(tab) = crate::ported::params::paramtab().read() {
                tab.iter()
                    .filter(|(_, pm)| pm.u_arr.is_none())
                    .map(|(k, pm)| (k.clone(), pm.u_str.clone().unwrap_or_default()))
                    .collect()
            } else {
                Vec::new()
            };
        let array_entries: Vec<(String, Vec<String>)> =
            if let Ok(tab) = crate::ported::params::paramtab().read() {
                tab.iter()
                    .filter_map(|(k, pm)| pm.u_arr.clone().map(|a| (k.clone(), a)))
                    .collect()
            } else {
                Vec::new()
            };
        let assoc_entries: Vec<(String, indexmap::IndexMap<String, String>)> =
            if let Ok(m) = crate::ported::params::paramtab_hashed_storage().lock() {
                m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
            } else {
                Vec::new()
            };
        if !scalar_entries.is_empty() || !array_entries.is_empty() || !assoc_entries.is_empty() {
            let mut params = serde_json::Map::new();
            for (k, v) in scalar_entries {
                params.insert(k, Value::String(v));
            }
            for (k, v) in array_entries {
                params.insert(
                    k,
                    Value::Array(v.iter().map(|s| Value::String(s.clone())).collect()),
                );
            }
            for (k, v) in assoc_entries {
                let inner: serde_json::Map<String, Value> = v
                    .iter()
                    .map(|(ik, iv)| (ik.clone(), Value::String(iv.clone())))
                    .collect();
                params.insert(k, Value::Object(inner));
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

        // zstyle: ordered Vec<zstyle_entry>. Serialize with debug to
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

/// Take owned `(name, value)` pairs (from alias_entries() etc.) and
/// serialize as a JSON object. Used for the alias snapshots which
/// now read from canonical aliastab via accessors rather than from
/// a HashMap reference.
fn entries_to_json(entries: &[(String, String)]) -> Value {
    let map: serde_json::Map<String, Value> = entries
        .iter()
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect();
    Value::Object(map)
}
