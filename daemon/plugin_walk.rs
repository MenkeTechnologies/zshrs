// plugin_walk.rs — discovers and caches plugin trees so the daemon can
// serve every plugin's state contributions to clients without each shell
// re-evaluating them.
//
// Per docs/DAEMON.md "Starting state served by daemon" + "Plugin discovery":
//   Plugin discovery happens at the same time as `.zshrc` analysis: daemon
//   walks the user's `.zshrc`, sees zinit/OMZ/source calls, descends into
//   each referenced plugin, parses + bytecode-compiles per-plugin shards
//   ({hash8}-plugin-{name}.rkyv), captures every state contribution
//   (alias declarations, function definitions, fpath additions, compdef
//   calls, zstyle declarations, bindkey calls, setopt calls, env exports),
//   and folds them into the consolidated starting-point caches.
//
// V1 covers zinit's on-disk layout — `~/.zinit/plugins/{owner}---{repo}/`
// and `~/.zinit/snippets/OMZP::*` — by walking each plugin dir, picking
// the conventional init script, running analyze_one_into on it, and
// writing a per-plugin shard. The plugin dir itself is added to the
// canonical fpath (zinit auto-fpaths every plugin dir for completions).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::ast_walker::analyze_one_into;
use super::canonical::CanonicalEngine;
use super::shard::{
    write_canonical_shard, CanonicalShard, ShardHeader, SHARD_FORMAT_VERSION, SHARD_MAGIC,
};
use super::zshrc_analysis::CanonicalState;
use super::Result;

/// Stats from one full plugin-tree walk.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PluginWalkStats {
    pub plugin_dirs_scanned: usize,
    pub plugins_analyzed: usize,
    pub snippets_analyzed: usize,
    pub init_scripts_found: usize,
    pub init_scripts_missing: usize,
    pub shards_written: usize,
    pub aliases_total: usize,
    pub functions_total: usize,
    pub compdef_total: usize,
    pub bindkeys_total: usize,
    pub setopt_total: usize,
    pub env_total: usize,
    pub fpath_additions: usize,
    pub duration_ms: u64,
}

/// Per-plugin record returned to callers (also used as IPC response payload).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginRecord {
    pub source: String, // "zinit-plugin" | "zinit-snippet" | ...
    pub name: String,   // owner---repo or OMZP::name
    pub dir: String,
    pub init_script: Option<String>,
    pub aliases: usize,
    pub functions: usize,
    pub compdef: usize,
    pub bindkeys: usize,
    pub setopt: usize,
    pub env: usize,
    pub fpath_added: bool,
    pub shard_path: Option<String>,
}

/// Run a full discovery + analysis pass. Walks plugin/snippet directories,
/// analyzes each, writes per-plugin shards, folds the union of state into
/// the canonical engine + persists the master canonical shard.
pub fn run_full_discovery(
    paths: &super::paths::CachePaths,
    canonical: &CanonicalEngine,
) -> Result<(PluginWalkStats, Vec<PluginRecord>)> {
    let start = std::time::Instant::now();
    let mut stats = PluginWalkStats::default();
    let mut records = Vec::new();

    let home = match dirs::home_dir() {
        Some(h) => h,
        None => return Ok((stats, records)),
    };

    let zinit_plugins = home.join(".zinit").join("plugins");
    if zinit_plugins.is_dir() {
        walk_zinit_plugins(&zinit_plugins, paths, canonical, &mut stats, &mut records)?;
    }

    let zinit_snippets = home.join(".zinit").join("snippets");
    if zinit_snippets.is_dir() {
        walk_zinit_snippets(&zinit_snippets, paths, canonical, &mut stats, &mut records)?;
    }

    // Persist the rolled-up canonical state once at the end (single rkyv
    // write covers every plugin contribution). SQLite mirror is hydrated
    // by the caller (op_first_init / op_plugin_discover) right after this
    // returns, so each `INFO ... canonical shard written` line is followed
    // by a fresh `canonical` table view.
    let generation = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
    let _ = canonical.persist(generation);

    stats.duration_ms = start.elapsed().as_millis() as u64;
    Ok((stats, records))
}

