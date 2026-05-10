//! hist.c - history mechanism
//!
//! Port of Src/hist.c
//!
//! The history lines are kept in a hash, and also doubly-linked in a ring.   // c:98

use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::ported::zsh_h::histent;
pub use crate::ported::zsh_h::CaseMod;

// =========================================================================
// File-scope globals from hist.c
// =========================================================================

// != 0 means history substitution is turned off                              // c:57
// (stophist is in zsh.h as an extern; we own it here.)

/// Port of `HashTable histtab` from Src/hist.c:101.
/// Lookup table for histent by name (placeholder until hashtable port lands). // c:101
pub static histtab: Mutex<Vec<usize>> = Mutex::new(Vec::new());              // c:101

/// Port of `mod_export Histent hist_ring` from Src/hist.c:103.
/// Doubly-linked ring of history entries; modelled here as a Vec<histent>
/// since each histent already has up/down pointers in the C struct.
pub static hist_ring: Mutex<Vec<histent>> = Mutex::new(Vec::new());          // c:103

/// Port of `struct histent curline` from Src/hist.c:91. Sentinel
/// histent for the in-progress edit; spliced into the ring head by
/// linkcurline() and removed by unlinkcurline().
pub static curline: Mutex<Option<histent>> = Mutex::new(None);               // c:91

/// Port of `zlong histsiz` from Src/hist.c:108.
pub static histsiz: AtomicI64 = AtomicI64::new(0);                           // c:108

/// Port of `zlong savehistsiz` from Src/hist.c:113.
pub static savehistsiz: AtomicI64 = AtomicI64::new(0);                       // c:113

/// Port of `int histdone` from Src/hist.c:119.
pub static histdone: AtomicI32 = AtomicI32::new(0);                          // c:119

/// Port of `int histactive` from Src/hist.c:124.
pub static histactive: AtomicU32 = AtomicU32::new(0);                        // c:124

/// Port of `int hist_ignore_all_dups` from Src/hist.c:130.
pub static hist_ignore_all_dups: AtomicI32 = AtomicI32::new(0);              // c:130

/// Port of `mod_export int hist_skip_flags` from Src/hist.c:135.
pub static hist_skip_flags: AtomicI32 = AtomicI32::new(0);                   // c:135

// Bits of histactive variable                                               // c:137
/// Port of `HA_ACTIVE` from Src/hist.c:138. History mechanism is active.
pub const HA_ACTIVE: u32 = 1 << 0;                                           // c:138
/// Port of `HA_NOINC` from Src/hist.c:139. Don't store, curhist not incremented.
pub const HA_NOINC: u32 = 1 << 1;                                            // c:139
/// Port of `HA_INWORD` from Src/hist.c:140. We're inside a word.
pub const HA_INWORD: u32 = 1 << 2;                                           // c:140
/// Port of `HA_UNGET` from Src/hist.c:142. Recursively ungetting.
pub const HA_UNGET: u32 = 1 << 3;                                            // c:142

/// Port of `short *chwords` from Src/hist.c:147.
/// Word beginning/end offsets in current history line.
pub static chwords: Mutex<Vec<i16>> = Mutex::new(Vec::new());                // c:147

/// Port of `int chwordlen` from Src/hist.c:154.
pub static chwordlen: AtomicI32 = AtomicI32::new(0);                         // c:154

/// Port of `int chwordpos` from Src/hist.c:154.
pub static chwordpos: AtomicI32 = AtomicI32::new(0);                         // c:154

/// Port of `char *hsubl` from Src/hist.c:159.
/// Last `l` for `s/l/r/` history substitution.
pub static hsubl: Mutex<Option<String>> = Mutex::new(None);                  // c:159

/// Port of `char *hsubr` from Src/hist.c:164.
pub static hsubr: Mutex<Option<String>> = Mutex::new(None);                  // c:164

/// Port of `int hsubpatopt` from Src/hist.c:169.
pub static hsubpatopt: AtomicI32 = AtomicI32::new(0);                        // c:169

/// Port of `mod_export char *hptr` from Src/hist.c:174.
/// Pointer into the history line; tracked as the byte length of `chline`.
pub static hptr: AtomicUsize = AtomicUsize::new(0);                          // c:174

/// Port of `mod_export char *chline` from Src/hist.c:179.
pub static chline: Mutex<String> = Mutex::new(String::new());                // c:179

/// Port of `mod_export char *zle_chline` from Src/hist.c:195.
pub static zle_chline: Mutex<Option<String>> = Mutex::new(None);             // c:195

/// Port of `int qbang` from Src/hist.c:201.
pub static qbang: AtomicBool = AtomicBool::new(false);                       // c:201

/// Port of `int hlinesz` from Src/hist.c:206.
pub static hlinesz: AtomicI32 = AtomicI32::new(0);                           // c:206

/// Port of `static zlong defev` from Src/hist.c:210.
static defev: AtomicI64 = AtomicI64::new(0);                                 // c:210

/// Port of `static int hist_keep_comment` from Src/hist.c:217.
static hist_keep_comment: AtomicI32 = AtomicI32::new(0);                     // c:217

/// Port of `static struct histfile_stats lasthist` from Src/hist.c:220-226.
#[allow(non_camel_case_types)]
pub struct histfile_stats {                                                  // c:220
    pub text: Option<String>,                                                // c:221
    pub stim: i64,                                                           // c:222 time_t
    pub mtim: i64,                                                           // c:222
    pub fpos: i64,                                                           // c:223 off_t
    pub fsiz: i64,                                                           // c:223
    pub interrupted: i32,                                                    // c:224
    pub next_write_ev: i64,                                                  // c:225 zlong
}
static lasthist: Mutex<histfile_stats> = Mutex::new(histfile_stats {         // c:226
    text: None, stim: 0, mtim: 0, fpos: 0, fsiz: 0,
    interrupted: 0, next_write_ev: 0,
});

/// Port of `static struct histsave` from Src/hist.c:228-238.
#[allow(non_camel_case_types)]
pub struct histsave {                                                        // c:228
    pub lasthist: histfile_stats,                                            // c:229
    pub histfile: Option<String>,                                            // c:230
    pub hist_ring: Vec<histent>,                                             // c:232
    pub curhist: i64,                                                        // c:233 zlong
    pub histlinect: i64,                                                     // c:234
    pub histsiz: i64,                                                        // c:235
    pub savehistsiz: i64,                                                    // c:236
    pub locallevel: i32,                                                     // c:237
}

/// Port of `static struct histsave *histsave_stack` from Src/hist.c:238.
#[allow(clippy::vec_init_then_push)]
static histsave_stack: Mutex<Vec<histsave>> = Mutex::new(Vec::new());        // c:238

/// Port of `static int histsave_stack_size` from Src/hist.c:239.
static histsave_stack_size: AtomicI32 = AtomicI32::new(0);                   // c:239

/// Port of `static int histsave_stack_pos` from Src/hist.c:240.
static histsave_stack_pos: AtomicI32 = AtomicI32::new(0);                    // c:240

/// Port of `static zlong histfile_linect` from Src/hist.c:242.
static histfile_linect: AtomicI64 = AtomicI64::new(0);                       // c:242

// =========================================================================
// Externs from other C files used in hist.c
// =========================================================================

/// Port of `int stophist` (extern from zsh.h, owned by other C files).
/// Track history-stop depth here so the hist module's save/restore work.
pub static stophist: AtomicI32 = AtomicI32::new(0);

/// Port of `zlong curhist` (extern). Current history event number.
pub static curhist: AtomicI64 = AtomicI64::new(0);

/// Port of `zlong histlinect` (extern). Number of entries currently in ring.
pub static histlinect: AtomicI64 = AtomicI64::new(0);

/// Port of `char bangchar` from `Src/params.c:130`. History expansion
/// lead character (`!` by default).
pub static bangchar: AtomicI32 = AtomicI32::new(b'!' as i32);

/// Resolve the active history file path. Mirrors C's
/// `getsparam("HISTFILE")` lookup used inside `lockhistfile()` (c:3188)
/// and `readhistfile()` / `savehistfile()` when their `fn` arg is NULL.
fn resolve_histfile() -> Option<String> {
    std::env::var("HISTFILE").ok()
}

/// Port of `int lockhistct` from Src/hist.c. Re-entrant lock counter.
static lockhistct: AtomicI32 = AtomicI32::new(0);

/// Port of `int lexstop` (extern from lex.c) — used by ihgetc/histsubchar.
pub static lexstop: AtomicBool = AtomicBool::new(false);

/// Port of `int exit_pending` (extern). Set by SIGINT/`exit` builtin.
pub static exit_pending: AtomicBool = AtomicBool::new(false);

