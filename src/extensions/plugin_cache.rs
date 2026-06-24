//! Plugin source cache — stores side effects of `source`/`.` in
//! SQLite.
//!
//! **zshrs-original infrastructure with strong C-zsh ancestry.** C
//! zsh has `bin_zcompile()` (Src/parse.c) which writes a parsed
//! AST to a `.zwc` file alongside the source so subsequent reads
//! skip parsing. zshrs takes the idea further: rather than caching
//! the AST and still re-running it, we capture the *side effects*
//! (params/aliases/options/funcs set) and replay those directly —
//! microseconds instead of milliseconds for plugin startup. The
//! key/invalidation model (canonical-path + mtime) matches the
//! `.zwc` invalidation scheme C zsh uses in `try_source_file()`
//! (Src/init.c:1551).
//!
//! First source: execute normally, capture state delta, write
//! cache on worker thread.
//! Subsequent sources: check mtime, replay cached side effects in
//! microseconds.
//!
//! Cache key: `(canonical_path, mtime_secs, mtime_nsecs)`.
//! Cache invalidation: mtime mismatch → re-source, update cache.

use crate::ported::utils::{errflag, ERRFLAG_ERROR};
#[allow(unused_imports)]
use crate::ported::vm_helper::ShellExecutor;
use crate::ported::zsh_h::PM_UNDEFINED;
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::env;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::OnceLock;

/// State snapshot for plugin delta computation.
pub(crate) struct PluginSnapshot {
    pub(crate) functions: std::collections::HashSet<String>,
    pub(crate) aliases: std::collections::HashSet<String>,
    pub(crate) global_aliases: std::collections::HashSet<String>,
    pub(crate) suffix_aliases: std::collections::HashSet<String>,
    pub(crate) variables: HashMap<String, String>,
    pub(crate) arrays: std::collections::HashSet<String>,
    pub(crate) assoc_arrays: std::collections::HashSet<String>,
    pub(crate) fpath: Vec<PathBuf>,
    pub(crate) options: HashMap<String, bool>,
    pub(crate) hooks: HashMap<String, Vec<String>>,
    pub(crate) autoloads: std::collections::HashSet<String>,
}

/// Mtime (seconds since epoch) of the running zshrs binary. Same
/// helper as `script_cache::current_binary_mtime_secs` — we duplicate
/// it here so plugin_cache doesn't need to take a script_cache dep
/// and so the OnceLock is per-cache (the value is identical anyway
/// since it's process-global). Returns None if the executable's
/// metadata can't be read (extremely rare — usually only if the
/// binary was deleted out from under us mid-run).
fn current_binary_mtime() -> Option<i64> {
    static BIN_MTIME: OnceLock<Option<i64>> = OnceLock::new();
    *BIN_MTIME.get_or_init(|| {
        let exe = std::env::current_exe().ok()?;
        let meta = std::fs::metadata(&exe).ok()?;
        Some(meta.mtime())
    })
}

// Script bytecode caching used to live here behind the BYTECODE_VERSION
// prefix + script_bytecode SQLite table. It now lives in the rkyv shard at
// ~/.zshrs/scripts.rkyv (see `crate::script_cache`). The header in
// that shard carries its own version pin (`zshrs_version`) so this prefix
// byte is no longer needed — a zshrs rebuild silently invalidates all
// cached entries via `binary_mtime_at_cache`.

/// Side effects captured from sourcing a plugin file.
#[derive(Debug, Clone, Default)]
pub struct PluginDelta {
    pub functions: Vec<(String, Vec<u8>)>, // name → bincode-serialized bytecode
    pub aliases: Vec<(String, String, AliasKind)>, // name → value, kind
    /// `global_aliases` field.
    pub global_aliases: Vec<(String, String)>,
    /// `suffix_aliases` field.
    pub suffix_aliases: Vec<(String, String)>,
    /// `variables` field.
    pub variables: Vec<(String, String)>,
    pub exports: Vec<(String, String)>, // also set in env
    /// `arrays` field.
    pub arrays: Vec<(String, Vec<String>)>,
    /// `assoc_arrays` field.
    pub assoc_arrays: Vec<(String, HashMap<String, String>)>,
    pub completions: Vec<(String, String)>, // command → function
    /// `fpath_additions` field.
    pub fpath_additions: Vec<String>,
    pub hooks: Vec<(String, String)>, // hook_name → function
    pub bindkeys: Vec<(String, String, String)>, // keyseq, widget, keymap
    pub zstyles: Vec<(String, String, String)>, // pattern, style, value
    pub options_changed: Vec<(String, bool)>, // option → on/off
    pub autoloads: Vec<(String, String)>, // function → flags
}
/// `AliasKind` — see variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasKind {
    /// `Regular` variant.
    Regular,
    /// `Global` variant.
    Global,
    /// `Suffix` variant.
    Suffix,
}

impl AliasKind {
    fn as_i32(self) -> i32 {
        match self {
            AliasKind::Regular => 0,
            AliasKind::Global => 1,
            AliasKind::Suffix => 2,
        }
    }
    fn from_i32(v: i32) -> Self {
        match v {
            1 => AliasKind::Global,
            2 => AliasKind::Suffix,
            _ => AliasKind::Regular,
        }
    }
}

/// SQLite-backed plugin cache.
pub struct PluginCache {
    /// `conn` field.
    conn: Connection,
}

