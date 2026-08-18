//! Bulk-compile every completer on `$fpath` into `~/.zshrs/autoloads.rkyv`.
//!
//! **zshrs-original — no C counterpart.** zsh parses a completion
//! function's file on first use, every process, forever; `zcompile`
//! only replaces the parse with a wordcode read of a digest the user
//! has to maintain by hand.
//!
//! The loader already installs from a cached chunk when one exists
//! (`vm_helper::run_autoload_definition`), but it fills the cache
//! write-through: a completer is compiled once per machine, and only
//! after someone has already paid for that compile at a prompt. This
//! pass front-loads the whole corpus so the FIRST `ls -<tab>` of a
//! fresh install is an O(1) probe into the shard instead of a parse.
//!
//! Measured on the corpus this was built for (46,647 completers,
//! debug build): a typical completer costs ~0.84 ms to parse+compile
//! and ~6.8 KB of bytecode; `_git`, the largest, costs 318 ms and
//! 4.6 MB. Decoding a cached chunk instead costs 229 µs for `_git`.
//!
//! # Why this runs in its own process
//!
//! `parse()` walks process-global lexer state. An earlier version of
//! this idea ran inside `compinit` on the worker pool, concurrently
//! with the interactive main thread, and corrupted it — the prompt
//! ended up spewing the xtrace prefix and stuck in PS2. So the entry
//! points are `zshrs --prewarm-autoloads` (a one-shot process that
//! never returns to a prompt), the recorder's end-of-run pass, and the
//! daemon op that spawns the former. Nothing here runs beside a live
//! ZLE.

use std::path::{Path, PathBuf};
use std::time::Instant;

/// What one prewarm pass did.
#[derive(Debug, Default, Clone)]
pub struct PrewarmStats {
    /// `_*` files seen across every directory.
    pub seen: usize,
    /// Entries already cached with matching source stamps.
    pub fresh: usize,
    /// Entries compiled and written this pass.
    pub compiled: usize,
    /// Files whose body would not parse (a broken completer, or syntax
    /// this port does not accept yet). Skipped, never cached.
    pub failed: usize,
    /// Total bytecode written, in bytes.
    pub bytes: usize,
    /// Wall time of the pass.
    pub elapsed_ms: u64,
}

/// Compile every `_*` file in `dirs` and bulk-write the chunks.
///
/// Directories are taken in order and de-duplicated by function name:
/// the FIRST directory holding a given name wins, which is how
/// `$fpath` resolution works (`getfpfunc` walks it in order), so the
/// cached chunk matches the file the loader would have read.
///
/// Files already cached with the same mtime + length are skipped
/// without being read, so re-running the pass after installing one
/// plugin costs one `stat` per completer rather than a recompile.
pub fn prewarm_fpath(dirs: &[PathBuf]) -> PrewarmStats {
    let t0 = Instant::now();
    let mut stats = PrewarmStats::default();
    let mut batch: Vec<(String, Vec<u8>, i64, u64)> = Vec::new();
    let mut claimed: std::collections::HashSet<String> = std::collections::HashSet::new();

    for dir in dirs {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::debug!(dir = %dir.display(), error = %e, "prewarm: unreadable fpath dir");
                continue;
            }
        };
        for entry in entries.flatten() {
            let name = match entry.file_name().into_string() {
                Ok(n) => n,
                Err(_) => continue,
            };
            // Same filename rule compinit/compaudit use: a leading `_`,
            // not a directory, not a compiled digest.
            if !name.starts_with('_') || name.ends_with(".zwc") {
                continue;
            }
            let path = entry.path();
            let meta = match std::fs::metadata(&path) {
                Ok(m) if m.is_file() => m,
                _ => continue,
            };
            if !claimed.insert(name.clone()) {
                continue; // shadowed by an earlier fpath dir
            }
            stats.seen += 1;
            let (mtime, len) = source_stamps(&meta);
            if crate::autoload_cache::try_load_for_source(&name, mtime, len).is_some() {
                stats.fresh += 1;
                continue;
            }
            match compile_one(&name, &path) {
                Some(blob) => {
                    stats.bytes += blob.len();
                    stats.compiled += 1;
                    batch.push((name, blob, mtime, len));
                }
                None => stats.failed += 1,
            }
        }
    }

    if let Err(e) = crate::autoload_cache::try_put_many(&batch) {
        tracing::warn!(error = %e, "prewarm: shard write failed");
    }
    stats.elapsed_ms = t0.elapsed().as_millis() as u64;
    tracing::info!(
        seen = stats.seen,
        compiled = stats.compiled,
        fresh = stats.fresh,
        failed = stats.failed,
        bytes = stats.bytes,
        ms = stats.elapsed_ms,
        "prewarm: autoload bytecode",
    );
    stats
}