fn walk_zinit_plugins(
    plugins_root: &Path,
    paths: &super::paths::CachePaths,
    canonical: &CanonicalEngine,
    stats: &mut PluginWalkStats,
    records: &mut Vec<PluginRecord>,
) -> Result<()> {
    let entries = match std::fs::read_dir(plugins_root) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let dir_name = match dir.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Skip `.zwc` companion entries and the local pseudo-plugin marker.
        if dir_name.ends_with(".zwc") {
            continue;
        }
        stats.plugin_dirs_scanned += 1;
        let plugin_name = dir_name.replace("---", "/");
        let init = pick_init_script(&dir, &dir_name);
        if init.is_some() {
            stats.init_scripts_found += 1;
        } else {
            stats.init_scripts_missing += 1;
        }
        let mut local = CanonicalState::default();
        if let Some(ref init_path) = init {
            // Best-effort — failure to read is non-fatal, plugin still adds fpath.
            let _ = analyze_one_into(&mut local, init_path);
            stats.plugins_analyzed += 1;
        }
        // Plugin init scripts often `source ${0:A:h}/sibling.zsh` which the
        // AST walker can't statically resolve (no $0 binding). To still
        // capture alias / function definitions in those siblings, analyze
        // every .zsh file at the plugin top level. Bounded to top-level
        // (no recursion) so we don't drown in nested test fixtures.
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for ent in rd.flatten() {
                let p = ent.path();
                if !p.is_file() {
                    continue;
                }
                let ext = p.extension().and_then(|e| e.to_str());
                if !matches!(ext, Some("zsh") | Some("sh")) {
                    continue;
                }
                if let Some(ref init_p) = init {
                    if &p == init_p {
                        continue; // already analyzed above
                    }
                }
                let _ = analyze_one_into(&mut local, &p);
            }
        }
        // zinit auto-fpaths every plugin dir.
        let fpath_entry = dir.display().to_string();
        if !local.fpath_additions.iter().any(|e| e == &fpath_entry) {
            local.fpath_additions.push(fpath_entry.clone());
        }
        // Heuristic for plugins like zsh-more-completions that nest
        // completion files in subdirs (`more_src/`, `man_src/`,
        // `architecture_src/`, …) and add them to fpath via dynamic
        // `fpath+=(${0:h}/subdir)` calls our static analyzer can't
        // resolve. Auto-add any first-level subdir containing `_*` files
        // to fpath. Bounded scan: stops at first `_*` hit per subdir.
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for sub in rd.flatten() {
                let sp = sub.path();
                if !sp.is_dir() {
                    continue;
                }
                let sname = match sp.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n,
                    None => continue,
                };
                if sname.starts_with('.') || sname == "__pycache__" || sname == ".git" {
                    continue;
                }
                let has_completion = std::fs::read_dir(&sp)
                    .map(|inner| {
                        inner
                            .flatten()
                            .any(|e| e.file_name().to_string_lossy().starts_with('_'))
                    })
                    .unwrap_or(false);
                if has_completion {
                    let s = sp.display().to_string();
                    if !local.fpath_additions.iter().any(|e| e == &s) {
                        local.fpath_additions.push(s);
                    }
                }
            }
        }
        merge_into_canonical(canonical, &local);
        let wrote = write_per_plugin_shard(paths, "zinit-plugin", &plugin_name, &dir, &local)?;
        if wrote {
            stats.shards_written += 1;
        }
        accumulate_stats(stats, &local);
        records.push(PluginRecord {
            source: "zinit-plugin".to_string(),
            name: plugin_name.clone(),
            dir: dir.display().to_string(),
            init_script: init.map(|p| p.display().to_string()),
            aliases: local.aliases.len(),
            functions: local.functions.len(),
            compdef: local.compdef.len(),
            bindkeys: local.bindkeys.len(),
            setopt: local.setopts.len() + local.unsetopts.len(),
            env: local.env_exports.len(),
            fpath_added: true,
            shard_path: Some(
                per_plugin_shard_path(paths, "zinit-plugin", &plugin_name)
                    .display()
                    .to_string(),
            ),
        });
    }
    Ok(())
}