impl PluginCache {
    /// `open` — see implementation.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        let cache = Self { conn };
        cache.init_schema()?;
        Ok(cache)
    }

    fn init_schema(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS plugins (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                mtime_secs INTEGER NOT NULL,
                mtime_nsecs INTEGER NOT NULL,
                source_time_ms INTEGER NOT NULL,
                cached_at INTEGER NOT NULL,
                binary_mtime INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS plugin_functions (
                plugin_id INTEGER NOT NULL REFERENCES plugins(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                body BLOB NOT NULL
            );

            CREATE TABLE IF NOT EXISTS plugin_aliases (
                plugin_id INTEGER NOT NULL REFERENCES plugins(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                value TEXT NOT NULL,
                kind INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS plugin_variables (
                plugin_id INTEGER NOT NULL REFERENCES plugins(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                value TEXT NOT NULL,
                is_export INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS plugin_arrays (
                plugin_id INTEGER NOT NULL REFERENCES plugins(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                value_json TEXT NOT NULL
            );

            -- Associative-array deltas (e.g. ZINIT[BIN_DIR]=...). Stored
            -- as JSON {key: value} so insertion order isn't load-bearing
            -- (matches HashMap semantics on the Rust side). Direct
            -- analogue of plugin_arrays for assoc shape.
            CREATE TABLE IF NOT EXISTS plugin_assoc_arrays (
                plugin_id INTEGER NOT NULL REFERENCES plugins(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                value_json TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS plugin_completions (
                plugin_id INTEGER NOT NULL REFERENCES plugins(id) ON DELETE CASCADE,
                command TEXT NOT NULL,
                function TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS plugin_fpath (
                plugin_id INTEGER NOT NULL REFERENCES plugins(id) ON DELETE CASCADE,
                path TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS plugin_hooks (
                plugin_id INTEGER NOT NULL REFERENCES plugins(id) ON DELETE CASCADE,
                hook TEXT NOT NULL,
                function TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS plugin_bindkeys (
                plugin_id INTEGER NOT NULL REFERENCES plugins(id) ON DELETE CASCADE,
                keyseq TEXT NOT NULL,
                widget TEXT NOT NULL,
                keymap TEXT NOT NULL DEFAULT 'main'
            );

            CREATE TABLE IF NOT EXISTS plugin_zstyles (
                plugin_id INTEGER NOT NULL REFERENCES plugins(id) ON DELETE CASCADE,
                pattern TEXT NOT NULL,
                style TEXT NOT NULL,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS plugin_options (
                plugin_id INTEGER NOT NULL REFERENCES plugins(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                enabled INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS plugin_autoloads (
                plugin_id INTEGER NOT NULL REFERENCES plugins(id) ON DELETE CASCADE,
                function TEXT NOT NULL,
                flags TEXT NOT NULL DEFAULT ''
            );

            -- compaudit cache: security audit results per fpath directory
            CREATE TABLE IF NOT EXISTS compaudit_cache (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                mtime_secs INTEGER NOT NULL,
                mtime_nsecs INTEGER NOT NULL,
                uid INTEGER NOT NULL,
                mode INTEGER NOT NULL,
                is_secure INTEGER NOT NULL,
                checked_at INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_plugins_path ON plugins(path);
            CREATE INDEX IF NOT EXISTS idx_compaudit_path ON compaudit_cache(path);

            -- Migration: legacy script_bytecode table (bytecode now lives in
            -- the rkyv shard at ~/.zshrs/scripts.rkyv). Drop on open so
            -- existing DBs reclaim the space and don't carry stale bytecode.
            DROP INDEX IF EXISTS idx_script_bytecode_path;
            DROP TABLE IF EXISTS script_bytecode;
        "#,
        )?;
        // Migrate pre-binary_mtime DBs (column added 2026-05): the
        // CREATE-IF-NOT-EXISTS above only adds the column for fresh
        // dbs. ALTER TABLE on an existing db is a one-time no-op
        // wrapped in an ignored-if-already-applied check. Mirrors the
        // C analogue of zsh's $ZSH_VERSION-keyed compdump rebuild —
        // any binary change invalidates the plugin replay shard so
        // we don't replay deltas captured under the old runtime
        // semantics. Without this, fixes to paramsubst / option
        // handling don't take effect until the user manually
        // `rm ~/.zshrs/plugins.db`.
        let _ = self.conn.execute(
            "ALTER TABLE plugins ADD COLUMN binary_mtime INTEGER NOT NULL DEFAULT 0",
            [],
        );
        Ok(())
    }

    /// Check if a cached entry exists with matching mtime AND the
    /// running zshrs binary's mtime is no newer than when the entry
    /// was cached. Direct port of script_cache.rs's invalidation
    /// logic (lines 188-194): any zshrs rebuild silently invalidates
    /// plugin-cached deltas because runtime semantics may have
    /// shifted (paramsubst flags, option aliases, builtin
    /// resolution, …). Without this guard, a new build reads stale
    /// deltas and replays them with the new engine — visible
    /// regression where `zinit.zsh`'s `${ZINIT[BIN_DIR]}` returned
    /// empty after re-source until the cache was manually cleared.
    pub fn check(&self, path: &str, mtime_secs: i64, mtime_nsecs: i64) -> Option<i64> {
        let row: Option<(i64, i64)> = self
            .conn
            .query_row(
                "SELECT id, binary_mtime FROM plugins WHERE path = ?1 AND mtime_secs = ?2 AND mtime_nsecs = ?3",
                params![path, mtime_secs, mtime_nsecs],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();
        let (id, cached_bin_mtime) = row?;
        if let Some(bin_mtime) = current_binary_mtime() {
            if cached_bin_mtime < bin_mtime {
                return None;
            }
        }
        Some(id)
    }

    /// Load cached delta for a plugin by id.
    pub fn load(&self, plugin_id: i64) -> rusqlite::Result<PluginDelta> {
        let mut delta = PluginDelta::default();

        // Functions (bincode-serialized AST blobs)
        let mut stmt = self
            .conn
            .prepare("SELECT name, body FROM plugin_functions WHERE plugin_id = ?1")?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        for r in rows {
            delta.functions.push(r?);
        }

        // Aliases
        let mut stmt = self
            .conn
            .prepare("SELECT name, value, kind FROM plugin_aliases WHERE plugin_id = ?1")?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                AliasKind::from_i32(row.get::<_, i32>(2)?),
            ))
        })?;
        for r in rows {
            delta.aliases.push(r?);
        }

        // Variables
        let mut stmt = self
            .conn
            .prepare("SELECT name, value, is_export FROM plugin_variables WHERE plugin_id = ?1")?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })?;
        for r in rows {
            let (name, value, is_export) = r?;
            if is_export {
                delta.exports.push((name, value));
            } else {
                delta.variables.push((name, value));
            }
        }

        // Arrays
        let mut stmt = self
            .conn
            .prepare("SELECT name, value_json FROM plugin_arrays WHERE plugin_id = ?1")?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in rows {
            let (name, json) = r?;
            // Simple JSON array: ["a","b","c"]
            let vals: Vec<String> = json
                .trim_matches(|c| c == '[' || c == ']')
                .split(',')
                .map(|s| s.trim().trim_matches('"').to_string())
                .filter(|s| !s.is_empty())
                .collect();
            delta.arrays.push((name, vals));
        }

        // Associative arrays (key→value JSON object). Falls back to
        // an empty map on parse failure rather than a load error so
        // a malformed row doesn't break the whole replay path.
        let mut stmt = self
            .conn
            .prepare("SELECT name, value_json FROM plugin_assoc_arrays WHERE plugin_id = ?1")?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in rows {
            let (name, json) = r?;
            let map: HashMap<String, String> = serde_json::from_str(&json).unwrap_or_default();
            delta.assoc_arrays.push((name, map));
        }

        // Completions
        let mut stmt = self
            .conn
            .prepare("SELECT command, function FROM plugin_completions WHERE plugin_id = ?1")?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in rows {
            delta.completions.push(r?);
        }

        // Fpath
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM plugin_fpath WHERE plugin_id = ?1")?;
        let rows = stmt.query_map(params![plugin_id], |row| row.get::<_, String>(0))?;
        for r in rows {
            delta.fpath_additions.push(r?);
        }

        // Hooks
        let mut stmt = self
            .conn
            .prepare("SELECT hook, function FROM plugin_hooks WHERE plugin_id = ?1")?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in rows {
            delta.hooks.push(r?);
        }

        // Bindkeys
        let mut stmt = self
            .conn
            .prepare("SELECT keyseq, widget, keymap FROM plugin_bindkeys WHERE plugin_id = ?1")?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for r in rows {
            delta.bindkeys.push(r?);
        }

        // Zstyles
        let mut stmt = self
            .conn
            .prepare("SELECT pattern, style, value FROM plugin_zstyles WHERE plugin_id = ?1")?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        for r in rows {
            delta.zstyles.push(r?);
        }

        // Options
        let mut stmt = self
            .conn
            .prepare("SELECT name, enabled FROM plugin_options WHERE plugin_id = ?1")?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
        })?;
        for r in rows {
            delta.options_changed.push(r?);
        }

        // Autoloads
        let mut stmt = self
            .conn
            .prepare("SELECT function, flags FROM plugin_autoloads WHERE plugin_id = ?1")?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in rows {
            delta.autoloads.push(r?);
        }

        Ok(delta)
    }

    /// Store a plugin delta. Replaces any existing entry for this path.
    pub fn store(
        &self,
        path: &str,
        mtime_secs: i64,
        mtime_nsecs: i64,
        source_time_ms: u64,
        delta: &PluginDelta,
    ) -> rusqlite::Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        // Delete old entry if exists
        self.conn
            .execute("DELETE FROM plugins WHERE path = ?1", params![path])?;

        let bin_mtime = current_binary_mtime().unwrap_or(0);
        self.conn.execute(
            "INSERT INTO plugins (path, mtime_secs, mtime_nsecs, source_time_ms, cached_at, binary_mtime) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![path, mtime_secs, mtime_nsecs, source_time_ms as i64, now, bin_mtime],
        )?;
        let plugin_id = self.conn.last_insert_rowid();

        // Functions
        for (name, body) in &delta.functions {
            self.conn.execute(
                "INSERT INTO plugin_functions (plugin_id, name, body) VALUES (?1, ?2, ?3)",
                params![plugin_id, name, body],
            )?;
        }

        // Aliases
        for (name, value, kind) in &delta.aliases {
            self.conn.execute(
                "INSERT INTO plugin_aliases (plugin_id, name, value, kind) VALUES (?1, ?2, ?3, ?4)",
                params![plugin_id, name, value, kind.as_i32()],
            )?;
        }

        // Variables + exports
        for (name, value) in &delta.variables {
            self.conn.execute(
                "INSERT INTO plugin_variables (plugin_id, name, value, is_export) VALUES (?1, ?2, ?3, 0)",
                params![plugin_id, name, value],
            )?;
        }
        for (name, value) in &delta.exports {
            self.conn.execute(
                "INSERT INTO plugin_variables (plugin_id, name, value, is_export) VALUES (?1, ?2, ?3, 1)",
                params![plugin_id, name, value],
            )?;
        }

        // Arrays
        for (name, vals) in &delta.arrays {
            let json = format!(
                "[{}]",
                vals.iter()
                    .map(|v| format!("\"{}\"", v.replace('"', "\\\"")))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            self.conn.execute(
                "INSERT INTO plugin_arrays (plugin_id, name, value_json) VALUES (?1, ?2, ?3)",
                params![plugin_id, name, json],
            )?;
        }

        // Associative arrays — JSON-encode the key/value map. Use
        // serde_json so quotes / backslashes / unicode round-trip
        // correctly through the cache (the simple `["a","b"]`
        // hand-format used for indexed arrays above doesn't escape
        // properly for arbitrary-content keys/values).
        for (name, map) in &delta.assoc_arrays {
            let json = serde_json::to_string(map).unwrap_or_else(|_| "{}".to_string());
            self.conn.execute(
                "INSERT INTO plugin_assoc_arrays (plugin_id, name, value_json) VALUES (?1, ?2, ?3)",
                params![plugin_id, name, json],
            )?;
        }

        // Completions
        for (cmd, func) in &delta.completions {
            self.conn.execute(
                "INSERT INTO plugin_completions (plugin_id, command, function) VALUES (?1, ?2, ?3)",
                params![plugin_id, cmd, func],
            )?;
        }

        // Fpath
        for p in &delta.fpath_additions {
            self.conn.execute(
                "INSERT INTO plugin_fpath (plugin_id, path) VALUES (?1, ?2)",
                params![plugin_id, p],
            )?;
        }

        // Hooks
        for (hook, func) in &delta.hooks {
            self.conn.execute(
                "INSERT INTO plugin_hooks (plugin_id, hook, function) VALUES (?1, ?2, ?3)",
                params![plugin_id, hook, func],
            )?;
        }

        // Bindkeys
        for (keyseq, widget, keymap) in &delta.bindkeys {
            self.conn.execute(
                "INSERT INTO plugin_bindkeys (plugin_id, keyseq, widget, keymap) VALUES (?1, ?2, ?3, ?4)",
                params![plugin_id, keyseq, widget, keymap],
            )?;
        }

        // Zstyles
        for (pattern, style, value) in &delta.zstyles {
            self.conn.execute(
                "INSERT INTO plugin_zstyles (plugin_id, pattern, style, value) VALUES (?1, ?2, ?3, ?4)",
                params![plugin_id, pattern, style, value],
            )?;
        }

        // Options
        for (name, enabled) in &delta.options_changed {
            self.conn.execute(
                "INSERT INTO plugin_options (plugin_id, name, enabled) VALUES (?1, ?2, ?3)",
                params![plugin_id, name, *enabled],
            )?;
        }

        // Autoloads
        for (func, flags) in &delta.autoloads {
            self.conn.execute(
                "INSERT INTO plugin_autoloads (plugin_id, function, flags) VALUES (?1, ?2, ?3)",
                params![plugin_id, func, flags],
            )?;
        }

        Ok(())
    }

    /// Stats for logging.
    pub fn stats(&self) -> (i64, i64) {
        let plugins: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM plugins", [], |r| r.get(0))
            .unwrap_or(0);
        let functions: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM plugin_functions", [], |r| r.get(0))
            .unwrap_or(0);
        (plugins, functions)
    }

    /// Count plugins whose file mtime no longer matches the cache.
    pub fn count_stale(&self) -> usize {
        let mut stmt = match self
            .conn
            .prepare("SELECT path, mtime_secs, mtime_nsecs FROM plugins")
        {
            Ok(s) => s,
            Err(_) => return 0,
        };
        let rows = match stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        }) {
            Ok(r) => r,
            Err(_) => return 0,
        };
        let mut count = 0;
        for (path, cached_s, cached_ns) in rows.flatten() {
            match file_mtime(std::path::Path::new(&path)) {
                Some((s, ns)) if s != cached_s || ns != cached_ns => count += 1,
                None => count += 1, // file deleted
                _ => {}
            }
        }
        count
    }

    // -----------------------------------------------------------------
    // compaudit cache — security audit results per fpath directory
    // -----------------------------------------------------------------

    /// Check if a directory's security audit result is cached and still valid.
    /// Returns Some(is_secure) if cache hit, None if miss or stale.
    pub fn check_compaudit(&self, dir: &str, mtime_secs: i64, mtime_nsecs: i64) -> Option<bool> {
        self.conn.query_row(
            "SELECT is_secure FROM compaudit_cache WHERE path = ?1 AND mtime_secs = ?2 AND mtime_nsecs = ?3",
            params![dir, mtime_secs, mtime_nsecs],
            |row| row.get::<_, bool>(0),
        ).ok()
    }

    /// Store a compaudit result for a directory.
    pub fn store_compaudit(
        &self,
        dir: &str,
        mtime_secs: i64,
        mtime_nsecs: i64,
        uid: u32,
        mode: u32,
        is_secure: bool,
    ) -> rusqlite::Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        self.conn.execute(
            "INSERT OR REPLACE INTO compaudit_cache (path, mtime_secs, mtime_nsecs, uid, mode, is_secure, checked_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![dir, mtime_secs, mtime_nsecs, uid as i64, mode as i64, is_secure, now],
        )?;
        Ok(())
    }

    /// Run a full compaudit against fpath directories, using cache where valid.
    /// Returns list of insecure directories (empty = all secure).
    pub fn compaudit_cached(&self, fpath: &[std::path::PathBuf]) -> Vec<String> {
        let euid = unsafe { libc::geteuid() };
        let mut insecure = Vec::new();

        for dir in fpath {
            let dir_str = dir.to_string_lossy().to_string();
            let meta = match std::fs::metadata(dir) {
                Ok(m) => m,
                Err(_) => continue, // dir doesn't exist, skip
            };
            let mt_s = meta.mtime();
            let mt_ns = meta.mtime_nsec();

            // Check cache first
            if let Some(is_secure) = self.check_compaudit(&dir_str, mt_s, mt_ns) {
                if !is_secure {
                    insecure.push(dir_str);
                }
                continue;
            }

            // Cache miss — do the actual security check
            let mode = meta.mode();
            let uid = meta.uid();
            let is_secure = Self::check_dir_security(&meta, euid);

            // Also check parent directory
            let parent_secure = dir
                .parent()
                .and_then(|p| std::fs::metadata(p).ok())
                .map(|pm| Self::check_dir_security(&pm, euid))
                .unwrap_or(true);

            let secure = is_secure && parent_secure;

            // Cache the result
            let _ = self.store_compaudit(&dir_str, mt_s, mt_ns, uid, mode, secure);

            if !secure {
                insecure.push(dir_str);
            }
        }

        if insecure.is_empty() {
            tracing::debug!(
                dirs = fpath.len(),
                "compaudit: all directories secure (cached)"
            );
        } else {
            tracing::warn!(
                insecure_count = insecure.len(),
                dirs = fpath.len(),
                "compaudit: insecure directories found"
            );
        }

        insecure
    }

    /// Enumerate every plugin currently in the `plugins` table.
    /// Returns `(path, mtime_secs)` tuples in insertion order.
    /// Used by `zshrs --dump-plugins` to feed the IntelliJ
    /// External Libraries view.
    pub fn list_plugin_paths(&self) -> Vec<(String, i64)> {
        let mut stmt = match self
            .conn
            .prepare("SELECT path, mtime_secs FROM plugins ORDER BY id")
        {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = match stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        }) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        rows.flatten().collect()
    }

    /// Check if a directory's permissions are secure.
    /// Insecure = world-writable or group-writable AND not owned by root or EUID.
    fn check_dir_security(meta: &std::fs::Metadata, euid: u32) -> bool {
        let mode = meta.mode();
        let uid = meta.uid();

        // Owned by root or the current user — always OK
        if uid == 0 || uid == euid {
            return true;
        }

        // Not owned by us — check if world/group writable
        let group_writable = mode & 0o020 != 0;
        let world_writable = mode & 0o002 != 0;

        !group_writable && !world_writable
    }
}

/// Get mtime from file metadata as (secs, nsecs).
pub fn file_mtime(path: &Path) -> Option<(i64, i64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.mtime(), meta.mtime_nsec()))
}

/// One plugin entry as exposed to the IntelliJ External Libraries view.
/// `manager` is the inferred plugin manager (zinit / oh-my-zsh / prezto /
/// antidote / antigen / zplug / zsh-more-completions / zpwr / loose).
/// `name` is the human-readable plugin identifier (`zsh-users/zsh-autosuggestions`,
/// `git`, etc.). `root` is the absolute directory that holds the plugin's files.
#[derive(Debug, Clone)]
pub struct PluginEntry {
    pub manager: String,
    pub name: String,
    pub root: PathBuf,
}

/// Classify a sourced plugin file path into `(manager, name, root_dir)`.
/// The match order matters — first hit wins so `~/.oh-my-zsh/custom/plugins/foo`
/// is "oh-my-zsh" not "loose".
fn classify_plugin_path(path: &Path) -> PluginEntry {
    let s = path.to_string_lossy();

    // zinit: `<root>/plugins/<user>---<repo>/<file>` where root is either
    // `~/.zinit` (legacy) or `$XDG_DATA_HOME/zinit` / `~/.local/share/zinit`.
    for marker in ["/.zinit/plugins/", "/zinit/plugins/"] {
        if let Some(start) = s.find(marker) {
            let after = &s[start + marker.len()..];
            if let Some(end) = after.find('/') {
                let dir = &after[..end];
                let name = dir.replacen("---", "/", 1);
                let root: PathBuf = s[..start + marker.len() + end].into();
                return PluginEntry {
                    manager: "zinit".into(),
                    name,
                    root,
                };
            }
        }
    }

    // oh-my-zsh: `~/.oh-my-zsh/{plugins,custom/plugins,themes,custom/themes}/<name>/<file>`
    for (marker, kind) in [
        ("/.oh-my-zsh/custom/plugins/", "plugin"),
        ("/.oh-my-zsh/plugins/", "plugin"),
        ("/.oh-my-zsh/custom/themes/", "theme"),
        ("/.oh-my-zsh/themes/", "theme"),
    ] {
        if let Some(start) = s.find(marker) {
            let after = &s[start + marker.len()..];
            let end = after.find('/').unwrap_or(after.len());
            let leaf = &after[..end];
            let name = if kind == "theme" {
                format!("{}.theme", leaf)
            } else {
                leaf.to_string()
            };
            let root: PathBuf = s[..start + marker.len() + end].into();
            return PluginEntry {
                manager: "oh-my-zsh".into(),
                name,
                root,
            };
        }
    }

    // prezto: `~/.zprezto/modules/<name>/init.zsh`.
    if let Some(start) = s.find("/.zprezto/modules/") {
        let after = &s[start + "/.zprezto/modules/".len()..];
        let end = after.find('/').unwrap_or(after.len());
        let name = after[..end].to_string();
        let root: PathBuf = s[..start + "/.zprezto/modules/".len() + end].into();
        return PluginEntry {
            manager: "prezto".into(),
            name,
            root,
        };
    }

    // antidote: `~/.cache/antidote/<user>/<repo>/<file>` or
    // `~/.local/share/antidote/repos/<user>/<repo>/<file>`.
    for marker in ["/antidote/repos/", "/.cache/antidote/"] {
        if let Some(start) = s.find(marker) {
            let after = &s[start + marker.len()..];
            // user/repo — two path components.
            let mut split = after.splitn(3, '/');
            if let (Some(user), Some(repo), _) = (split.next(), split.next(), split.next()) {
                let name = format!("{}/{}", user, repo);
                let root: PathBuf =
                    format!("{}{}/{}", &s[..start + marker.len()], user, repo).into();
                return PluginEntry {
                    manager: "antidote".into(),
                    name,
                    root,
                };
            }
        }
    }

    // antigen: `~/.antigen/bundles/<user>/<repo>/<file>`.
    if let Some(start) = s.find("/.antigen/bundles/") {
        let after = &s[start + "/.antigen/bundles/".len()..];
        let mut split = after.splitn(3, '/');
        if let (Some(user), Some(repo), _) = (split.next(), split.next(), split.next()) {
            let name = format!("{}/{}", user, repo);
            let root: PathBuf = format!(
                "{}/{}/{}",
                &s[..start + "/.antigen/bundles".len()],
                user,
                repo
            )
            .into();
            return PluginEntry {
                manager: "antigen".into(),
                name,
                root,
            };
        }
    }

    // zplug: `~/.zplug/repos/<user>/<repo>/<file>`.
    if let Some(start) = s.find("/.zplug/repos/") {
        let after = &s[start + "/.zplug/repos/".len()..];
        let mut split = after.splitn(3, '/');
        if let (Some(user), Some(repo), _) = (split.next(), split.next(), split.next()) {
            let name = format!("{}/{}", user, repo);
            let root: PathBuf =
                format!("{}/{}/{}", &s[..start + "/.zplug/repos".len()], user, repo).into();
            return PluginEntry {
                manager: "zplug".into(),
                name,
                root,
            };
        }
    }

    // zsh-more-completions: the user's own 16k-file corpus. Group every
    // file under one logical library so the IDE doesn't render 16k leaves.
    if let Some(start) = s.find("/zsh-more-completions/") {
        let root: PathBuf = s[..start + "/zsh-more-completions".len()].into();
        return PluginEntry {
            manager: "zsh-more-completions".into(),
            name: "zsh-more-completions".into(),
            root,
        };
    }

    // zpwr: the user's CLI suite. One library, root = `$ZPWR` or `~/.zpwr`.
    for marker in ["/.zpwr/", "/zpwr/"] {
        if let Some(start) = s.find(marker) {
            let root: PathBuf = s[..start + marker.len() - 1].into();
            return PluginEntry {
                manager: "zpwr".into(),
                name: "zpwr".into(),
                root,
            };
        }
    }

    // Loose: the file's parent directory is the root, basename is the name.
    let root = path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| path.into());
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "(loose)".into());
    PluginEntry {
        manager: "loose".into(),
        name,
        root,
    }
}

