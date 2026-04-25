//! SQLite-backed cache for compsys
//!
//! MAXIMUM SPEED OPTIMIZATIONS:
//! - FTS5 for prefix search (O(1) vs O(n) LIKE)
//! - WAL mode for concurrent reads
//! - Memory-mapped I/O (mmap)
//! - No JOINs, no GROUP BY, no subqueries
//! - Denormalized flat tables with covering indexes
//! - Prepared statement caching

use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// SQLite cache for completion system
pub struct CompsysCache {
    conn: Connection,
}

/// Returns the default cache path: ~/.cache/zshrs/compsys.db
pub fn default_cache_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".cache/zshrs/compsys.db")
}

impl CompsysCache {
    /// Access the underlying SQLite connection (for dbview etc.)
    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Count rows in a table.
    pub fn count_table(&self, table: &str) -> rusqlite::Result<usize> {
        // Table name is not user input — it comes from our code.
        let sql = format!("SELECT COUNT(*) FROM {}", table);
        self.conn.query_row(&sql, [], |row| row.get::<_, i64>(0).map(|n| n as usize))
    }

    /// Count rows matching a WHERE clause.
    pub fn count_table_where(&self, table: &str, condition: &str) -> rusqlite::Result<usize> {
        let sql = format!("SELECT COUNT(*) FROM {} WHERE {}", table, condition);
        self.conn.query_row(&sql, [], |row| row.get::<_, i64>(0).map(|n| n as usize))
    }