fn walk_zinit_snippets(
    snippets_root: &Path,
    paths: &super::paths::CachePaths,
    canonical: &CanonicalEngine,
    stats: &mut PluginWalkStats,
    records: &mut Vec<PluginRecord>,
) -> Result<()> {
    let entries = match std::fs::read_dir(snippets_root) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        if name.ends_with(".zwc") {
            continue;
        }
        let mut local = CanonicalState::default();
        let (init_path, fpath_dir) = if path.is_dir() {
            // OMZP::name layout — directory containing <name>.plugin.zsh.
            let dir_name = name.clone();
            let inner = pick_init_script(&path, &dir_name);
            (inner, Some(path.clone()))
        } else if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("zsh") {
            // OMZL::*.zsh layout — direct .zsh file.
            (Some(path.clone()), None)
        } else {
            (None, None)
        };
        if let Some(ref init) = init_path {
            let _ = analyze_one_into(&mut local, init);
            stats.snippets_analyzed += 1;
            stats.init_scripts_found += 1;
        } else {
            stats.init_scripts_missing += 1;
        }
        // Same trick as walk_zinit_plugins: analyze every additional .zsh /
        // bare-name file in the snippet dir so siblings sourced via dynamic
        // `${0:A:h}/x` get captured. Bounded to top-level.
        if path.is_dir() {
            if let Ok(rd) = std::fs::read_dir(&path) {
                for ent in rd.flatten() {
                    let p = ent.path();
                    if !p.is_file() {
                        continue;
                    }
                    let ext = p.extension().and_then(|e| e.to_str());
                    let is_zsh = matches!(ext, Some("zsh") | Some("sh"));
                    let is_bare = ext.is_none()
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| !n.starts_with('.') && n != "LICENSE" && n != "README")
                            .unwrap_or(false);
                    if !is_zsh && !is_bare {
                        continue;
                    }
                    if let Some(ref init_p) = init_path {
                        if &p == init_p {
                            continue;
                        }
                    }
                    let _ = analyze_one_into(&mut local, &p);
                }
            }
        }
        if let Some(d) = fpath_dir.as_ref() {
            let entry = d.display().to_string();
            if !local.fpath_additions.iter().any(|e| e == &entry) {
                local.fpath_additions.push(entry);
            }
        }
        merge_into_canonical(canonical, &local);
        let dir_path = fpath_dir.clone().unwrap_or_else(|| path.clone());
        let wrote = write_per_plugin_shard(paths, "zinit-snippet", &name, &dir_path, &local)?;
        if wrote {
            stats.shards_written += 1;
        }
        accumulate_stats(stats, &local);
        records.push(PluginRecord {
            source: "zinit-snippet".to_string(),
            name: name.clone(),
            dir: dir_path.display().to_string(),
            init_script: init_path.map(|p| p.display().to_string()),
            aliases: local.aliases.len(),
            functions: local.functions.len(),
            compdef: local.compdef.len(),
            bindkeys: local.bindkeys.len(),
            setopt: local.setopts.len() + local.unsetopts.len(),
            env: local.env_exports.len(),
            fpath_added: fpath_dir.is_some(),
            shard_path: Some(
                per_plugin_shard_path(paths, "zinit-snippet", &name)
                    .display()
                    .to_string(),
            ),
        });
    }
    Ok(())
}