/// Read every entry in `plugins` and group by `(manager, name, root)`.
/// Returns one `PluginEntry` per unique plugin (de-duplicated across the
/// many files a single plugin typically sources).
pub fn list_plugins(cache_path: &Path) -> Vec<PluginEntry> {
    let cache = match PluginCache::open(cache_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut seen: std::collections::BTreeMap<(String, String, PathBuf), PluginEntry> =
        std::collections::BTreeMap::new();
    for (path, _mtime) in cache.list_plugin_paths() {
        let entry = classify_plugin_path(Path::new(&path));
        seen.entry((
            entry.manager.clone(),
            entry.name.clone(),
            entry.root.clone(),
        ))
        .or_insert(entry);
    }
    seen.into_values().collect()
}

/// JSON consumed by the IntelliJ `AdditionalLibraryRootsProvider`.
/// Schema:
/// ```json
/// {
///   "schema": 1,
///   "plugins": [
///     {"manager": "zinit", "name": "zsh-users/zsh-autosuggestions",
///      "root": "/Users/wizard/.zinit/plugins/zsh-users---zsh-autosuggestions"}
///   ]
/// }
/// ```
/// Manager+name uniquely identify a plugin; root is the directory to
/// expose as a synthetic library root.
pub fn dump_plugins_json() -> String {
    let entries = list_plugins(&default_cache_path());
    let mut s = String::from("{\"schema\":1,\"plugins\":[");
    for (i, e) in entries.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!(
            "{{\"manager\":{},\"name\":{},\"root\":{}}}",
            json_str(&e.manager),
            json_str(&e.name),
            json_str(&e.root.to_string_lossy())
        ));
    }
    s.push_str("]}");
    s
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Default path for the plugin cache db. Honors $ZSHRS_HOME so the
/// shell agrees with the daemon on where state lives.
pub fn default_cache_path() -> PathBuf {
    if let Some(custom) = std::env::var_os("ZSHRS_HOME") {
        return PathBuf::from(custom).join("plugins.db");
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".zshrs/plugins.db")
}

