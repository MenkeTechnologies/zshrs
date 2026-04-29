// catalog.db — daemon-owned SQLite mirror of the rkyv corpus.
//
// Per docs/DAEMON.md "catalog.db schema (daemon-only writer)":
//
//   plugins        (name, version, source, installed_at, enabled)
//   plugin_deps    (plugin, dep, constraint)
//   entries        (fq_name, plugin_id, kind, image_path, byte_offset, source_loc, bytecode BLOB)
//   hooks          (kind, name, fq_name)
//   entry_stats    (fq_name, last_called_at, call_count, total_ns)
//   compiled_files (path PRIMARY KEY, kind, mtime, inode, hash, bytecode BLOB,
//                   last_used_at, use_count, bytes_in, bytes_out, sensitive,
//                   parent_paths)
//
// Foundation v1 just creates the schema. Hydration of entries/compiled_files happens
// later when the parse + bytecode-compile pipelines land. catalog.db starts empty
// after the daemon's first boot.
//
// PRAGMA journal_mode=WAL so client read-only queries (rare; mostly dbview) don't
// block the daemon's writer.

use std::path::Path;

use rusqlite::Connection;

use super::{paths::CachePaths, Result};

/// Schema version stamped into `PRAGMA user_version`. Bumped on incompatible changes;
/// migrations live in `migrate_to_current` (only one version for now).
pub const SCHEMA_VERSION: i64 = 1;

/// Open (or create) catalog.db, set WAL mode + foreign keys, and ensure the
/// schema is at SCHEMA_VERSION.
pub fn open(paths: &CachePaths) -> Result<Connection> {
    open_at(&paths.catalog_db)
}

/// Variant for tests/imports that target a path other than the daemon's canonical one.
pub fn open_at(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    super::paths::ensure_file_600(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;

    let current_version: i64 =
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?;

    if current_version == 0 {
        create_schema(&conn)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    } else if current_version != SCHEMA_VERSION {
        // Future: real migration step. For v1, only version 1 exists.
        return Err(super::DaemonError::other(format!(
            "catalog.db schema version mismatch: file is {}, daemon expects {}",
            current_version, SCHEMA_VERSION
        )));
    }

    Ok(conn)
}

fn create_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS plugins (
            name          TEXT PRIMARY KEY,
            version       TEXT,
            source        TEXT,            -- 'zinit' / 'omz' / 'antigen' / 'manual' / 'system' / etc.
            installed_at  INTEGER,         -- ns since epoch
            enabled       INTEGER NOT NULL DEFAULT 1
        );

        CREATE TABLE IF NOT EXISTS plugin_deps (
            plugin     TEXT NOT NULL,
            dep        TEXT NOT NULL,
            constraint_  TEXT,             -- semver/glob constraint expression
            PRIMARY KEY (plugin, dep),
            FOREIGN KEY (plugin) REFERENCES plugins(name) ON DELETE CASCADE
        );

        -- One row per cacheable named entity (function, completion handler, etc.).
        CREATE TABLE IF NOT EXISTS entries (
            fq_name        TEXT PRIMARY KEY,
            plugin_id      TEXT,           -- shard slug, '' for system / orphan
            kind           TEXT NOT NULL,  -- 'function' / 'completion' / 'autoload' / 'hook' / 'alias'
            image_path     TEXT,           -- absolute path of the rkyv shard
            byte_offset    INTEGER,        -- offset within shard (rkyv slice start)
            source_loc     TEXT,           -- 'file.zsh:line' for traceability
            bytecode       BLOB            -- compiled output (queryable copy)
        );

        CREATE INDEX IF NOT EXISTS entries_plugin_idx ON entries(plugin_id);
        CREATE INDEX IF NOT EXISTS entries_kind_idx ON entries(kind);

        CREATE TABLE IF NOT EXISTS hooks (
            kind     TEXT NOT NULL,        -- 'precmd' / 'preexec' / 'chpwd' / 'periodic' / etc.
            name     TEXT NOT NULL,        -- the hook's registered name
            fq_name  TEXT NOT NULL,        -- target entry in `entries`
            PRIMARY KEY (kind, name)
        );

        CREATE INDEX IF NOT EXISTS hooks_fqname_idx ON hooks(fq_name);

        -- Frecency / call-count / wallclock cost. Survives entries rebuild via
        -- ON CONFLICT DO UPDATE keyed on fq_name.
        CREATE TABLE IF NOT EXISTS entry_stats (
            fq_name        TEXT PRIMARY KEY,
            last_called_at INTEGER,
            call_count     INTEGER NOT NULL DEFAULT 0,
            total_ns       INTEGER NOT NULL DEFAULT 0
        );

        -- Files the daemon has parsed + bytecode-cached, looked up by absolute path.
        -- Covers: `zshrs FILE`, `source FILE`, .zshrc, plugin entry-points, autoload files.
        CREATE TABLE IF NOT EXISTS compiled_files (
            path           TEXT PRIMARY KEY,
            kind           TEXT NOT NULL,  -- 'script' / 'source' / 'zshrc' / 'plugin_init' / 'autoload'
            mtime          INTEGER,        -- ns since epoch
            inode          INTEGER,
            hash           BLOB,           -- content hash
            bytecode       BLOB,
            last_used_at   INTEGER,
            use_count      INTEGER NOT NULL DEFAULT 0,
            bytes_in       INTEGER,        -- source size
            bytes_out      INTEGER,        -- bytecode size
            sensitive      INTEGER NOT NULL DEFAULT 0, -- 1 if heuristics flagged secrets
            parent_paths   TEXT            -- JSON array of files that transitively included this
        );

        CREATE INDEX IF NOT EXISTS compiled_files_kind_idx ON compiled_files(kind);
        CREATE INDEX IF NOT EXISTS compiled_files_sensitive_idx ON compiled_files(sensitive);
        "#,
    )
}

