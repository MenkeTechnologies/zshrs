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
use std::path::{Path, PathBuf};

/// On-disk format version for cached fusevm chunks. Bumped when fusevm's
/// bincode layout changes in a non-backward-compat way. Cached blobs are
/// stored as `[VERSION_BYTE, bincode_bytes...]`; readers verify the prefix
/// and treat any mismatch as a cache miss (the source file is recompiled).
///
/// Version history:
///   0  — fusevm 0.10.0 (Phase F baseline; no version prefix in storage)
///   1  — fusevm 0.10.1 (Tier C: argv-flatten in Op::Exec/ExecBg/CallFunction
///        + ShellHost::exec_bg; current)
///
/// Bumping is a one-line change. Existing caches transparently rebuild — no
/// migration code needed because the unwrap function returns None on
/// mismatch and the caller's "cache miss → compile" path takes over.
pub const BYTECODE_VERSION: u8 = 1;

/// Wrap raw bincode bytes with the format version prefix. Called by
/// `store_bytecode` (and any other persisted-chunk writer in the future)
/// before the INSERT.
#[inline]
fn wrap_bytecode(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 1);
    out.push(BYTECODE_VERSION);
    out.extend_from_slice(bytes);
    out
}

/// Strip and verify the format version prefix. Returns `Some(inner_bytes)`
/// if the prefix matches the current `BYTECODE_VERSION`, `None` otherwise.
/// `None` triggers cache miss in the caller, which silently recompiles from
/// source — no warning, no error (the maintainer's "no nag" rule).
#[inline]
fn unwrap_bytecode(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.is_empty() || bytes[0] != BYTECODE_VERSION {
        return None;
    }
    Some(bytes[1..].to_vec())
}

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

            -- Bytecode cache: skip lex+parse+compile entirely on cache hit
            CREATE TABLE IF NOT EXISTS script_bytecode (
                id INTEGER PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                mtime_secs INTEGER NOT NULL,
                mtime_nsecs INTEGER NOT NULL,
                bytecode BLOB NOT NULL,
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
            CREATE INDEX IF NOT EXISTS idx_script_bytecode_path ON script_bytecode(path);
            CREATE INDEX IF NOT EXISTS idx_compaudit_path ON compaudit_cache(path);
        "#,
        )?;
        Ok(())
    }

    /// Check if a cached entry exists with matching mtime.
    /// Returns the plugin id if cache is valid, None if miss.
    pub fn check(&self, path: &str, mtime_secs: i64, mtime_nsecs: i64) -> Option<i64> {
        self.conn
            .query_row(
                "SELECT id FROM plugins WHERE path = ?1 AND mtime_secs = ?2 AND mtime_nsecs = ?3",
                params![path, mtime_secs, mtime_nsecs],
                |row| row.get(0),
            )
            .ok()
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

    /// Count bytecode cache entries whose file mtime no longer matches.
    pub fn count_stale_bytecode(&self) -> usize {
        let mut stmt = match self
            .conn
            .prepare("SELECT path, mtime_secs, mtime_nsecs FROM script_bytecode")
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
                None => count += 1,
                _ => {}
            }
        }
        count
    }

    // -----------------------------------------------------------------
    // Script bytecode cache — skip lex+parse+compile entirely
    // -----------------------------------------------------------------

    /// Check if cached bytecode exists with matching mtime AND a current
    /// format-version prefix. A prefix mismatch returns None — the caller
    /// treats this as a cache miss and recompiles from source. No warning is
    /// printed; the rebuild is silent.
    pub fn check_bytecode(&self, path: &str, mtime_secs: i64, mtime_nsecs: i64) -> Option<Vec<u8>> {
        let raw = self.conn.query_row(
            "SELECT bytecode FROM script_bytecode WHERE path = ?1 AND mtime_secs = ?2 AND mtime_nsecs = ?3",
            params![path, mtime_secs, mtime_nsecs],
            |row| row.get::<_, Vec<u8>>(0),
        ).ok()?;
        unwrap_bytecode(&raw)
    }

    /// Store compiled bytecode for a script file. The blob is wrapped with
    /// the format version prefix so future readers (after a fusevm bump) can
    /// detect the staleness without parsing the bincode body.
    pub fn store_bytecode(
        &self,
        path: &str,
        mtime_secs: i64,
        mtime_nsecs: i64,
        bytecode: &[u8],
    ) -> rusqlite::Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let wrapped = wrap_bytecode(bytecode);
        self.conn
            .execute("DELETE FROM script_bytecode WHERE path = ?1", params![path])?;
        self.conn.execute(
            "INSERT INTO script_bytecode (path, mtime_secs, mtime_nsecs, bytecode, cached_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![path, mtime_secs, mtime_nsecs, wrapped, now],
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
mod version_tests {
    use super::*;

    #[test]
    fn wrap_unwrap_round_trip() {
        let raw = b"some-bincode-blob".to_vec();
        let wrapped = wrap_bytecode(&raw);
        assert_eq!(wrapped[0], BYTECODE_VERSION);
        let unwrapped = unwrap_bytecode(&wrapped).expect("matching version unwraps");
        assert_eq!(unwrapped, raw);
    }

    #[test]
    fn unwrap_rejects_old_version() {
        // Pre-version-byte cache (or a bumped version) should be rejected.
        // The caller's cache-miss branch then recompiles from source.
        let mut bogus = vec![0u8]; // version 0 (pre-Tier C)
        bogus.extend_from_slice(b"old-bincode-blob");
        assert!(unwrap_bytecode(&bogus).is_none());

        let mut future = vec![BYTECODE_VERSION.wrapping_add(1)];
        future.extend_from_slice(b"future-bincode-blob");
        assert!(unwrap_bytecode(&future).is_none());
    }

    #[test]
    fn unwrap_rejects_empty_blob() {
        assert!(unwrap_bytecode(&[]).is_none());
    }

    #[test]
    fn store_then_check_round_trips_through_sqlite() {
        // End-to-end: serialize a tiny chunk-shaped blob, store via the
        // cache, read it back, confirm it matches. Proves the version byte
        // is invisible to callers under normal operation.
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test_cache.db");
        let cache = PluginCache::open(&db_path).expect("open temp cache");

        let path = "/fake/script.zsh";
        let blob = b"bincode-bytes-here".to_vec();
        cache
            .store_bytecode(path, 12345, 6789, &blob)
            .expect("store");
        let got = cache.check_bytecode(path, 12345, 6789).expect("hit");
        assert_eq!(got, blob);
    }

    #[test]
    fn manually_inserted_old_version_invalidates() {
        // Simulate a pre-Tier-C cache by INSERTing a row with version byte 0.
        // check_bytecode must return None so the caller falls back to
        // recompile-from-source.
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("legacy_cache.db");
        let cache = PluginCache::open(&db_path).expect("open temp cache");

        let mut legacy = vec![0u8]; // wrong version
        legacy.extend_from_slice(b"would-be-bincode");
        cache
            .conn
            .execute(
                "INSERT INTO script_bytecode (path, mtime_secs, mtime_nsecs, bytecode, cached_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params!["/fake/legacy.zsh", 0i64, 0i64, legacy, 0i64],
            )
            .unwrap();

        let result = cache.check_bytecode("/fake/legacy.zsh", 0, 0);
        assert!(result.is_none(), "legacy bytecode must invalidate");
    }
}
