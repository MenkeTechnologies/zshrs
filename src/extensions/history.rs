//! SQLite-backed command history for zshrs.
//!
//! **zshrs-original infrastructure with strong C-zsh ancestry.** C
//! zsh keeps history in a flat file (`Src/hist.c::savehistfile()`)
//! and an in-memory linked list of `Histent` entries. zshrs
//! replaces both with a SQLite database for two reasons: (1) FTS5
//! full-text search makes fzf-style fuzzy matching microsecond-
//! latency vs zsh's `O(N)` linear scan, (2) frequency / recency /
//! per-directory tracking can layer on top of the same row without
//! parallel files. The interactive surface (the `fc` builtin,
//! `$HISTFILE` semantics, `setopt SHARE_HISTORY` etc.) preserves
//! the C source's behavior — we just swap the storage backend.
//!
//! Features:
//! - Persistent history across sessions
//! - Frequency and recency tracking
//! - FTS5 full-text search for fzf-style matching
//! - Per-directory history context
//! - Deduplication with timestamp updates

use rusqlite::{params, Connection};
use std::io::Read;
use std::io::Write as _;
use std::io::Write;
use std::io::{Seek, SeekFrom};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// SQLite-backed history engine.
/// Replaces the in-memory `histent` doubly-linked list +
/// `histfile` flat-file pair from Src/hist.c — same logical
/// history but with FTS5 search, frequency tracking, and
/// per-directory context.
pub struct HistoryEngine {
    /// `conn` field.
    conn: Connection,
}

/// One history record.
/// Port of `struct histent` from Src/zsh.h (`text` / `stim` /
/// `ftim` fields) plus zshrs additions (`exit_code`, `cwd`,
/// `frequency`, `duration_ms`) the SQLite schema captures from
/// the `precmd`/`preexec` hooks.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    /// `id` field.
    pub id: i64,
    /// `command` field.
    pub command: String,
    /// `timestamp` field.
    pub timestamp: i64,
    /// `duration_ms` field.
    pub duration_ms: Option<i64>,
    /// `exit_code` field.
    pub exit_code: Option<i32>,
    /// `cwd` field.
    pub cwd: Option<String>,
    /// `frequency` field.
    pub frequency: u32,
}

