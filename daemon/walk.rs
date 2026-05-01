// Walk-lifecycle multi-pass evaluator.
//
// Per docs/DAEMON.md "Walk lifecycle — first init + cache bust only":
//
//   Pass 1 — parse system + user dotfiles in zsh's standard order.
//   Pass 2 — replay state mutations to resolve final $PATH / $FPATH / etc.
//   Pass 3 — walk now-resolved $PATH / $FPATH / plugin-tree directories.
//   Pass 4 — serialize into rkyv shards + hydrate catalog.db entries.
//
// V1 scope:
//   - Pass 1+2 are stubbed: we read $PATH and $FPATH from the daemon's process env
//     instead of parsing user dotfiles. Real .zshrc analysis arrives in src/daemon/zshrc_analysis.rs.
//   - Pass 3+4 are real: we walk the resolved directories, build command_hash +
//     autoload_table, write a `system` shard with synthetic placeholder bytecode
//     entries, hydrate catalog.entries with file paths.
//
// This makes `zcache rebuild` produce a real shard with real entries derived from
// the user's environment, even before .zshrc parsing is wired.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::shard::{self, Shard};
use super::state::DaemonState;
use super::Result;

/// Result of a walk pass — derived state ready for serialization.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WalkResult {
    pub command_hash: HashMap<String, String>, // cmd → absolute path
    pub autoload_table: HashMap<String, String>, // function name → file path
    pub completion_files: Vec<String>,         // _foo files in fpath
    pub fpath: Vec<String>,
    pub path: Vec<String>,
    pub stats: WalkStats,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WalkStats {
    pub path_dirs_walked: usize,
    pub path_dirs_missing: usize,
    pub fpath_dirs_walked: usize,
    pub fpath_dirs_missing: usize,
    pub commands_found: usize,
    pub autoload_funcs_found: usize,
    pub completion_files_found: usize,
    pub duration_ms: u64,
}

/// Walk the directories in `path_dirs` and `fpath_dirs`. Returns the populated
/// command_hash + autoload_table.
pub fn walk_paths(path_dirs: &[String], fpath_dirs: &[String]) -> WalkResult {
    let start = std::time::Instant::now();
    let mut result = WalkResult::default();
    result.path = path_dirs.to_vec();
    result.fpath = fpath_dirs.to_vec();

    // Walk $PATH dirs — flat scan only (zsh doesn't recurse into PATH).
    for dir_str in path_dirs {
        let dir = Path::new(dir_str);
        if !dir.exists() || !dir.is_dir() {
            result.stats.path_dirs_missing += 1;
            continue;
        }
        result.stats.path_dirs_walked += 1;
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Ok(meta) = entry.metadata() {
                    if !meta.is_file() {
                        continue;
                    }
                    use std::os::unix::fs::PermissionsExt;
                    if meta.permissions().mode() & 0o111 == 0 {
                        continue;
                    }
                }
                if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                    // First match wins (zsh PATH semantics).
                    result
                        .command_hash
                        .entry(name.to_string())
                        .or_insert_with(|| p.display().to_string());
                }
            }
        }
    }
    result.stats.commands_found = result.command_hash.len();

    // Walk $FPATH dirs — collect autoload function file names + completion files.
    for dir_str in fpath_dirs {
        let dir = Path::new(dir_str);
        if !dir.exists() || !dir.is_dir() {
            result.stats.fpath_dirs_missing += 1;
            continue;
        }
        result.stats.fpath_dirs_walked += 1;
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                    if name.starts_with('.') {
                        continue;
                    }
                    let path_str = p.display().to_string();
                    // Completion files: _foo, _bar etc.
                    if name.starts_with('_') {
                        result.completion_files.push(path_str.clone());
                    }
                    result
                        .autoload_table
                        .entry(name.to_string())
                        .or_insert(path_str);
                }
            }
        }
    }
    result.stats.autoload_funcs_found = result.autoload_table.len();
    result.stats.completion_files_found = result.completion_files.len();
    result.stats.duration_ms = start.elapsed().as_millis() as u64;

    result
}

/// Read $PATH/$FPATH from the daemon's own process env. Real Pass 1+2 from .zshrc
/// analysis replaces this; v1 fallback is the env the daemon was launched with.
pub fn current_env_paths() -> (Vec<String>, Vec<String>) {
    let path = std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    let fpath = std::env::var("FPATH")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    (path, fpath)
}

/// Build a `system` shard from the walk result.
pub fn build_system_shard(walk: &WalkResult, generation: u64) -> Shard {
    let mut shard = Shard::new("system", "/", generation);
    // V1: store walk JSON as a single entry, plus per-command/per-function entries
    // pointing to source-paths. Future iteration replaces JSON with rkyv-archived
    // structured data.
    let walk_json = serde_json::to_vec(walk).unwrap_or_default();
    shard.insert("__walk_meta__", walk_json);

    for (name, path) in &walk.command_hash {
        shard.insert(format!("cmd:{}", name), path.as_bytes().to_vec());
    }
    for (name, path) in &walk.autoload_table {
        shard.insert(format!("fn:{}", name), path.as_bytes().to_vec());
    }
    shard
}

