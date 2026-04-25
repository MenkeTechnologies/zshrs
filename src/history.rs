//! SQLite-backed command history for zshrs
//!
//! Features:
//! - Persistent history across sessions
//! - Frequency and recency tracking
//! - FTS5 full-text search for fzf-style matching
//! - Per-directory history context
//! - Deduplication with timestamp updates

use rusqlite::{params, Connection};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct HistoryEngine {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub id: i64,
    pub command: String,
    pub timestamp: i64,
    pub duration_ms: Option<i64>,
    pub exit_code: Option<i32>,
    pub cwd: Option<String>,
    pub frequency: u32,
}

impl HistoryEngine {
    pub fn new() -> rusqlite::Result<Self> {
        let path = Self::db_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let conn = Connection::open(&path)?;
        let engine = Self { conn };
        engine.init_schema()?;
        let count = engine.count().unwrap_or(0);
        let db_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        tracing::info!(
            entries = count,
            db_bytes = db_size,
            path = %path.display(),
            "history: sqlite opened"
        );
        Ok(engine)
    }

    pub fn in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let engine = Self { conn };
        engine.init_schema()?;
        Ok(engine)
    }

    fn db_path() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("zshrs")
            .join("history.db")
    }

    fn init_schema(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(r#"
            CREATE TABLE IF NOT EXISTS history (
                id INTEGER PRIMARY KEY,
                command TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                duration_ms INTEGER,
                exit_code INTEGER,
                cwd TEXT,
                frequency INTEGER DEFAULT 1
            );

            CREATE INDEX IF NOT EXISTS idx_history_timestamp ON history(timestamp DESC);
            CREATE INDEX IF NOT EXISTS idx_history_cwd ON history(cwd);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_history_command ON history(command);

            CREATE VIRTUAL TABLE IF NOT EXISTS history_fts USING fts5(
                command,
                content='history',
                content_rowid='id',
                tokenize='trigram'
            );

            CREATE TRIGGER IF NOT EXISTS history_ai AFTER INSERT ON history BEGIN
                INSERT INTO history_fts(rowid, command) VALUES (new.id, new.command);
            END;

            CREATE TRIGGER IF NOT EXISTS history_ad AFTER DELETE ON history BEGIN
                INSERT INTO history_fts(history_fts, rowid, command) VALUES('delete', old.id, old.command);
            END;

            CREATE TRIGGER IF NOT EXISTS history_au AFTER UPDATE ON history BEGIN
                INSERT INTO history_fts(history_fts, rowid, command) VALUES('delete', old.id, old.command);
                INSERT INTO history_fts(rowid, command) VALUES (new.id, new.command);
            END;
        "#)?;
        Ok(())
    }

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    /// Add a command to history, updating frequency if it already exists
    pub fn add(&self, command: &str, cwd: Option<&str>) -> rusqlite::Result<i64> {
        let command = command.trim();
        if command.is_empty() || command.starts_with(' ') {
            return Ok(0);
        }

        let now = Self::now();

        // Try to update existing entry
        let updated = self.conn.execute(
            "UPDATE history SET timestamp = ?1, frequency = frequency + 1, cwd = COALESCE(?2, cwd)
             WHERE command = ?3",
            params![now, cwd, command],
        )?;

        if updated > 0 {
            // Return the existing ID
            let id: i64 = self.conn.query_row(
                "SELECT id FROM history WHERE command = ?1",
                params![command],
                |row| row.get(0),
            )?;
            return Ok(id);
        }

        // Insert new entry
        self.conn.execute(
            "INSERT INTO history (command, timestamp, cwd) VALUES (?1, ?2, ?3)",
            params![command, now, cwd],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    /// Update the duration and exit code of the last command
    pub fn update_last(&self, id: i64, duration_ms: i64, exit_code: i32) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE history SET duration_ms = ?1, exit_code = ?2 WHERE id = ?3",
            params![duration_ms, exit_code, id],
        )?;
        Ok(())
    }

    /// Search history with FTS5 (fuzzy/substring matching)
    pub fn search(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<HistoryEntry>> {
        if query.is_empty() {
            return self.recent(limit);
        }

        // Escape special FTS5 characters and use prefix matching
        let escaped = query.replace('"', "\"\"");
        let fts_query = format!("\"{}\"*", escaped);

        let mut stmt = self.conn.prepare(
            r#"SELECT h.id, h.command, h.timestamp, h.duration_ms, h.exit_code, h.cwd, h.frequency
               FROM history h
               JOIN history_fts f ON h.id = f.rowid
               WHERE history_fts MATCH ?1
               ORDER BY h.frequency DESC, h.timestamp DESC
               LIMIT ?2"#,
        )?;

        let entries = stmt.query_map(params![fts_query, limit as i64], |row| {
            Ok(HistoryEntry {
                id: row.get(0)?,
                command: row.get(1)?,
                timestamp: row.get(2)?,
                duration_ms: row.get(3)?,
                exit_code: row.get(4)?,
                cwd: row.get(5)?,
                frequency: row.get(6)?,
            })
        })?;

        entries.collect()
    }

    /// Search history with prefix matching (for up-arrow completion)
    pub fn search_prefix(&self, prefix: &str, limit: usize) -> rusqlite::Result<Vec<HistoryEntry>> {
        if prefix.is_empty() {
            return self.recent(limit);
        }

        let mut stmt = self.conn.prepare(
            r#"SELECT id, command, timestamp, duration_ms, exit_code, cwd, frequency
               FROM history
               WHERE command LIKE ?1 || '%' ESCAPE '\'
               ORDER BY timestamp DESC
               LIMIT ?2"#,
        )?;

        // Escape SQL LIKE special chars
        let escaped = prefix
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_");

        let entries = stmt.query_map(params![escaped, limit as i64], |row| {
            Ok(HistoryEntry {
                id: row.get(0)?,
                command: row.get(1)?,
                timestamp: row.get(2)?,
                duration_ms: row.get(3)?,
                exit_code: row.get(4)?,
                cwd: row.get(5)?,
                frequency: row.get(6)?,
            })
        })?;

        entries.collect()
    }

    /// Get recent history entries
    pub fn recent(&self, limit: usize) -> rusqlite::Result<Vec<HistoryEntry>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, command, timestamp, duration_ms, exit_code, cwd, frequency
               FROM history
               ORDER BY timestamp DESC
               LIMIT ?1"#,
        )?;

        let entries = stmt.query_map(params![limit as i64], |row| {
            Ok(HistoryEntry {
                id: row.get(0)?,
                command: row.get(1)?,
                timestamp: row.get(2)?,
                duration_ms: row.get(3)?,
                exit_code: row.get(4)?,
                cwd: row.get(5)?,
                frequency: row.get(6)?,
            })
        })?;

        entries.collect()
    }

    /// Get history for a specific directory
    pub fn for_directory(&self, cwd: &str, limit: usize) -> rusqlite::Result<Vec<HistoryEntry>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, command, timestamp, duration_ms, exit_code, cwd, frequency
               FROM history
               WHERE cwd = ?1
               ORDER BY frequency DESC, timestamp DESC
               LIMIT ?2"#,
        )?;

        let entries = stmt.query_map(params![cwd, limit as i64], |row| {
            Ok(HistoryEntry {
                id: row.get(0)?,
                command: row.get(1)?,
                timestamp: row.get(2)?,
                duration_ms: row.get(3)?,
                exit_code: row.get(4)?,
                cwd: row.get(5)?,
                frequency: row.get(6)?,
            })
        })?;

        entries.collect()
    }

    /// Delete a history entry
    pub fn delete(&self, id: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM history WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Clear all history
    pub fn clear(&self) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM history", [])?;
        Ok(())
    }

    /// Get total history count
    pub fn count(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM history", [], |row| row.get(0))
    }

    /// Get entry by index from end (0 = most recent, like !-1)
    pub fn get_by_offset(&self, offset: usize) -> rusqlite::Result<Option<HistoryEntry>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, command, timestamp, duration_ms, exit_code, cwd, frequency
               FROM history
               ORDER BY timestamp DESC
               LIMIT 1 OFFSET ?1"#,
        )?;

        let mut rows = stmt.query(params![offset as i64])?;
        if let Some(row) = rows.next()? {
            Ok(Some(HistoryEntry {
                id: row.get(0)?,
                command: row.get(1)?,
                timestamp: row.get(2)?,
                duration_ms: row.get(3)?,
                exit_code: row.get(4)?,
                cwd: row.get(5)?,
                frequency: row.get(6)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Get entry by absolute history number (like !123)
    pub fn get_by_number(&self, num: i64) -> rusqlite::Result<Option<HistoryEntry>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT id, command, timestamp, duration_ms, exit_code, cwd, frequency
               FROM history
               WHERE id = ?1"#,
        )?;

        let mut rows = stmt.query(params![num])?;
        if let Some(row) = rows.next()? {
            Ok(Some(HistoryEntry {
                id: row.get(0)?,
                command: row.get(1)?,
                timestamp: row.get(2)?,
                duration_ms: row.get(3)?,
                exit_code: row.get(4)?,
                cwd: row.get(5)?,
                frequency: row.get(6)?,
            }))
        } else {
            Ok(None)
        }
    }
}

/// Reedline history adapter
pub struct ReedlineHistory {
    engine: HistoryEngine,
    session_history: Vec<String>,
    cursor: usize,
}

impl ReedlineHistory {
    pub fn new() -> rusqlite::Result<Self> {
        Ok(Self {
            engine: HistoryEngine::new()?,
            session_history: Vec::new(),
            cursor: 0,
        })
    }

    pub fn add(&mut self, command: &str) -> rusqlite::Result<i64> {
        self.session_history.push(command.to_string());
        self.cursor = self.session_history.len();
        let cwd = std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().to_string());
        self.engine.add(command, cwd.as_deref())
    }

    pub fn search(&self, query: &str) -> Vec<String> {
        self.engine
            .search(query, 50)
            .unwrap_or_default()
            .into_iter()
            .map(|e| e.command)
            .collect()
    }

    pub fn previous(&mut self, prefix: &str) -> Option<String> {
        if self.cursor == 0 {
            return None;
        }

        // Search backwards in session history first
        for i in (0..self.cursor).rev() {
            if self.session_history[i].starts_with(prefix) {
                self.cursor = i;
                return Some(self.session_history[i].clone());
            }
        }

        // Fall back to database
        self.engine
            .search_prefix(prefix, 1)
            .ok()
            .and_then(|v| v.into_iter().next())
            .map(|e| e.command)
    }

    pub fn next(&mut self, prefix: &str) -> Option<String> {
        if self.cursor >= self.session_history.len() {
            return None;
        }

        for i in (self.cursor + 1)..self.session_history.len() {
            if self.session_history[i].starts_with(prefix) {
                self.cursor = i;
                return Some(self.session_history[i].clone());
            }
        }

        self.cursor = self.session_history.len();
        None
    }

    pub fn reset_cursor(&mut self) {
        self.cursor = self.session_history.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_search() {
        let engine = HistoryEngine::in_memory().unwrap();

        engine.add("ls -la", Some("/home/user")).unwrap();
        engine.add("cd /tmp", Some("/home/user")).unwrap();
        engine.add("echo hello", Some("/tmp")).unwrap();

        // Use prefix search for short queries (trigram FTS5 needs 3+ chars)
        let results = engine.search_prefix("ls", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].command, "ls -la");
    }

    #[test]
    fn test_frequency_tracking() {
        let engine = HistoryEngine::in_memory().unwrap();

        engine.add("git status", None).unwrap();
        engine.add("git status", None).unwrap();
        engine.add("git status", None).unwrap();

        let results = engine.recent(10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].frequency, 3);
    }

    #[test]
    fn test_prefix_search() {
        let engine = HistoryEngine::in_memory().unwrap();

        engine.add("git status", None).unwrap();
        engine.add("git commit -m 'test'", None).unwrap();
        engine.add("grep foo bar", None).unwrap();

        let results = engine.search_prefix("git", 10).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_directory_history() {
        let engine = HistoryEngine::in_memory().unwrap();

        engine.add("make build", Some("/project")).unwrap();
        engine.add("cargo test", Some("/project")).unwrap();
        engine.add("ls", Some("/tmp")).unwrap();

        let results = engine.for_directory("/project", 10).unwrap();
        assert_eq!(results.len(), 2);
    }
}
