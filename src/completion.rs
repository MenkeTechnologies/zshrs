//! SQLite-backed completion engine for zshrs
//!
//! Features:
//! - FTS5 full-text search for instant fuzzy matching
//! - Frequency tracking from command history
//! - Persistent index survives shell restarts
//! - Sub-millisecond queries on 40k+ completions

use rusqlite::{params, Connection};
use std::path::PathBuf;

pub struct CompletionEngine {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct Completion {
    pub name: String,
    pub kind: CompletionKind,
    pub description: Option<String>,
    pub frequency: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    Command,
    Builtin,
    Function,
    Alias,
    File,
    Directory,
    Variable,
    Option,
}

impl CompletionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Builtin => "builtin",
            Self::Function => "function",
            Self::Alias => "alias",
            Self::File => "file",
            Self::Directory => "directory",
            Self::Variable => "variable",
            Self::Option => "option",
        }
    }

    fn from_str(s: &str) -> Self {
        match s {
            "command" => Self::Command,
            "builtin" => Self::Builtin,
            "function" => Self::Function,
            "alias" => Self::Alias,
            "file" => Self::File,
            "directory" => Self::Directory,
            "variable" => Self::Variable,
            "option" => Self::Option,
            _ => Self::Command,
        }
    }
}

impl CompletionEngine {
    pub fn new() -> rusqlite::Result<Self> {
        let db_path = Self::db_path();
        std::fs::create_dir_all(db_path.parent().unwrap()).ok();
        let conn = Connection::open(&db_path)?;

        let engine = Self { conn };
        engine.init_schema()?;
        Ok(engine)
    }