    /// Open or create cache database with maximum performance settings
    pub fn open(path: impl AsRef<Path>) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        let cache = Self { conn };
        cache.configure_for_speed()?;
        cache.init_schema()?;
        Ok(cache)
    }

    /// In-memory cache (for testing)
    pub fn memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let cache = Self { conn };
        cache.configure_for_speed()?;
        cache.init_schema()?;
        Ok(cache)
    }

    /// Configure SQLite for maximum read performance (called on every open)
    fn configure_for_speed(&self) -> rusqlite::Result<()> {
        // WAL mode persists, but cache/mmap need to be set each session
        self.conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA cache_size = -64000;
            PRAGMA mmap_size = 268435456;
            PRAGMA temp_store = MEMORY;
            "#,
        )
    }

    fn init_schema(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            r#"
            -- Autoloads: flat table, PRIMARY KEY = clustered index
            -- body stores actual function definition - NO filesystem access on autoload -Xz
            -- compinit reads from .zwc or plain files ONCE, stores body here
            CREATE TABLE IF NOT EXISTS autoloads (
                name TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                offset INTEGER NOT NULL,
                size INTEGER NOT NULL,
                body TEXT,
                bytecode BLOB
            ) WITHOUT ROWID;

            -- zstyle: flat lookup by pattern+style
            CREATE TABLE IF NOT EXISTS zstyles (
                pattern TEXT NOT NULL,
                style TEXT NOT NULL,
                value TEXT NOT NULL,
                eval INTEGER DEFAULT 0,
                PRIMARY KEY (pattern, style)
            ) WITHOUT ROWID;

            -- Completion mappings: direct key lookup
            CREATE TABLE IF NOT EXISTS comps (
                command TEXT PRIMARY KEY,
                function TEXT NOT NULL
            ) WITHOUT ROWID;

            -- Pattern completions
            CREATE TABLE IF NOT EXISTS patcomps (
                pattern TEXT PRIMARY KEY,
                function TEXT NOT NULL
            ) WITHOUT ROWID;

            -- Key completions
            CREATE TABLE IF NOT EXISTS keycomps (
                key TEXT PRIMARY KEY,
                function TEXT NOT NULL
            ) WITHOUT ROWID;

            -- Services
            CREATE TABLE IF NOT EXISTS services (
                command TEXT PRIMARY KEY,
                service TEXT NOT NULL
            ) WITHOUT ROWID;

            -- Result cache
            CREATE TABLE IF NOT EXISTS cache (
                context TEXT PRIMARY KEY,
                data BLOB NOT NULL,
                mtime INTEGER NOT NULL
            ) WITHOUT ROWID;

            -- PATH executables: flat, fast prefix via FTS5
            CREATE TABLE IF NOT EXISTS executables (
                name TEXT PRIMARY KEY,
                path TEXT NOT NULL
            ) WITHOUT ROWID;

            -- Named directories
            CREATE TABLE IF NOT EXISTS named_dirs (
                name TEXT PRIMARY KEY,
                path TEXT NOT NULL
            ) WITHOUT ROWID;

            -- Shell functions
            CREATE TABLE IF NOT EXISTS shell_functions (
                name TEXT PRIMARY KEY,
                source TEXT NOT NULL
            ) WITHOUT ROWID;

            -- Metadata
            CREATE TABLE IF NOT EXISTS metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            ) WITHOUT ROWID;

            -- FTS5 for lightning-fast prefix search (standalone, not content-synced)
            CREATE VIRTUAL TABLE IF NOT EXISTS fts_comps USING fts5(
                command,
                tokenize='unicode61'
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS fts_executables USING fts5(
                name,
                tokenize='unicode61'
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS fts_shell_functions USING fts5(
                name,
                tokenize='unicode61'
            );

            -- Covering index for comps prefix search (fallback if FTS unavailable)
            CREATE INDEX IF NOT EXISTS idx_comps_cmd ON comps(command);
            CREATE INDEX IF NOT EXISTS idx_comps_func ON comps(function);
            CREATE INDEX IF NOT EXISTS idx_executables_name ON executables(name);
            CREATE INDEX IF NOT EXISTS idx_shell_functions_name ON shell_functions(name);
            CREATE INDEX IF NOT EXISTS idx_named_dirs_name ON named_dirs(name);
        "#,
        )?;
        self.migrate()?;
        Ok(())
    }

    /// Schema migrations for existing databases.
    fn migrate(&self) -> rusqlite::Result<()> {
        // Add bytecode BLOB column to autoloads if missing (pre-v0.8.16 databases)
        let has_bytecode: bool = self
            .conn
            .prepare("SELECT bytecode FROM autoloads LIMIT 0")
            .is_ok();
        if !has_bytecode {
            // Try renaming old ast column, or add new one
            let has_ast: bool = self
                .conn
                .prepare("SELECT ast FROM autoloads LIMIT 0")
                .is_ok();
            if has_ast {
                self.conn
                    .execute_batch("ALTER TABLE autoloads RENAME COLUMN ast TO bytecode")?;
            } else {
                self.conn
                    .execute_batch("ALTER TABLE autoloads ADD COLUMN bytecode BLOB")?;
            }
        }
        Ok(())
    }

    // =========================================================================
    // Autoloads - function stubs
    // =========================================================================

    /// Register an autoload stub (without body)
    pub fn add_autoload(
        &self,
        name: &str,
        source: &str,
        offset: i64,
        size: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO autoloads (name, source, offset, size, body) VALUES (?1, ?2, ?3, ?4, NULL)",
            params![name, source, offset, size],
        )?;
        Ok(())
    }

    /// Register an autoload with full function body (for instant loading)
    pub fn add_autoload_with_body(
        &self,
        name: &str,
        source: &str,
        body: &str,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO autoloads (name, source, offset, size, body) VALUES (?1, ?2, 0, ?3, ?4)",
            params![name, source, body.len() as i64, body],
        )?;
        Ok(())
    }

    /// Bulk insert autoloads (much faster)
    pub fn add_autoloads_bulk(
        &mut self,
        autoloads: &[(String, String, i64, i64)],
    ) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO autoloads (name, source, offset, size, body) VALUES (?1, ?2, ?3, ?4, NULL)"
            )?;
            for (name, source, offset, size) in autoloads {
                stmt.execute(params![name, source, offset, size])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Bulk insert autoloads with bodies (for compinit to cache function definitions)
    pub fn add_autoloads_with_bodies_bulk(
        &mut self,
        autoloads: &[(String, String, String)], // (name, source, body)
    ) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO autoloads (name, source, offset, size, body) VALUES (?1, ?2, 0, ?3, ?4)"
            )?;
            for (name, source, body) in autoloads {
                stmt.execute(params![name, source, body.len() as i64, body])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Lookup autoload by name
    pub fn get_autoload(&self, name: &str) -> rusqlite::Result<Option<AutoloadStub>> {
        self.conn
            .query_row(
                "SELECT source, offset, size, body FROM autoloads WHERE name = ?1",
                params![name],
                |row| {
                    Ok(AutoloadStub {
                        name: name.to_string(),
                        source: row.get(0)?,
                        offset: row.get(1)?,
                        size: row.get(2)?,
                        body: row.get(3)?,
                    })
                },
            )
            .optional()
    }

    /// Get function body directly (fast path for autoload -Xz)
    pub fn get_autoload_body(&self, name: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT body FROM autoloads WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .optional()
    }

    /// Get pre-parsed AST blob for a function (skip lex+parse on cache hit).
    /// Returns None if no AST is cached — caller falls back to parsing the body.
    pub fn get_autoload_bytecode(&self, name: &str) -> rusqlite::Result<Option<Vec<u8>>> {
        self.conn
            .query_row(
                "SELECT bytecode FROM autoloads WHERE name = ?1 AND bytecode IS NOT NULL",
                params![name],
                |row| row.get(0),
            )
            .optional()
    }

    /// Store pre-parsed AST blob for a function.
    pub fn set_autoload_bytecode(&self, name: &str, ast: &[u8]) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE autoloads SET bytecode = ?2 WHERE name = ?1",
            params![name, ast],
        )?;
        Ok(())
    }

    /// Get a batch of autoloads that have a body but no cached AST blob.
    /// Returns up to `limit` entries to avoid loading all 16k bodies into RAM.
    /// Caller should loop until this returns an empty vec.
    pub fn get_autoloads_missing_bytecode(&self) -> rusqlite::Result<Vec<(String, String)>> {
        self.get_autoloads_missing_bytecode_batch(100)
    }

    /// Get a batch of autoloads missing AST blobs, with configurable limit.
    pub fn get_autoloads_missing_bytecode_batch(&self, limit: usize) -> rusqlite::Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, body FROM autoloads WHERE body IS NOT NULL AND bytecode IS NULL LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect()
    }

    /// Count autoloads missing AST blobs (cheap — no body data loaded).
    pub fn count_autoloads_missing_bytecode(&self) -> rusqlite::Result<usize> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM autoloads WHERE body IS NOT NULL AND bytecode IS NULL",
            [],
            |row| row.get::<_, i64>(0).map(|n| n as usize),
        )
    }

    /// Bulk store AST blobs during compinit (one transaction for all functions).
    pub fn set_autoload_bytecodes_bulk(
        &mut self,
        entries: &[(String, Vec<u8>)], // (name, ast_blob)
    ) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "UPDATE autoloads SET bytecode = ?2 WHERE name = ?1",
            )?;
            for (name, ast) in entries {
                stmt.execute(params![name, ast])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Get function body with ZWC fallback
    /// 1. If body column has content, return it (fast path)
    /// 2. If body is NULL but source/offset/size exist, read from ZWC file
    /// 3. Returns None if function not found or ZWC read fails
    pub fn get_autoload_body_or_zwc(&self, name: &str) -> Option<String> {
        let stub = self.get_autoload(name).ok()??;

        // Fast path: body is cached
        if let Some(body) = stub.body {
            return Some(body);
        }

        // Fallback: read from ZWC file
        if stub.size > 0 && !stub.source.is_empty() {
            return Self::read_function_from_zwc(&stub.source, stub.offset, stub.size);
        }

        None
    }

    /// Read function body from ZWC file at given offset/size
    fn read_function_from_zwc(zwc_path: &str, offset: i64, size: i64) -> Option<String> {
        use std::io::{Read, Seek, SeekFrom};

        let mut file = std::fs::File::open(zwc_path).ok()?;
        file.seek(SeekFrom::Start(offset as u64)).ok()?;

        let mut buf = vec![0u8; size as usize];
        file.read_exact(&mut buf).ok()?;

        // ZWC stores tokenized strings - need to untokenize
        // For now, just try to interpret as UTF-8 (works for most cases)
        // TODO: proper untokenization like zwc.rs does
        match String::from_utf8(buf) {
            Ok(s) => Some(s),
            Err(e) => Some(String::from_utf8_lossy(e.as_bytes()).into_owned()),
        }
    }

    /// Count autoloads
    pub fn autoload_count(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM autoloads", [], |row| row.get(0))
    }

    /// List all autoload names (for debugging)
    pub fn list_autoloads(&self, limit: usize) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT name FROM autoloads LIMIT ?1")?;
        let rows = stmt.query_map(params![limit as i64], |row| row.get(0))?;
        rows.collect()
    }

    /// List all autoload names (no limit)
    pub fn list_autoload_names(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT name FROM autoloads")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }

    // =========================================================================
    // zstyle database
    // =========================================================================

    /// Set a zstyle
    pub fn set_zstyle(
        &self,
        pattern: &str,
        style: &str,
        values: &[String],
        eval: bool,
    ) -> rusqlite::Result<()> {
        let value_json = serde_values_to_json(values);
        self.conn.execute(
            "INSERT OR REPLACE INTO zstyles (pattern, style, value, eval) VALUES (?1, ?2, ?3, ?4)",
            params![pattern, style, value_json, eval as i32],
        )?;
        Ok(())
    }

    /// Bulk insert zstyles
    pub fn set_zstyles_bulk(
        &mut self,
        styles: &[(String, String, Vec<String>, bool)],
    ) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO zstyles (pattern, style, value, eval) VALUES (?1, ?2, ?3, ?4)"
            )?;
            for (pattern, style, values, eval) in styles {
                let value_json = serde_values_to_json(values);
                stmt.execute(params![pattern, style, value_json, *eval as i32])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Delete a zstyle
    pub fn delete_zstyle(&self, pattern: &str, style: Option<&str>) -> rusqlite::Result<usize> {
        if let Some(s) = style {
            self.conn.execute(
                "DELETE FROM zstyles WHERE pattern = ?1 AND style = ?2",
                params![pattern, s],
            )
        } else {
            self.conn
                .execute("DELETE FROM zstyles WHERE pattern = ?1", params![pattern])
        }
    }

    /// Lookup zstyle - returns all matching patterns sorted by specificity
    pub fn lookup_zstyle(
        &self,
        context: &str,
        style: &str,
    ) -> rusqlite::Result<Option<ZStyleEntry>> {
        let mut stmt = self
            .conn
            .prepare("SELECT pattern, value, eval FROM zstyles WHERE style = ?1")?;

        let entries: Vec<(String, String, bool)> = stmt
            .query_map(params![style], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get::<_, i32>(2)? != 0))
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Find best match by specificity
        let mut best: Option<(i32, String, bool)> = None;
        for (pattern, value, eval) in entries {
            if pattern_matches_context(&pattern, context) {
                let weight = calculate_pattern_weight(&pattern);
                if best.is_none() || weight > best.as_ref().unwrap().0 {
                    best = Some((weight, value, eval));
                }
            }
        }

        Ok(best.map(|(_, value, eval)| ZStyleEntry {
            values: serde_json_to_values(&value),
            eval,
        }))
    }

    /// List all zstyles (for `zstyle -L`)
    pub fn list_zstyles(&self) -> rusqlite::Result<Vec<(String, String, Vec<String>, bool)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT pattern, style, value, eval FROM zstyles ORDER BY pattern, style")?;
        let rows = stmt.query_map([], |row| {
            let pattern: String = row.get(0)?;
            let style: String = row.get(1)?;
            let value: String = row.get(2)?;
            let eval: bool = row.get::<_, i32>(3)? != 0;
            Ok((pattern, style, serde_json_to_values(&value), eval))
        })?;
        rows.collect()
    }

    /// Count zstyles
    pub fn zstyle_count(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM zstyles", [], |row| row.get(0))
    }

    // =========================================================================
    // Completion mappings (_comps)
    // =========================================================================

    /// Register a completion function for a command
    pub fn set_comp(&self, command: &str, function: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO comps (command, function) VALUES (?1, ?2)",
            params![command, function],
        )?;
        Ok(())
    }

    /// Bulk insert comps + populate FTS5 index
    pub fn set_comps_bulk(&mut self, comps: &[(String, String)]) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        // Clear and repopulate both tables
        tx.execute("DELETE FROM comps", [])?;
        tx.execute("DELETE FROM fts_comps", [])?;
        {
            let mut stmt = tx.prepare("INSERT INTO comps (command, function) VALUES (?1, ?2)")?;
            let mut fts_stmt = tx.prepare("INSERT INTO fts_comps (command) VALUES (?1)")?;
            for (command, function) in comps {
                stmt.execute(params![command, function])?;
                fts_stmt.execute(params![command])?;
            }
        }
        tx.commit()
    }

    /// Fast prefix search using FTS5 (O(log n) vs O(n) for LIKE)
    pub fn comps_prefix_fts(&self, prefix: &str) -> rusqlite::Result<Vec<(String, String)>> {
        if prefix.is_empty() {
            return self.comps_kv();
        }
        // FTS5 prefix search: "git*" matches git, github, gitk, etc.
        let pattern = format!("{}*", prefix);
        let mut stmt = self.conn.prepare(
            "SELECT c.command, c.function FROM fts_comps f, comps c WHERE f.command MATCH ?1 AND c.command = f.command"
        )?;
        let rows = stmt.query_map(params![pattern], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// Fast prefix search (LIKE with index scan, ORDER BY is free on indexed column)
    pub fn comps_prefix(&self, prefix: &str) -> rusqlite::Result<Vec<(String, String)>> {
        if prefix.is_empty() {
            return self.comps_kv();
        }
        let pattern = format!("{}%", prefix);
        let mut stmt = self.conn.prepare(
            "SELECT command, function FROM comps WHERE command LIKE ?1 ORDER BY command",
        )?;
        let rows = stmt.query_map(params![pattern], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// Lookup completion function for command
    pub fn get_comp(&self, command: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT function FROM comps WHERE command = ?1",
                params![command],
                |row| row.get(0),
            )
            .optional()
    }

    /// Get all comps as HashMap (for compatibility)
    pub fn get_all_comps(&self) -> rusqlite::Result<HashMap<String, String>> {
        let mut stmt = self.conn.prepare("SELECT command, function FROM comps")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut map = HashMap::new();
        for row in rows {
            let (k, v) = row?;
            map.insert(k, v);
        }
        Ok(map)
    }

    /// Count comps
    pub fn comp_count(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM comps", [], |row| row.get(0))
    }

    /// Delete a completion registration
    pub fn delete_comp(&self, command: &str) -> rusqlite::Result<usize> {
        self.conn
            .execute("DELETE FROM comps WHERE command = ?1", params![command])
    }

    // =========================================================================
    // Pattern completions (_patcomps)
    // =========================================================================

    /// Register a pattern completion
    pub fn set_patcomp(&self, pattern: &str, function: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO patcomps (pattern, function) VALUES (?1, ?2)",
            params![pattern, function],
        )?;
        Ok(())
    }

    /// Find matching pattern completion
    pub fn find_patcomp(&self, command: &str) -> rusqlite::Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT pattern, function FROM patcomps")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;

        for row in rows {
            let (pattern, function) = row?;
            if glob_matches(&pattern, command) {
                return Ok(Some(function));
            }
        }
        Ok(None)
    }

    // =========================================================================
    // Key completions
    // =========================================================================

    /// Register a key completion (for -K)
    pub fn set_keycomp(&self, key: &str, function: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO keycomps (key, function) VALUES (?1, ?2)",
            params![key, function],
        )?;
        Ok(())
    }

    /// Lookup key completion
    pub fn get_keycomp(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT function FROM keycomps WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
    }

    // =========================================================================
    // Result cache
    // =========================================================================

    /// Cache completion results
    pub fn cache_results(&self, context: &str, data: &[u8], mtime: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO cache (context, data, mtime) VALUES (?1, ?2, ?3)",
            params![context, data, mtime],
        )?;
        Ok(())
    }

    /// Get cached results if not stale
    pub fn get_cached(&self, context: &str, max_age: i64) -> rusqlite::Result<Option<Vec<u8>>> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.conn
            .query_row(
                "SELECT data FROM cache WHERE context = ?1 AND mtime > ?2",
                params![context, now - max_age],
                |row| row.get(0),
            )
            .optional()
    }

    /// Clear old cache entries
    pub fn clear_stale_cache(&self, max_age: i64) -> rusqlite::Result<usize> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        self.conn
            .execute("DELETE FROM cache WHERE mtime < ?1", params![now - max_age])
    }

    /// Clear all cache
    pub fn clear_cache(&self) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM cache", [])?;
        Ok(())
    }

    // =========================================================================
    // Maintenance
    // =========================================================================

    /// Vacuum database
    pub fn vacuum(&self) -> rusqlite::Result<()> {
        self.conn.execute("VACUUM", [])?;
        Ok(())
    }

    /// Get database stats
    pub fn stats(&self) -> rusqlite::Result<CacheStats> {
        Ok(CacheStats {
            autoloads: self.autoload_count()?,
            zstyles: self.zstyle_count()?,
            comps: self.comp_count()?,
            patcomps: self
                .conn
                .query_row("SELECT COUNT(*) FROM patcomps", [], |r| r.get(0))?,
            keycomps: self
                .conn
                .query_row("SELECT COUNT(*) FROM keycomps", [], |r| r.get(0))?,
            services: self
                .conn
                .query_row("SELECT COUNT(*) FROM services", [], |r| r.get(0))?,
            cache_entries: self
                .conn
                .query_row("SELECT COUNT(*) FROM cache", [], |r| r.get(0))?,
        })
    }
}

