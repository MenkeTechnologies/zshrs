//! Shared-handle accessors for the completion match accumulators.
//!
//! NOT a port of any C function — this is Rust-original glue that makes
//! explicit the *pointer aliasing* zsh's C relies on. In C the file-scope
//! `matches`/`fmatches`/`expls`/`allccs` LinkLists ARE the current group's
//! `l*` lists (`begcmgroup` does `matches = mgroup->lmatches`), so a
//! `compadd` append flows into the group with zero copy. The port replaced
//! that alias with owned copies + a manual `endcmgroup` flush, which is the
//! whole "copy-vs-alias" divergence class (nmatches reading 0, the message
//! flush gap, mgroup/amatches divergence).
//!
//! This module restores the alias: each `compcore::{matches,…}` global is a
//! rebindable HANDLE (`Mutex<Arc<Mutex<Vec<…>>>>`) whose inner `Arc` points
//! at the current group's `l*` field. `begcmgroup` rebinds via
//! [`rebind_current`]; every reader takes the `Arc` through the accessors
//! below. Lives OUTSIDE `src/ported/` deliberately: (a) it is not a C-fn
//! port, so it does not belong under the port tree (per the port-gate rule),
//! and (b) centralising the "lock handle → clone Arc → DROP handle guard →
//! return Arc" sequence guarantees the handle mutex is never held while the
//! inner `Vec` mutex is locked — the only way a completer body could deadlock
//! against `begcmgroup` (std `Mutex` is not reentrant).

use crate::ported::zle::comp_h::{Cexpl, Cmatch};
use crate::ported::zle::compcore::{allccs, expls, fmatches, matches};
use std::sync::{Arc, Mutex};

/// The current group's `lmatches` accumulator (the file-scope `matches`
/// alias). The handle guard is dropped before returning, so callers only
/// ever hold the inner `Vec` lock.
pub fn matches_arc() -> Arc<Mutex<Vec<Cmatch>>> {
    Arc::clone(&matches.get_or_init(default_match_handle).lock().unwrap())
}

/// The current group's `lfmatches` accumulator (file-scope `fmatches` alias).
pub fn fmatches_arc() -> Arc<Mutex<Vec<Cmatch>>> {
    Arc::clone(&fmatches.get_or_init(default_match_handle).lock().unwrap())
}

/// The current group's `lexpls` accumulator (file-scope `expls` alias).
pub fn expls_arc() -> Arc<Mutex<Vec<Cexpl>>> {
    Arc::clone(&expls.get_or_init(default_expl_handle).lock().unwrap())
}

/// The current group's `lallccs` accumulator (file-scope `allccs` alias).
pub fn allccs_arc() -> Arc<Mutex<Vec<String>>> {
    Arc::clone(&allccs.get_or_init(default_ccs_handle).lock().unwrap())
}

/// Point all four file-scope handles at the group `l*` fields — the Rust
/// equivalent of C's `begcmgroup` alias assignment
/// (`matches = mgroup->lmatches; …`). Called by `begcmgroup` for both the
/// reuse and the fresh-group paths. Each store briefly locks a handle mutex
/// and drops it; no inner `Vec` lock is taken here.
#[allow(clippy::type_complexity)]
pub fn rebind_current(
    lmatches: &Arc<Mutex<Vec<Cmatch>>>,
    lfmatches: &Arc<Mutex<Vec<Cmatch>>>,
    lexpls: &Arc<Mutex<Vec<Cexpl>>>,
    lallccs: &Arc<Mutex<Vec<String>>>,
) {
    *matches.get_or_init(default_match_handle).lock().unwrap() = Arc::clone(lmatches);
    *fmatches.get_or_init(default_match_handle).lock().unwrap() = Arc::clone(lfmatches);
    *expls.get_or_init(default_expl_handle).lock().unwrap() = Arc::clone(lexpls);
    *allccs.get_or_init(default_ccs_handle).lock().unwrap() = Arc::clone(lallccs);
}

fn default_match_handle() -> Mutex<Arc<Mutex<Vec<Cmatch>>>> {
    Mutex::new(Arc::new(Mutex::new(Vec::new())))
}
fn default_expl_handle() -> Mutex<Arc<Mutex<Vec<Cexpl>>>> {
    Mutex::new(Arc::new(Mutex::new(Vec::new())))
}
fn default_ccs_handle() -> Mutex<Arc<Mutex<Vec<String>>>> {
    Mutex::new(Arc::new(Mutex::new(Vec::new())))
}