impl HistoryEngine {
    /// `new` — see implementation.
    pub fn new() -> rusqlite::Result<Self> {
        let path = Self::db_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        // Open (and materialize the WAL/SHM side files) with the low fds held,
        // so sqlite's descriptors land above the script's range. sqlite caches
        // the fd NUMBER internally, so this cannot be fixed up after the open —
        // it has to land high in the first place. See crate::lowfd.
        let _lowfd = crate::lowfd::LowFdGuard::new();
        let conn = Connection::open(&path)?;
        crate::startup_trace::mark("hist: sqlite open");
        // c:Src/utils.c:2007-2010 — every descriptor the shell takes for
        // itself is recorded as `FDT_INTERNAL`. SQLite never hands back a
        // descriptor, so the registration cannot ride along with the open
        // the way `movefd` does; sweep for it instead.
        crate::lowfd::register_internal_fds();
        let engine = Self { conn };
        engine.init_schema()?;
        crate::startup_trace::mark("hist: init_schema");
        // The exact row count is a `SELECT COUNT(*)` — a full table scan that
        // costs ~37 ms on a 775k-row / 1.2 GB history and ran on EVERY shell
        // start purely to fill in this log line (measured with
        // `ZSHRS_STARTUP_TRACE=1`: it was the single largest item in
        // time-to-first-prompt). `MAX(rowid)` answers from the b-tree's
        // rightmost leaf instead — O(depth), and equal to the count until rows
        // are deleted, which is why it is reported as an ESTIMATE. Callers
        // that need the true number (the `--doctor` report, `fc -l` bounds)
        // still call `count()`.
        let est = engine.count_estimate().unwrap_or(0);
        crate::startup_trace::mark("hist: count estimate");
        let db_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        tracing::info!(
            entries_est = est,
            db_bytes = db_size,
            path = %path.display(),
            "history: sqlite opened"
        );

        // Rehydrate the flat text mirror from the sqlite index when
        // the text file is missing or stale (size 0 with a populated
        // db). Cheap: one-shot chronological dump, no FTS / no joins.
        crate::startup_trace::mark("hist: metadata + log");
        if let Err(e) = engine.rehydrate_text_if_stale() {
            tracing::warn!(?e, "history: failed to rehydrate text mirror; continuing");
        }
        crate::startup_trace::mark("hist: rehydrate_text_if_stale");
        Ok(engine)
    }
    /// `in_memory` — see implementation.
    pub fn in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let engine = Self { conn };
        engine.init_schema()?;
        Ok(engine)
    }

    /// `$ZSHRS_HOME/zshrs_history.db` — sqlite index that powers FTS5
    /// search, frequency tracking, dedup. Hidden under `.db` so the
    /// user-facing artifact is the flat-text mirror at
    /// `zshrs_history` (see `text_path`), zsh-compatible so muscle
    /// memory + `cat` / `grep` / external history tools all keep
    /// working.
    ///
    /// The daemon owns its OWN history db at `~/.zshrs/history.db`
    /// (different schema, daemon-only writer); shells append to it via
    /// `history_append` IPC. This shell-side db is the fallback path
    /// used when the daemon is absent.
    fn db_path() -> PathBuf {
        Self::root().join("zshrs_history.db")
    }

    /// `$ZSHRS_HOME/zshrs_history` — flat text mirror, one line per
    /// command in zsh extended-history format:
    ///
    /// ```text
    /// : <unix_ts>:<duration>;<command>
    /// ```
    ///
    /// Newlines inside multi-line commands are escaped as the literal
    /// two-character sequence `\\n` (matches `setopt EXTENDED_HISTORY`
    /// — `zsh/Src/hist.c:gethistent`). Every `add` appends one line;
    /// `update_last` rewrites the trailing line in place when the
    /// duration becomes known. The sqlite index at `zshrs_history.db`
    /// is the query-side mirror of this file — they're kept in lockstep
    /// by the writer, and a divergence-repair pass on open re-reads
    /// the text file if the sqlite is missing or older.
    pub fn text_path() -> PathBuf {
        Self::root().join("zshrs_history")
    }

    fn root() -> PathBuf {
        if let Some(custom) = std::env::var_os("ZSHRS_HOME") {
            PathBuf::from(custom)
        } else {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".zshrs")
        }
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

        let id = self.conn.last_insert_rowid();

        // Mirror to the flat zsh-extended-history file. Best-effort —
        // a write failure here doesn't fail the sqlite insert (e.g.
        // disk full mid-write should still let the shell record state
        // in the index). The duration is unknown at this point;
        // `update_last` rewrites the trailing line once it knows.
        if let Err(e) = append_text_line(now, 0, command) {
            tracing::warn!(?e, "history: text mirror append failed");
        }

        Ok(id)
    }

    /// Update the duration and exit code of the last command
    pub fn update_last(&self, id: i64, duration_ms: i64, exit_code: i32) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE history SET duration_ms = ?1, exit_code = ?2 WHERE id = ?3",
            params![duration_ms, exit_code, id],
        )?;

        // Update the trailing line of the text mirror with the now-known
        // duration. Look up the command by id so the rewrite stays
        // consistent even if `add` deduped to an earlier entry.
        if let Ok((ts, command)) = self.conn.query_row(
            "SELECT timestamp, command FROM history WHERE id = ?1",
            params![id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        ) {
            let duration_secs = (duration_ms / 1000).max(0);
            if let Err(e) = rewrite_last_text_line(ts, duration_secs, &command) {
                tracing::warn!(?e, "history: text mirror update failed");
            }
        }
        Ok(())
    }

    /// If the text mirror is missing or empty but the sqlite db has
    /// entries, dump the db chronologically into the text file. Used
    /// by `new()` after the first-time rename migration so users get
    /// the full backlog in the user-facing text file from day one.
    fn rehydrate_text_if_stale(&self) -> rusqlite::Result<()> {
        let text = Self::text_path();
        let text_size = std::fs::metadata(&text).map(|m| m.len()).unwrap_or(0);
        if text_size > 0 {
            return Ok(());
        }
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0))?;
        if count == 0 {
            return Ok(());
        }
        let mut stmt = self.conn.prepare(
            "SELECT timestamp, COALESCE(duration_ms, 0), command \
             FROM history ORDER BY timestamp ASC, id ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&text)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let mut w = std::io::BufWriter::new(file);
        let mut written: u64 = 0;
        for row in rows {
            let (ts, dur_ms, cmd) = row?;
            let line = format_text_line(ts, (dur_ms / 1000).max(0), &cmd);
            w.write_all(line.as_bytes())
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
            written += 1;
        }
        w.flush()
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        tracing::info!(
            entries = written,
            path = %text.display(),
            "history: rehydrated text mirror from sqlite index"
        );
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

    /// Case-SENSITIVE prefix probe for the per-keystroke ZLE paths
    /// (autosuggest, up-arrow search). Uses GLOB, which is case-sensitive and
    /// therefore eligible for the SQLite prefix optimization over
    /// `idx_history_command` (BINARY collation). `search_prefix`'s LIKE is
    /// case-insensitive by default and CANNOT use that index — at 500k+ rows
    /// it full-scans, which is far too slow to run on every keystroke.
    pub fn search_prefix_cs(
        &self,
        prefix: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<HistoryEntry>> {
        if prefix.is_empty() {
            return self.recent(limit);
        }

        let mut stmt = self.conn.prepare_cached(
            r#"SELECT id, command, timestamp, duration_ms, exit_code, cwd, frequency
               FROM history
               WHERE command GLOB ?1
               ORDER BY timestamp DESC
               LIMIT ?2"#,
        )?;

        // Escape GLOB metachars via character classes ('[' first — the
        // replacement brackets must not themselves get re-escaped).
        let escaped = prefix
            .replace('[', "[[]")
            .replace('*', "[*]")
            .replace('?', "[?]");
        let pattern = format!("{escaped}*");

        let entries = stmt.query_map(params![pattern, limit as i64], |row| {
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

    /// Row-count ESTIMATE that does not scan the table.
    ///
    /// `MAX(rowid)` is resolved by descending to the rightmost leaf of the
    /// rowid b-tree, so it is independent of table size where `COUNT(*)` is
    /// linear in it (37 ms at 775k rows). It equals the true count for an
    /// append-only history and over-reports by the number of deleted rows
    /// otherwise — good enough for a log line, never used for arithmetic on
    /// history events.
    pub fn count_estimate(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT IFNULL(MAX(rowid), 0) FROM history", [], |row| {
                row.get(0)
            })
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

// ---------------------------------------------------------------------
// Interactive-session sink — the SQLite index is zshrs's DEFAULT
// history store; the flat $HISTFILE path in ported hist.rs stays fully
// functional for users who override via HISTFILE/SAVEHIST. `hend()`
// (src/ported/hist.rs) calls `history_sqlite_add` for every accepted
// interactive line (same accept policy as the $HISTFILE write:
// HIST_TMPSTORE / HIST_NOWRITE excluded); `preprompt()`
// (src/ported/utils.rs) calls `history_sqlite_finish` right after
// execode returns to stamp duration + exit status — mirroring what the
// `-c` path does via ShellExecutor.history at bins/zshrs.rs
// (engine.add → update_last). Thread-local because
// rusqlite::Connection is !Sync; the interactive loop is
// single-threaded on the main thread.
// ---------------------------------------------------------------------

thread_local! {
    /// Lazily-opened engine for the interactive loop. Outer Option =
    /// "tried to open yet?", inner Option = open result (None when the
    /// db can't be opened — sink becomes a no-op, never an error).
    static SESSION_HISTORY: std::cell::RefCell<Option<Option<HistoryEngine>>> =
        const { std::cell::RefCell::new(None) };
    /// Row id + start instant of the line whose command is currently
    /// executing, armed by `history_sqlite_add`, consumed by
    /// `history_sqlite_finish`.
    static HIST_PENDING: std::cell::Cell<Option<(i64, std::time::Instant)>> =
        const { std::cell::Cell::new(None) };
}

pub fn with_session_engine<R>(f: impl FnOnce(&HistoryEngine) -> R) -> Option<R> {
    SESSION_HISTORY.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(HistoryEngine::new().ok());
        }
        slot.as_ref().unwrap().as_ref().map(f)
    })
}

/// Append an accepted interactive line to the SQLite history index
/// (+ text mirror) and arm the pending row for `history_sqlite_finish`.
pub fn history_sqlite_add(line: &str) {
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string());
    let id = with_session_engine(|e| e.add(line, cwd.as_deref()).ok()).flatten();
    if let Some(id) = id {
        HIST_PENDING.with(|p| p.set(Some((id, std::time::Instant::now()))));
    }
}