    pub fn in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let engine = Self { conn };
        engine.init_schema()?;
        Ok(engine)
    }

    fn db_path() -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("zshrs")
            .join("completions.db")
    }

    fn init_schema(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS completions (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                description TEXT,
                frequency INTEGER DEFAULT 0,
                UNIQUE(name, kind)
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS completions_fts USING fts5(
                name,
                description,
                content='completions',
                content_rowid='id'
            );

            CREATE TRIGGER IF NOT EXISTS completions_ai AFTER INSERT ON completions BEGIN
                INSERT INTO completions_fts(rowid, name, description)
                VALUES (new.id, new.name, new.description);
            END;

            CREATE TRIGGER IF NOT EXISTS completions_ad AFTER DELETE ON completions BEGIN
                INSERT INTO completions_fts(completions_fts, rowid, name, description)
                VALUES ('delete', old.id, old.name, old.description);
            END;

            CREATE TRIGGER IF NOT EXISTS completions_au AFTER UPDATE ON completions BEGIN
                INSERT INTO completions_fts(completions_fts, rowid, name, description)
                VALUES ('delete', old.id, old.name, old.description);
                INSERT INTO completions_fts(rowid, name, description)
                VALUES (new.id, new.name, new.description);
            END;

            CREATE INDEX IF NOT EXISTS idx_completions_name ON completions(name);
            CREATE INDEX IF NOT EXISTS idx_completions_kind ON completions(kind);
            CREATE INDEX IF NOT EXISTS idx_completions_frequency ON completions(frequency DESC);
        "#,
        )?;
        Ok(())
    }

    pub fn add_completion(
        &self,
        name: &str,
        kind: CompletionKind,
        description: Option<&str>,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO completions (name, kind, description) VALUES (?1, ?2, ?3)",
            params![name, kind.as_str(), description],
        )?;
        Ok(())
    }

    pub fn add_completions(
        &self,
        completions: &[(String, CompletionKind, Option<String>)],
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = self.conn.prepare(
                "INSERT OR IGNORE INTO completions (name, kind, description) VALUES (?1, ?2, ?3)",
            )?;
            for (name, kind, desc) in completions {
                stmt.execute(params![name, kind.as_str(), desc.as_deref()])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn increment_frequency(&self, name: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE completions SET frequency = frequency + 1 WHERE name = ?1",
            params![name],
        )?;
        Ok(())
    }

    pub fn search(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<Completion>> {
        if query.is_empty() {
            return self.get_top_by_frequency(limit);
        }

        // Try prefix match first (faster)
        let prefix_results = self.search_prefix(query, limit)?;
        if prefix_results.len() >= limit {
            return Ok(prefix_results);
        }

        // Fall back to FTS5 fuzzy search
        self.search_fts(query, limit)
    }

    fn search_prefix(&self, prefix: &str, limit: usize) -> rusqlite::Result<Vec<Completion>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, kind, description, frequency FROM completions 
             WHERE name LIKE ?1 || '%'
             ORDER BY frequency DESC, name ASC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![prefix, limit as i64], |row| {
            Ok(Completion {
                name: row.get(0)?,
                kind: CompletionKind::from_str(&row.get::<_, String>(1)?),
                description: row.get(2)?,
                frequency: row.get(3)?,
            })
        })?;

        rows.collect()
    }

    fn search_fts(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<Completion>> {
        let fts_query = format!("{}*", query);
        let mut stmt = self.conn.prepare(
            "SELECT c.name, c.kind, c.description, c.frequency 
             FROM completions c
             JOIN completions_fts fts ON c.id = fts.rowid
             WHERE completions_fts MATCH ?1
             ORDER BY c.frequency DESC, rank
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![fts_query, limit as i64], |row| {
            Ok(Completion {
                name: row.get(0)?,
                kind: CompletionKind::from_str(&row.get::<_, String>(1)?),
                description: row.get(2)?,
                frequency: row.get(3)?,
            })
        })?;

        rows.collect()
    }

    fn get_top_by_frequency(&self, limit: usize) -> rusqlite::Result<Vec<Completion>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, kind, description, frequency FROM completions 
             ORDER BY frequency DESC, name ASC
             LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(Completion {
                name: row.get(0)?,
                kind: CompletionKind::from_str(&row.get::<_, String>(1)?),
                description: row.get(2)?,
                frequency: row.get(3)?,
            })
        })?;

        rows.collect()
    }

    pub fn count(&self) -> rusqlite::Result<usize> {
        self.conn
            .query_row("SELECT COUNT(*) FROM completions", [], |row| row.get(0))
    }

    pub fn index_system_commands(&self) -> rusqlite::Result<usize> {
        let path = std::env::var("PATH").unwrap_or_default();
        let mut completions = Vec::new();

        for dir in path.split(':') {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    if let Ok(ft) = entry.file_type() {
                        if ft.is_file() || ft.is_symlink() {
                            if let Some(name) = entry.file_name().to_str() {
                                completions.push((name.to_string(), CompletionKind::Command, None));
                            }
                        }
                    }
                }
            }
        }

        let count = completions.len();
        self.add_completions(&completions)?;
        Ok(count)
    }

    pub fn index_shell_builtins(&self) -> rusqlite::Result<usize> {
        let builtins = [
            ("cd", "Change directory"),
            ("pwd", "Print working directory"),
            ("echo", "Print arguments"),
            ("export", "Set environment variable"),
            ("unset", "Unset environment variable"),
            ("alias", "Define alias"),
            ("unalias", "Remove alias"),
            ("source", "Execute file in current shell"),
            ("exit", "Exit the shell"),
            ("jobs", "List background jobs"),
            ("fg", "Bring job to foreground"),
            ("bg", "Continue job in background"),
            ("history", "Show command history"),
            ("set", "Set shell options"),
            ("unset", "Unset shell options"),
            ("type", "Show command type"),
            ("which", "Show command path"),
            ("builtin", "Execute builtin command"),
            ("command", "Execute external command"),
            ("exec", "Replace shell with command"),
            ("eval", "Evaluate arguments as command"),
            ("read", "Read input"),
            ("printf", "Formatted print"),
            ("test", "Evaluate conditional expression"),
            ("true", "Return success"),
            ("false", "Return failure"),
            (":", "Null command"),
            ("return", "Return from function"),
            ("break", "Break from loop"),
            ("continue", "Continue loop"),
            ("shift", "Shift positional parameters"),
            ("wait", "Wait for background jobs"),
            ("trap", "Set signal handler"),
            ("umask", "Set file creation mask"),
            ("ulimit", "Set resource limits"),
            ("times", "Show shell times"),
            ("kill", "Send signal to process"),
            ("let", "Evaluate arithmetic expression"),
            ("declare", "Declare variable"),
            ("local", "Declare local variable"),
            ("readonly", "Make variable readonly"),
            ("typeset", "Declare variable type"),
            ("hash", "Remember command path"),
            ("dirs", "Show directory stack"),
            ("pushd", "Push directory"),
            ("popd", "Pop directory"),
            ("getopts", "Parse options"),
            ("enable", "Enable/disable builtins"),
            ("logout", "Exit login shell"),
            ("suspend", "Suspend shell"),
            ("disown", "Remove job from table"),
        ];

        let completions: Vec<_> = builtins
            .iter()
            .map(|(name, desc)| {
                (
                    name.to_string(),
                    CompletionKind::Builtin,
                    Some(desc.to_string()),
                )
            })
            .collect();

        let count = completions.len();
        self.add_completions(&completions)?;
        Ok(count)
    }
}

impl Default for CompletionEngine {
    fn default() -> Self {
        Self::new().expect("Failed to create completion engine")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_completion_engine() {
        let engine = CompletionEngine::in_memory().unwrap();

        engine
            .add_completion("git", CompletionKind::Command, Some("Version control"))
            .unwrap();
        engine
            .add_completion("grep", CompletionKind::Command, Some("Search text"))
            .unwrap();
        engine
            .add_completion("gzip", CompletionKind::Command, Some("Compress files"))
            .unwrap();

        let results = engine.search("g", 10).unwrap();
        assert_eq!(results.len(), 3);

        let results = engine.search("gi", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "git");
    }

    #[test]
    fn test_frequency_ranking() {
        let engine = CompletionEngine::in_memory().unwrap();

        engine
            .add_completion("aaa", CompletionKind::Command, None)
            .unwrap();
        engine
            .add_completion("aab", CompletionKind::Command, None)
            .unwrap();

        // Increment aab frequency
        for _ in 0..5 {
            engine.increment_frequency("aab").unwrap();
        }

        let results = engine.search("aa", 10).unwrap();
        assert_eq!(results[0].name, "aab"); // Higher frequency first
    }
}
