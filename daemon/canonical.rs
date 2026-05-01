// canonical.rs — rkyv-backed canonical state store.
//
// rkyv = source of truth (mmap'd shard); SQLite is a hydrated mirror for
// inspection only. Per docs/DAEMON.md "Cache layout" + "Daemon = sole writer".
//
// Daemon keeps the authoritative copy in memory (RwLock<HashMap<subsystem,
// HashMap<key, value>>>). Every mutation:
//
//   1. Updates the in-memory state atomically.
//   2. Persists the updated rkyv shard via atomic-rename.
//   3. Broadcasts `canonical_changed` to subscribed clients.
//
// SQLite hydration is a one-shot derived view: `hydrate_sqlite_view`
// refreshes the `canonical` table to reflect the in-memory state. Triggered
// by `zcache hydrate-view` (or implicitly on every push if cheap enough).
//
// Reads from `op_pull_canonical` / `op_view` / `op_export` go straight
// against the in-memory map — no SQLite round trip on the hot path.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use super::paths::CachePaths;
use super::shard::{
    read_canonical_shard, write_canonical_shard, CanonicalShard, ShardHeader, SHARD_FORMAT_VERSION,
    SHARD_MAGIC,
};
use super::Result;

/// Canonical slug used by the user-overlay promotion shard
/// (target of `zsync up`). Per docs/DAEMON.md cache layout.
const PROMOTIONS_SLUG: &str = "promotions";
const PROMOTIONS_SOURCE_ROOT: &str = "user-overlay";

/// One row in the canonical table. Stores the JSON-encoded value (so list /
/// scalar / map values all round-trip cleanly through `unjson` in export.rs).
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CanonicalRow {
    pub key: String,
    pub value: String,      // already JSON-encoded
    pub set_at_ns: i64,
    pub set_by_shell: Option<i64>,
}

/// In-memory canonical state, keyed first by subsystem, then by key. Values
/// are pre-encoded JSON strings to match the existing wire format (the JSON
/// quoting is stripped on emit by `unjson` in export.rs).
#[derive(Default)]
struct InMemory {
    /// subsystem → key → row
    rows: BTreeMap<String, BTreeMap<String, CanonicalRow>>,
    /// Most-recent rkyv-shard mtime per file (for staleness checks; daemon-
    /// only — clients never read this).
    last_persist_at_ns: i64,
}

/// Canonical-state engine. One instance lives in DaemonState; clients address
/// it through Arc.
pub struct CanonicalEngine {
    inner: RwLock<InMemory>,
    paths: CachePaths,
}

impl CanonicalEngine {
    pub fn new(paths: CachePaths) -> Arc<Self> {
        Arc::new(Self {
            inner: RwLock::new(InMemory::default()),
            paths,
        })
    }

    /// Reload the in-memory state from the on-disk rkyv shard if present.
    /// Idempotent; safe to call at daemon start. Missing shard = empty state
    /// (cold cache).
    pub fn load_from_disk(&self) -> Result<()> {
        let path = self.shard_path();
        if !path.exists() {
            return Ok(());
        }
        let shard = read_canonical_shard(&path)?;
        let mut g = self.inner.write();
        g.rows.clear();
        Self::ingest_shard_into(&mut g.rows, &shard);
        g.last_persist_at_ns = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        tracing::info!(
            path = %path.display(),
            generation = shard.header.generation,
            entries = subsystem_total(&g.rows),
            "canonical engine loaded from disk"
        );
        Ok(())
    }

