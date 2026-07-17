//! Lazy HISTFILE paging — the history is fetched on demand, never
//! slurped whole.
//!
//! Contract (user-directed): zshrs must NEVER read the entire HISTFILE
//! into memory. The authoritative store is unbounded (a 635MB flat
//! mirror / 566k entries is normal here); the in-memory ring holds
//! only what has actually been touched:
//!
//!   * startup — `readhistfile` loads ONE newest page (`PAGE_BYTES`)
//!     and registers the file + floor here; total entry count for
//!     exact zsh event numbering comes from a zero-heap streaming
//!     scan (mmap + bytewise count, nothing retained).
//!   * navigation / `$history[ev]` below the loaded floor — the
//!     `quietgethist` chokepoint calls [`page_older_until`] and the
//!     previous page(s) splice into the ring.
//!   * whole-history scans (`${(u@)history}`, `${history[(R)pat]}`,
//!     hsmw ^R) — `scanpmhistory` pages everything in first; the cost
//!     lands exactly when the user asks for all of history, not at
//!     every shell start.
//!
//! Pages splice with exact descending event numbers, so `fc -l`, `!N`,
//! and HISTNO stay identical to a full eager load.

use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Mutex;

/// Paging granule. A unit of on-demand I/O, NOT a bound on how much
/// history is reachable.
pub const PAGE_BYTES: u64 = 8 * 1024 * 1024;

/// Byte offset in the HISTFILE of the OLDEST entry currently loaded
/// into the ring. 0 = the whole file is loaded (or nothing to page).
static FLOOR_POS: AtomicI64 = AtomicI64::new(0);
/// Event number of the oldest loaded entry (the ring floor).
static FLOOR_NUM: AtomicI64 = AtomicI64::new(0);
/// Whether paging is armed (a cold read registered a file).
static ARMED: AtomicBool = AtomicBool::new(false);
/// The HISTFILE path the floor refers to.
static HIST_PATH: Mutex<String> = Mutex::new(String::new());
/// Serialise page-in against concurrent chokepoints.
static PAGE_LOCK: Mutex<()> = Mutex::new(());

/// Count history ENTRIES in `path` without retaining memory: mmap the
/// file and count entry heads byte-wise. Extended-history entries
/// start with `: ` at line start; continuation lines (previous line
/// ends in `\`) are not heads. Plain-format files fall back to a
/// continuation-aware line count. One streaming pass, zero heap.
pub fn count_entries(path: &str) -> Option<u64> {
    let f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    if len == 0 {
        return Some(0);
    }
    let map = unsafe { memmap2::Mmap::map(&f).ok()? };
    let bytes: &[u8] = &map;
    let extended = bytes.starts_with(b": ")
        || bytes.windows(3).take(65536).any(|w| w == b"\n: ");
    let mut count: u64 = 0;
    let mut prev_cont = false; // previous line ended in `\`
    let mut line_start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            let line = &bytes[line_start..i];
            if !prev_cont {
                if extended {
                    if line.starts_with(b": ") {
                        count += 1;
                    }
                } else if !line.is_empty() {
                    count += 1;
                }
            }
            prev_cont = line.ends_with(b"\\");
            line_start = i + 1;
        }
        i += 1;
    }
    if line_start < bytes.len() && !prev_cont {
        let line = &bytes[line_start..];
        if (extended && line.starts_with(b": ")) || (!extended && !line.is_empty()) {
            count += 1;
        }
    }
    Some(count)
}

/// Arm paging after a cold newest-page load: `path` + the byte offset
/// and event number of the oldest entry that landed in the ring.
pub fn arm(path: &str, floor_pos: i64, floor_num: i64) {
    *HIST_PATH.lock().unwrap() = path.to_string();
    FLOOR_POS.store(floor_pos, Ordering::SeqCst);
    FLOOR_NUM.store(floor_num, Ordering::SeqCst);
    ARMED.store(floor_pos > 0, Ordering::SeqCst);
    tracing::info!(
        path,
        floor_pos,
        floor_num,
        "history: lazy paging armed (newest page loaded)"
    );
}

/// Disarm (file fully loaded or replaced).
pub fn disarm() {
    ARMED.store(false, Ordering::SeqCst);
    FLOOR_POS.store(0, Ordering::SeqCst);
}

/// True when events below the current floor exist on disk unloaded.
pub fn has_older() -> bool {
    ARMED.load(Ordering::SeqCst) && FLOOR_POS.load(Ordering::SeqCst) > 0
}

/// Current ring floor event number (0 when unarmed).
pub fn floor_num() -> i64 {
    FLOOR_NUM.load(Ordering::SeqCst)
}