/// Port of `int strin` from Src/hist.c — counts nested string-input
/// scopes (eval/source/here-string).
static strin: AtomicI32 = AtomicI32::new(0);

// =========================================================================
// HIST_* flags (from zsh.h)
// =========================================================================

/// Port of `HIST_OLD` from Src/zsh.h. Entry came from the history file.
pub const HIST_OLD: u32 = 1 << 0;
/// Port of `HIST_DUP` from Src/zsh.h.
pub const HIST_DUP: u32 = 1 << 1;
/// Port of `HIST_FOREIGN` from Src/zsh.h.
pub const HIST_FOREIGN: u32 = 1 << 2;
/// Port of `HIST_TMPSTORE` from Src/zsh.h.
pub const HIST_TMPSTORE: u32 = 1 << 3;
/// Port of `HIST_NOWRITE` from Src/zsh.h.
pub const HIST_NOWRITE: u32 = 1 << 4;

// =========================================================================
// CASMOD_ enum (port of zsh.h:3122-3127)
// =========================================================================

/// Port of `enum { CASMOD_NONE, CASMOD_UPPER, CASMOD_LOWER, CASMOD_CAPS }`
/// from Src/zsh.h:3122.
// =========================================================================
// HISTFLAG_* (port of zsh.h)
// =========================================================================

/// Port of `HISTFLAG_DONE` from Src/zsh.h.
pub const HISTFLAG_DONE: i32 = 1;
/// Port of `HISTFLAG_NOEXEC` from Src/zsh.h.
pub const HISTFLAG_NOEXEC: i32 = 2;
/// Port of `HISTFLAG_RECALL` from Src/zsh.h.
pub const HISTFLAG_RECALL: i32 = 4;
/// Port of `HISTFLAG_SETTY` from Src/zsh.h.
pub const HISTFLAG_SETTY: i32 = 8;

// =========================================================================
// Helper inline accessors for the ring (private — match C internal use)
// =========================================================================

fn ring_get(ev: i64) -> Option<histent> {
    let ring = hist_ring.lock().unwrap();
    for h in ring.iter() {
        if h.histnum == ev {
            return Some(clone_histent(h));
        }
    }
    None
}

fn clone_histent(h: &histent) -> histent {
    histent {
        node: crate::ported::zsh_h::hashnode {
            next: None,
            nam: h.node.nam.clone(),
            flags: h.node.flags,
        },
        up: None,
        down: None,
        zle_text: h.zle_text.clone(),
        stim: h.stim,
        ftim: h.ftim,
        words: h.words.clone(),
        nwords: h.nwords,
        histnum: h.histnum,
    }
}

fn ring_position(ev: i64) -> Option<usize> {
    hist_ring.lock().unwrap().iter().position(|h| h.histnum == ev)
}

fn ring_at(idx: usize) -> i64 {
    hist_ring.lock().unwrap()[idx].histnum
}

fn ring_len() -> usize {
    hist_ring.lock().unwrap().len()
}

fn ring_oldest() -> Option<i64> {
    hist_ring.lock().unwrap().last().map(|h| h.histnum)
}

fn ring_latest() -> Option<histent> {
    hist_ring.lock().unwrap().first().map(clone_histent)
}

// =========================================================================
// Functions from hist.c
// =========================================================================

/// Port of `void hist_context_save(struct hist_stack *hs, int toplevel)`
/// from Src/hist.c:248.
pub fn hist_context_save(hs: &mut crate::ported::zsh_h::hist_stack, toplevel: i32) { // c:248
    if toplevel != 0 {                                                       // c:250
        // top level, make this version visible to ZLE                       // c:251
        *zle_chline.lock().unwrap() = Some(chline.lock().unwrap().clone());  // c:252
        // ensure line stored is NULL-terminated — implicit in String        // c:253-255
    }
    hs.histactive = histactive.load(Ordering::SeqCst) as i32;                // c:257
    hs.histdone = histdone.load(Ordering::SeqCst);                           // c:258
    hs.stophist = stophist.load(Ordering::SeqCst);                           // c:259
    hs.hline = Some(chline.lock().unwrap().clone());                         // c:260
    hs.hptr = Some(hptr.load(Ordering::SeqCst).to_string());                 // c:261
    hs.chwords = chwords.lock().unwrap().clone();                            // c:262
    hs.chwordlen = chwordlen.load(Ordering::SeqCst);                         // c:263
    hs.chwordpos = chwordpos.load(Ordering::SeqCst);                         // c:264
    // hs->hgetc / hungetc / hwaddc / hwbegin / hwabort / hwend / addtoline  // c:265-271
    // are runtime-mutable function-pointer globals in C; the Rust port
    // dispatches statically via crate::ported::input.
    hs.hlinesz = hlinesz.load(Ordering::SeqCst);                             // c:272
    hs.defev = defev.load(Ordering::SeqCst);                                 // c:273
    hs.hist_keep_comment = hist_keep_comment.load(Ordering::SeqCst);         // c:274
    // hs->cstack = cmdstack; hs->csp = cmdsp;                               // c:281-282
    hs.csp = 0;

    stophist.store(0, Ordering::SeqCst);                                     // c:284
    chline.lock().unwrap().clear();                                          // c:285
    hptr.store(0, Ordering::SeqCst);                                         // c:286
    histactive.store(0, Ordering::SeqCst);                                   // c:287
    // cmdstack = zalloc(CMDSTACKSZ); cmdsp = 0;                             // c:288-289
}

/// Port of `void hist_context_restore(const struct hist_stack *hs, int toplevel)`
/// from Src/hist.c:296.
pub fn hist_context_restore(hs: &crate::ported::zsh_h::hist_stack, toplevel: i32) { // c:296
    if toplevel != 0 {                                                       // c:298
        // Back to top level: don't need special ZLE value                   // c:299
        // DPUTS(hs->hline != zle_chline, "BUG: Ouch, wrong chline for ZLE") // c:300
        *zle_chline.lock().unwrap() = None;                                  // c:301
    }
    histactive.store(hs.histactive as u32, Ordering::SeqCst);                // c:303
    histdone.store(hs.histdone, Ordering::SeqCst);                           // c:304
    stophist.store(hs.stophist, Ordering::SeqCst);                           // c:305
    *chline.lock().unwrap() = hs.hline.clone().unwrap_or_default();          // c:306
    hptr.store(hs.hptr.as_ref().and_then(|s| s.parse().ok()).unwrap_or(0),   // c:307
               Ordering::SeqCst);
    *chwords.lock().unwrap() = hs.chwords.clone();                           // c:308
    chwordlen.store(hs.chwordlen, Ordering::SeqCst);                         // c:309
    chwordpos.store(hs.chwordpos, Ordering::SeqCst);                         // c:310
    // hgetc / hungetc / hwaddc / hwbegin / hwabort / hwend / addtoline      // c:311-317
    hlinesz.store(hs.hlinesz, Ordering::SeqCst);                             // c:318
    defev.store(hs.defev, Ordering::SeqCst);                                 // c:319
    hist_keep_comment.store(hs.hist_keep_comment, Ordering::SeqCst);         // c:320
    // cmdstack = hs->cstack; cmdsp = hs->csp;                               // c:323-324
}

/// Port of `void hist_in_word(int yesno)` from Src/hist.c.
pub fn hist_in_word(yesno: i32) {
    if yesno != 0 {
        histactive.fetch_or(HA_INWORD, Ordering::SeqCst);
    } else {
        histactive.fetch_and(!HA_INWORD, Ordering::SeqCst);
    }
}

/// Port of `int hist_is_in_word(void)` from Src/hist.c.
pub fn hist_is_in_word() -> i32 {
    if (histactive.load(Ordering::SeqCst) & HA_INWORD) != 0 { 1 } else { 0 }
}

/// Port of `void herrflush(void)` from Src/hist.c.
pub fn herrflush() {                                                         // c:477
    // Reset history error flags - in zsh this clears the input buffer
    // accumulated during the failed expansion.
}

/// Port of `int digitcount(char *s)` from Src/hist.c.
pub fn digitcount(s: &str) -> usize {                                        // c:574
    s.chars().take_while(|c| c.is_ascii_digit()).count()
}

/// Port of `static void nohw(UNUSED(int c))` from Src/hist.c:1062.
pub fn nohw(_c: i32) { /* do nothing */ }                                    // c:1062

/// Port of `static void nohwabort(void)` from Src/hist.c:1067.
pub fn nohwabort() { /* do nothing */ }                                      // c:1067

/// Port of `static void nohwe(void)` from Src/hist.c:1072.
pub fn nohwe() { /* do nothing */ }                                          // c:1072