/// zinit's plugin-init resolution order (best-effort match):
///   1. `*.plugin.zsh` (OMZ convention)
///   2. `init.zsh`
///   3. `<basename-without-prefix>.zsh` (drop owner---repo prefix → repo.zsh)
///   4. `<basename-without-prefix>.zsh-theme`
///   5. the lone `.zsh` file if exactly one exists
fn pick_init_script(dir: &Path, dir_name: &str) -> Option<PathBuf> {
    // owner---repo → repo (drop everything up to last `---`)
    let bare = dir_name.rsplit_once("---").map(|x| x.1).unwrap_or(dir_name);

    // candidate files in priority order
    let entries: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(it) => it.flatten().map(|e| e.path()).collect(),
        Err(_) => return None,
    };
    let plugin_zsh_files: Vec<&PathBuf> = entries
        .iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.ends_with(".plugin.zsh"))
                .unwrap_or(false)
        })
        .collect();
    if let Some(first) = plugin_zsh_files.first() {
        return Some((*first).clone());
    }

    let candidates: &[String] = &[
        "init.zsh".to_string(),
        format!("{}.zsh", bare),
        format!("{}.zsh-theme", bare),
        format!("{}.plugin.zsh", bare),
        // zinit-snippet convention: file named identically to the dir.
        // OMZP::git/OMZP::git, OMZP::docker/OMZP::docker, etc.
        dir_name.to_string(),
        bare.to_string(),
    ];
    for c in candidates {
        let p = dir.join(c);
        if p.is_file() {
            return Some(p);
        }
    }
    // Fall back to the only `.zsh` if there's exactly one in the dir.
    let zsh_files: Vec<&PathBuf> = entries
        .iter()
        .filter(|p| {
            p.is_file()
                && p.extension().and_then(|e| e.to_str()) == Some("zsh")
                && !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map_or(false, |s| s.ends_with(".plugin.zsh"))
        })
        .collect();
    if zsh_files.len() == 1 {
        return Some(zsh_files[0].clone());
    }
    None
}

fn merge_into_canonical(canon: &CanonicalEngine, local: &CanonicalState) {
    let json_string = |s: &str| serde_json::Value::String(s.to_string()).to_string();
    for (k, v) in &local.aliases {
        canon.upsert("alias", k, &json_string(v), None);
    }
    for (k, v) in &local.global_aliases {
        canon.upsert("galias", k, &json_string(v), None);
    }
    for (k, v) in &local.suffix_aliases {
        canon.upsert("salias", k, &json_string(v), None);
    }
    for (k, v) in &local.functions {
        canon.upsert("function", k, &json_string(v), None);
    }
    for (k, v) in &local.bindkeys {
        canon.upsert("bindkey", k, &json_string(v), None);
    }
    for (k, v) in &local.named_dirs {
        canon.upsert("named_dir", k, &json_string(v), None);
    }
    for (k, v) in &local.compdef {
        canon.upsert("compdef", k, &json_string(v), None);
    }
    for (k, v) in &local.env_exports {
        canon.upsert("env", k, &json_string(v), None);
    }
    for (k, v) in &local.params {
        canon.upsert("params", k, &json_string(v), None);
    }
    for opt in &local.setopts {
        canon.upsert("setopt", opt, "\"on\"", None);
    }
    for opt in &local.unsetopts {
        canon.upsert("setopt", opt, "\"off\"", None);
    }
    for module in &local.zmodload {
        canon.upsert("zmodload", module, "\"loaded\"", None);
    }
    for (i, dir) in local.fpath_additions.iter().enumerate() {
        // Use shard-name-prefixed key so multiple plugins don't overwrite
        // each other's fpath slots (canonical fpath subsystem is keyed by
        // index — collide-on-zero would lose all but the last). The export
        // path serializes by ordering the values, so any ordering works.
        let key = format!("plugin-{}-{}", short_hash(dir), i);
        canon.upsert("fpath", &key, &json_string(dir), None);
    }
    for (pat, rest) in &local.zstyle {
        canon.upsert("zstyle", pat, &json_string(rest), None);
    }
}