/// Stamp duration + exit status onto the row `history_sqlite_add`
/// armed. No-op when nothing is pending.
pub fn history_sqlite_finish(exit_code: i32) {
    let Some((id, start)) = HIST_PENDING.with(|p| p.take()) else {
        return;
    };
    let dur = start.elapsed().as_millis() as i64;
    let _ = with_session_engine(|e| e.update_last(id, dur, exit_code));
}

/// Detect whether `path` is a sqlite database by sniffing the magic
/// header (first 16 bytes start with `SQLite format 3\0`). Errors /
/// short files / unknown content all return false (safe default —
/// leave unknown content alone).
fn is_sqlite_file(path: &std::path::Path) -> bool {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut header = [0u8; 16];
    if f.read_exact(&mut header).is_err() {
        return false;
    }
    &header == b"SQLite format 3\0"
}

/// Format one zsh-extended-history line:
///
/// ```text
/// : <unix_ts>:<duration>;<command>\n
/// ```
///
/// Multi-line commands escape literal `\n` to the two-character
/// sequence `\\n` so each entry stays on a single line; the unescape
/// is the inverse done at read time. Matches what zsh writes when
/// `EXTENDED_HISTORY` is set (zsh/Src/hist.c:savehistfile).
fn format_text_line(ts: i64, duration_secs: i64, command: &str) -> String {
    let escaped = command.replace('\\', "\\\\").replace('\n', "\\\n");
    format!(": {}:{};{}\n", ts, duration_secs, escaped)
}