/// Port of `void ihwbegin(int offset)` from Src/hist.c.
pub fn ihwbegin(offset: i32) {                                               // c:1656
    let stop = stophist.load(Ordering::SeqCst);
    let active = histactive.load(Ordering::SeqCst);
    if stop == 2 || (active & HA_INWORD) != 0 {
        return;
    }
    let pos = chwordpos.load(Ordering::SeqCst);
    if pos % 2 != 0 {
        chwordpos.fetch_sub(1, Ordering::SeqCst);
    }
    let start = (chline.lock().unwrap().len() as i32 + offset).max(0) as i16;
    let mut words = chwords.lock().unwrap();
    let idx = chwordpos.load(Ordering::SeqCst) as usize;
    if words.len() <= idx {
        words.resize(idx + 1, 0);
    }
    words[idx] = start;
    chwordpos.fetch_add(1, Ordering::SeqCst);
}

/// Port of `void ihwabort(void)` from Src/hist.c.
pub fn ihwabort() {                                                          // c:1675
    let pos = chwordpos.load(Ordering::SeqCst);
    if pos % 2 != 0 {
        chwordpos.fetch_sub(1, Ordering::SeqCst);
    }
    hist_keep_comment.store(1, Ordering::SeqCst);
}

/// Port of `void ihwend(void)` from Src/hist.c.
pub fn ihwend() {                                                            // c:1686
    let stop = stophist.load(Ordering::SeqCst);
    let active = histactive.load(Ordering::SeqCst);
    if stop == 2 || (active & HA_INWORD) != 0 {
        return;
    }
    let pos = chwordpos.load(Ordering::SeqCst);
    if pos % 2 == 0 {
        return;
    }
    let cur = chline.lock().unwrap().len() as i16;
    let mut words = chwords.lock().unwrap();
    let start_idx = (pos - 1) as usize;
    if cur > words[start_idx] {
        let end_idx = pos as usize;
        if words.len() <= end_idx {
            words.resize(end_idx + 1, 0);
        }
        words[end_idx] = cur;
        chwordpos.fetch_add(1, Ordering::SeqCst);
    } else {
        chwordpos.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Port of `void ihwaddc(int c)` from Src/hist.c:357.
pub fn ihwaddc(c: i32) {                                                     // c:357
    chline.lock().unwrap().push(c as u8 as char);
}

/// Port of `void iaddtoline(int c)` from Src/hist.c:397.
pub fn iaddtoline(c: i32) {                                                  // c:397
    chline.lock().unwrap().push(c as u8 as char);
}

/// Port of `void safeinungetc(int c)` from Src/hist.c.
pub fn safeinungetc(_c: i32) {
    // The C version pushes back via inungetc; without the input-stream
    // port wired here the call site uses InputBuffer::inungetc directly.
}

/// Port of `Histent quietgethist(zlong ev)` from Src/hist.c.
pub fn quietgethist(ev: i64) -> Option<histent> {                            // c:2433
    ring_get(ev)
}

/// Port of `Histent gethist(zlong ev)` from Src/hist.c.
pub fn gethist(ev: i64) -> Option<histent> {                                 // c:2440
    let ret = quietgethist(ev);
    if ret.is_none() {
        herrflush();
        crate::ported::utils::zerr(&format!("no such event: {}", ev));
    }
    ret
}

/// Port of `Histent up_histent(Histent he)` from Src/hist.c.
pub fn up_histent(current: i64) -> Option<i64> {                             // c:1304
    let pos = ring_position(current)?;
    if pos + 1 >= ring_len() {
        None
    } else {
        Some(ring_at(pos + 1))
    }
}

/// Port of `Histent down_histent(Histent he)` from Src/hist.c.
pub fn down_histent(current: i64) -> Option<i64> {                           // c:1311
    let pos = ring_position(current)?;
    if pos == 0 {
        None
    } else {
        Some(ring_at(pos - 1))
    }
}

/// Port of `Histent gethistent(zlong ev, int nearmatch)` from Src/hist.c.
pub fn gethistent(ev: i64, nearmatch: i32) -> Option<i64> {                  // c:1318
    if ring_len() == 0 {
        return None;
    }
    if ring_get(ev).is_some() {
        return Some(ev);
    }
    if nearmatch == 0 {
        return None;
    }
    let mut best_older: Option<i64> = None;
    let mut best_newer: Option<i64> = None;
    for i in 0..ring_len() {
        let n = ring_at(i);
        if n < ev && best_older.map_or(true, |b| n > b) {
            best_older = Some(n);
        } else if n > ev && best_newer.map_or(true, |b| n < b) {
            best_newer = Some(n);
        }
    }
    if nearmatch < 0 { best_older } else { best_newer }
}

/// Port of `Histent prepnexthistent(void)` from Src/hist.c.
pub fn prepnexthistent() -> i64 {                                            // c:1387
    let cap = histsiz.load(Ordering::SeqCst);
    if histlinect.load(Ordering::SeqCst) >= cap {
        if let Some(oldest) = ring_oldest() {
            // Drop oldest from ring
            let mut ring = hist_ring.lock().unwrap();
            ring.retain(|h| h.histnum != oldest);
            histlinect.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let n = curhist.fetch_add(1, Ordering::SeqCst) + 1;
    n
}

/// Port of `int putoldhistentryontop(int keep_going)` from Src/hist.c.
pub fn putoldhistentryontop(_keep_going: i32) -> i32 {
    let mut ring = hist_ring.lock().unwrap();
    if let Some(oldest) = ring.last().map(|h| h.histnum) {
        let pos = ring.iter().position(|h| h.histnum == oldest).unwrap();
        let entry = ring.remove(pos);
        ring.insert(0, entry);
        return 1;
    }
    0
}

/// Port of `void resizehistents(void)` from Src/hist.c.
pub fn resizehistents() {
    let cap = histsiz.load(Ordering::SeqCst);
    while histlinect.load(Ordering::SeqCst) > cap {
        if let Some(oldest) = ring_oldest() {
            let mut ring = hist_ring.lock().unwrap();
            ring.retain(|h| h.histnum != oldest);
            histlinect.fetch_sub(1, Ordering::SeqCst);
        } else {
            break;
        }
    }
}

/// Port of `static void linkcurline(void)` from Src/hist.c:1079.
pub fn linkcurline() {                                                       // c:1079
    let new_hist = curhist.fetch_add(1, Ordering::SeqCst) + 1;               // c:1089 ++curhist
    let mut cur = curline.lock().unwrap();
    *cur = Some(make_histent(new_hist, String::new()));                      // c:1089 curline.histnum
    // Splicing into the ring (c:1081-1088) is encoded by the Vec::insert
    // at hist_ring index 0 done by hend() on commit. The sentinel itself
    // lives in `curline` until then.
}

/// Port of `static void unlinkcurline(void)` from Src/hist.c:1093.
pub fn unlinkcurline() {                                                     // c:1093
    *curline.lock().unwrap() = None;                                         // c:1095-1102
    curhist.fetch_sub(1, Ordering::SeqCst);                                  // c:1103
}

/// Port of `Histent movehistent(Histent he, int n, int xflags)` from Src/hist.c.
pub fn movehistent(start: i64, mut n: i32, xflags: u32) -> Option<i64> {
    let mut cur = start;
    while n < 0 {
        cur = up_histent(cur)?;
        if let Some(e) = ring_get(cur) {
            if (e.node.flags as u32 & xflags) == 0 {
                n += 1;
            }
        }
    }
    while n > 0 {
        cur = down_histent(cur)?;
        if let Some(e) = ring_get(cur) {
            if (e.node.flags as u32 & xflags) == 0 {
                n -= 1;
            }
        }
    }
    Some(cur)
}

/// Port of `int hcomsearch(char *str)` from Src/hist.c.
pub fn hcomsearch(prefix: &str) -> Option<i64> {
    let mut cur = curhist.load(Ordering::SeqCst);
    while let Some(prev) = up_histent(cur) {
        cur = prev;
        if let Some(entry) = ring_get(cur) {
            if (entry.node.flags as u32 & HIST_FOREIGN) != 0 {
                continue;
            }
            if entry.node.nam.starts_with(prefix) {
                return Some(cur);
            }
        }
    }
    None
}

/// Port of `int hconsearch(char *str, int *back)` from Src/hist.c.
pub fn hconsearch(needle: &str, start: Option<i64>) -> Option<i64> {
    let mut cur = start.unwrap_or_else(|| curhist.load(Ordering::SeqCst));
    while let Some(prev) = up_histent(cur) {
        cur = prev;
        if let Some(entry) = ring_get(cur) {
            if (entry.node.flags as u32 & HIST_FOREIGN) != 0 {
                continue;
            }
            if entry.node.nam.contains(needle) {
                return Some(cur);
            }
        }
    }
    None
}

/// Port of `int checkcurline(void)` from Src/hist.c.
pub fn checkcurline(line: &str) -> i32 {
    if let Some(latest) = ring_latest() {
        if latest.node.nam == line { 1 } else { 0 }
    } else {
        0
    }
}

/// Port of `void hbegin(int dohist)` from Src/hist.c:1110.
pub fn hbegin(dohist: i32) {                                                 // c:1110
    use crate::ported::options::opt_state_get;

    // isfirstln/isfirstch live on ZshLexer in zshrs_parse, not as
    // globals — caller resets them via lexer instance API.            // c:1114

    crate::ported::utils::set_errflag(                                       // c:1115
        crate::ported::utils::errflag() & !crate::ported::utils::ERRFLAG_ERROR);
    histdone.store(0, Ordering::SeqCst);                                     // c:1116
    let interact = opt_state_get("interactive").unwrap_or(false);
    let shinstdin = opt_state_get("shinstdin").unwrap_or(false);
    if dohist == 0 {                                                         // c:1117
        stophist.store(2, Ordering::SeqCst);                                 // c:1118
    } else if dohist != 2 {                                                  // c:1119
        stophist.store(if !interact || !shinstdin { 2 } else { 0 },          // c:1120
            Ordering::SeqCst);
    } else {                                                                 // c:1121
        stophist.store(0, Ordering::SeqCst);                                 // c:1122
    }

    if stophist.load(Ordering::SeqCst) == 2 {                                // c:1134
        chline.lock().unwrap().clear();                                      // c:1135 chline = NULL
        hptr.store(0, Ordering::SeqCst);                                     // c:1135 hptr = NULL
        hlinesz.store(0, Ordering::SeqCst);                                  // c:1136
        chwords.lock().unwrap().clear();                                     // c:1137
        chwordlen.store(0, Ordering::SeqCst);                                // c:1138
        // hgetc/hungetc/hwaddc/hwbegin/hwabort/hwend/addtoline are       c:1139-1145
        // function-pointer slots in C; Rust dispatches statically.
    } else {                                                                 // c:1146
        let mut buf = chline.lock().unwrap();                                // c:1147
        buf.clear();
        buf.reserve(64);
        hlinesz.store(64, Ordering::SeqCst);                                 // c:1147
        drop(buf);
        let mut w = chwords.lock().unwrap();                                 // c:1148
        w.clear();
        w.reserve(64);
        chwordlen.store(64, Ordering::SeqCst);
        drop(w);
        // hgetc/hungetc/hwaddc/hwbegin/hwabort/hwend/addtoline c:1149-1155 — see c:1139.
        if !opt_state_get("banghist").unwrap_or(true) {                      // c:1156
            stophist.store(4, Ordering::SeqCst);                             // c:1157
        }
    }
    chwordpos.store(0, Ordering::SeqCst);                                    // c:1159

    {                                                                        // c:1161
        let mut ring = hist_ring.lock().unwrap();
        if let Some(top) = ring.first_mut() {
            if top.ftim == 0 && strin.load(Ordering::SeqCst) == 0 {
                top.ftim = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);                                           // c:1162
            }
        }
    }
    if (dohist == 2 || (interact && shinstdin))                              // c:1163
        && strin.load(Ordering::SeqCst) == 0
    {
        histactive.store(HA_ACTIVE, Ordering::SeqCst);                       // c:1164
        // attachtty(mypgrp);                                                // c:1165 — TTY infra not ported here
        linkcurline();                                                       // c:1166
        defev.store(addhistnum(curhist.load(Ordering::SeqCst),               // c:1167
                               -1, HIST_FOREIGN as i32),
                    Ordering::SeqCst);
    } else {
        histactive.store(HA_ACTIVE | HA_NOINC, Ordering::SeqCst);            // c:1169
    }

    if opt_state_get("incappendhistorytime").unwrap_or(false)                // c:1189
        && !opt_state_get("sharehistory").unwrap_or(false)
        && !opt_state_get("incappendhistory").unwrap_or(false)
        && (histactive.load(Ordering::SeqCst) & HA_NOINC) == 0
        && strin.load(Ordering::SeqCst) == 0
        && histsave_stack_pos.load(Ordering::SeqCst) == 0
    {
        let hf = resolve_histfile();                                         // c:1192
        savehistfile(hf.as_deref(), 0                                        // c:1193
            | crate::ported::zsh_h::HFILE_USE_OPTIONS as i32
            | crate::ported::zsh_h::HFILE_FAST as i32);
    }
}

/// Port of `static int should_ignore_line(Eprog prog)` from Src/hist.c:1425.
fn should_ignore_line(prog: Option<&[u8]>) -> i32 {                          // c:1425
    use crate::ported::options::opt_state_get;
    let line = chline.lock().unwrap().clone();
    if opt_state_get("histignorespace").unwrap_or(false) {                   // c:1427
        if line.starts_with(' ') /* aliasspaceflag — alias state TBD */ {    // c:1428
            return 1;                                                        // c:1429
        }
    }
    if prog.is_none() {                                                      // c:1432
        return 0;                                                            // c:1433
    }
    if opt_state_get("histnofunctions").unwrap_or(false) {                   // c:1435
        // Inspecting an Eprog requires the wordcode VM port — leave the
        // funcdef detection to the executor; conservatively return 0.    // c:1436-1440
        return 0;
    }
    if opt_state_get("histnostore").unwrap_or(false) {                       // c:1443
        // getjobtext(prog, NULL) — text reconstruction also needs the
        // wordcode VM. Apply the simpler text-based filters on chline
        // for the cases the C code carves out.
        let mut b: &str = &line;
        let mut saw_builtin = false;
        if let Some(rest) = b.strip_prefix("builtin ") {                     // c:1446
            b = rest;
            saw_builtin = true;
        }
        if (b == "history" || b.starts_with("history "))                     // c:1451
            && (saw_builtin /* || shfunctab.getnode("history").is_none() */)
        {
            return 1;                                                        // c:1453
        }
        if (b == "r" || b.starts_with("r "))                                 // c:1454
            && (saw_builtin /* || shfunctab.getnode("r").is_none() */)
        {
            return 1;
        }
        if let Some(rest) = b.strip_prefix("fc -") {                         // c:1457
            if (saw_builtin /* || shfunctab.getnode("fc").is_none() */)
                && rest.chars().take_while(|c| c.is_ascii_alphabetic())
                       .any(|c| c == 'l')
            {
                return 1;                                                    // c:1462
            }
        }
    }
    0                                                                        // c:1467
}

/// Port of `int hend(Eprog prog)` from Src/hist.c:1474.
pub fn hend(prog: Option<&[u8]>) -> i32 {                                    // c:1474
    use crate::ported::options::opt_state_get;
    use crate::ported::zsh_h::{HFILE_FAST, HFILE_USE_OPTIONS};

    let stack_pos = histsave_stack_pos.load(Ordering::SeqCst);               // c:1476
    let mut save: i32 = 1;                                                   // c:1484
    let mut hookret: i32 = 0;

    // DPUTS(stophist != 2 && !(inbufflags & INP_ALIAS) && !chline,       c:1487
    //       "BUG: chline is NULL in hend()");
    crate::ported::signals::queue_signals();                                 // c:1489
    if (histdone.load(Ordering::SeqCst) & HISTFLAG_SETTY) != 0 {             // c:1490
        // settyinfo(&shttyinfo) — TTY-state singleton not ported.          // c:1491
    }
    let active = histactive.load(Ordering::SeqCst);
    if (active & HA_NOINC) == 0 {                                            // c:1492
        unlinkcurline();                                                     // c:1493
    }
    if (active & HA_NOINC) != 0 {                                            // c:1494
        chline.lock().unwrap().clear();                                      // c:1495 zfree(chline)
        chwords.lock().unwrap().clear();                                     // c:1496 zfree(chwords)
        hptr.store(0, Ordering::SeqCst);                                     // c:1497
        histactive.store(0, Ordering::SeqCst);                               // c:1499
        crate::ported::signals::unqueue_signals();                           // c:1500
        return 1;                                                            // c:1501
    }
    let cur_ignore_all = if opt_state_get("histignorealldups")               // c:1503
        .unwrap_or(false) { 1 } else { 0 };
    let prev_ignore_all = hist_ignore_all_dups.load(Ordering::SeqCst);
    if prev_ignore_all != cur_ignore_all                                     // c:1503
        && {
            hist_ignore_all_dups.store(cur_ignore_all, Ordering::SeqCst);    // c:1504
            cur_ignore_all != 0
        }
    {
        histremovedups();                                                    // c:1505
    }
    // *hptr = '\0';                                                         // c:1513 — String is implicit
    let chline_text = chline.lock().unwrap().clone();
    if !chline_text.is_empty() {                                             // c:1515
        let save_errflag = crate::ported::utils::errflag();                  // c:1517
        crate::ported::utils::set_errflag(0);                                // c:1518
        let args = vec!["zshaddhistory".to_string(), chline_text.clone()];   // c:1520-1521
        hookret = crate::ported::utils::callhookfunc(                        // c:1522
            "zshaddhistory", Some(&args), true);
        let new_errflag = (crate::ported::utils::errflag()                   // c:1524-1525
            & !crate::ported::utils::ERRFLAG_ERROR) | save_errflag;
        crate::ported::utils::set_errflag(new_errflag);
    }
    let hf = resolve_histfile();                                             // c:1528
    if opt_state_get("sharehistory").unwrap_or(false)                        // c:1529
        && lockhistfile(hf.as_deref(), 0) == 0
    {
        readhistfile(hf.as_deref(), 0,                                       // c:1530
            HFILE_USE_OPTIONS as i32 | HFILE_FAST as i32);
        // curline.histnum = curhist + 1                                     // c:1531
    }
    let flag = histdone.load(Ordering::SeqCst);                              // c:1533
    histdone.store(0, Ordering::SeqCst);                                     // c:1534
    let hptr_pos = hptr.load(Ordering::SeqCst);
    let mut text = chline_text;
    if hptr_pos < 1 {                                                        // c:1535 hptr < chline + 1
        save = 0;                                                            // c:1536
    } else {
        if text.ends_with('\n') {                                            // c:1538 hptr[-1] == '\n'
            if text.len() > 1 {                                              // c:1539 chline[1]
                text.pop();                                                  // c:1540 *--hptr = '\0'
                if hptr.load(Ordering::SeqCst) > 0 {
                    hptr.fetch_sub(1, Ordering::SeqCst);
                }
            } else {
                save = 0;                                                    // c:1542
            }
        }
        if chwordpos.load(Ordering::SeqCst) <= 2                             // c:1544
            && hist_keep_comment.load(Ordering::SeqCst) == 0
        {
            save = 0;                                                        // c:1545
        } else if should_ignore_line(prog) != 0 {                            // c:1546
            save = -1;                                                       // c:1547
        } else if hookret == 2 {                                             // c:1548
            save = -2;                                                       // c:1549
        } else if hookret != 0 {                                             // c:1550
            save = -1;                                                       // c:1551
        }
    }
    if (flag & (HISTFLAG_DONE | HISTFLAG_RECALL)) != 0 {                     // c:1553
        let ptr = text.clone();                                              // c:1556 ztrdup(chline)
        if (flag & (HISTFLAG_DONE | HISTFLAG_RECALL)) == HISTFLAG_DONE {     // c:1557
            // zputs(ptr, shout); fputc('\n', shout); fflush(shout);         // c:1558-1560
            print!("{}\n", ptr);
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
        if (flag & HISTFLAG_RECALL) != 0 {                                   // c:1562
            // zpushnode(bufstack, ptr) — bufstack push not yet wired here. // c:1563
            save = 0;                                                        // c:1564
        }
    }
    if save != 0 || text.starts_with(' ') {                                  // c:1568
        // Walk up the ring skipping HIST_FOREIGN entries; if the topmost
        // non-foreign entry is HIST_TMPSTORE, drop it.                     // c:1569-1576
        let mut ring = hist_ring.lock().unwrap();
        let mut idx: usize = 0;
        while idx < ring.len()
            && (ring[idx].node.flags as u32 & HIST_FOREIGN) != 0
        {
            idx += 1;
        }
        if idx < ring.len()
            && (ring[idx].node.flags as u32 & HIST_TMPSTORE) != 0
        {
            if idx == 0 {                                                    // c:1573 he == hist_ring
                curhist.fetch_sub(1, Ordering::SeqCst);                      // c:1574
            }
            ring.remove(idx);                                                // c:1575 freehistnode
            histlinect.fetch_sub(1, Ordering::SeqCst);
        }
    }
    if save != 0 {                                                           // c:1578
        // chwordpos parity guard — if odd, hwend() to close.            // c:1583-1587
        if chwordpos.load(Ordering::SeqCst) % 2 != 0 {
            ihwend();
        }
        // Strip trailing \n which we already nulled out.                    // c:1589-1595
        let cwp = chwordpos.load(Ordering::SeqCst);
        if cwp > 1 {
            let words = chwords.lock().unwrap();
            let last = words.get((cwp - 2) as usize).copied().unwrap_or(0);
            // C: !chline[chwords[chwordpos-2]] — index past end after NUL.
            if (last as usize) >= text.len() {                               // c:1590
                drop(words);
                chwordpos.fetch_sub(2, Ordering::SeqCst);
            } else {
                drop(words);
            }
            if opt_state_get("histreduceblanks").unwrap_or(false) {          // c:1593
                text = histreduceblanks(&text);                              // c:1594
            }
        }
        let newflags: u32 = if save == -1 { HIST_TMPSTORE }                  // c:1596-1601
            else if save == -2 { HIST_NOWRITE }
            else { 0 };
        let mut he_idx: Option<usize> = None;
        let mut overwrite_old: u32 = 0;
        if (opt_state_get("histignoredups").unwrap_or(false)
            || opt_state_get("histignorealldups").unwrap_or(false))
            && save > 0
        {
            let ring = hist_ring.lock().unwrap();
            if let Some(top) = ring.first() {
                if top.node.nam == text {                                    // c:1603 histstrcmp
                    overwrite_old = top.node.flags as u32 & HIST_OLD;        // c:1610
                    he_idx = Some(0);
                }
            }
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let cwp = chwordpos.load(Ordering::SeqCst);
        let chwords_snapshot: Vec<i16> = chwords.lock().unwrap().clone();
        let nwords = (cwp / 2) as i32;
        if let Some(0) = he_idx {                                            // c:1609 reuse top
            let mut ring = hist_ring.lock().unwrap();
            if let Some(top) = ring.first_mut() {
                top.node.nam = text.clone();                                 // c:1616
                top.stim = now;                                              // c:1617
                top.ftim = 0;                                                // c:1618
                top.node.flags = (newflags | overwrite_old) as i32;          // c:1619
                top.nwords = nwords;                                         // c:1621
                top.words = if cwp > 0 {
                    chwords_snapshot[..cwp as usize].to_vec()                // c:1622-1623
                } else {
                    Vec::new()
                };
            }
        } else {
            let n = prepnexthistent();                                       // c:1614
            let mut he = make_histent(n, text.clone());
            he.stim = now;
            he.ftim = 0;
            he.node.flags = newflags as i32;
            he.nwords = nwords;
            if cwp > 0 {
                he.words = chwords_snapshot[..cwp as usize].to_vec();
            }
            let mut ring = hist_ring.lock().unwrap();
            ring.insert(0, he);
            histlinect.fetch_add(1, Ordering::SeqCst);
            if (newflags & HIST_TMPSTORE) == 0 {                             // c:1625
                // addhistnode(histtab, he->node.nam, he) — hashtable wiring c:1626
                // routes through crate::ported::hashtable::addhistnode.
                crate::ported::hashtable::addhistnode(&text, n as i32);
            }
        }
    }
    chline.lock().unwrap().clear();                                          // c:1628 zfree(chline)
    chwords.lock().unwrap().clear();                                         // c:1629 zfree(chwords)
    hptr.store(0, Ordering::SeqCst);                                         // c:1630
    histactive.store(0, Ordering::SeqCst);                                   // c:1632

    let share = opt_state_get("sharehistory").unwrap_or(false);
    let do_inc = if share {
        histfileIsLocked() != 0                                              // c:1636
    } else {
        opt_state_get("incappendhistory").unwrap_or(false)                   // c:1637
            || (opt_state_get("incappendhistorytime").unwrap_or(false)       // c:1637
                && histsave_stack_pos.load(Ordering::SeqCst) != 0)           // c:1638
    };
    if do_inc {
        savehistfile(hf.as_deref(), 0                                        // c:1639
            | HFILE_USE_OPTIONS as i32
            | HFILE_FAST as i32);
    }
    unlockhistfile(hf.as_deref().unwrap_or(""));                             // c:1640

    while histsave_stack_pos.load(Ordering::SeqCst) > stack_pos {            // c:1645
        pophiststack();                                                      // c:1646
    }
    hist_keep_comment.store(0, Ordering::SeqCst);                            // c:1647
    crate::ported::signals::unqueue_signals();                               // c:1648
    if (flag & HISTFLAG_NOEXEC) != 0 || crate::ported::utils::errflag() != 0 {
        0                                                                    // c:1649
    } else {
        1
    }
}

/// Port of `void strinbeg(int dohist)` from Src/hist.c.
pub fn strinbeg(dohist: i32) {
    strin.fetch_add(1, Ordering::SeqCst);
    hbegin(dohist);
}

/// Port of `void strinend(void)` from Src/hist.c.
pub fn strinend() {
    hend(None);
    strin.fetch_sub(1, Ordering::SeqCst);
}

/// Port of `void inithist(void)` from Src/hist.c:2613.
pub fn inithist() {                                                          // c:2613
    histsiz.store(1000, Ordering::SeqCst);
    savehistsiz.store(1000, Ordering::SeqCst);
    curhist.store(0, Ordering::SeqCst);
    histlinect.store(0, Ordering::SeqCst);
}

/// Port of `int lockhistfile(char *fn, int keep_trying)` from Src/hist.c:3181.
pub fn lockhistfile(fn_path: Option<&str>, keep_trying: i32) -> i32 {        // c:3181
    let path: String = match fn_path {                                       // c:3188
        Some(p) => p.to_string(),
        None => match resolve_histfile() {
            Some(p) => p,
            None => return 1,                                                // c:3189
        },
    };
    if lockhistct.fetch_add(1, Ordering::SeqCst) > 0 {
        return 0;
    }
    let max_tries = if keep_trying != 0 { 30 } else { 1 };
    for attempt in 0..max_tries {
        if flockhistfile(&path) != 0 {
            return 0;
        }
        if attempt + 1 < max_tries {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    lockhistct.fetch_sub(1, Ordering::SeqCst);
    if keep_trying != 0 { 2 } else { 1 }
}

/// Port of `void unlockhistfile(char *fn)` from Src/hist.c.
pub fn unlockhistfile(path: &str) {
    let prev = lockhistct.fetch_sub(1, Ordering::SeqCst);
    if prev <= 0 {
        lockhistct.store(0, Ordering::SeqCst);
        return;
    }
    if prev == 1 {
        let lockpath = format!("{}.lock", path);
        let _ = std::fs::remove_file(&lockpath);
    }
}

/// Port of `int flockhistfile(char *fn)` from Src/hist.c.
pub fn flockhistfile(path: &str) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        if let Ok(file) = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(format!("{}.lock", path))
        {
            let fd = file.as_raw_fd();
            return unsafe { if libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) == 0 { 1 } else { 0 } };
        }
        0
    }
    #[cfg(not(unix))]
    { let _ = path; 1 }
}

/// Port of `int histfileIsLocked(void)` from Src/hist.c.
#[allow(non_snake_case)]
pub fn histfileIsLocked() -> i32 {
    if lockhistct.load(Ordering::SeqCst) > 0 { 1 } else { 0 }
}

/// Port of `int checklocktime(char *fn, time_t mtim)` from Src/hist.c.
pub fn checklocktime(path: &str, max_age_secs: u64) -> i32 {
    let lockfile = format!("{}.lock", path);
    if let Ok(meta) = std::fs::metadata(&lockfile) {
        if let Ok(modified) = meta.modified() {
            if let Ok(age) = modified.elapsed() {
                if age.as_secs() < max_age_secs { return 1; }
            }
        }
    }
    0
}

/// Port of `void readhistfile(char *fn, int err, int readflags)` from Src/hist.c:2675.
pub fn readhistfile(fn_path: Option<&str>, _err: i32, _readflags: i32) {     // c:2675
    let path: String = match fn_path {
        Some(p) => p.to_string(),
        None => match resolve_histfile() {
            Some(p) => p,
            None => return,
        },
    };
    let contents = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return,
    };
    if contents.is_empty() { return; }
    let _ = lockhistfile(Some(&path), 1);

    let mut current: Option<(i64, i64, String)> = None;
    for raw_line in contents.lines() {
        if let Some((stim, ftim, ref mut text)) = current {
            if text.ends_with('\\') {
                text.pop();
                text.push('\n');
                text.push_str(raw_line);
                current = Some((stim, ftim, text.clone()));
                continue;
            }
            let n = curhist.fetch_add(1, Ordering::SeqCst) + 1;
            let mut entry = make_histent(n, text.clone());
            entry.stim = stim;
            entry.ftim = ftim;
            entry.node.flags |= HIST_OLD as i32;
            hist_ring.lock().unwrap().insert(0, entry);
            histlinect.fetch_add(1, Ordering::SeqCst);
            current = None;
        }
        if let Some(rest) = raw_line.strip_prefix(": ") {
            if let Some((meta, text)) = rest.split_once(';') {
                if let Some((stim_s, dur_s)) = meta.split_once(':') {
                    let stim: i64 = stim_s.parse().unwrap_or(0);
                    let dur: i64 = dur_s.parse().unwrap_or(0);
                    let ftim = stim + dur;
                    current = Some((stim, ftim, text.to_string()));
                    continue;
                }
            }
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        current = Some((now, now, raw_line.to_string()));
    }
    if let Some((stim, ftim, text)) = current {
        let n = curhist.fetch_add(1, Ordering::SeqCst) + 1;
        let mut entry = make_histent(n, text);
        entry.stim = stim;
        entry.ftim = ftim;
        entry.node.flags |= HIST_OLD as i32;
        hist_ring.lock().unwrap().insert(0, entry);
        histlinect.fetch_add(1, Ordering::SeqCst);
    }
    unlockhistfile(&path);
    resizehistents();
}

/// Port of `void savehistfile(char *fn, int err, int writeflags)` from Src/hist.c:2922.
pub fn savehistfile(fn_path: Option<&str>, _writeflags: i32) {               // c:2922
    use std::io::Write;
    let path: String = match fn_path {
        Some(p) => p.to_string(),
        None => match resolve_histfile() {
            Some(p) => p,
            None => return,
        },
    };
    let _ = lockhistfile(Some(&path), 1);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .write(true).create(true).truncate(true).open(&path)
    {
        let cap = savehistsiz.load(Ordering::SeqCst).max(0) as usize;
        let ring = hist_ring.lock().unwrap();
        let mut count = 0;
        for entry in ring.iter().rev() {
            if cap > 0 && count >= cap { break; }
            let dur = entry.ftim.saturating_sub(entry.stim);
            let _ = writeln!(file, ": {}:{};{}", entry.stim, dur, entry.node.nam);
            count += 1;
        }
    }
    unlockhistfile(&path);
}

/// Port of `void readhistline(char *line, ...)` from Src/hist.c.
pub fn readhistline(line: &str) -> Option<histent> {
    let line = line.trim();
    if line.is_empty() { return None; }
    if let Some(rest) = line.strip_prefix(": ") {
        if let Some(semi) = rest.find(';') {
            let meta = &rest[..semi];
            let cmd = &rest[semi + 1..];
            let parts: Vec<&str> = meta.splitn(2, ':').collect();
            let timestamp = parts.first().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
            let mut entry = make_histent(0, cmd.to_string());
            entry.stim = timestamp;
            return Some(entry);
        }
    }
    Some(make_histent(0, line.to_string()))
}

/// WARNING FAKE
fn make_histent(num: i64, text: String) -> histent {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    histent {
        node: crate::ported::zsh_h::hashnode {
            next: None,
            nam: text,
            flags: 0,
        },
        up: None,
        down: None,
        zle_text: None,
        stim: now,
        ftim: now,
        words: Vec::new(),
        nwords: 0,
        histnum: num,
    }
}

/// Port of `int pushhiststack(char *hf, zlong hs, zlong shs, int level)` from Src/hist.c:3865.
pub fn pushhiststack(hf: Option<&str>, hs: i64, shs: i64, level: i32) {      // c:3865
    let snap = histsave {                                                    // c:3870
        lasthist: histfile_stats {
            text: None, stim: 0, mtim: 0, fpos: 0, fsiz: 0,
            interrupted: 0, next_write_ev: 0,
        },
        histfile: hf.map(|s| s.to_string()),                                 // c:3872 h->histfile = histfile
        hist_ring: std::mem::take(&mut *hist_ring.lock().unwrap()),          // c:3874 h->hist_ring = hist_ring
        curhist: curhist.load(Ordering::SeqCst),                             // c:3875 h->curhist = curhist
        histlinect: histlinect.load(Ordering::SeqCst),                       // c:3876
        histsiz: histsiz.load(Ordering::SeqCst),                             // c:3877
        savehistsiz: savehistsiz.load(Ordering::SeqCst),                     // c:3878
        locallevel: level,                                                   // c:3879
    };
    histsave_stack.lock().unwrap().push(snap);                               // c:3884
    histsave_stack_size.fetch_add(1, Ordering::SeqCst);
    histsave_stack_pos.fetch_add(1, Ordering::SeqCst);
    histsiz.store(hs, Ordering::SeqCst);                                     // c:3886
    savehistsiz.store(shs, Ordering::SeqCst);                                // c:3887
    curhist.store(0, Ordering::SeqCst);                                      // c:3885 curhist = histlinect = 0
    histlinect.store(0, Ordering::SeqCst);
    let _ = hf;
}

/// Port of `int pophiststack(void)` from Src/hist.c:3902.
pub fn pophiststack() {                                                      // c:3902
    if let Some(snap) = histsave_stack.lock().unwrap().pop() {
        *hist_ring.lock().unwrap() = snap.hist_ring;
        curhist.store(snap.curhist, Ordering::SeqCst);
        histlinect.store(snap.histlinect, Ordering::SeqCst);
        histsiz.store(snap.histsiz, Ordering::SeqCst);
        let _ = snap.histfile;                                               // restored via param system in C
        savehistsiz.store(snap.savehistsiz, Ordering::SeqCst);
        histsave_stack_size.fetch_sub(1, Ordering::SeqCst);
        histsave_stack_pos.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Port of `void saveandpophiststack(int writeflags)` from Src/hist.c.
pub fn saveandpophiststack(writeflags: i32) {
    savehistfile(None, writeflags);
    pophiststack();
}

/// Port of `char *chrealpath(char **pathptr)` from Src/hist.c.
pub fn chrealpath(path: &str) -> Option<String> {
    std::fs::canonicalize(path).ok().map(|p| p.to_string_lossy().into_owned())
}

/// Port of `char *chabspath(char **pathptr)` from Src/hist.c.
pub fn chabspath(input: &str) -> Option<String> {
    if input.is_empty() { return Some(String::new()); }
    let mut path = if !input.starts_with('/') {
        let cwd = std::env::current_dir().ok()?;
        let cwd_s = cwd.to_string_lossy().into_owned();
        if cwd_s.ends_with('/') { format!("{}{}", cwd_s, input) }
        else { format!("{}/{}", cwd_s, input) }
    } else {
        input.to_string()
    };
    let chars: Vec<char> = path.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' {
            out.push('/');
            i += 1;
            while i < chars.len() && chars[i] == '/' { i += 1; }
        } else if c == '.' && i + 1 < chars.len() && chars[i + 1] == '.'
            && (i + 2 == chars.len() || chars[i + 2] == '/')
        {
            if out.len() <= 1 {
                if out.is_empty() || out == ['/'] { return None; }
                out.push('.'); out.push('.');
            } else if out.len() >= 3 && &out[out.len() - 3..] == &['.', '.', '/'] {
                out.push('.'); out.push('.');
            } else {
                if out.last() == Some(&'/') && out.len() > 1 { out.pop(); }
                while out.last().map(|c| *c != '/').unwrap_or(false) { out.pop(); }
            }
            i += 2;
            if i < chars.len() && chars[i] == '/' { i += 1; }
        } else if c == '.' && (i + 1 == chars.len() || chars[i + 1] == '/') {
            i += 1;
            while i < chars.len() && chars[i] == '/' { i += 1; }
        } else {
            out.push(c); i += 1;
        }
    }
    while out.len() > 1 && out.last() == Some(&'/') { out.pop(); }
    path = out.into_iter().collect();
    if path.is_empty() { Some("/".to_string()) } else { Some(path) }
}

/// Port of `int bufferwords(LinkList list, char *buf, int *index, int flags)` from Src/hist.c.
pub fn bufferwords(line: &str, cursor_pos: usize) -> (Vec<String>, usize) {
    let words: Vec<String> = line.split_whitespace().map(String::from).collect();
    let mut pos = 0;
    let mut word_idx = 0;
    for (i, word) in line.split_whitespace().enumerate() {
        if let Some(start) = line[pos..].find(word) {
            let wstart = pos + start;
            let wend = wstart + word.len();
            if cursor_pos >= wstart && cursor_pos <= wend {
                word_idx = i;
                break;
            }
            pos = wend;
        }
    }
    (words, word_idx)
}

/// Port of `int histsplitwords(char *line, ...)` from Src/hist.c.
pub fn histsplitwords(line: &str) -> Vec<(usize, usize)> {
    let mut words = Vec::new();
    let mut in_word = false;
    let mut word_start = 0;
    let mut in_quote = false;
    let mut quote_char = '\0';
    for (i, c) in line.char_indices() {
        if in_quote {
            if c == quote_char { in_quote = false; }
            continue;
        }
        if c == '\'' || c == '"' {
            in_quote = true;
            quote_char = c;
            if !in_word { word_start = i; in_word = true; }
            continue;
        }
        if c.is_ascii_whitespace() {
            if in_word { words.push((word_start, i)); in_word = false; }
        } else if !in_word {
            word_start = i; in_word = true;
        }
    }
    if in_word { words.push((word_start, line.len())); }
    words
}

/// Port of `char *subst(...)` from Src/hist.c:2336.
pub fn subst(s: &str, in_pattern: &str, out_pattern: &str, global: bool) -> String {
    if in_pattern.is_empty() { return s.to_string(); }
    let mut anchor_start = false;
    let mut anchor_end = false;
    let mut pat = in_pattern;
    if let Some(rest) = pat.strip_prefix('#') { anchor_start = true; pat = rest; }
    if let Some(rest) = pat.strip_prefix('%') { anchor_end = true; pat = rest; }
    if pat.is_empty() { return s.to_string(); }
    let out_expanded = convamps(out_pattern, pat);
    if anchor_start && anchor_end {
        if s == pat { return out_expanded; }
        return s.to_string();
    }
    if anchor_start {
        if let Some(rest) = s.strip_prefix(pat) {
            return format!("{}{}", out_expanded, rest);
        }
        return s.to_string();
    }
    if anchor_end {
        if s.ends_with(pat) {
            let prefix_len = s.len() - pat.len();
            return format!("{}{}", &s[..prefix_len], out_expanded);
        }
        return s.to_string();
    }
    if global { s.replace(pat, &out_expanded) } else { s.replacen(pat, &out_expanded, 1) }
}

/// Port of `char *convamps(char *out, char *in)` from Src/hist.c.
fn convamps(out: &str, in_pattern: &str) -> String {
    let mut result = String::with_capacity(out.len());
    let mut chars = out.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() { result.push(next); chars.next(); }
        } else if c == '&' {
            result.push_str(in_pattern);
        } else {
            result.push(c);
        }
    }
    result
}

/// Port of `char *casemodify(char *str, int how)` from Src/hist.c:2196.
pub fn casemodify(s: &str, how: CaseMod) -> String {                         // c:2196
    let mut result = String::with_capacity(s.len());
    let mut nextupper = true;
    for c in s.chars() {
        let modified = match how {
            CaseMod::CASMOD_LOWER => c.to_lowercase().collect::<String>(),
            CaseMod::CASMOD_UPPER => c.to_uppercase().collect::<String>(),
            CaseMod::CASMOD_CAPS => {
                if !c.is_alphanumeric() {
                    nextupper = true;
                    c.to_string()
                } else if nextupper {
                    nextupper = false;
                    c.to_uppercase().collect::<String>()
                } else {
                    c.to_lowercase().collect::<String>()
                }
            }
            CaseMod::CASMOD_NONE => c.to_string(),
        };
        result.push_str(&modified);
    }
    result
}

/// Port of `char *remtpath(char **str, int count)` from Src/hist.c:2056.
pub fn remtpath(s: &str, count: i32) -> String {                             // c:2056
    let s = s.trim_end_matches('/');
    if s.is_empty() { return "/".to_string(); }
    if count == 0 {
        if let Some(pos) = s.rfind('/') {
            if pos == 0 { return "/".to_string(); }
            return s[..pos].trim_end_matches('/').to_string();
        }
        return ".".to_string();
    }
    let bytes = s.as_bytes();
    let mut remaining = count;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'/' {
            remaining -= 1;
            if remaining <= 0 {
                if i == 0 { return "/".to_string(); }
                return s[..i].to_string();
            }
            while i + 1 < bytes.len() && bytes[i + 1] == b'/' { i += 1; }
        }
        i += 1;
    }
    s.to_string()
}

/// Port of `char *remlpaths(char **str, int count)` from Src/hist.c:2152.
pub fn remlpaths(s: &str, count: i32) -> String {                            // c:2152
    let s = s.trim_end_matches('/');
    if s.is_empty() { return String::new(); }
    let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
    let n = if count == 0 { 1 } else { count as usize };
    let take_n = n.min(parts.len());
    if take_n == 0 { return String::new(); }
    parts.iter().rev().take(take_n).rev().copied().collect::<Vec<&str>>().join("/")
}

/// Port of `char *remtext(char **str)` from Src/hist.c:2122.
pub fn remtext(s: &str) -> String {                                          // c:2122
    if let Some(slash_pos) = s.rfind('/') {
        let after_slash = &s[slash_pos + 1..];
        if let Some(dot_pos) = after_slash.rfind('.') {
            if dot_pos > 0 {
                return format!("{}/{}", &s[..slash_pos], &after_slash[..dot_pos]);
            }
        }
        return s.to_string();
    }
    if let Some(dot_pos) = s.rfind('.') {
        if dot_pos > 0 { return s[..dot_pos].to_string(); }
    }
    s.to_string()
}

/// Port of `char *rembutext(char **str)` from Src/hist.c:2136.
pub fn rembutext(s: &str) -> String {                                        // c:2136
    if let Some(slash_pos) = s.rfind('/') {
        let after_slash = &s[slash_pos + 1..];
        if let Some(dot_pos) = after_slash.rfind('.') {
            return after_slash[dot_pos + 1..].to_string();
        }
        return String::new();
    }
    if let Some(dot_pos) = s.rfind('.') {
        return s[dot_pos + 1..].to_string();
    }
    String::new()
}

/// Port of `char *quotebreak(char **tr)` from Src/hist.c:2527.
pub fn quotebreak(s: &str) -> String {                                       // c:2527
    let mut result = String::with_capacity(s.len() + 10);
    result.push('\'');
    for c in s.chars() {
        if c == '\'' { result.push_str("'\\''"); }
        else if c.is_whitespace() {
            result.push('\''); result.push(c); result.push('\'');
        } else {
            result.push(c);
        }
    }
    result.push('\'');
    result
}

/// Port of `char *quote(char **tr)` from Src/hist.c.
pub fn quote(s: &str) -> String {
    let bytes: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(bytes.len() + 3);
    out.push('\'');
    let mut inquotes = false;
    let mut prev: char = '\0';
    for (i, &c) in bytes.iter().enumerate() {
        if c == '\'' {
            inquotes = !inquotes;
            out.push('\''); out.push('\\'); out.push('\''); out.push('\'');
        } else if c.is_whitespace() && !inquotes && prev != '\\' {
            out.push('\''); out.push(c); out.push('\'');
        } else {
            out.push(c);
        }
        prev = if i < bytes.len() { c } else { prev };
    }
    out.push('\'');
    out
}

/// Port of `int getargspec(int argc, int marg, int evset)` from Src/hist.c.
pub fn getargspec(argc: usize, c: char, marg: Option<usize>, evset: bool) -> Option<usize> {
    match c {
        '0' => Some(0),
        '1'..='9' => Some(c.to_digit(10).unwrap() as usize),
        '^' => Some(1),
        '$' => Some(argc),
        '%' => { if evset { return None; } marg }
        _ => None,
    }
}

/// Port of `char *histreduceblanks(void)` from Src/hist.c.
pub fn histreduceblanks(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !prev_space { result.push(' '); prev_space = true; }
        } else {
            result.push(c); prev_space = false;
        }
    }
    result.trim().to_string()
}