fn accumulate_stats(stats: &mut PluginWalkStats, local: &CanonicalState) {
    stats.aliases_total +=
        local.aliases.len() + local.global_aliases.len() + local.suffix_aliases.len();
    stats.functions_total += local.functions.len();
    stats.compdef_total += local.compdef.len();
    stats.bindkeys_total += local.bindkeys.len();
    stats.setopt_total += local.setopts.len() + local.unsetopts.len();
    stats.env_total += local.env_exports.len();
    stats.fpath_additions += local.fpath_additions.len();
}

fn write_per_plugin_shard(
    paths: &super::paths::CachePaths,
    source: &str,
    name: &str,
    dir: &Path,
    local: &CanonicalState,
) -> Result<bool> {
    let slug = format!("{}-{}", source, sanitize_name(name));
    let source_root = dir.display().to_string();
    let generation = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
    let mut shard = CanonicalShard {
        header: ShardHeader {
            magic: SHARD_MAGIC,
            format_version: SHARD_FORMAT_VERSION,
            generation,
            built_at_ns: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64,
            slug: slug.clone(),
            source_root: source_root.clone(),
            entry_count: 0,
        },
        ..Default::default()
    };
    shard.aliases = local.aliases.clone().into_iter().collect();
    shard.global_aliases = local.global_aliases.clone().into_iter().collect();
    shard.suffix_aliases = local.suffix_aliases.clone().into_iter().collect();
    shard.functions = local.functions.clone().into_iter().collect();
    shard.bindkeys = local.bindkeys.clone().into_iter().collect();
    shard.named_dirs = local.named_dirs.clone().into_iter().collect();
    shard.compdef = local.compdef.clone().into_iter().collect();
    shard.env_exports = local.env_exports.clone().into_iter().collect();
    shard.params = local.params.clone().into_iter().collect();
    shard.path = local.path_additions.clone();
    shard.fpath = local.fpath_additions.clone();
    shard.manpath = local.manpath_additions.clone();
    shard.zstyle = local.zstyle.clone();
    shard.zmodload = local.zmodload.iter().cloned().collect();
    shard.setopts = local.setopts.iter().cloned().collect();
    shard.unsetopts = local.unsetopts.iter().cloned().collect();
    let total = shard.aliases.len()
        + shard.global_aliases.len()
        + shard.suffix_aliases.len()
        + shard.functions.len()
        + shard.bindkeys.len()
        + shard.named_dirs.len()
        + shard.compdef.len()
        + shard.env_exports.len()
        + shard.params.len()
        + shard.path.len()
        + shard.fpath.len()
        + shard.manpath.len()
        + shard.zstyle.len()
        + shard.zmodload.len()
        + shard.setopts.len()
        + shard.unsetopts.len();
    // Skip writing empty shards. zinit-snippet entries that point at OMZ
    // plugins the user has *mentioned* (in their .zshrc) but never actually
    // loaded — their init scripts produce no aliases / functions / fpath /
    // anything — would otherwise generate a 350-byte rkyv stub per entry.
    // Those are pure disk litter. Per docs/DAEMON.md "Plugin discovery"
    // section: shard creation captures the *state contribution* of the
    // plugin; zero contribution = no shard.
    if total == 0 {
        tracing::debug!(slug = %slug, "skipping empty per-plugin shard");
        return Ok(false);
    }
    shard.header.entry_count = total as u32;
    write_canonical_shard(paths, &shard)?;
    Ok(true)
}

fn per_plugin_shard_path(paths: &super::paths::CachePaths, source: &str, name: &str) -> PathBuf {
    let slug = format!("{}-{}", source, sanitize_name(name));
    super::shard::shard_path(paths, &slug, &slug)
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn short_hash(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(s.as_bytes());
    digest
        .iter()
        .take(4)
        .map(|b| format!("{:02x}", b))
        .collect()
}

#[allow(dead_code)]
fn _silence(_: HashMap<(), ()>, _: HashSet<()>) {}