/// Hydrate catalog.entries with one row per (cmd, autoload, completion).
pub fn hydrate_catalog(state: &DaemonState, walk: &WalkResult, image_path: &Path) -> Result<usize> {
    let path_str = image_path.display().to_string();
    let mut count = 0usize;
    state.with_catalog(|conn| {
        // Clear prior entries with plugin_id='system' so reruns are idempotent.
        conn.execute("DELETE FROM entries WHERE plugin_id = 'system'", [])?;

        let mut insert_entry = conn.prepare(
            "INSERT OR REPLACE INTO entries
             (fq_name, plugin_id, kind, image_path, byte_offset, source_loc, bytecode)
             VALUES (?, 'system', ?, ?, 0, ?, NULL)",
        )?;

        for (name, source) in &walk.command_hash {
            insert_entry.execute(rusqlite::params![
                format!("cmd:{}", name),
                "command",
                path_str.clone(),
                source,
            ])?;
            count += 1;
        }
        for (name, source) in &walk.autoload_table {
            let kind = if name.starts_with('_') {
                "completion"
            } else {
                "autoload"
            };
            insert_entry.execute(rusqlite::params![
                format!("fn:{}", name),
                kind,
                path_str.clone(),
                source,
            ])?;
            count += 1;
        }
        Ok::<_, rusqlite::Error>(())
    })?;
    Ok(count)
}

/// Run the full Pass 3 + Pass 4 pipeline against the daemon's current env.
/// Returns (image_path, hydrated_count, walk_stats).
pub fn run_full_rebuild(
    state: &DaemonState,
    generation: u64,
) -> Result<(PathBuf, usize, WalkStats)> {
    let (path, fpath) = current_env_paths();
    let walk = walk_paths(&path, &fpath);
    let stats = walk.stats.clone();
    let shard = build_system_shard(&walk, generation);
    let image_path = shard::write_shard(&state.paths, &shard)?;
    let hydrated = hydrate_catalog(state, &walk, &image_path)?;

    // Register fsnotify watches on every walked $PATH and $FPATH dir.
    for dir in path.iter().chain(fpath.iter()) {
        let dir_path = PathBuf::from(dir);
        if !dir_path.exists() {
            continue;
        }
        let wp = super::fsnotify::WatchedPath {
            path: dir_path.clone(),
            shard_slug: "system".to_string(),
            source_root: dir.clone(),
            kind: super::fsnotify::WatchKind::FpathDir,
        };
        if let Err(e) = state.fs_watcher.watch_path(wp, false) {
            tracing::warn!(?e, dir = %dir, "fsnotify watch failed (non-fatal)");
        }
    }

    Ok((image_path, hydrated, stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn walk_finds_executables_in_path_dir() {
        let tmp = TempDir::new().unwrap();
        let bindir = tmp.path().join("bin");
        std::fs::create_dir(&bindir).unwrap();

        // Create an executable file.
        let exe = bindir.join("greet");
        std::fs::write(&exe, b"#!/bin/sh\necho hi").unwrap();
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&exe).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&exe, perms).unwrap();

        // Non-executable file (should be skipped).
        std::fs::write(bindir.join("not_exe.txt"), b"hi").unwrap();

        let result = walk_paths(&[bindir.display().to_string()], &[]);
        assert_eq!(result.command_hash.len(), 1);
        assert!(result.command_hash.contains_key("greet"));
        assert_eq!(result.stats.path_dirs_walked, 1);
        assert_eq!(result.stats.commands_found, 1);
    }

    #[test]
    fn walk_finds_autoload_files_in_fpath_dir() {
        let tmp = TempDir::new().unwrap();
        let fpdir = tmp.path().join("funcs");
        std::fs::create_dir(&fpdir).unwrap();

        std::fs::write(fpdir.join("_git"), b"# completion").unwrap();
        std::fs::write(fpdir.join("_docker"), b"# completion").unwrap();
        std::fs::write(fpdir.join("zpwrAbout"), b"# function").unwrap();
        std::fs::write(fpdir.join(".hidden"), b"# ignored").unwrap();

        let result = walk_paths(&[], &[fpdir.display().to_string()]);
        assert_eq!(result.autoload_table.len(), 3);
        assert_eq!(result.completion_files.len(), 2);
        assert!(result.autoload_table.contains_key("_git"));
        assert!(result.autoload_table.contains_key("_docker"));
        assert!(result.autoload_table.contains_key("zpwrAbout"));
        assert!(!result.autoload_table.contains_key(".hidden"));
    }

    #[test]
    fn walk_handles_missing_dirs() {
        let tmp = TempDir::new().unwrap();
        let result = walk_paths(
            &[tmp.path().join("does_not_exist").display().to_string()],
            &[],
        );
        assert_eq!(result.stats.path_dirs_walked, 0);
        assert_eq!(result.stats.path_dirs_missing, 1);
        assert!(result.command_hash.is_empty());
    }

    #[test]
    fn walk_first_match_wins() {
        let tmp = TempDir::new().unwrap();
        let dir1 = tmp.path().join("a");
        let dir2 = tmp.path().join("b");
        std::fs::create_dir(&dir1).unwrap();
        std::fs::create_dir(&dir2).unwrap();

        for d in [&dir1, &dir2] {
            let exe = d.join("greet");
            std::fs::write(&exe, b"#!/bin/sh").unwrap();
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&exe).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&exe, perms).unwrap();
        }

        let result = walk_paths(
            &[dir1.display().to_string(), dir2.display().to_string()],
            &[],
        );
        assert_eq!(
            result.command_hash["greet"],
            dir1.join("greet").display().to_string()
        );
    }

    #[test]
    fn build_system_shard_packs_entries() {
        let mut walk = WalkResult::default();
        walk.command_hash
            .insert("ls".to_string(), "/usr/bin/ls".to_string());
        walk.autoload_table
            .insert("_git".to_string(), "/u/funcs/_git".to_string());

        let shard = build_system_shard(&walk, 1);
        assert!(shard.entries.contains_key("__walk_meta__"));
        assert!(shard.entries.contains_key("cmd:ls"));
        assert!(shard.entries.contains_key("fn:_git"));
        assert_eq!(
            shard.entries.get("cmd:ls").unwrap().as_slice(),
            b"/usr/bin/ls"
        );
    }
}