/// Append one line to `$ZSHRS_HOME/zshrs_history`.
fn append_text_line(ts: i64, duration_secs: i64, command: &str) -> std::io::Result<()> {
    let path = HistoryEngine::text_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let line = format_text_line(ts, duration_secs, command);
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    f.write_all(line.as_bytes())
}

/// Rewrite the trailing entry of the text file in place — used by
/// `update_last` once the duration is known. Strategy: read the file
/// to the last newline-delimited record, replace it with a freshly
/// formatted line. For multi-MB history files we only buffer the
/// trailing record's tail bytes (`max_tail` cap) — anything older
/// stays untouched on disk.
fn rewrite_last_text_line(ts: i64, duration_secs: i64, command: &str) -> std::io::Result<()> {
    let path = HistoryEngine::text_path();
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)?;
    let len = f.metadata()?.len();
    // 64 KiB is enough for any realistic single-command record (zsh
    // commands top out at ~1-4 KiB). Beyond that, give up and append
    // a corrected line rather than risk truncating the file.
    let max_tail = 65_536u64.min(len);
    let read_from = len - max_tail;
    f.seek(SeekFrom::Start(read_from))?;
    let mut tail = Vec::with_capacity(max_tail as usize);
    f.read_to_end(&mut tail)?;
    // Find the offset (within `tail`) where the last record begins.
    // A record begins at the byte AFTER the second-to-last newline,
    // or at offset 0 if there is none.
    let mut last_record_start = 0usize;
    let mut nl_count = 0;
    for (i, b) in tail.iter().enumerate().rev() {
        if *b == b'\n' {
            nl_count += 1;
            if nl_count == 2 {
                last_record_start = i + 1;
                break;
            }
        }
    }
    let new_record = format_text_line(ts, duration_secs, command);
    let new_abs = read_from + last_record_start as u64;
    f.seek(SeekFrom::Start(new_abs))?;
    f.write_all(new_record.as_bytes())?;
    let new_len = new_abs + new_record.len() as u64;
    if new_len < len {
        f.set_len(new_len)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_search() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        let engine = HistoryEngine::in_memory().unwrap();

        engine.add("git status", None).unwrap();
        engine.add("git commit -m 'test'", None).unwrap();
        engine.add("grep foo bar", None).unwrap();

        let results = engine.search_prefix("git", 10).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_directory_history() {
        let _g = crate::test_util::global_state_lock();
        let engine = HistoryEngine::in_memory().unwrap();

        engine.add("make build", Some("/project")).unwrap();
        engine.add("cargo test", Some("/project")).unwrap();
        engine.add("ls", Some("/tmp")).unwrap();

        let results = engine.for_directory("/project", 10).unwrap();
        assert_eq!(results.len(), 2);
    }

    // ========================================================
    // format_text_line — zsh EXTENDED_HISTORY line emission
    // ========================================================

    #[test]
    fn format_text_line_emits_canonical_prefix() {
        let line = format_text_line(1_700_000_000, 5, "echo hi");
        assert_eq!(line, ": 1700000000:5;echo hi\n");
    }

    #[test]
    fn format_text_line_zero_duration_still_renders() {
        let line = format_text_line(0, 0, "true");
        assert_eq!(line, ": 0:0;true\n");
    }

    #[test]
    fn format_text_line_escapes_literal_backslash() {
        // `\` doubles so the inverse unescape recovers the original.
        let line = format_text_line(1, 0, r"\n");
        // Source `\n` → escaped `\\n`.
        assert!(
            line.ends_with(";\\\\n\n"),
            "expected backslash-escape, got: {:?}",
            line
        );
    }

    #[test]
    fn format_text_line_escapes_embedded_newline_to_backslash_newline() {
        // Multi-line commands escape literal newlines so each
        // logical entry stays on one disk line for read-back.
        let line = format_text_line(2, 1, "line1\nline2");
        // After replacement: `\\\n` = backslash then newline.
        assert!(
            line.contains("line1\\\nline2"),
            "expected escaped newline, got: {:?}",
            line
        );
        // Still terminates with one trailing real newline.
        assert!(line.ends_with('\n'));
    }

    #[test]
    fn format_text_line_handles_empty_command() {
        let line = format_text_line(42, 7, "");
        assert_eq!(line, ": 42:7;\n");
    }

    #[test]
    fn format_text_line_negative_duration_round_trips() {
        // Defensive: negative durations are unusual but represent
        // "duration unknown" sentinels in some pre-commit paths.
        let line = format_text_line(100, -1, "foo");
        assert_eq!(line, ": 100:-1;foo\n");
    }

    #[test]
    fn format_text_line_only_terminating_newline_present() {
        // Exactly one trailing `\n` regardless of command bytes.
        let line = format_text_line(1, 1, "abc");
        let nls = line.matches('\n').count();
        assert_eq!(nls, 1);
    }

    // ========================================================
    // is_sqlite_file — magic-header sniff
    // ========================================================

    #[test]
    fn is_sqlite_file_true_for_real_header() {
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir().join("zshrs_history_sqlite_magic.bin");
        std::fs::write(&tmp, b"SQLite format 3\0extra junk after").unwrap();
        assert!(is_sqlite_file(&tmp));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn is_sqlite_file_false_for_plain_text() {
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir().join("zshrs_history_text_not_db.txt");
        std::fs::write(&tmp, b": 1700000000:5;ls -la\n").unwrap();
        assert!(!is_sqlite_file(&tmp));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn is_sqlite_file_false_for_short_file() {
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir().join("zshrs_history_short.bin");
        std::fs::write(&tmp, b"abc").unwrap();
        // Less than 16 bytes — `read_exact` fails → false.
        assert!(!is_sqlite_file(&tmp));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn is_sqlite_file_false_for_missing_path() {
        let _g = crate::test_util::global_state_lock();
        assert!(!is_sqlite_file(std::path::Path::new(
            "/nonexistent/zshrs/sqlite/magic.db"
        )));
    }

    #[test]
    fn is_sqlite_file_false_for_almost_matching_header() {
        // 16 bytes but not the magic — must reject.
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir().join("zshrs_history_fake_magic.bin");
        std::fs::write(&tmp, b"SQLite format 4\0").unwrap();
        assert!(!is_sqlite_file(&tmp));
        let _ = std::fs::remove_file(&tmp);
    }

    // ========================================================
    // HistoryEntry / engine — semantic coverage
    // ========================================================

    #[test]
    fn search_prefix_respects_limit() {
        let _g = crate::test_util::global_state_lock();
        let engine = HistoryEngine::in_memory().unwrap();
        for n in 0..5 {
            engine.add(&format!("git-{}", n), None).unwrap();
        }
        let results = engine.search_prefix("git-", 2).unwrap();
        assert!(results.len() <= 2, "limit not honored: {}", results.len());
    }
}
