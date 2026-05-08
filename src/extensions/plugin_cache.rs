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

use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
#[allow(unused_imports)]
use crate::ported::exec::ShellExecutor;

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
    use std::os::unix::fs::MetadataExt;
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
    pub global_aliases: Vec<(String, String)>,
    pub suffix_aliases: Vec<(String, String)>,
    pub variables: Vec<(String, String)>,
    pub exports: Vec<(String, String)>, // also set in env
    pub arrays: Vec<(String, Vec<String>)>,
    pub assoc_arrays: Vec<(String, HashMap<String, String>)>,
    pub completions: Vec<(String, String)>, // command → function
    pub fpath_additions: Vec<String>,
    pub hooks: Vec<(String, String)>, // hook_name → function
    pub bindkeys: Vec<(String, String, String)>, // keyseq, widget, keymap
    pub zstyles: Vec<(String, String, String)>, // pattern, style, value
    pub options_changed: Vec<(String, bool)>, // option → on/off
    pub autoloads: Vec<(String, String)>, // function → flags
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasKind {
    Regular,
    Global,
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
    conn: Connection,
}

impl PluginCache {
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
        let _ = self
            .conn
            .execute("ALTER TABLE plugins ADD COLUMN binary_mtime INTEGER NOT NULL DEFAULT 0", []);
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
        use std::os::unix::fs::MetadataExt;

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