/// Diagnostic record returned by `summary` — used by `zcache info`.
#[derive(serde::Serialize, Debug)]
pub struct CatalogSummary {
    pub schema_version: i64,
    pub plugin_count: i64,
    pub entries_count: i64,
    pub hooks_count: i64,
    pub entry_stats_count: i64,
    pub compiled_files_count: i64,
    pub size_bytes: u64,
}

/// Quick stats — used by `zcache info` and integration tests.
pub fn summary(conn: &Connection, db_path: &Path) -> Result<CatalogSummary> {
    let schema_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let plugin_count: i64 = conn.query_row("SELECT COUNT(*) FROM plugins", [], |r| r.get(0))?;
    let entries_count: i64 = conn.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))?;
    let hooks_count: i64 = conn.query_row("SELECT COUNT(*) FROM hooks", [], |r| r.get(0))?;
    let entry_stats_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM entry_stats", [], |r| r.get(0))?;
    let compiled_files_count: i64 =
        conn.query_row("SELECT COUNT(*) FROM compiled_files", [], |r| r.get(0))?;
    let size_bytes = std::fs::metadata(db_path).map(|m| m.len()).unwrap_or(0);
    Ok(CatalogSummary {
        schema_version,
        plugin_count,
        entries_count,
        hooks_count,
        entry_stats_count,
        compiled_files_count,
        size_bytes,
    })
}