/// Port of `char *hgetline(void)` from Src/hist.c.
pub fn hgetline(entry: &histent) -> String {
    entry.node.nam.clone()
}

/// Port of `zlong addhistnum(zlong hl, int n, int xflags)` from Src/hist.c:1266.
pub fn addhistnum(hl: i64, mut n: i32, xflags: i32) -> i64 {                 // c:1266
    let dir: i32 = if n < 0 { -1 } else if n > 0 { 1 } else { 0 };           // c:1268
    let he = gethistent(hl, dir);                                            // c:1269
    let he = match he {
        None => return 0,                                                    // c:1271-1272
        Some(h) => h,
    };
    if he != hl {                                                            // c:1273
        n -= dir;                                                            // c:1274
    }
    let final_he = if n != 0 {                                               // c:1275
        movehistent(he, n, xflags as u32)                                    // c:1276
    } else {
        Some(he)
    };
    match final_he {                                                         // c:1277
        None => {
            if dir < 0 {                                                     // c:1278
                firsthist() - 1
            } else {
                curhist.load(Ordering::SeqCst) + 1
            }
        }
        Some(h) => h,                                                        // c:1279
    }
}

/// Port of `zlong firsthist(void)` from Src/hist.c.
pub fn firsthist() -> i64 {
    let ring = hist_ring.lock().unwrap();
    ring.last().map(|h| h.histnum).unwrap_or(1)
}