/// mtime + length, the pair every cache entry is stamped with.
fn source_stamps(meta: &std::fs::Metadata) -> (i64, u64) {
    use std::os::unix::fs::MetadataExt;
    (meta.mtime(), meta.len())
}

/// Parse + compile one completer into the chunk the loader installs.
///
/// `ksh_style = false`: compinit autoloads every completer with
/// `autoload -rUz` (sh:337/541), which is zsh-style, and the loader
/// declines to use a cached chunk for a ksh-style autoload anyway — so
/// caching one would be dead weight at best and wrong at worst.
fn compile_one(name: &str, path: &Path) -> Option<Vec<u8>> {
    let body = std::fs::read_to_string(path).ok()?;
    // `noerrs = 1` (c:Src/utils.c — the `zerr` gate) for the whole
    // pass: a completer this port cannot parse yet is counted and
    // skipped, not announced. Without it the `stripkshdef` probe parse
    // inside `autoload_definition_source` printed a bare
    // ":30: parse error near `;;'" from somewhere in a 13k-file sweep,
    // with no filename and nothing the user could act on. errflag is
    // still SET under noerrs, which is what the caller reads.
    let saved_noerrs = {
        let mut g = crate::ported::utils::noerrs_lock().lock().ok()?;
        let prev = *g;
        *g = 1;
        prev
    };
    let result = compile_one_inner(name, path, &body);
    if let Ok(mut g) = crate::ported::utils::noerrs_lock().lock() {
        *g = saved_noerrs;
    }
    result
}

/// The body of [`compile_one`], with diagnostics already muted.
fn compile_one_inner(name: &str, path: &Path, body: &str) -> Option<Vec<u8>> {
    let source = crate::vm_helper::autoload_definition_source(name, body, false);
    // Mirror `strinbeg()` (c:Src/hist.c:1033): parsing a STRING must
    // report EOF when the lexer buffer drains instead of falling
    // through to `inputline()` and reading the process's stdin.
    crate::ported::input::strin.with(|s| s.set(s.get() + 1));
    let saved_errflag = crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed);
    crate::ported::utils::errflag.fetch_and(
        !crate::ported::utils::ERRFLAG_ERROR,
        std::sync::atomic::Ordering::Relaxed,
    );
    crate::ported::parse::parse_init(&source);
    let program = crate::ported::parse::parse();
    let failed = (crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed)
        & crate::ported::utils::ERRFLAG_ERROR)
        != 0;
    crate::ported::utils::errflag.store(saved_errflag, std::sync::atomic::Ordering::Relaxed);
    crate::ported::input::strin.with(|s| s.set(s.get() - 1));
    if failed || program.lists.is_empty() {
        tracing::debug!(name, path = %path.display(), "prewarm: body did not parse");
        return None;
    }
    let chunk = crate::compile_zsh::ZshCompiler::new().compile(&program);
    bincode::serialize(&chunk).ok()
}

/// The directories to prewarm when the caller names none: the live
/// `$fpath` array if a shell has one, else `$FPATH` from the
/// environment.
pub fn default_dirs() -> Vec<PathBuf> {
    let from_array = crate::ported::params::getaparam("fpath").unwrap_or_default();
    if !from_array.is_empty() {
        return from_array.into_iter().map(PathBuf::from).collect();
    }
    std::env::var("FPATH")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}
