// `plugins.db` registry writer.
//
// `zshrs --dump-plugins` (the IntelliJ External Libraries feed) reads the
// `plugins` table of `plugins.db` through `plugin_cache::list_plugins`. The
// shell never writes that table; the daemon does, from the sourced files and
// fpath directories of each recorder shard it ingests.
//
// The table DDL below mirrors `PluginCache::init_schema`
// (src/extensions/plugin_cache.rs); the daemon crate cannot depend on the
// shell library, so the one table is declared here too. A test in the library
// round-trips a row written here through `list_plugins`.
//
// Rows are written with `binary_mtime = 0`, which no running binary matches,
// so the shell's replay-validity check never treats them as replayable
// cache entries — they name plugins, nothing more.

use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use rusqlite::{params, Connection};

use super::recorder_shard::plugin_from_path;

/// Replace the `plugins` table in `db_path` with the plugins named by
/// `sourced` and `fpath`: one row per `(manager, name)`, keyed by the first
/// path seen for it, so the shell's classifier resolves the row back to the
/// plugin directory. An fpath directory is stored with a trailing `/` because
/// the classifier wants a path *inside* the plugin directory. Paths that no
/// longer exist are skipped. Returns the number of rows written.
pub fn record(db_path: &Path, sourced: &[String], fpath: &[String]) -> rusqlite::Result<usize> {
    let dirs = fpath.iter().map(|d| format!("{}/", d.trim_end_matches('/')));
    let mut first: BTreeMap<(String, String), String> = BTreeMap::new();
    for path in sourced.iter().cloned().chain(dirs) {
        if let Some(key) = plugin_from_path(&path) {
            first.entry(key).or_insert(path);
        }
    }

    let conn = Connection::open(db_path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;")?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS plugins (
            id INTEGER PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            mtime_secs INTEGER NOT NULL,
            mtime_nsecs INTEGER NOT NULL,
            source_time_ms INTEGER NOT NULL,
            cached_at INTEGER NOT NULL,
            binary_mtime INTEGER NOT NULL DEFAULT 0,
            binary_len INTEGER NOT NULL DEFAULT 0
        );",
    )?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM plugins", [])?;
    let mut written = 0;
    for path in first.values() {
        let Ok(meta) = std::fs::metadata(path) else {
            continue;
        };
        written += tx.execute(
            "INSERT OR REPLACE INTO plugins
                 (path, mtime_secs, mtime_nsecs, source_time_ms, cached_at, binary_mtime, binary_len)
             VALUES (?1, ?2, ?3, 0, ?4, 0, 0)",
            params![path, meta.mtime(), meta.mtime_nsec(), now],
        )?;
    }
    tx.commit()?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_rows_and_skips_missing_paths_and_non_plugins() {
        let home = tempfile::tempdir().unwrap();
        let plug = home.path().join(".zinit/plugins/a---b");
        std::fs::create_dir_all(&plug).unwrap();
        let file = plug.join("a.plugin.zsh");
        std::fs::write(&file, "").unwrap();
        let rc = home.path().join(".zshrc");
        std::fs::write(&rc, "").unwrap();
        let ghost = home.path().join(".zinit/plugins/c---d/c.plugin.zsh");
        let db = home.path().join("plugins.db");
        let s = |p: &Path| p.to_string_lossy().into_owned();

        let n = record(&db, &[s(&file), s(&rc), s(&ghost)], &[]).unwrap();
        assert_eq!(n, 1, "only the existing plugin file is a row");

        assert_eq!(record(&db, &[], &[]).unwrap(), 0, "a later shard replaces the set");
        let conn = Connection::open(&db).unwrap();
        let rows: i64 = conn.query_row("SELECT COUNT(*) FROM plugins", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 0);
    }
}