/// Port of `int hwrep(char *cmdstr, char *matchstr, char *prevcmd)` from Src/hist.c.
pub fn hwrep(entry: &histent, replacement: &str, word_idx: usize) -> String {
    let words: Vec<&str> = entry.node.nam.split_whitespace().collect();
    if word_idx >= words.len() { return entry.node.nam.clone(); }
    let mut new_words: Vec<String> = words.iter().map(|s| s.to_string()).collect();
    new_words[word_idx] = replacement.to_string();
    new_words.join(" ")
}

/// Port of `void histremovedups(void)` from Src/hist.c.
pub fn histremovedups() {
    let mut ring = hist_ring.lock().unwrap();
    let mut seen = std::collections::HashSet::new();
    ring.retain(|h| seen.insert(h.node.nam.clone()));
    let new_ct = ring.len() as i64;
    drop(ring);
    histlinect.store(new_ct, Ordering::SeqCst);
}

/// Port of `int getargc(Histent ehist)` from Src/hist.c.
pub fn getargc(entry: &histent) -> usize {
    entry.nwords as usize
}

/// Port of `void substfailed(void)` from Src/hist.c.
pub fn substfailed() {
    crate::ported::utils::zerr("substitution failed");
}

/// Port of `int hwget(char **startptr)` from Src/hist.c.
pub fn hwget() -> Option<(i32, String)> {
    let pos = chwordpos.load(Ordering::SeqCst);
    if pos == 0 || pos % 2 != 0 { return None; }
    let words = chwords.lock().unwrap();
    let start_idx = (pos - 2) as usize;
    let end_idx = (pos - 1) as usize;
    if end_idx >= words.len() { return None; }
    let start = words[start_idx];
    let end = words[end_idx];
    let line = chline.lock().unwrap();
    let s = (start.max(0)) as usize;
    let e = (end.max(0) as usize).min(line.len());
    if s > e || s >= line.len() { return None; }
    Some((start as i32, line[s..e].to_string()))
}