    fn ingest_shard_into(
        rows: &mut BTreeMap<String, BTreeMap<String, CanonicalRow>>,
        shard: &CanonicalShard,
    ) {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let push = |rows: &mut BTreeMap<String, BTreeMap<String, CanonicalRow>>,
                    sub: &str,
                    k: &str,
                    v: String| {
            rows.entry(sub.to_string()).or_default().insert(
                k.to_string(),
                CanonicalRow {
                    key: k.to_string(),
                    value: v,
                    set_at_ns: now,
                    set_by_shell: None,
                },
            );
        };
        for (k, v) in &shard.aliases {
            push(rows, "alias", k, json_string(v));
        }
        for (k, v) in &shard.global_aliases {
            push(rows, "galias", k, json_string(v));
        }
        for (k, v) in &shard.suffix_aliases {
            push(rows, "salias", k, json_string(v));
        }
        for (k, v) in &shard.functions {
            push(rows, "function", k, json_string(v));
        }
        for (k, v) in &shard.bindkeys {
            push(rows, "bindkey", k, json_string(v));
        }
        for (k, v) in &shard.named_dirs {
            push(rows, "named_dir", k, json_string(v));
        }
        for (k, v) in &shard.compdef {
            push(rows, "compdef", k, json_string(v));
        }
        for (k, v) in &shard.env_exports {
            push(rows, "env", k, json_string(v));
        }
        for (k, v) in &shard.params {
            push(rows, "params", k, json_string(v));
        }
        for opt in &shard.setopts {
            push(rows, "setopt", opt, "\"on\"".to_string());
        }
        for opt in &shard.unsetopts {
            push(rows, "setopt", opt, "\"off\"".to_string());
        }
        for module in &shard.zmodload {
            push(rows, "zmodload", module, "\"loaded\"".to_string());
        }
        for (i, dir) in shard.path.iter().enumerate() {
            push(rows, "path", &i.to_string(), json_string(dir));
        }
        for (i, dir) in shard.fpath.iter().enumerate() {
            push(rows, "fpath", &i.to_string(), json_string(dir));
        }
        for (i, dir) in shard.manpath.iter().enumerate() {
            push(rows, "manpath", &i.to_string(), json_string(dir));
        }
        for (pat, rest) in &shard.zstyle {
            push(rows, "zstyle", pat, json_string(rest));
        }
    }

    /// Upsert one row. JSON-encode the value before storing if it isn't
    /// already (callers can pass a pre-encoded JSON value to insert object /
    /// array shapes directly).
    pub fn upsert(
        &self,
        subsystem: &str,
        key: &str,
        json_value: &str,
        set_by_shell: Option<u64>,
    ) -> usize {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let mut g = self.inner.write();
        g.rows
            .entry(subsystem.to_string())
            .or_default()
            .insert(
                key.to_string(),
                CanonicalRow {
                    key: key.to_string(),
                    value: json_value.to_string(),
                    set_at_ns: now,
                    set_by_shell: set_by_shell.map(|n| n as i64),
                },
            );
        1
    }

    /// Bulk-replace one subsystem's rows from a (key, json_value) iterator.
    /// Returns the number of rows after the replace.
    pub fn replace_subsystem<I: IntoIterator<Item = (String, String)>>(
        &self,
        subsystem: &str,
        rows: I,
        set_by_shell: Option<u64>,
    ) -> usize {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let mut g = self.inner.write();
        let map = g.rows.entry(subsystem.to_string()).or_default();
        map.clear();
        for (k, v) in rows {
            let r = CanonicalRow {
                key: k.clone(),
                value: v,
                set_at_ns: now,
                set_by_shell: set_by_shell.map(|n| n as i64),
            };
            map.insert(k, r);
        }
        map.len()
    }