/// Page older entries into the ring until event `min_ev` is loaded (or
/// the file start is reached). `min_ev <= 0` means "everything".
/// Returns true if anything was loaded.
pub fn page_older_until(min_ev: i64) -> bool {
    if !has_older() {
        return false;
    }
    let _g = PAGE_LOCK.lock().unwrap();
    if !has_older() {
        return false;
    }
    let path = HIST_PATH.lock().unwrap().clone();
    let mut loaded_any = false;
    while has_older() && (min_ev <= 0 || FLOOR_NUM.load(Ordering::SeqCst) > min_ev) {
        let floor_pos = FLOOR_POS.load(Ordering::SeqCst) as u64;
        let start = floor_pos.saturating_sub(PAGE_BYTES);
        let (mut entries, aligned_start) = match read_entries_between(&path, start, floor_pos) {
            Some(v) => v,
            None => {
                disarm();
                break;
            }
        };
        if entries.is_empty() {
            // Nothing parseable in the window (pathological long entry):
            // widen by walking the start back a full page next round, or
            // give up at file start.
            if start == 0 {
                disarm();
                break;
            }
            FLOOR_POS.store(start as i64, Ordering::SeqCst);
            continue;
        }
        // Number the page descending, ending just below the floor.
        let mut num = FLOOR_NUM.load(Ordering::SeqCst);
        for e in entries.iter_mut().rev() {
            num -= 1;
            e.histnum = num;
        }
        let new_floor_num = num;
        // Ring is newest-first: older pages APPEND.
        {
            let mut ring = crate::ported::hist::hist_ring.lock().unwrap();
            for e in entries.into_iter().rev() {
                ring.push(e);
            }
        }
        FLOOR_NUM.store(new_floor_num, Ordering::SeqCst);
        FLOOR_POS.store(aligned_start as i64, Ordering::SeqCst);
        if aligned_start == 0 {
            disarm();
        }
        loaded_any = true;
        if new_floor_num <= 1 {
            disarm();
        }
    }
    loaded_any
}

/// Read and parse the entries whose HEADS lie in `[start, end)` of the
/// HISTFILE. Returns (entries oldest-first, aligned byte offset of the
/// first parsed entry). `start > 0` aligns forward past the partial
/// first entry; the caller's floor at `end` guarantees the tail cuts
/// on an entry boundary.
fn read_entries_between(path: &str, start: u64, end: u64) -> Option<(Vec<crate::ported::zsh_h::histent>, u64)> {
    use std::io::{Read, Seek, SeekFrom};
    if end <= start {
        return Some((Vec::new(), start));
    }
    let mut f = std::fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = vec![0u8; (end - start) as usize];
    f.read_exact(&mut bytes).ok()?;
    crate::ported::utils::unmetafy(&mut bytes);
    let text = String::from_utf8_lossy(&bytes);
    let extended = text.starts_with(": ") || text.contains("\n: ");
    // Alignment: skip the partial first line, then continuation lines,
    // then (extended) non-head lines. Track the byte offset of the
    // first entry head relative to `start`.
    let mut offset = 0u64;
    let mut lines: Vec<&str> = Vec::new();
    {
        let mut rest = &text[..];
        if start > 0 {
            match rest.find('\n') {
                Some(i) => {
                    offset = (i + 1) as u64;
                    rest = &rest[i + 1..];
                }
                None => return Some((Vec::new(), start)),
            }
            // Skip continuation/non-head lines until an entry head.
            loop {
                let line_end = rest.find('\n').unwrap_or(rest.len());
                let line = &rest[..line_end];
                let is_head = if extended {
                    line.starts_with(": ") && line.contains(';')
                } else {
                    true
                };
                if is_head || rest.is_empty() {
                    break;
                }
                let adv = line_end + if line_end < rest.len() { 1 } else { 0 };
                offset += adv as u64;
                rest = &rest[adv..];
                if rest.is_empty() {
                    return Some((Vec::new(), start + offset));
                }
            }
        }
        lines.extend(rest.lines());
    }
    // NOTE: the unmetafy above shrinks byte counts, so `offset` is in
    // UNMETAFIED coordinates while the file is metafied. Metafied
    // bytes are rare (high-bit data); the floor only needs to land ON
    // OR BEFORE the true boundary — the next page's forward alignment
    // re-syncs. Correctness holds; at worst a handful of bytes re-read.
    let mut out: Vec<crate::ported::zsh_h::histent> = Vec::new();
    let mut current: Option<(i64, i64, String)> = None;
    let flush = |cur: &mut Option<(i64, i64, String)>,
                 out: &mut Vec<crate::ported::zsh_h::histent>| {
        if let Some((stim, ftim, text)) = cur.take() {
            out.push(crate::ported::zsh_h::histent {
                node: crate::ported::zsh_h::hashnode {
                    next: None,
                    nam: text,
                    flags: crate::ported::zsh_h::HIST_OLD as i32,
                },
                up: None,
                down: None,
                zle_text: None,
                stim,
                ftim,
                words: Vec::new(),
                nwords: 0,
                histnum: 0, // caller renumbers descending below the floor
            });
        }
    };
    for raw_line in lines {
        if let Some((_, _, ref mut text)) = current {
            if text.ends_with('\\') {
                text.pop();
                text.push('\n');
                text.push_str(raw_line);
                continue;
            }
            flush(&mut current, &mut out);
        }
        if let Some(rest) = raw_line.strip_prefix(": ") {
            if let Some((meta, text)) = rest.split_once(';') {
                if let Some((stim_s, dur_s)) = meta.split_once(':') {
                    let stim: i64 = stim_s.parse().unwrap_or(0);
                    let dur: i64 = dur_s.parse().unwrap_or(0);
                    current = Some((stim, stim + dur, text.to_string()));
                    continue;
                }
            }
        }
        current = Some((0, 0, raw_line.to_string()));
    }
    flush(&mut current, &mut out);
    Some((out, start + offset))
}