    /// Check if a directory's permissions are secure.
    /// Insecure = world-writable or group-writable AND not owned by root or EUID.
    fn check_dir_security(meta: &std::fs::Metadata, euid: u32) -> bool {
        use std::os::unix::fs::MetadataExt;
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
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.mtime(), meta.mtime_nsec()))
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
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// Snapshot executor state before sourcing a plugin (for delta computation).
    pub(crate) fn snapshot_state(&self) -> PluginSnapshot {
        PluginSnapshot {
            functions: self.function_names().into_iter().collect(),
            aliases: self.aliases.keys().cloned().collect(),
            global_aliases: self.global_aliases.keys().cloned().collect(),
            suffix_aliases: self.suffix_aliases.keys().cloned().collect(),
            variables: self.variables.clone(),
            arrays: self.arrays.keys().cloned().collect(),
            assoc_arrays: self.assoc_arrays.keys().cloned().collect(),
            fpath: self.fpath.clone(),
            options: self.options.clone(),
            hooks: self.hook_functions.clone(),
            autoloads: self.autoload_pending.keys().cloned().collect(),
        }
    }
    /// Compute the delta between current state and a previous snapshot.
    pub(crate) fn diff_state(&self, snap: &PluginSnapshot) -> crate::plugin_cache::PluginDelta {
        use crate::plugin_cache::{AliasKind, PluginDelta};
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
                          map: &indexmap::IndexMap<String, String>,
                          snap_set: &std::collections::HashSet<String>,
                          kind: AliasKind| {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for name in keys {
                if !snap_set.contains(name) {
                    let value = map.get(name).unwrap();
                    delta.aliases.push((name.clone(), value.clone(), kind));
                }
            }
        };
        push_alias(&mut delta, &self.aliases, &snap.aliases, AliasKind::Regular);
        push_alias(
            &mut delta,
            &self.global_aliases,
            &snap.global_aliases,
            AliasKind::Global,
        );
        push_alias(
            &mut delta,
            &self.suffix_aliases,
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
            "0", "_", "?", "$", "!", "PPID", "RANDOM", "SECONDS",
            "EPOCHSECONDS", "EPOCHREALTIME", "LINENO", "OLDPWD", "PWD",
            "STATUS", "OPTIND", "OPTARG", "IFS", "FUNCNAME",
            "BASHPID", "BASH_LINENO", "BASH_SOURCE",
            "ZSH_ARGZERO", "ZSH_EVAL_CONTEXT", "ZSH_SUBSHELL",
            "HISTCMD", "MATCH", "MBEGIN", "MEND",
        ];
        let mut var_keys: Vec<&String> = self.variables.keys().collect();
        var_keys.sort();
        for name in var_keys {
            if NON_REPLAYABLE_VARS.contains(&name.as_str()) {
                continue;
            }
            let value = self.variables.get(name).unwrap();
            match snap.variables.get(name) {
                Some(old) if old == value => {} // unchanged
                _ => {
                    // Check if it's also exported
                    if env::var(name).ok().as_ref() == Some(value) {
                        delta.exports.push((name.clone(), value.clone()));
                    } else {
                        delta.variables.push((name.clone(), value.clone()));
                    }
                }
            }
        }

        // New arrays
        let mut arr_keys: Vec<&String> = self.arrays.keys().collect();
        arr_keys.sort();
        for name in arr_keys {
            if !snap.arrays.contains(name) {
                let values = self.arrays.get(name).unwrap();
                delta.arrays.push((name.clone(), values.clone()));
            }
        }

        // New / changed associative arrays. zinit creates `ZINIT[…]`
        // entries during sourcing; without this capture, the cache
        // replay path saw an empty ZINIT and `${ZINIT[BIN_DIR]}`
        // returned "" on every subsequent shell start. Direct port of
        // zsh's plugin-replay model — assoc deltas are first-class
        // captures alongside scalars and arrays.
        let mut assoc_keys: Vec<&String> = self.assoc_arrays.keys().collect();
        assoc_keys.sort();
        for name in assoc_keys {
            if !snap.assoc_arrays.contains(name) {
                let map = self.assoc_arrays.get(name).unwrap();
                // Executor's assoc storage is IndexMap (insertion-
                // ordered, required by `(kv)` etc.). The plugin_cache
                // delta uses a plain HashMap since the cache replay
                // reseeds the assoc and order is reconstructed by
                // the script's own typeset ordering. Convert here.
                let plain: HashMap<String, String> =
                    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                delta.assoc_arrays.push((name.clone(), plain));
            }
        }

        // New fpath entries
        for p in &self.fpath {
            if !snap.fpath.contains(p) {
                delta.fpath_additions.push(p.to_string_lossy().to_string());
            }
        }

        // Changed options
        let mut opt_keys: Vec<&String> = self.options.keys().collect();
        opt_keys.sort();
        for name in opt_keys {
            let value = self.options.get(name).unwrap();
            match snap.options.get(name) {
                Some(old) if old == value => {}
                _ => delta.options_changed.push((name.clone(), *value)),
            }
        }

        // New hooks
        let mut hook_keys: Vec<&String> = self.hook_functions.keys().collect();
        hook_keys.sort();
        for hook in hook_keys {
            let funcs = self.hook_functions.get(hook).unwrap();
            let old_funcs = snap.hooks.get(hook);
            for f in funcs {
                let is_new = old_funcs.is_none_or(|old| !old.contains(f));
                if is_new {
                    delta.hooks.push((hook.clone(), f.clone()));
                }
            }
        }

        // New autoloads
        let mut autoload_keys: Vec<&String> = self.autoload_pending.keys().collect();
        autoload_keys.sort();
        for name in autoload_keys {
            if !snap.autoloads.contains(name) {
                let flags = self.autoload_pending.get(name).unwrap();
                delta.autoloads.push((name.clone(), format!("{:?}", flags)));
            }
        }

        delta
    }
    /// Replay a cached plugin delta into the executor state.
    pub(crate) fn replay_plugin_delta(&mut self, delta: &crate::plugin_cache::PluginDelta) {
        use crate::plugin_cache::AliasKind;

        // Aliases
        for (name, value, kind) in &delta.aliases {
            match kind {
                AliasKind::Regular => {
                    self.aliases.insert(name.clone(), value.clone());
                }
                AliasKind::Global => {
                    self.global_aliases.insert(name.clone(), value.clone());
                }
                AliasKind::Suffix => {
                    self.suffix_aliases.insert(name.clone(), value.clone());
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
            "0", "_", "?", "$", "!", "PPID", "RANDOM", "SECONDS",
            "EPOCHSECONDS", "EPOCHREALTIME", "LINENO", "OLDPWD", "PWD",
            "STATUS", "OPTIND", "OPTARG", "IFS", "FUNCNAME",
            "BASHPID", "BASH_LINENO", "BASH_SOURCE",
            "ZSH_ARGZERO", "ZSH_EVAL_CONTEXT", "ZSH_SUBSHELL",
            "HISTCMD", "MATCH", "MBEGIN", "MEND",
        ];
        for (name, value) in &delta.variables {
            if NON_REPLAYABLE_VARS.contains(&name.as_str()) {
                continue;
            }
            self.variables.insert(name.clone(), value.clone());
        }

        // Exports (set in both variables and process env)
        for (name, value) in &delta.exports {
            if NON_REPLAYABLE_VARS.contains(&name.as_str()) {
                continue;
            }
            self.variables.insert(name.clone(), value.clone());
            env::set_var(name, value);
        }

        // Arrays
        for (name, values) in &delta.arrays {
            self.arrays.insert(name.clone(), values.clone());
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
            self.assoc_arrays.insert(name.clone(), idx_map);
        }

        // Fpath additions
        for p in &delta.fpath_additions {
            let pb = PathBuf::from(p);
            if !self.fpath.contains(&pb) {
                self.fpath.push(pb);
            }
        }

        // Completions
        for (cmd, func) in &delta.completions {
            if let Some(ref mut comps) = self.assoc_arrays.get_mut("_comps") {
                comps.insert(cmd.clone(), func.clone());
            }
        }

        // Options
        for (name, enabled) in &delta.options_changed {
            self.options.insert(name.clone(), *enabled);
        }

        // Hooks
        for (hook, func) in &delta.hooks {
            self.hook_functions
                .entry(hook.clone())
                .or_default()
                .push(func.clone());
        }

        // Plugin cache replay: each bincode blob is a ShellCommand AST.
        // Replay each function's source text through ZshParser + ZshCompiler.
        // Delta format: name → UTF-8 source bytes (no AST round-trip needed).
        for (name, bytes) in &delta.functions {
            let Ok(source) = std::str::from_utf8(bytes) else {
                continue;
            };
            if let Some(program) = crate::parse::ZshParser::new(source)
                .parse()
                .ok()
                .filter(|p| !p.lists.is_empty())
            {
                let chunk = crate::compile_zsh::ZshCompiler::new().compile(&program);
                self.functions_compiled.insert(name.clone(), chunk);
                self.function_source
                    .insert(name.clone(), source.to_string());
            }
        }
    }
}
// END moved-from-exec-rs