#[cfg(test)]
mod migration_tests {
    use super::*;

    #[test]
    fn opening_an_existing_db_drops_legacy_script_bytecode_table() {
        let _g = crate::test_util::global_state_lock();
        // Simulate a pre-migration DB: open with an old schema that still
        // had script_bytecode, insert a row, close, then re-open via the
        // current `PluginCache::open` path. The migration in `init_schema`
        // must leave the table gone so SQLite holds zero bytecode bytes.
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("legacy.db");

        // Hand-build the legacy table.
        let pre = Connection::open(&db_path).unwrap();
        pre.execute_batch(
            r#"
            CREATE TABLE script_bytecode (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                mtime_secs INTEGER NOT NULL,
                mtime_nsecs INTEGER NOT NULL,
                bytecode BLOB NOT NULL,
                cached_at INTEGER NOT NULL
            );
            CREATE INDEX idx_script_bytecode_path ON script_bytecode(path);
            INSERT INTO script_bytecode (id, path, mtime_secs, mtime_nsecs, bytecode, cached_at)
                VALUES (1, '/fake/legacy.zsh', 0, 0, x'00deadbeef', 0);
            "#,
        )
        .unwrap();
        drop(pre);

        // Re-open via the production path — migration runs.
        let _cache = PluginCache::open(&db_path).expect("open after migration");

        // Confirm script_bytecode is gone.
        let post = Connection::open(&db_path).unwrap();
        let exists: i64 = post
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='script_bytecode'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 0, "legacy script_bytecode must be dropped");
    }

    // ========================================================
    // default_cache_path — ZSHRS_HOME precedence
    // ========================================================

    fn with_zshrs_home<F: FnOnce()>(value: Option<&str>, f: F) {
        let prev = std::env::var_os("ZSHRS_HOME");
        match value {
            Some(v) => std::env::set_var("ZSHRS_HOME", v),
            None => std::env::remove_var("ZSHRS_HOME"),
        }
        f();
        match prev {
            Some(v) => std::env::set_var("ZSHRS_HOME", v),
            None => std::env::remove_var("ZSHRS_HOME"),
        }
    }

    #[test]
    fn default_cache_path_honors_zshrs_home() {
        let _g = crate::test_util::global_state_lock();
        with_zshrs_home(Some("/tmp/zshrs-plugin-cache-home"), || {
            assert_eq!(
                default_cache_path(),
                PathBuf::from("/tmp/zshrs-plugin-cache-home/plugins.db")
            );
        });
    }

    #[test]
    fn default_cache_path_filename_is_plugins_db() {
        let _g = crate::test_util::global_state_lock();
        with_zshrs_home(Some("/tmp/zshrs-plugin-fname"), || {
            assert_eq!(
                default_cache_path().file_name().and_then(|s| s.to_str()),
                Some("plugins.db")
            );
        });
    }

    #[test]
    fn default_cache_path_falls_back_to_home_dot_zshrs() {
        let _g = crate::test_util::global_state_lock();
        with_zshrs_home(None, || {
            let p = default_cache_path();
            // Path must end in `.zshrs/plugins.db` regardless of HOME.
            let s = p.to_string_lossy();
            assert!(
                s.ends_with(".zshrs/plugins.db"),
                "expected .zshrs/plugins.db tail, got: {}",
                s
            );
        });
    }

    #[test]
    fn default_cache_path_uses_distinct_dir_per_zshrs_home_change() {
        let _g = crate::test_util::global_state_lock();
        with_zshrs_home(Some("/tmp/zshrs-plugin-a"), || {
            let a = default_cache_path();
            with_zshrs_home(Some("/tmp/zshrs-plugin-b"), || {
                let b = default_cache_path();
                assert_ne!(a, b, "different ZSHRS_HOME must yield different paths");
            });
        });
    }

    // ========================================================
    // file_mtime — metadata sniff
    // ========================================================

    #[test]
    fn file_mtime_returns_some_for_existing_file() {
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir().join("zshrs_plugin_cache_mtime.txt");
        std::fs::write(&tmp, b"x").unwrap();
        let mt = file_mtime(&tmp);
        assert!(mt.is_some(), "existing file should produce mtime");
        // Seconds since epoch is positive on any sane system clock.
        let (secs, _ns) = mt.unwrap();
        assert!(secs > 0, "mtime secs must be positive: {}", secs);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn file_mtime_returns_none_for_missing_path() {
        let _g = crate::test_util::global_state_lock();
        assert!(file_mtime(Path::new("/nonexistent/zshrs/missing.bin")).is_none());
    }

    #[test]
    fn file_mtime_secs_monotonic_after_rewrite() {
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir().join("zshrs_plugin_cache_mtime_two.txt");
        std::fs::write(&tmp, b"a").unwrap();
        let first = file_mtime(&tmp).unwrap();
        // Sleep slightly to ensure mtime resolution boundary.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&tmp, b"b").unwrap();
        let second = file_mtime(&tmp).unwrap();
        // Second mtime >= first mtime (clocks don't go backwards
        // on any unit-test host we care about).
        assert!(
            second >= first,
            "mtime regressed: first={:?} second={:?}",
            first,
            second
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn file_mtime_path_with_special_chars_resolves() {
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir().join("zshrs plugin cache (space).bin");
        std::fs::write(&tmp, b"x").unwrap();
        let mt = file_mtime(&tmp);
        assert!(mt.is_some(), "spaces in filename must not block resolution");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn default_cache_path_relative_zshrs_home_taken_verbatim() {
        let _g = crate::test_util::global_state_lock();
        with_zshrs_home(Some("relative-dir"), || {
            assert_eq!(
                default_cache_path(),
                PathBuf::from("relative-dir/plugins.db")
            );
        });
    }

    #[test]
    fn default_cache_path_empty_zshrs_home_is_empty_dir_plus_db() {
        let _g = crate::test_util::global_state_lock();
        with_zshrs_home(Some(""), || {
            // env::var_os("") returns Some("") — code takes the
            // override branch and joins "" + "plugins.db" = "plugins.db".
            assert_eq!(default_cache_path(), PathBuf::from("plugins.db"));
        });
    }
}

#[cfg(test)]
mod classify_tests {
    use super::*;
    use std::path::Path;

    fn classify(p: &str) -> (String, String, String) {
        let e = classify_plugin_path(Path::new(p));
        (e.manager, e.name, e.root.to_string_lossy().into_owned())
    }

    #[test]
    fn zinit_legacy_dir_user_repo() {
        let (m, n, r) = classify(
            "/Users/wizard/.zinit/plugins/zsh-users---zsh-autosuggestions/zsh-autosuggestions.plugin.zsh",
        );
        assert_eq!(m, "zinit");
        assert_eq!(n, "zsh-users/zsh-autosuggestions");
        assert_eq!(
            r,
            "/Users/wizard/.zinit/plugins/zsh-users---zsh-autosuggestions"
        );
    }

    #[test]
    fn zinit_xdg_dir_user_repo() {
        let (m, n, _) =
            classify("/home/u/.local/share/zinit/plugins/romkatv---powerlevel10k/p10k.zsh");
        assert_eq!(m, "zinit");
        assert_eq!(n, "romkatv/powerlevel10k");
    }

    #[test]
    fn oh_my_zsh_core_plugin() {
        let (m, n, r) = classify("/Users/wizard/.oh-my-zsh/plugins/git/git.plugin.zsh");
        assert_eq!(m, "oh-my-zsh");
        assert_eq!(n, "git");
        assert_eq!(r, "/Users/wizard/.oh-my-zsh/plugins/git");
    }

    #[test]
    fn oh_my_zsh_custom_plugin() {
        let (m, n, _) = classify(
            "/Users/wizard/.oh-my-zsh/custom/plugins/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh",
        );
        assert_eq!(m, "oh-my-zsh");
        assert_eq!(n, "zsh-syntax-highlighting");
    }

    #[test]
    fn oh_my_zsh_theme_tagged_with_theme_suffix() {
        let (m, n, _) = classify("/Users/wizard/.oh-my-zsh/themes/agnoster.zsh-theme");
        assert_eq!(m, "oh-my-zsh");
        assert_eq!(n, "agnoster.zsh-theme.theme");
    }

    #[test]
    fn prezto_module() {
        let (m, n, _) = classify("/Users/wizard/.zprezto/modules/git/init.zsh");
        assert_eq!(m, "prezto");
        assert_eq!(n, "git");
    }

    #[test]
    fn antidote_repo() {
        let (m, n, _) = classify(
            "/Users/wizard/.cache/antidote/zsh-users/zsh-autosuggestions/zsh-autosuggestions.zsh",
        );
        assert_eq!(m, "antidote");
        assert_eq!(n, "zsh-users/zsh-autosuggestions");
    }

    #[test]
    fn antigen_bundle() {
        let (m, n, _) = classify(
            "/Users/wizard/.antigen/bundles/zsh-users/zsh-completions/zsh-completions.plugin.zsh",
        );
        assert_eq!(m, "antigen");
        assert_eq!(n, "zsh-users/zsh-completions");
    }

    #[test]
    fn zplug_repo() {
        let (m, n, _) = classify(
            "/Users/wizard/.zplug/repos/zsh-users/zsh-history-substring-search/zsh-history-substring-search.zsh",
        );
        assert_eq!(m, "zplug");
        assert_eq!(n, "zsh-users/zsh-history-substring-search");
    }

    #[test]
    fn zsh_more_completions_groups_into_one() {
        let (m, n, _) =
            classify("/Users/wizard/forkedRepos/zsh-more-completions/src/_some_long_completion");
        assert_eq!(m, "zsh-more-completions");
        assert_eq!(n, "zsh-more-completions");
    }

    #[test]
    fn zpwr_root_recognized() {
        let (m, n, _) = classify("/Users/wizard/.zpwr/local/.aliases.sh");
        assert_eq!(m, "zpwr");
        assert_eq!(n, "zpwr");
    }

    #[test]
    fn loose_plugin_uses_parent_dir_as_name() {
        let (m, n, r) = classify("/opt/local/share/zsh/something/init.zsh");
        assert_eq!(m, "loose");
        assert_eq!(n, "something");
        assert_eq!(r, "/opt/local/share/zsh/something");
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::vm_helper::ShellExecutor {
    /// Snapshot executor state before sourcing a plugin (for delta computation).
    pub(crate) fn snapshot_state(&self) -> PluginSnapshot {
        PluginSnapshot {
            functions: self.function_names().into_iter().collect(),
            aliases: self.alias_entries().into_iter().map(|(k, _)| k).collect(),
            global_aliases: self
                .global_alias_entries()
                .into_iter()
                .map(|(k, _)| k)
                .collect(),
            suffix_aliases: self
                .suffix_alias_entries()
                .into_iter()
                .map(|(k, _)| k)
                .collect(),
            variables: if let Ok(tab) = crate::ported::params::paramtab().read() {
                tab.iter()
                    .filter(|(_, pm)| pm.u_arr.is_none())
                    .map(|(k, pm)| (k.clone(), pm.u_str.clone().unwrap_or_default()))
                    .collect()
            } else {
                std::collections::HashMap::new()
            },
            arrays: if let Ok(tab) = crate::ported::params::paramtab().read() {
                tab.iter()
                    .filter(|(_, pm)| pm.u_arr.is_some())
                    .map(|(k, _)| k.clone())
                    .collect()
            } else {
                std::collections::HashSet::new()
            },
            assoc_arrays: if let Ok(m) = crate::ported::params::paramtab_hashed_storage().lock() {
                m.keys().cloned().collect()
            } else {
                std::collections::HashSet::new()
            },
            fpath: self.fpath.clone(),
            options: crate::ported::options::opt_state_snapshot(),
            hooks: {
                // Snapshot `<hook>_functions` arrays from canonical paramtab.
                let names = [
                    "chpwd",
                    "precmd",
                    "preexec",
                    "periodic",
                    "zshexit",
                    "zshaddhistory",
                ];
                let mut m = std::collections::HashMap::new();
                for h in &names {
                    let arr_name = format!("{}_functions", h);
                    if let Some(arr) = self.array(&arr_name) {
                        if !arr.is_empty() {
                            m.insert(h.to_string(), arr);
                        }
                    }
                }
                m
            },
            autoloads: {
                // Walk canonical shfunctab for autoload-pending entries
                // (PM_UNDEFINED set). Snapshot is keyed by function name.
                crate::ported::hashtable::shfunctab_lock()
                    .read()
                    .ok()
                    .map(|t| {
                        t.iter()
                            .filter(|(_, shf)| (shf.node.flags as u32 & PM_UNDEFINED) != 0)
                            .map(|(name, _)| name.clone())
                            .collect()
                    })
                    .unwrap_or_default()
            },
        }
    }
    /// Compute the delta between current state and a previous snapshot.
    pub(crate) fn diff_state(&self, snap: &PluginSnapshot) -> crate::plugin_cache::PluginDelta {
        let mut delta = PluginDelta::default();

        // Walk every HashMap in sorted-key order so the resulting
        // PluginDelta serializes byte-identically across runs of an
        // identical state. Without sorting, rkyv-encoded delta blobs
        // differ run-to-run, defeating cache reuse and tripping
        // diff-based snapshot tests.

        // New functions — serialize canonical source text (UTF-8 bytes)
        // for instant replay. Replay parses + compiles via the new pipeline.
        let mut fn_keys: Vec<&String> = self.function_source.keys().collect();
        fn_keys.sort();
        for name in fn_keys {
            if !snap.functions.contains(name) {
                let source = self.function_source.get(name).unwrap();
                delta
                    .functions
                    .push((name.clone(), source.as_bytes().to_vec()));
            }
        }

        let push_alias = |delta: &mut PluginDelta,
                          entries: Vec<(String, String)>,
                          snap_set: &std::collections::HashSet<String>,
                          kind: AliasKind| {
            let mut entries = entries;
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (name, value) in entries {
                if !snap_set.contains(&name) {
                    delta.aliases.push((name, value, kind));
                }
            }
        };
        push_alias(
            &mut delta,
            self.alias_entries(),
            &snap.aliases,
            AliasKind::Regular,
        );
        push_alias(
            &mut delta,
            self.global_alias_entries(),
            &snap.global_aliases,
            AliasKind::Global,
        );
        push_alias(
            &mut delta,
            self.suffix_alias_entries(),
            &snap.suffix_aliases,
            AliasKind::Suffix,
        );

        // New/changed variables. Skip shell-special parameters whose
        // values are runtime-state, not script-state — replaying them
        // poisons subsequent shells with values frozen from the
        // capture run. C zsh maintains these per-process and never
        // serializes them: `_` (last argv of last command, Src/init.c
        // special_params; gets `/tmp/foo` from a prior bash test then
        // gets fed into `(( $_ ))` math in a user's .zshrc), `?`
        // (last exit), `$`/`!`/`PPID` (process IDs), `RANDOM`,
        // `SECONDS`, `EPOCHSECONDS`, `LINENO`, `OLDPWD`, `PWD`
        // (volatile; cwd is re-read on replay anyway), `STATUS`,
        // `OPTIND`, `IFS` (must default to whitespace at shell
        // startup unless user explicitly sets it). Direct port of
        // the C analogue's PM_SPECIAL flag — those params don't
        // round-trip through the parameter-table dump path.
        const NON_REPLAYABLE_VARS: &[&str] = &[
            "0",
            "_",
            "?",
            "$",
            "!",
            "PPID",
            "RANDOM",
            "SECONDS",
            "EPOCHSECONDS",
            "EPOCHREALTIME",
            "LINENO",
            "OLDPWD",
            "PWD",
            "STATUS",
            "OPTIND",
            "OPTARG",
            "IFS",
            "FUNCNAME",
            "BASHPID",
            "BASH_LINENO",
            "BASH_SOURCE",
            "ZSH_ARGZERO",
            "ZSH_EVAL_CONTEXT",
            "ZSH_SUBSHELL",
            "HISTCMD",
            "MATCH",
            "MBEGIN",
            "MEND",
        ];
        let mut var_keys: Vec<String> = if let Ok(tab) = crate::ported::params::paramtab().read() {
            tab.iter()
                .filter(|(_, pm)| pm.u_arr.is_none())
                .map(|(k, _)| k.clone())
                .collect()
        } else {
            Vec::new()
        };
        var_keys.sort();
        for name in &var_keys {
            if NON_REPLAYABLE_VARS.contains(&name.as_str()) {
                continue;
            }
            let value = crate::ported::params::getsparam(name).unwrap_or_default();
            match snap.variables.get(name) {
                Some(old) if old == &value => {} // unchanged
                _ => {
                    // Check if it's also exported
                    if env::var(name).ok().as_ref() == Some(&value) {
                        delta.exports.push((name.clone(), value.clone()));
                    } else {
                        delta.variables.push((name.clone(), value.clone()));
                    }
                }
            }
        }

        // New arrays — iterate paramtab for PM_ARRAY entries.
        let arr_entries: Vec<(String, Vec<String>)> =
            if let Ok(tab) = crate::ported::params::paramtab().read() {
                let mut v: Vec<(String, Vec<String>)> = tab
                    .iter()
                    .filter_map(|(k, pm)| pm.u_arr.clone().map(|a| (k.clone(), a)))
                    .collect();
                v.sort_by(|a, b| a.0.cmp(&b.0));
                v
            } else {
                Vec::new()
            };
        for (name, values) in arr_entries {
            if !snap.arrays.contains(&name) {
                delta.arrays.push((name, values));
            }
        }

        // New / changed associative arrays. zinit creates `ZINIT[…]`
        // entries during sourcing; without this capture, the cache
        // replay path saw an empty ZINIT and `${ZINIT[BIN_DIR]}`
        // returned "" on every subsequent shell start. Direct port of
        // zsh's plugin-replay model — assoc deltas are first-class
        // captures alongside scalars and arrays.
        let assoc_entries: Vec<(String, indexmap::IndexMap<String, String>)> =
            if let Ok(m) = crate::ported::params::paramtab_hashed_storage().lock() {
                let mut v: Vec<(String, indexmap::IndexMap<String, String>)> =
                    m.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                v.sort_by(|a, b| a.0.cmp(&b.0));
                v
            } else {
                Vec::new()
            };
        for (name, map) in assoc_entries {
            if !snap.assoc_arrays.contains(&name) {
                // Executor's assoc storage is IndexMap (insertion-
                // ordered, required by `(kv)` etc.). The plugin_cache
                // delta uses a plain HashMap since the cache replay
                // reseeds the assoc and order is reconstructed by
                // the script's own typeset ordering. Convert here.
                let plain: HashMap<String, String> =
                    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                delta.assoc_arrays.push((name, plain));
            }
        }

        // New fpath entries
        for p in &self.fpath {
            if !snap.fpath.contains(p) {
                delta.fpath_additions.push(p.to_string_lossy().to_string());
            }
        }

        // Changed options — diff against canonical OPTS_LIVE snapshot.
        let current = crate::ported::options::opt_state_snapshot();
        let mut opt_keys: Vec<&String> = current.keys().collect();
        opt_keys.sort();
        for name in opt_keys {
            let value = current.get(name).unwrap();
            match snap.options.get(name) {
                Some(old) if old == value => {}
                _ => delta.options_changed.push((name.clone(), *value)),
            }
        }

        // New hooks — read from canonical `<hook>_functions` arrays.
        let names = [
            "chpwd",
            "precmd",
            "preexec",
            "periodic",
            "zshexit",
            "zshaddhistory",
        ];
        let mut hook_names: Vec<&&str> = names.iter().collect();
        hook_names.sort();
        for &h in hook_names {
            let arr_name = format!("{}_functions", h);
            let funcs = self.array(&arr_name).unwrap_or_default();
            let old_funcs = snap.hooks.get(h);
            for f in &funcs {
                let is_new = old_funcs.is_none_or(|old| !old.contains(f));
                if is_new {
                    delta.hooks.push((h.to_string(), f.clone()));
                }
            }
        }

        // New autoloads — read PM_UNDEFINED entries from canonical shfunctab.
        let current_autoloads: Vec<String> = crate::ported::hashtable::shfunctab_lock()
            .read()
            .ok()
            .map(|t| {
                t.iter()
                    .filter(|(_, shf)| (shf.node.flags as u32 & PM_UNDEFINED) != 0)
                    .map(|(name, _)| name.clone())
                    .collect()
            })
            .unwrap_or_default();
        let mut autoload_keys: Vec<&String> = current_autoloads.iter().collect();
        autoload_keys.sort();
        for name in autoload_keys {
            if !snap.autoloads.contains(name) {
                // Flags stub-string: canonical autoload sets only PM_UNDEFINED
                // (the -U/-z/-k/-t/-d details were never consumed by replay).
                delta.autoloads.push((name.clone(), String::new()));
            }
        }

        delta
    }
    /// Replay a cached plugin delta into the executor state.
    pub(crate) fn replay_plugin_delta(&mut self, delta: &crate::plugin_cache::PluginDelta) {
        // Aliases
        for (name, value, kind) in &delta.aliases {
            match kind {
                AliasKind::Regular => {
                    self.set_alias(name.clone(), value.clone());
                }
                AliasKind::Global => {
                    self.set_global_alias(name.clone(), value.clone());
                }
                AliasKind::Suffix => {
                    self.set_suffix_alias(name.clone(), value.clone());
                }
            }
        }

        // Variables. Drop shell-special parameters even on the
        // replay side — pre-existing caches from before the
        // diff_state filter was added still contain entries for
        // `_`, `PPID`, etc.; replaying them poisons the new shell.
        // Keeping the same exclusion list as `diff_state` so old
        // caches self-heal on next read.
        const NON_REPLAYABLE_VARS: &[&str] = &[
            "0",
            "_",
            "?",
            "$",
            "!",
            "PPID",
            "RANDOM",
            "SECONDS",
            "EPOCHSECONDS",
            "EPOCHREALTIME",
            "LINENO",
            "OLDPWD",
            "PWD",
            "STATUS",
            "OPTIND",
            "OPTARG",
            "IFS",
            "FUNCNAME",
            "BASHPID",
            "BASH_LINENO",
            "BASH_SOURCE",
            "ZSH_ARGZERO",
            "ZSH_EVAL_CONTEXT",
            "ZSH_SUBSHELL",
            "HISTCMD",
            "MATCH",
            "MBEGIN",
            "MEND",
        ];
        for (name, value) in &delta.variables {
            if NON_REPLAYABLE_VARS.contains(&name.as_str()) {
                continue;
            }
            self.set_scalar(name.clone(), value.clone());
        }

        // Exports (set in both variables and process env)
        for (name, value) in &delta.exports {
            if NON_REPLAYABLE_VARS.contains(&name.as_str()) {
                continue;
            }
            self.set_scalar(name.clone(), value.clone());
            env::set_var(name, value);
        }

        // Arrays
        for (name, values) in &delta.arrays {
            self.set_array(name.clone(), values.clone());
        }

        // Associative arrays — restore plugin-defined assocs (e.g.
        // ZINIT, ZINIT_SNIPPETS, ZINIT_REPORTS) so subsequent shells
        // see the same `${ZINIT[BIN_DIR]}` etc. that the original
        // sourcing established. Mirrors the diff_state capture above.
        for (name, map) in &delta.assoc_arrays {
            // Plugin cache uses HashMap; executor uses IndexMap.
            // Reseed by inserting key-by-key so the IndexMap variant
            // is constructed without needing a HashMap→IndexMap
            // From impl that may not be available.
            let mut idx_map: indexmap::IndexMap<String, String> =
                indexmap::IndexMap::with_capacity(map.len());
            // Sort for deterministic order (the diff_state stored
            // a HashMap which has no defined order; the original
            // insertion order was lost). Sort is the simplest
            // reproducible choice — matches `(o)`-flag default.
            let mut entries: Vec<(&String, &String)> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            for (k, v) in entries {
                idx_map.insert(k.clone(), v.clone());
            }
            self.set_assoc(name.clone(), idx_map);
        }

        // Fpath additions
        for p in &delta.fpath_additions {
            let pb = PathBuf::from(p);
            if !self.fpath.contains(&pb) {
                self.fpath.push(pb);
            }
        }

        // Completions
        if !delta.completions.is_empty() {
            let mut comps = self.assoc("_comps").unwrap_or_default();
            for (cmd, func) in &delta.completions {
                comps.insert(cmd.clone(), func.clone());
            }
            self.set_assoc("_comps".to_string(), comps);
        }

        // Options — write into canonical OPTS_LIVE.
        for (name, enabled) in &delta.options_changed {
            crate::ported::options::opt_state_set(name, *enabled);
        }

        // Hooks — append into the canonical `<hook>_functions`
        // paramtab array (port of `Src/Functions/Misc/add-zsh-hook`
        // shell-function idiom). NOT the C-module HOOKTAB
        // (src/ported/module.rs `addhookfunc`), which stores
        // Hookfn fn pointers for C-internal hookdefs.
        for (hook, func) in &delta.hooks {
            let array_name = format!("{}_functions", hook);
            let mut arr = self.array(&array_name).unwrap_or_default();
            if !arr.iter().any(|f| f == func) {
                arr.push(func.clone());
                crate::ported::params::setaparam(&array_name, arr);
            }
        }

        // Plugin cache replay: each bincode blob is a ShellCommand AST.
        // Replay each function's source text through parse_init + parse + ZshCompiler.
        // Delta format: name → UTF-8 source bytes (no AST round-trip needed).
        for (name, bytes) in &delta.functions {
            let Ok(source) = std::str::from_utf8(bytes) else {
                continue;
            };
            // Mirror Src/init.c errflag save/clear/check around parse.
            let saved_errflag = errflag.load(Ordering::Relaxed);
            errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
            crate::ported::parse::parse_init(source);
            let program = crate::ported::parse::parse();
            let parse_failed = (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0;
            errflag.store(saved_errflag, Ordering::Relaxed);
            if parse_failed || program.lists.is_empty() {
                continue;
            }
            let chunk = crate::compile_zsh::ZshCompiler::new().compile(&program);
            self.functions_compiled.insert(name.clone(), chunk);
            self.function_source
                .insert(name.clone(), source.to_string());
        }
    }
}
// END moved-from-exec-rs