/// Autoload stub info
#[derive(Debug, Clone)]
pub struct AutoloadStub {
    pub name: String,
    pub source: String,
    pub offset: i64,
    pub size: i64,
    /// Cached function body - if present, no need to read from source file
    pub body: Option<String>,
}

/// zstyle entry
#[derive(Debug, Clone)]
pub struct ZStyleEntry {
    pub values: Vec<String>,
    pub eval: bool,
}

/// Cache statistics
#[derive(Debug)]
pub struct CacheStats {
    pub autoloads: i64,
    pub zstyles: i64,
    pub comps: i64,
    pub patcomps: i64,
    pub keycomps: i64,
    pub services: i64,
    pub cache_entries: i64,
}

// Helper: serialize values to JSON
fn serde_values_to_json(values: &[String]) -> String {
    let escaped: Vec<String> = values
        .iter()
        .map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect();
    format!("[{}]", escaped.join(","))
}

// Helper: deserialize JSON to values
fn serde_json_to_values(json: &str) -> Vec<String> {
    let trimmed = json.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return vec![json.to_string()];
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.is_empty() {
        return vec![];
    }

    let mut values = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut escape = false;

    for c in inner.chars() {
        if escape {
            current.push(c);
            escape = false;
        } else if c == '\\' {
            escape = true;
        } else if c == '"' {
            in_string = !in_string;
        } else if c == ',' && !in_string {
            values.push(current.trim().to_string());
            current = String::new();
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        values.push(current.trim().to_string());
    }

    values
}

// Helper: check if zstyle pattern matches context
fn pattern_matches_context(pattern: &str, context: &str) -> bool {
    let pat_parts: Vec<&str> = pattern.split(':').collect();
    let ctx_parts: Vec<&str> = context.split(':').collect();

    if pat_parts.len() > ctx_parts.len() {
        return false;
    }

    for (p, c) in pat_parts.iter().zip(ctx_parts.iter()) {
        if *p != "*" && *p != *c {
            return false;
        }
    }

    true
}

// Helper: calculate pattern weight for specificity
fn calculate_pattern_weight(pattern: &str) -> i32 {
    let parts: Vec<&str> = pattern.split(':').filter(|s| !s.is_empty()).collect();
    let mut weight = parts.len() as i32 * 100;

    for part in &parts {
        if *part != "*" {
            weight += 10;
        }
    }

    weight
}

// Helper: glob matching for patcomps
fn glob_matches(pattern: &str, text: &str) -> bool {
    let mut pat_chars = pattern.chars().peekable();
    let mut txt_chars = text.chars().peekable();

    while let Some(p) = pat_chars.next() {
        match p {
            '*' => {
                if pat_chars.peek().is_none() {
                    return true;
                }
                while txt_chars.peek().is_some() {
                    if glob_matches(
                        &pat_chars.clone().collect::<String>(),
                        &txt_chars.clone().collect::<String>(),
                    ) {
                        return true;
                    }
                    txt_chars.next();
                }
                return false;
            }
            '?' => {
                if txt_chars.next().is_none() {
                    return false;
                }
            }
            c => {
                if txt_chars.next() != Some(c) {
                    return false;
                }
            }
        }
    }

    txt_chars.peek().is_none()
}

// =========================================================================
// Shell-visible arrays (_comps, _services, _patcomps, etc.)
// These back the zsh special arrays that users query with $#_comps etc.
// =========================================================================

impl CompsysCache {
    /// Get count of _comps entries (for $#_comps)
    pub fn comps_count(&self) -> rusqlite::Result<i64> {
        self.comp_count()
    }

    /// Get all _comps keys (for ${(k)_comps}) - ORDER BY is free on PRIMARY KEY
    pub fn comps_keys(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT command FROM comps ORDER BY command")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }

    /// Get all _comps values (for ${(v)_comps})
    pub fn comps_values(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT function FROM comps ORDER BY command")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }

    /// Get _comps as key-value pairs (for ${(kv)_comps})
    pub fn comps_kv(&self) -> rusqlite::Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT command, function FROM comps ORDER BY command")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    // --- _patcomps ---

    /// Get count of _patcomps
    pub fn patcomps_count(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM patcomps", [], |row| row.get(0))
    }

    /// Get all _patcomps keys
    pub fn patcomps_keys(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT pattern FROM patcomps")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }

    /// Get all _patcomps as kv
    pub fn patcomps_kv(&self) -> rusqlite::Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT pattern, function FROM patcomps")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    // --- _services ---

    /// Set a service mapping
    pub fn set_service(&self, command: &str, service: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO services (command, service) VALUES (?1, ?2)",
            params![command, service],
        )?;
        Ok(())
    }

    /// Get service for command
    pub fn get_service(&self, command: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT service FROM services WHERE command = ?1",
                params![command],
                |row| row.get(0),
            )
            .optional()
    }

    /// Get count of _services
    pub fn services_count(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM services", [], |row| row.get(0))
    }

    /// Get all _services keys
    pub fn services_keys(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT command FROM services")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }

    /// Bulk insert services
    pub fn set_services_bulk(&mut self, services: &[(String, String)]) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt =
                tx.prepare("INSERT OR REPLACE INTO services (command, service) VALUES (?1, ?2)")?;
            for (command, service) in services {
                stmt.execute(params![command, service])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    // --- _compautos (autoloaded completion functions) ---

    /// Get count of autoloaded functions
    pub fn compautos_count(&self) -> rusqlite::Result<i64> {
        self.autoload_count()
    }

    /// Get all autoload names (for ${(k)_compautos})
    pub fn compautos_keys(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT name FROM autoloads")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }

    // =========================================================================
    // PATH executables cache
    // =========================================================================

    /// Check if executables cache is populated
    pub fn has_executables(&self) -> rusqlite::Result<bool> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM executables", [], |row| row.get(0))?;
        Ok(count > 0)
    }

    /// Store executables in bulk + populate FTS5 index
    pub fn set_executables_bulk(
        &mut self,
        executables: &[(String, String)],
    ) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM executables", [])?;
        tx.execute("DELETE FROM fts_executables", [])?;
        {
            let mut stmt =
                tx.prepare("INSERT OR IGNORE INTO executables (name, path) VALUES (?1, ?2)")?;
            let mut fts_stmt =
                tx.prepare("INSERT OR IGNORE INTO fts_executables (name) VALUES (?1)")?;
            for (name, path) in executables {
                stmt.execute(params![name, path])?;
                fts_stmt.execute(params![name])?;
            }
        }
        tx.commit()
    }

    /// Get all executable names (fast lookup set)
    pub fn get_executable_names(&self) -> rusqlite::Result<std::collections::HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT name FROM executables")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<Result<std::collections::HashSet<_>, _>>()
    }

    /// Check if an executable exists in cache (O(1) lookup)
    pub fn has_executable(&self, name: &str) -> rusqlite::Result<bool> {
        // Use EXISTS for faster check (stops at first match)
        let exists: i64 = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM executables WHERE name = ?1)",
            params![name],
            |row| row.get(0),
        )?;
        Ok(exists == 1)
    }

    /// Get executable path by name (direct key lookup)
    pub fn get_executable_path(&self, name: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT path FROM executables WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .optional()
    }

    /// Fast prefix search using FTS5
    pub fn get_executables_prefix_fts(
        &self,
        prefix: &str,
    ) -> rusqlite::Result<Vec<(String, String)>> {
        if prefix.is_empty() {
            let mut stmt = self.conn.prepare("SELECT name, path FROM executables")?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            return rows.collect();
        }
        let pattern = format!("{}*", prefix);
        let mut stmt = self.conn.prepare(
            "SELECT e.name, e.path FROM fts_executables f, executables e WHERE f.name MATCH ?1 AND e.name = f.name"
        )?;
        let rows = stmt.query_map(params![pattern], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// Get executables matching prefix (LIKE with index, ORDER BY free on PRIMARY KEY)
    pub fn get_executables_prefix(&self, prefix: &str) -> rusqlite::Result<Vec<(String, String)>> {
        if prefix.is_empty() {
            let mut stmt = self
                .conn
                .prepare("SELECT name, path FROM executables ORDER BY name")?;
            let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
            return rows.collect();
        }
        let pattern = format!("{}%", prefix);
        let mut stmt = self
            .conn
            .prepare("SELECT name, path FROM executables WHERE name LIKE ?1 ORDER BY name")?;
        let rows = stmt.query_map(params![pattern], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// Count executables
    pub fn executables_count(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM executables", [], |row| row.get(0))
    }

    // =========================================================================
    // Named directories cache (hash -d)
    // =========================================================================

    /// Check if named_dirs cache is populated
    pub fn has_named_dirs(&self) -> rusqlite::Result<bool> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM named_dirs", [], |row| row.get(0))?;
        Ok(count > 0)
    }

    /// Store named directories in bulk (clears existing)
    pub fn set_named_dirs_bulk(&mut self, dirs: &[(String, String)]) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM named_dirs", [])?;
        {
            let mut stmt = tx.prepare("INSERT INTO named_dirs (name, path) VALUES (?1, ?2)")?;
            for (name, path) in dirs {
                stmt.execute(params![name, path])?;
            }
        }
        tx.commit()
    }

    /// Get all named directories (ORDER BY free on PRIMARY KEY)
    pub fn get_named_dirs(&self) -> rusqlite::Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, path FROM named_dirs ORDER BY name")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// Get named directories matching prefix
    pub fn get_named_dirs_prefix(&self, prefix: &str) -> rusqlite::Result<Vec<(String, String)>> {
        if prefix.is_empty() {
            return self.get_named_dirs();
        }
        let pattern = format!("{}%", prefix);
        let mut stmt = self
            .conn
            .prepare("SELECT name, path FROM named_dirs WHERE name LIKE ?1 ORDER BY name")?;
        let rows = stmt.query_map(params![pattern], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// Count named directories
    pub fn named_dirs_count(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM named_dirs", [], |row| row.get(0))
    }

    // =========================================================================
    // Shell functions cache (FPATH)
    // =========================================================================

    /// Check if shell_functions cache is populated
    pub fn has_shell_functions(&self) -> rusqlite::Result<bool> {
        let count: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM shell_functions", [], |row| row.get(0))?;
        Ok(count > 0)
    }

    /// Store shell functions in bulk + populate FTS5 index
    pub fn set_shell_functions_bulk(&mut self, funcs: &[(String, String)]) -> rusqlite::Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM shell_functions", [])?;
        tx.execute("DELETE FROM fts_shell_functions", [])?;
        {
            let mut stmt =
                tx.prepare("INSERT OR IGNORE INTO shell_functions (name, source) VALUES (?1, ?2)")?;
            let mut fts_stmt =
                tx.prepare("INSERT OR IGNORE INTO fts_shell_functions (name) VALUES (?1)")?;
            for (name, source) in funcs {
                stmt.execute(params![name, source])?;
                fts_stmt.execute(params![name])?;
            }
        }
        tx.commit()
    }

    /// Get all shell function names (ORDER BY free on PRIMARY KEY)
    pub fn get_shell_function_names(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM shell_functions ORDER BY name")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }

    /// Get shell functions with source paths
    pub fn get_shell_functions(&self) -> rusqlite::Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, source FROM shell_functions ORDER BY name")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// Fast prefix search using FTS5 (note: FTS5 doesn't preserve order, needs post-sort)
    pub fn get_shell_functions_prefix_fts(
        &self,
        prefix: &str,
    ) -> rusqlite::Result<Vec<(String, String)>> {
        if prefix.is_empty() {
            return self.get_shell_functions();
        }
        let pattern = format!("{}*", prefix);
        let mut stmt = self.conn.prepare(
            "SELECT s.name, s.source FROM fts_shell_functions f, shell_functions s WHERE f.name MATCH ?1 AND s.name = f.name ORDER BY s.name"
        )?;
        let rows = stmt.query_map(params![pattern], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// Get shell functions matching prefix (LIKE with index, ORDER BY free)
    pub fn get_shell_functions_prefix(
        &self,
        prefix: &str,
    ) -> rusqlite::Result<Vec<(String, String)>> {
        if prefix.is_empty() {
            return self.get_shell_functions();
        }
        let pattern = format!("{}%", prefix);
        let mut stmt = self
            .conn
            .prepare("SELECT name, source FROM shell_functions WHERE name LIKE ?1 ORDER BY name")?;
        let rows = stmt.query_map(params![pattern], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect()
    }

    /// Count shell functions
    pub fn shell_functions_count(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM shell_functions", [], |row| row.get(0))
    }

    // =========================================================================
    // Metadata for cache versioning/invalidation
    // =========================================================================

    /// Set metadata key-value
    pub fn set_metadata(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    /// Get metadata value
    pub fn get_metadata(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM metadata WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
    }

    // =========================================================================
    // Zstyle helpers
    // =========================================================================

    /// Check if zstyles cache is populated
    pub fn has_zstyles(&self) -> rusqlite::Result<bool> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM zstyles", [], |row| row.get(0))?;
        Ok(count > 0)
    }

    /// Count zstyles
    pub fn zstyles_count(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM zstyles", [], |row| row.get(0))
    }

    /// Get all zstyles (for debugging)
    pub fn get_all_zstyles(&self) -> rusqlite::Result<Vec<(String, String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT pattern, style, value FROM zstyles ORDER BY pattern, style")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_basic() {
        let cache = CompsysCache::memory().unwrap();

        cache
            .add_autoload("_git", "more_src.zwc", 1024, 5000)
            .unwrap();
        cache
            .add_autoload("_docker", "more_src.zwc", 6024, 3000)
            .unwrap();

        let stub = cache.get_autoload("_git").unwrap().unwrap();
        assert_eq!(stub.source, "more_src.zwc");
        assert_eq!(stub.offset, 1024);

        assert!(cache.get_autoload("_nonexistent").unwrap().is_none());
    }

    #[test]
    fn test_zstyle_cache() {
        let cache = CompsysCache::memory().unwrap();

        cache
            .set_zstyle(":completion:*", "menu", &["select".to_string()], false)
            .unwrap();
        cache
            .set_zstyle(
                ":completion:*:descriptions",
                "format",
                &["%d".to_string()],
                false,
            )
            .unwrap();

        let entry = cache
            .lookup_zstyle(":completion:foo", "menu")
            .unwrap()
            .unwrap();
        assert_eq!(entry.values, vec!["select"]);

        let entry = cache
            .lookup_zstyle(":completion:foo:descriptions", "format")
            .unwrap()
            .unwrap();
        assert_eq!(entry.values, vec!["%d"]);
    }

    #[test]
    fn test_zstyle_specificity() {
        let cache = CompsysCache::memory().unwrap();

        cache
            .set_zstyle(":completion:*", "menu", &["no".to_string()], false)
            .unwrap();
        cache
            .set_zstyle(
                ":completion:*:*:*:default",
                "menu",
                &["yes".to_string()],
                false,
            )
            .unwrap();

        let entry = cache
            .lookup_zstyle(":completion:foo:bar:baz:default", "menu")
            .unwrap()
            .unwrap();
        assert_eq!(entry.values, vec!["yes"]);
    }

    #[test]
    fn test_comps_cache() {
        let mut cache = CompsysCache::memory().unwrap();

        let comps = vec![
            ("git".to_string(), "_git".to_string()),
            ("docker".to_string(), "_docker".to_string()),
            ("cargo".to_string(), "_cargo".to_string()),
        ];
        cache.set_comps_bulk(&comps).unwrap();

        assert_eq!(cache.get_comp("git").unwrap(), Some("_git".to_string()));
        assert_eq!(
            cache.get_comp("docker").unwrap(),
            Some("_docker".to_string())
        );
        assert!(cache.get_comp("nonexistent").unwrap().is_none());

        assert_eq!(cache.comp_count().unwrap(), 3);
    }

    #[test]
    fn test_bulk_autoloads() {
        let mut cache = CompsysCache::memory().unwrap();

        let autoloads: Vec<(String, String, i64, i64)> = (0..1000)
            .map(|i| (format!("_func{}", i), "test.zwc".to_string(), i * 100, 100))
            .collect();

        cache.add_autoloads_bulk(&autoloads).unwrap();
        assert_eq!(cache.autoload_count().unwrap(), 1000);

        let stub = cache.get_autoload("_func500").unwrap().unwrap();
        assert_eq!(stub.offset, 50000);
        assert!(stub.body.is_none()); // No body when bulk inserted without
    }

    #[test]
    fn test_autoload_with_body() {
        let cache = CompsysCache::memory().unwrap();

        let body = r#"
local -a opts
opts=(--help --version --verbose)
_arguments $opts
"#;
        cache
            .add_autoload_with_body("_mycommand", "/usr/share/zsh/functions/_mycommand", body)
            .unwrap();

        let stub = cache.get_autoload("_mycommand").unwrap().unwrap();
        assert_eq!(stub.body.as_deref(), Some(body));
        assert_eq!(stub.size, body.len() as i64);

        // Fast path: get body directly
        let direct_body = cache.get_autoload_body("_mycommand").unwrap();
        assert_eq!(direct_body.as_deref(), Some(body));
    }

    #[test]
    fn test_bulk_autoloads_with_bodies() {
        let mut cache = CompsysCache::memory().unwrap();

        let autoloads: Vec<(String, String, String)> = (0..100)
            .map(|i| {
                (
                    format!("_func{}", i),
                    format!("/path/to/_func{}", i),
                    format!("# Function {}\necho hello", i),
                )
            })
            .collect();

        cache.add_autoloads_with_bodies_bulk(&autoloads).unwrap();
        assert_eq!(cache.autoload_count().unwrap(), 100);

        let stub = cache.get_autoload("_func50").unwrap().unwrap();
        assert!(stub.body.is_some());
        assert!(stub.body.unwrap().contains("Function 50"));
    }

    #[test]
    fn test_get_autoload_body_or_zwc_with_body() {
        let cache = CompsysCache::memory().unwrap();

        let body = "echo from sqlite";
        cache
            .add_autoload_with_body("_cached", "/some/path", body)
            .unwrap();

        // Should return body from SQLite (fast path)
        let result = cache.get_autoload_body_or_zwc("_cached");
        assert_eq!(result, Some(body.to_string()));
    }

    #[test]
    fn test_get_autoload_body_or_zwc_no_body() {
        let cache = CompsysCache::memory().unwrap();

        // Add autoload without body (just ZWC reference)
        cache
            .add_autoload("_nocache", "nonexistent.zwc", 0, 100)
            .unwrap();

        // Should return None since ZWC file doesn't exist
        let result = cache.get_autoload_body_or_zwc("_nocache");
        assert!(result.is_none());
    }

    #[test]
    fn test_get_autoload_body_or_zwc_not_found() {
        let cache = CompsysCache::memory().unwrap();

        // Function doesn't exist at all
        let result = cache.get_autoload_body_or_zwc("_nonexistent");
        assert!(result.is_none());
    }

    #[test]
    fn test_patcomp() {
        let cache = CompsysCache::memory().unwrap();

        cache.set_patcomp("git-*", "_git").unwrap();
        cache.set_patcomp("docker-*", "_docker").unwrap();

        assert_eq!(
            cache.find_patcomp("git-commit").unwrap(),
            Some("_git".to_string())
        );
        assert_eq!(
            cache.find_patcomp("docker-compose").unwrap(),
            Some("_docker".to_string())
        );
        assert!(cache.find_patcomp("cargo").unwrap().is_none());
    }

    #[test]
    fn test_glob_matches() {
        assert!(glob_matches("git-*", "git-commit"));
        assert!(glob_matches("*-compose", "docker-compose"));
        assert!(glob_matches("*.rs", "main.rs"));
        assert!(!glob_matches("git-*", "docker-compose"));
        assert!(glob_matches("???", "abc"));
        assert!(!glob_matches("???", "abcd"));
    }

    #[test]
    fn test_json_serde() {
        let values = vec!["hello".to_string(), "world".to_string()];
        let json = serde_values_to_json(&values);
        let back = serde_json_to_values(&json);
        assert_eq!(back, values);

        let values = vec!["with \"quotes\"".to_string()];
        let json = serde_values_to_json(&values);
        let back = serde_json_to_values(&json);
        assert_eq!(back, vec!["with \"quotes\""]);
    }

    #[test]
    fn test_stats() {
        let mut cache = CompsysCache::memory().unwrap();

        cache.add_autoload("_git", "test.zwc", 0, 100).unwrap();
        cache
            .set_zstyle(":completion:*", "menu", &["select".to_string()], false)
            .unwrap();
        cache.set_comp("git", "_git").unwrap();

        let stats = cache.stats().unwrap();
        assert_eq!(stats.autoloads, 1);
        assert_eq!(stats.zstyles, 1);
        assert_eq!(stats.comps, 1);
    }

    #[test]
    fn test_large_scale() {
        let mut cache = CompsysCache::memory().unwrap();

        // Simulate 500k autoloads
        let autoloads: Vec<(String, String, i64, i64)> = (0..10000)
            .map(|i| {
                (
                    format!("_func{}", i),
                    format!("src{}.zwc", i % 10),
                    i * 50,
                    50,
                )
            })
            .collect();

        cache.add_autoloads_bulk(&autoloads).unwrap();

        // Fast lookup
        let stub = cache.get_autoload("_func9999").unwrap().unwrap();
        assert_eq!(stub.offset, 9999 * 50);

        assert_eq!(cache.autoload_count().unwrap(), 10000);
    }

    #[test]
    fn test_executables_cache() {
        let mut cache = CompsysCache::memory().unwrap();

        let executables = vec![
            ("ls".to_string(), "/bin/ls".to_string()),
            ("cat".to_string(), "/bin/cat".to_string()),
            ("git".to_string(), "/usr/bin/git".to_string()),
        ];
        cache.set_executables_bulk(&executables).unwrap();

        assert!(cache.has_executables().unwrap());
        assert!(cache.has_executable("ls").unwrap());
        assert!(cache.has_executable("git").unwrap());
        assert!(!cache.has_executable("nonexistent").unwrap());

        assert_eq!(
            cache.get_executable_path("ls").unwrap(),
            Some("/bin/ls".to_string())
        );
        assert_eq!(cache.executables_count().unwrap(), 3);
    }

    #[test]
    fn test_executables_prefix_search() {
        let mut cache = CompsysCache::memory().unwrap();

        let executables = vec![
            ("git".to_string(), "/usr/bin/git".to_string()),
            ("gitk".to_string(), "/usr/bin/gitk".to_string()),
            ("grep".to_string(), "/bin/grep".to_string()),
            ("gzip".to_string(), "/bin/gzip".to_string()),
        ];
        cache.set_executables_bulk(&executables).unwrap();

        // FTS prefix search returns (name, path) tuples
        let git_cmds = cache.get_executables_prefix_fts("git").unwrap();
        assert_eq!(git_cmds.len(), 2);
        assert!(git_cmds.iter().any(|(name, _)| name == "git"));
        assert!(git_cmds.iter().any(|(name, _)| name == "gitk"));

        let g_cmds = cache.get_executables_prefix_fts("g").unwrap();
        assert_eq!(g_cmds.len(), 4);
    }

    #[test]
    fn test_named_dirs_cache() {
        let mut cache = CompsysCache::memory().unwrap();

        let dirs = vec![
            ("proj".to_string(), "/home/user/projects".to_string()),
            ("docs".to_string(), "/home/user/documents".to_string()),
        ];
        cache.set_named_dirs_bulk(&dirs).unwrap();

        assert!(cache.has_named_dirs().unwrap());

        let all = cache.get_named_dirs().unwrap();
        assert_eq!(all.len(), 2);

        let p_dirs = cache.get_named_dirs_prefix("p").unwrap();
        assert_eq!(p_dirs.len(), 1);
        assert_eq!(p_dirs[0].0, "proj");
    }

    #[test]
    fn test_shell_functions_cache() {
        let mut cache = CompsysCache::memory().unwrap();

        let functions = vec![
            ("myFunc".to_string(), "/home/user/.zshrc".to_string()),
            (
                "zpwrClearList".to_string(),
                "/home/user/.zpwr/autoload".to_string(),
            ),
            (
                "zpwrTop".to_string(),
                "/home/user/.zpwr/autoload".to_string(),
            ),
        ];
        cache.set_shell_functions_bulk(&functions).unwrap();

        assert!(cache.has_shell_functions().unwrap());
        assert_eq!(cache.shell_functions_count().unwrap(), 3);

        let zpwr = cache.get_shell_functions_prefix("zpwr").unwrap();
        assert_eq!(zpwr.len(), 2);
        // Results are tuples (name, source)
        assert!(zpwr.iter().any(|(name, _)| name == "zpwrClearList"));
        assert!(zpwr.iter().any(|(name, _)| name == "zpwrTop"));
    }

    #[test]
    fn test_metadata() {
        let cache = CompsysCache::memory().unwrap();

        cache.set_metadata("version", "1.0.0").unwrap();
        cache.set_metadata("build_time", "2026-04-22").unwrap();

        assert_eq!(
            cache.get_metadata("version").unwrap(),
            Some("1.0.0".to_string())
        );
        assert_eq!(
            cache.get_metadata("build_time").unwrap(),
            Some("2026-04-22".to_string())
        );
        assert_eq!(cache.get_metadata("nonexistent").unwrap(), None);
    }

    #[test]
    fn test_comps_keys() {
        let mut cache = CompsysCache::memory().unwrap();

        let comps = vec![
            ("git".to_string(), "_git".to_string()),
            ("docker".to_string(), "_docker".to_string()),
        ];
        cache.set_comps_bulk(&comps).unwrap();

        let keys = cache.comps_keys().unwrap();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"docker".to_string()));
        assert!(keys.contains(&"git".to_string()));
    }

    #[test]
    fn test_comps_prefix() {
        let mut cache = CompsysCache::memory().unwrap();

        let comps = vec![
            ("git".to_string(), "_git".to_string()),
            ("gitk".to_string(), "_gitk".to_string()),
            ("docker".to_string(), "_docker".to_string()),
        ];
        cache.set_comps_bulk(&comps).unwrap();

        let git_comps = cache.comps_prefix("git").unwrap();
        assert_eq!(git_comps.len(), 2);
    }

    #[test]
    fn test_zstyles_bulk() {
        let mut cache = CompsysCache::memory().unwrap();

        let styles = vec![
            (
                ":completion:*".to_string(),
                "menu".to_string(),
                vec!["select".to_string()],
                false,
            ),
            (
                ":completion:*".to_string(),
                "verbose".to_string(),
                vec!["yes".to_string()],
                false,
            ),
            (
                ":completion:*:descriptions".to_string(),
                "format".to_string(),
                vec!["%d".to_string()],
                false,
            ),
        ];
        cache.set_zstyles_bulk(&styles).unwrap();

        assert!(cache.has_zstyles().unwrap());
        assert_eq!(cache.zstyles_count().unwrap(), 3);
    }

    #[test]
    fn test_services() {
        let cache = CompsysCache::memory().unwrap();

        cache.set_service("git", "scm").unwrap();
        cache.set_service("hg", "scm").unwrap();

        assert_eq!(cache.get_service("git").unwrap(), Some("scm".to_string()));
        assert_eq!(cache.get_service("unknown").unwrap(), None);
    }

    #[test]
    fn test_cache_overwrite() {
        let cache = CompsysCache::memory().unwrap();

        cache.set_comp("git", "_git_old").unwrap();
        assert_eq!(cache.get_comp("git").unwrap(), Some("_git_old".to_string()));

        cache.set_comp("git", "_git_new").unwrap();
        assert_eq!(cache.get_comp("git").unwrap(), Some("_git_new".to_string()));
    }

    #[test]
    fn test_executable_names() {
        let mut cache = CompsysCache::memory().unwrap();

        let executables = vec![
            ("alpha".to_string(), "/bin/alpha".to_string()),
            ("beta".to_string(), "/bin/beta".to_string()),
            ("gamma".to_string(), "/bin/gamma".to_string()),
        ];
        cache.set_executables_bulk(&executables).unwrap();

        let names = cache.get_executable_names().unwrap();
        assert_eq!(names.len(), 3);
        // Returns a HashSet, so check contains
        assert!(names.contains("alpha"));
        assert!(names.contains("beta"));
        assert!(names.contains("gamma"));
    }
}