/// PRAGMA integrity_check — used by `zcache verify`.
pub fn integrity_check(conn: &Connection) -> Result<bool> {
    let result: String =
        conn.query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0))?;
    Ok(result == "ok")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh_paths() -> (TempDir, CachePaths) {
        let tmp = TempDir::new().unwrap();
        let paths = CachePaths::with_root(tmp.path().join("zshrs"));
        paths.ensure_dirs().unwrap();
        (tmp, paths)
    }

    #[test]
    fn open_creates_schema_at_version_1() {
        let (_tmp, paths) = fresh_paths();
        let conn = open(&paths).unwrap();
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    #[test]
    fn open_idempotent() {
        let (_tmp, paths) = fresh_paths();
        {
            let _c = open(&paths).unwrap();
        }
        let _c = open(&paths).unwrap();
    }

    #[test]
    fn schema_has_expected_tables() {
        let (_tmp, paths) = fresh_paths();
        let conn = open(&paths).unwrap();

        let mut tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        tables.retain(|n| !n.starts_with("sqlite_"));

        let expected = [
            "compiled_files",
            "entries",
            "entry_stats",
            "hooks",
            "plugin_deps",
            "plugins",
        ];
        for want in expected {
            assert!(tables.contains(&want.to_string()), "missing table {}", want);
        }
    }

    #[test]
    fn summary_starts_empty() {
        let (_tmp, paths) = fresh_paths();
        let conn = open(&paths).unwrap();
        let s = summary(&conn, &paths.catalog_db).unwrap();
        assert_eq!(s.schema_version, SCHEMA_VERSION);
        assert_eq!(s.plugin_count, 0);
        assert_eq!(s.entries_count, 0);
        assert_eq!(s.hooks_count, 0);
        assert_eq!(s.entry_stats_count, 0);
        assert_eq!(s.compiled_files_count, 0);
        assert!(s.size_bytes > 0);
    }

    #[test]
    fn integrity_check_passes_on_fresh_db() {
        let (_tmp, paths) = fresh_paths();
        let conn = open(&paths).unwrap();
        assert!(integrity_check(&conn).unwrap());
    }

    #[test]
    fn entries_insert_and_select() {
        let (_tmp, paths) = fresh_paths();
        let conn = open(&paths).unwrap();
        conn.execute(
            "INSERT INTO entries (fq_name, plugin_id, kind, image_path, byte_offset, source_loc) \
             VALUES (?, ?, ?, ?, ?, ?)",
            rusqlite::params!["_git", "zpwr", "completion", "/tmp/zpwr.rkyv", 0, "_git:1"],
        )
        .unwrap();

        let s = summary(&conn, &paths.catalog_db).unwrap();
        assert_eq!(s.entries_count, 1);
    }

    #[test]
    fn entry_stats_on_conflict_update() {
        let (_tmp, paths) = fresh_paths();
        let conn = open(&paths).unwrap();

        // Insert.
        conn.execute(
            "INSERT INTO entry_stats (fq_name, last_called_at, call_count, total_ns) \
             VALUES (?, ?, ?, ?)",
            rusqlite::params!["_git", 100i64, 1i64, 1000i64],
        )
        .unwrap();

        // ON CONFLICT DO UPDATE.
        conn.execute(
            "INSERT INTO entry_stats (fq_name, last_called_at, call_count, total_ns) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT(fq_name) DO UPDATE SET \
                last_called_at = excluded.last_called_at, \
                call_count = entry_stats.call_count + excluded.call_count, \
                total_ns = entry_stats.total_ns + excluded.total_ns",
            rusqlite::params!["_git", 200i64, 1i64, 1500i64],
        )
        .unwrap();

        let (last, count, total): (i64, i64, i64) = conn
            .query_row(
                "SELECT last_called_at, call_count, total_ns FROM entry_stats WHERE fq_name=?",
                rusqlite::params!["_git"],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(last, 200);
        assert_eq!(count, 2);
        assert_eq!(total, 2500);
    }

    #[test]
    fn compiled_files_kind_index() {
        let (_tmp, paths) = fresh_paths();
        let conn = open(&paths).unwrap();
        conn.execute(
            "INSERT INTO compiled_files (path, kind, mtime, inode, bytes_in, bytes_out, sensitive) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                "/Users/wizard/.zpwr/local/.tokens.sh",
                "source",
                1735305600000000000i64,
                123456i64,
                12000i64,
                40000i64,
                1i64
            ],
        )
        .unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM compiled_files WHERE sensitive=1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn wal_mode_enabled() {
        let (_tmp, paths) = fresh_paths();
        let conn = open(&paths).unwrap();
        let mode: String =
            conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "wal");
    }
}
