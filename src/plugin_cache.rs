//! Plugin source cache — stores side effects of `source`/`.` in SQLite.
//!
//! First source: execute normally, capture state delta, write cache on worker thread.
//! Subsequent sources: check mtime, replay cached side effects in microseconds.
//!
//! Cache key: (canonical_path, mtime_secs, mtime_nsecs)
//! Cache invalidation: mtime mismatch → re-source, update cache.

use crate::parser::ShellCommand;
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Side effects captured from sourcing a plugin file.
#[derive(Debug, Clone, Default)]
pub struct PluginDelta {
    pub functions: Vec<(String, Vec<u8>)>,         // name → bincode-serialized bytecode
    pub aliases: Vec<(String, String, AliasKind)>, // name → value, kind
    pub global_aliases: Vec<(String, String)>,
    pub suffix_aliases: Vec<(String, String)>,
    pub variables: Vec<(String, String)>,
    pub exports: Vec<(String, String)>,            // also set in env
    pub arrays: Vec<(String, Vec<String>)>,
    pub assoc_arrays: Vec<(String, HashMap<String, String>)>,
    pub completions: Vec<(String, String)>,         // command → function
    pub fpath_additions: Vec<String>,
    pub hooks: Vec<(String, String)>,               // hook_name → function
    pub bindkeys: Vec<(String, String, String)>,    // keyseq, widget, keymap
    pub zstyles: Vec<(String, String, String)>,     // pattern, style, value
    pub options_changed: Vec<(String, bool)>,        // option → on/off
    pub autoloads: Vec<(String, String)>,            // function → flags
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
        self.conn.execute_batch(r#"
            CREATE TABLE IF NOT EXISTS plugins (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                mtime_secs INTEGER NOT NULL,
                mtime_nsecs INTEGER NOT NULL,
                source_time_ms INTEGER NOT NULL,
                cached_at INTEGER NOT NULL
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

            -- Full parsed AST cache: skip lex+parse entirely on cache hit
            CREATE TABLE IF NOT EXISTS script_ast (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                mtime_secs INTEGER NOT NULL,
                mtime_nsecs INTEGER NOT NULL,
                ast BLOB NOT NULL,
                cached_at INTEGER NOT NULL
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
            CREATE INDEX IF NOT EXISTS idx_script_ast_path ON script_ast(path);
            CREATE INDEX IF NOT EXISTS idx_compaudit_path ON compaudit_cache(path);
        "#)?;
        Ok(())
    }

    /// Check if a cached entry exists with matching mtime.
    /// Returns the plugin id if cache is valid, None if miss.
    pub fn check(&self, path: &str, mtime_secs: i64, mtime_nsecs: i64) -> Option<i64> {
        self.conn.query_row(
            "SELECT id FROM plugins WHERE path = ?1 AND mtime_secs = ?2 AND mtime_nsecs = ?3",
            params![path, mtime_secs, mtime_nsecs],
            |row| row.get(0),
        ).ok()
    }

    /// Load cached delta for a plugin by id.
    pub fn load(&self, plugin_id: i64) -> rusqlite::Result<PluginDelta> {
        let mut delta = PluginDelta::default();

        // Functions (bincode-serialized AST blobs)
        let mut stmt = self.conn.prepare(
            "SELECT name, body FROM plugin_functions WHERE plugin_id = ?1"
        )?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        })?;
        for r in rows { delta.functions.push(r?); }

        // Aliases
        let mut stmt = self.conn.prepare(
            "SELECT name, value, kind FROM plugin_aliases WHERE plugin_id = ?1"
        )?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, AliasKind::from_i32(row.get::<_, i32>(2)?)))
        })?;
        for r in rows { delta.aliases.push(r?); }

        // Variables
        let mut stmt = self.conn.prepare(
            "SELECT name, value, is_export FROM plugin_variables WHERE plugin_id = ?1"
        )?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, bool>(2)?))
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
        let mut stmt = self.conn.prepare(
            "SELECT name, value_json FROM plugin_arrays WHERE plugin_id = ?1"
        )?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in rows {
            let (name, json) = r?;
            // Simple JSON array: ["a","b","c"]
            let vals: Vec<String> = json.trim_matches(|c| c == '[' || c == ']')
                .split(',')
                .map(|s| s.trim().trim_matches('"').to_string())
                .filter(|s| !s.is_empty())
                .collect();
            delta.arrays.push((name, vals));
        }

        // Completions
        let mut stmt = self.conn.prepare(
            "SELECT command, function FROM plugin_completions WHERE plugin_id = ?1"
        )?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in rows { delta.completions.push(r?); }

        // Fpath
        let mut stmt = self.conn.prepare(
            "SELECT path FROM plugin_fpath WHERE plugin_id = ?1"
        )?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            row.get::<_, String>(0)
        })?;
        for r in rows { delta.fpath_additions.push(r?); }

        // Hooks
        let mut stmt = self.conn.prepare(
            "SELECT hook, function FROM plugin_hooks WHERE plugin_id = ?1"
        )?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in rows { delta.hooks.push(r?); }

        // Bindkeys
        let mut stmt = self.conn.prepare(
            "SELECT keyseq, widget, keymap FROM plugin_bindkeys WHERE plugin_id = ?1"
        )?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?;
        for r in rows { delta.bindkeys.push(r?); }

        // Zstyles
        let mut stmt = self.conn.prepare(
            "SELECT pattern, style, value FROM plugin_zstyles WHERE plugin_id = ?1"
        )?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?;
        for r in rows { delta.zstyles.push(r?); }

        // Options
        let mut stmt = self.conn.prepare(
            "SELECT name, enabled FROM plugin_options WHERE plugin_id = ?1"
        )?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
        })?;
        for r in rows { delta.options_changed.push(r?); }

        // Autoloads
        let mut stmt = self.conn.prepare(
            "SELECT function, flags FROM plugin_autoloads WHERE plugin_id = ?1"
        )?;
        let rows = stmt.query_map(params![plugin_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for r in rows { delta.autoloads.push(r?); }

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
        self.conn.execute("DELETE FROM plugins WHERE path = ?1", params![path])?;

        self.conn.execute(
            "INSERT INTO plugins (path, mtime_secs, mtime_nsecs, source_time_ms, cached_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![path, mtime_secs, mtime_nsecs, source_time_ms as i64, now],
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
            let json = format!("[{}]", vals.iter().map(|v| format!("\"{}\"", v.replace('"', "\\\""))).collect::<Vec<_>>().join(","));
            self.conn.execute(
                "INSERT INTO plugin_arrays (plugin_id, name, value_json) VALUES (?1, ?2, ?3)",
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
        let plugins: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM plugins", [], |r| r.get(0)
        ).unwrap_or(0);
        let functions: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM plugin_functions", [], |r| r.get(0)
        ).unwrap_or(0);
        (plugins, functions)
    }

    /// Count plugins whose file mtime no longer matches the cache.
    pub fn count_stale(&self) -> usize {
        let mut stmt = match self.conn.prepare(
            "SELECT path, mtime_secs, mtime_nsecs FROM plugins"
        ) {
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
        for row in rows {
            if let Ok((path, cached_s, cached_ns)) = row {
                match file_mtime(std::path::Path::new(&path)) {
                    Some((s, ns)) if s != cached_s || ns != cached_ns => count += 1,
                    None => count += 1, // file deleted
                    _ => {}
                }
            }
        }
        count
    }

    /// Count AST cache entries whose file mtime no longer matches.
    pub fn count_stale_ast(&self) -> usize {
        let mut stmt = match self.conn.prepare(
            "SELECT path, mtime_secs, mtime_nsecs FROM script_ast"
        ) {
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
        for row in rows {
            if let Ok((path, cached_s, cached_ns)) = row {
                match file_mtime(std::path::Path::new(&path)) {
                    Some((s, ns)) if s != cached_s || ns != cached_ns => count += 1,
                    None => count += 1,
                    _ => {}
                }
            }
        }
        count
    }

    // -----------------------------------------------------------------
    // Script AST cache — skip lex+parse entirely
    // -----------------------------------------------------------------

    /// Check if a cached AST exists with matching mtime.
    pub fn check_ast(&self, path: &str, mtime_secs: i64, mtime_nsecs: i64) -> Option<Vec<u8>> {
        self.conn.query_row(
            "SELECT ast FROM script_ast WHERE path = ?1 AND mtime_secs = ?2 AND mtime_nsecs = ?3",
            params![path, mtime_secs, mtime_nsecs],
            |row| row.get::<_, Vec<u8>>(0),
        ).ok()
    }

    /// Store a parsed AST for a script file.
    pub fn store_ast(
        &self,
        path: &str,
        mtime_secs: i64,
        mtime_nsecs: i64,
        ast_bytes: &[u8],
    ) -> rusqlite::Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        self.conn.execute("DELETE FROM script_ast WHERE path = ?1", params![path])?;
        self.conn.execute(
            "INSERT INTO script_ast (path, mtime_secs, mtime_nsecs, ast, cached_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![path, mtime_secs, mtime_nsecs, ast_bytes, now],
        )?;
        Ok(())
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
            let parent_secure = dir.parent()
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
            tracing::debug!(dirs = fpath.len(), "compaudit: all directories secure (cached)");
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

/// Default path for the plugin cache db.
pub fn default_cache_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".cache/zshrs/plugins.db")
}
