//! Background compinit pre-warm — extension; no zsh C counterpart.
#[allow(unused_imports)]
use crate::ported::exec::ShellExecutor;
#[allow(unused_imports)]
use std::{env, collections::HashMap, path::PathBuf};
use compsys::cache::CompsysCache;
use compsys::CompInitResult;

/// Result from background compinit thread.
/// Outcome of background `compinit` autoload.
/// zshrs-original — Src/Modules/complete.c blocks on `compinit`
/// inline. The Rust port runs it on the worker pool.
pub struct CompInitBgResult {
    pub result: CompInitResult,
    pub cache: CompsysCache,
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// Non-blocking drain of background compinit results.
    /// Call this before any completion lookup (prompt, tab-complete, etc.).
    /// If the background thread hasn't finished yet, this is a no-op.
    pub fn drain_compinit_bg(&mut self) {
        if let Some((rx, start)) = self.compinit_pending.take() {
            match rx.try_recv() {
                Ok(bg) => {
                    let comps = bg.result.comps.len();
                    self.assoc_arrays
                        .insert("_comps".to_string(), bg.result.comps.into_iter().collect());
                    self.assoc_arrays.insert(
                        "_services".to_string(),
                        bg.result.services.into_iter().collect(),
                    );
                    self.assoc_arrays.insert(
                        "_patcomps".to_string(),
                        bg.result.patcomps.into_iter().collect(),
                    );
                    self.compsys_cache = Some(bg.cache);
                    tracing::info!(
                        wall_ms = start.elapsed().as_millis() as u64,
                        comps,
                        "compinit: background results merged"
                    );
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    // Not ready yet — put the receiver back for next poll
                    self.compinit_pending = Some((rx, start));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    tracing::warn!("compinit: background thread died without sending results");
                }
            }
        }
    }
    /// Traditional zsh compinit (--zsh-compat mode)
    /// Uses fpath scanning, .zcompdump files, no SQLite
    pub(crate) fn compinit_compat(
        &mut self,
        quiet: bool,
        no_dump: bool,
        dump_file: Option<String>,
        use_cache: bool,
    ) -> i32 {
        let zdotdir = self
            .variables
            .get("ZDOTDIR")
            .cloned()
            .or_else(|| std::env::var("ZDOTDIR").ok())
            .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()));

        let dump_path = dump_file
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(&zdotdir).join(".zcompdump"));

        // -C: Try to use existing .zcompdump if valid
        if use_cache
            && dump_path.exists()
            && compsys::check_dump(&dump_path, &self.fpath, "zshrs-0.1.0")
        {
            // Valid dump - source it to load _comps
            // For now, just rescan (proper impl would source the dump file)
            if !quiet {
                tracing::info!("compinit: .zcompdump valid, rescanning for compat");
            }
        }

        // Full fpath scan (traditional zsh algorithm)
        let result = compsys::compinit(&self.fpath);

        if !quiet {
            tracing::info!(
                functions = result.files_scanned,
                comps = result.comps.len(),
                dirs = result.dirs_scanned,
                ms = result.scan_time_ms,
                "compinit: fpath scan complete"
            );
        }

        // Write .zcompdump unless -D
        if !no_dump {
            let _ = compsys::compdump(&result, &dump_path, "zshrs-0.1.0");
        }

        // Set up _comps associative array
        self.assoc_arrays.insert(
            "_comps".to_string(),
            result.comps.clone().into_iter().collect(),
        );
        self.assoc_arrays.insert(
            "_services".to_string(),
            result.services.clone().into_iter().collect(),
        );
        self.assoc_arrays.insert(
            "_patcomps".to_string(),
            result.patcomps.clone().into_iter().collect(),
        );

        // No SQLite cache in compat mode
        self.compsys_cache = None;

        0
    }
}
// END moved-from-exec-rs