/// Port of `int histbackword(void)` from Src/hist.c.
pub fn histbackword(line: &str, pos: usize) -> usize {
    if pos == 0 { return 0; }
    let bytes = line.as_bytes();
    let mut p = pos.min(bytes.len());
    while p > 0 && bytes[p - 1].is_ascii_whitespace() { p -= 1; }
    while p > 0 && !bytes[p - 1].is_ascii_whitespace() { p -= 1; }
    p
}

/// Port of `char *hdynread(int stop)` from Src/hist.c.
pub fn hdynread(_stop: i32) -> Option<String> {
    None
}

/// Port of `char *hdynread2(int stop)` from Src/hist.c.
pub fn hdynread2(stop: char, input: &str) -> (String, usize) {
    let mut out = String::new();
    let mut consumed = 0usize;
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        consumed += c.len_utf8();
        if c == stop || c == '\n' {
            if c == '\n' { consumed -= c.len_utf8(); }
            return (out, consumed);
        }
        if c == '\\' {
            if let Some(esc) = chars.next() {
                consumed += esc.len_utf8();
                out.push(esc);
            }
        } else {
            out.push(c);
        }
    }
    (out, consumed)
}

/// Port of `char *getargs(Histent ehist, int arg1, int arg2)` from Src/hist.c.
pub fn getargs(entry: &histent, arg1: usize, arg2: usize) -> Option<String> {
    let nwords = entry.words.len() / 2;
    if nwords == 0 || arg2 < arg1 || arg1 >= nwords || arg2 >= nwords {
        herrflush();
        crate::ported::utils::zerr("no such word in event");
        return None;
    }
    if arg1 == 0 && arg2 == nwords - 1 {
        return Some(entry.node.nam.clone());
    }
    let pos1 = entry.words.get(arg1 * 2).copied().unwrap_or(0) as usize;
    let pos2 = entry.words.get(arg2 * 2 + 1).copied().unwrap_or(0) as usize;
    if pos2 > entry.node.nam.len() || pos1 > pos2 {
        herrflush();
        crate::ported::utils::zerr("history event too long, can't index requested words");
        return None;
    }
    Some(entry.node.nam[pos1..pos2].to_string())
}