    /// Read every row in a subsystem, ordered by key.
    pub fn rows_for(&self, subsystem: &str) -> Vec<CanonicalRow> {
        let g = self.inner.read();
        g.rows
            .get(subsystem)
            .map(|m| m.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Read one row by (subsystem, key).
    pub fn row(&self, subsystem: &str, key: &str) -> Option<CanonicalRow> {
        let g = self.inner.read();
        g.rows.get(subsystem).and_then(|m| m.get(key)).cloned()
    }

    /// Total row count across all subsystems (for `info` / stats).
    pub fn total_rows(&self) -> usize {
        let g = self.inner.read();
        subsystem_total(&g.rows)
    }

    /// Snapshot the entire in-memory state as a CanonicalShard, ready to
    /// rkyv-serialize. Generation is set by the caller so the same value can
    /// flow through the canonical_changed event.
    pub fn snapshot_shard(&self, generation: u64) -> CanonicalShard {
        let g = self.inner.read();
        let mut shard = CanonicalShard {
            header: ShardHeader {
                magic: SHARD_MAGIC,
                format_version: SHARD_FORMAT_VERSION,
                generation,
                built_at_ns: chrono::Utc::now()
                    .timestamp_nanos_opt()
                    .unwrap_or(0) as u64,
                slug: PROMOTIONS_SLUG.to_string(),
                source_root: PROMOTIONS_SOURCE_ROOT.to_string(),
                entry_count: 0,
            },
            ..Default::default()
        };

        for (sub, map) in g.rows.iter() {
            for (k, row) in map.iter() {
                let plain = unjson(&row.value);
                match sub.as_str() {
                    "alias" => {
                        shard.aliases.insert(k.clone(), plain);
                    }
                    "galias" => {
                        shard.global_aliases.insert(k.clone(), plain);
                    }
                    "salias" => {
                        shard.suffix_aliases.insert(k.clone(), plain);
                    }
                    "function" => {
                        shard.functions.insert(k.clone(), plain);
                    }
                    "bindkey" => {
                        shard.bindkeys.insert(k.clone(), plain);
                    }
                    "named_dir" => {
                        shard.named_dirs.insert(k.clone(), plain);
                    }
                    "compdef" => {
                        shard.compdef.insert(k.clone(), plain);
                    }
                    "env" => {
                        shard.env_exports.insert(k.clone(), plain);
                    }
                    "params" => {
                        shard.params.insert(k.clone(), plain);
                    }
                    "setopt" => {
                        if plain == "on" || plain == "true" || plain == "1" {
                            shard.setopts.push(k.clone());
                        } else {
                            shard.unsetopts.push(k.clone());
                        }
                    }
                    "zmodload" => {
                        shard.zmodload.push(k.clone());
                    }
                    "path" => {
                        push_indexed(&mut shard.path, k, plain);
                    }
                    "fpath" => {
                        push_indexed(&mut shard.fpath, k, plain);
                    }
                    "manpath" => {
                        push_indexed(&mut shard.manpath, k, plain);
                    }
                    "zstyle" => {
                        shard.zstyle.push((k.clone(), plain));
                    }
                    _ => {} // unknown subsystem — silently skipped
                }
            }
        }
        let total = subsystem_total(&g.rows);
        shard.header.entry_count = total as u32;
        shard
    }

    /// Persist the current state to disk (atomic-rename rkyv shard).
    pub fn persist(&self, generation: u64) -> Result<PathBuf> {
        let shard = self.snapshot_shard(generation);
        let path = write_canonical_shard(&self.paths, &shard)?;
        let mut g = self.inner.write();
        g.last_persist_at_ns = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        Ok(path)
    }

    /// Replace SQLite's `canonical` table with the current in-memory state.
    /// SQLite is the inspection mirror only — never the source of truth.
    /// Returns the number of rows written.
    pub fn hydrate_sqlite_view(
        &self,
        state: &super::state::DaemonState,
    ) -> Result<usize> {
        super::zsync::ensure_schema(state)?;
        let g = self.inner.read();
        let mut count = 0usize;
        state.with_catalog(|conn| -> std::result::Result<(), super::DaemonError> {
            let tx = conn.unchecked_transaction()?;
            tx.execute("DELETE FROM canonical", [])?;
            for (sub, map) in g.rows.iter() {
                for (k, row) in map.iter() {
                    tx.execute(
                        "INSERT INTO canonical (subsystem, key, value, set_at_ns, set_by_shell) \
                         VALUES (?, ?, ?, ?, ?)",
                        rusqlite::params![sub, k, row.value, row.set_at_ns, row.set_by_shell],
                    )?;
                    count += 1;
                }
            }
            tx.commit()?;
            Ok(())
        })?;
        Ok(count)
    }

    fn shard_path(&self) -> PathBuf {
        super::shard::shard_path(&self.paths, PROMOTIONS_SOURCE_ROOT, PROMOTIONS_SLUG)
    }

    pub fn last_persist_at_ns(&self) -> i64 {
        self.inner.read().last_persist_at_ns
    }
}

fn subsystem_total(rows: &BTreeMap<String, BTreeMap<String, CanonicalRow>>) -> usize {
    rows.values().map(|m| m.len()).sum()
}

fn push_indexed(target: &mut Vec<String>, key: &str, value: String) {
    if let Ok(idx) = key.parse::<usize>() {
        while target.len() <= idx {
            target.push(String::new());
        }
        target[idx] = value;
    } else {
        target.push(value);
    }
}

fn json_string(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}

fn unjson(s: &str) -> String {
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(s) {
            if let Some(s2) = v.as_str() {
                return s2.to_string();
            }
        }
    }
    s.to_string()
}
