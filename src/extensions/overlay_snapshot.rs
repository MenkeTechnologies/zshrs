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
        let opts_snap = crate::ported::options::opt_state_snapshot();
        if !opts_snap.is_empty() {
            // Bool → string ("on"/"off") so the canonical store's
            // string-only value column doesn't have to special-case.
            let map: serde_json::Map<String, Value> = opts_snap
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        Value::String(if *v { "on" } else { "off" }.into()),
                    )
                })
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

        // named_dir: hash -d entries from canonical `nameddirtab`
        // (port of `Src/hashnameddir.c`).
        let nd_snap: Vec<(String, String)> = crate::ported::hashnameddir::nameddirtab()
            .lock()
            .ok()
            .map(|g| g.iter().map(|(k, v)| (k.clone(), v.dir.clone())).collect())
            .unwrap_or_default();
        if !nd_snap.is_empty() {
            let map: serde_json::Map<String, Value> = nd_snap
                .into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect();
            out.push(("named_dir".into(), Value::Object(map)));
        }
        let _ = exec; // keep param naming convention for symmetry

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // === entries_to_json: pair-slice serialization ===

    #[test]
    fn entries_to_json_empty_yields_empty_object() {
        let v = entries_to_json(&[]);
        assert!(v.is_object(), "result must be a JSON object");
        assert_eq!(v.as_object().unwrap().len(), 0);
    }

    #[test]
    fn entries_to_json_preserves_keys_and_string_values() {
        let entries = vec![
            ("ll".to_string(), "ls -la".to_string()),
            ("gs".to_string(), "git status".to_string()),
            ("..".to_string(), "cd ..".to_string()),
        ];
        let v = entries_to_json(&entries);
        let obj = v.as_object().expect("object");
        assert_eq!(obj.len(), 3);
        assert_eq!(obj.get("ll").and_then(Value::as_str), Some("ls -la"));
        assert_eq!(obj.get("gs").and_then(Value::as_str), Some("git status"));
        assert_eq!(obj.get("..").and_then(Value::as_str), Some("cd .."));
    }

    #[test]
    fn entries_to_json_all_values_are_string_type() {
        // Wire-format invariant: every value is JSON string (not int,
        // not array). The daemon side relies on this for `zsync pull`
        // to know it can re-emit the value verbatim as an alias.
        let entries = vec![
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "true".to_string()),
            ("c".to_string(), "null".to_string()),
        ];
        let v = entries_to_json(&entries);
        for (_, val) in v.as_object().expect("object").iter() {
            assert!(val.is_string(), "every value must serialize as string");
        }
    }

    #[test]
    fn entries_to_json_handles_duplicate_keys_last_wins() {
        // Vec<(String, String)> can legitimately carry duplicates if
        // the upstream entries() accessor doesn't dedupe; serde_json
        // `Map::collect` on duplicates keeps the LAST value, matching
        // standard JSON object semantics.
        let entries = vec![
            ("k".to_string(), "first".to_string()),
            ("k".to_string(), "second".to_string()),
        ];
        let v = entries_to_json(&entries);
        let obj = v.as_object().expect("object");
        assert_eq!(obj.len(), 1, "dup keys collapse to one slot");
        assert_eq!(obj.get("k").and_then(Value::as_str), Some("second"));
    }

    #[test]
    fn entries_to_json_empty_string_values_preserved() {
        // `alias foo=` produces an empty value — must survive the
        // snapshot, not get coerced to JSON null / missing key.
        let entries = vec![("empty".to_string(), "".to_string())];
        let v = entries_to_json(&entries);
        assert_eq!(
            v.as_object().unwrap().get("empty").and_then(Value::as_str),
            Some("")
        );
    }

    #[test]
    fn entries_to_json_unicode_keys_and_values() {
        // Verify the JSON encoder handles arbitrary UTF-8 in both
        // positions (alias names and bodies can contain anything).
        let entries = vec![
            ("café".to_string(), "echo 茶".to_string()),
            ("→".to_string(), "echo ←".to_string()),
        ];
        let v = entries_to_json(&entries);
        let obj = v.as_object().expect("object");
        assert_eq!(obj.get("café").and_then(Value::as_str), Some("echo 茶"));
        assert_eq!(obj.get("→").and_then(Value::as_str), Some("echo ←"));
    }

    // === map_to_json: iterator-based serialization ===

    #[test]
    fn map_to_json_empty_iterator_yields_empty_object() {
        let empty: HashMap<String, String> = HashMap::new();
        let v = map_to_json(empty.iter());
        assert!(v.is_object());
        assert_eq!(v.as_object().unwrap().len(), 0);
    }

    #[test]
    fn map_to_json_round_trips_hashmap_entries() {
        let mut m: HashMap<String, String> = HashMap::new();
        m.insert("PATH".to_string(), "/usr/bin:/bin".to_string());
        m.insert("HOME".to_string(), "/home/user".to_string());
        m.insert("SHELL".to_string(), "/usr/bin/zsh".to_string());

        let v = map_to_json(m.iter());
        let obj = v.as_object().expect("object");
        assert_eq!(obj.len(), 3);
        assert_eq!(
            obj.get("PATH").and_then(Value::as_str),
            Some("/usr/bin:/bin")
        );
        assert_eq!(obj.get("HOME").and_then(Value::as_str), Some("/home/user"));
        assert_eq!(
            obj.get("SHELL").and_then(Value::as_str),
            Some("/usr/bin/zsh")
        );
    }

    #[test]
    fn map_to_json_clones_owned_strings() {
        // map_to_json takes references — the produced JSON must own
        // its strings so the source map can drop without affecting it.
        let mut m: HashMap<String, String> = HashMap::new();
        m.insert("k".to_string(), "v".to_string());
        let v = map_to_json(m.iter());
        drop(m);
        // Still usable after source drop.
        assert_eq!(
            v.as_object().unwrap().get("k").and_then(Value::as_str),
            Some("v")
        );
    }

    #[test]
    fn map_to_json_values_serialize_as_strings_not_inferred_types() {
        let mut m: HashMap<String, String> = HashMap::new();
        m.insert("INT_LIKE".to_string(), "42".to_string());
        m.insert("BOOL_LIKE".to_string(), "true".to_string());
        m.insert("NULL_LIKE".to_string(), "null".to_string());
        let v = map_to_json(m.iter());
        for (_, val) in v.as_object().expect("object").iter() {
            assert!(
                val.is_string(),
                "shell values are always strings on the wire"
            );
        }
    }

    #[test]
    fn map_to_json_serialized_form_is_valid_json() {
        // End-to-end check: the Value can be serialized to a string
        // that round-trips back to an equal Value. Wire-protocol
        // smoke test for the daemon transport.
        let mut m: HashMap<String, String> = HashMap::new();
        m.insert("a".to_string(), "1".to_string());
        m.insert("b".to_string(), "two".to_string());

        let v = map_to_json(m.iter());
        let s = serde_json::to_string(&v).expect("serialize");
        let back: Value = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(back, v);
    }

    #[test]
    fn entries_to_json_serialized_form_is_valid_json() {
        let entries = vec![
            ("alpha".to_string(), "first".to_string()),
            ("omega".to_string(), "last".to_string()),
        ];
        let v = entries_to_json(&entries);
        let s = serde_json::to_string(&v).expect("serialize");
        let back: Value = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(back, v);
    }
}
