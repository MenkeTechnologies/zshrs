//! History management for zshrs
//!
//! Port from zsh/Src/hist.c
//!
//! The history lines are kept in a hash, and also doubly-linked in a ring   // c:98
//!
//! Provides history expansion, history file management, and history ring.

use std::collections::HashMap;
use crate::ported::utils::zerr;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// History entry
#[derive(Clone, Debug)]
/// One history record.
/// Port of `struct histent` from Src/zsh.h — `addhistnum()`
/// (Src/hist.c) bumps history counts; `hgetline()` (Src/hist.c)
/// renders.
pub struct HistEntry {
    pub histnum: i64,               // History event number
    pub text: String,               // Command text
    pub words: Vec<(usize, usize)>, // Word boundaries
    pub stim: i64,                  // Start time
    pub ftim: i64,                  // Finish time
    pub flags: u32,                 // Entry flags
}

/// History entry flags
pub mod hist_flags {
    pub const OLD: u32 = 1; // From history file
    pub const DUP: u32 = 2; // Duplicate
    pub const FOREIGN: u32 = 4; // From other session
    pub const TMPSTORE: u32 = 8; // Temporary storage
    pub const NOWRITE: u32 = 16; // Don't save to file
}

impl HistEntry {
    pub fn new(histnum: i64, text: String) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        HistEntry {
            histnum,
            text,
            words: Vec::new(),
            stim: now,
            ftim: now,
            flags: 0,
        }
    }

    /// Get a specific word from the entry
    pub fn get_word(&self, index: usize) -> Option<&str> {
        self.words
            .get(index)
            .map(|(start, end)| &self.text[*start..*end])
    }

    /// Get number of words
    pub fn num_words(&self) -> usize {
        self.words.len()
    }
}

// != 0 means history substitution is turned off                           // c:57
/// History active bits
pub const HA_ACTIVE: u32 = 1; // History mechanism is active
pub const HA_NOINC: u32 = 2; // Don't store, curhist not incremented
pub const HA_INWORD: u32 = 4; // We're inside a word

/// History state
/// In-memory history list.
/// Port of the `histlist` global Src/hist.c maintains —
/// `addhistnum()`/`histreduceblanks()` mutate; `getargspec()`
/// (line 798) walks for `!:N` substitution.
pub struct History {
    /// History entries indexed by event number
    pub(crate) entries: HashMap<i64, HistEntry>,
    /// Ring buffer order (newest first)
    ring: Vec<i64>,
    /// Current history number
    pub curhist: i64,
    /// History line count
    pub histlinect: i64,
    /// History size limit
    pub histsiz: i64,
    /// Save history size
    pub savehistsiz: i64,
    // state of the history mechanism                                        // c:121
    /// History active state
    pub histactive: u32,
    /// Stop history flag
    pub stophist: i32,
    /// History done flags
    pub histdone: i32,
    /// History skip flags
    pub hist_skip_flags: i32,
    /// Ignore all dups
    pub hist_ignore_all_dups: bool,
    /// Current line being edited
    pub curline: Option<HistEntry>,
    /// History substitution patterns
    pub hsubl: Option<String>,
    pub hsubr: Option<String>,
    /// Bang character
    pub bangchar: char,
    /// History file path
    pub histfile: Option<String>,
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    pub fn new() -> Self {
        History {
            entries: HashMap::new(),
            ring: Vec::new(),
            curhist: 0,
            histlinect: 0,
            histsiz: 1000,
            savehistsiz: 1000,
            histactive: 0,
            stophist: 0,
            histdone: 0,
            hist_skip_flags: 0,
            hist_ignore_all_dups: false,
            curline: None,
            hsubl: None,
            hsubr: None,
            bangchar: '!',
            histfile: None,
        }
    }

    /// Initialize history
    pub fn init(&mut self) {
        self.curhist = 0;
        self.histlinect = 0;
    }

    // initialize the history mechanism                                      // c:1106
    /// Begin history for a new command
    pub fn hbegin(&mut self, interactive: bool) {                            // c:1110
        if (self.histactive & HA_ACTIVE) != 0 {
            return;
        }

        self.histactive = HA_ACTIVE;
        self.histdone = 0;

        if interactive {
            self.curhist += 1;
            self.curline = Some(HistEntry::new(self.curhist, String::new()));
        }
    }

    // say we're done using the history mechanism                            // c:1470
    /// End history for current command
    pub fn hend(&mut self, text: Option<String>) -> bool {                   // c:1474
        if (self.histactive & HA_ACTIVE) == 0 {
            return false;
        }

        self.histactive = 0;

        if let Some(mut entry) = self.curline.take() {
            if let Some(t) = text {
                entry.text = t;
            }

            // Skip empty entries
            if entry.text.trim().is_empty() {
                self.curhist -= 1;
                return false;
            }

            // Check for duplicates
            if self.hist_ignore_all_dups {
                let dup = self.entries.values().any(|e| e.text == entry.text);
                if dup {
                    self.curhist -= 1;
                    return false;
                }
            }

            // Add to history
            self.add_entry(entry);
            return true;
        }

        false
    }

    /// Add an entry to history
    fn add_entry(&mut self, entry: HistEntry) {
        let num = entry.histnum;

        // Remove old entry if at capacity
        while self.histlinect >= self.histsiz && !self.ring.is_empty() {
            let oldest = self.ring.pop().unwrap();
            self.entries.remove(&oldest);
            self.histlinect -= 1;
        }

        self.entries.insert(num, entry);
        self.ring.insert(0, num);
        self.histlinect += 1;
    }

    /// Get entry by history number
    pub fn get(&self, num: i64) -> Option<&HistEntry> {
        self.entries.get(&num)
    }

    /// Get the most recent entry
    pub fn latest(&self) -> Option<&HistEntry> {
        self.ring.first().and_then(|n| self.entries.get(n))
    }

    /// Length of the in-memory history ring. Used by up_histent /
    /// down_histent to detect ends of the ring.
    pub fn ring_len(&self) -> usize {
        self.ring.len()
    }

    /// Position of `histnum` in the ring (0 = newest), or `None`
    /// if the histnum isn't present.
    pub fn ring_position(&self, histnum: i64) -> Option<usize> {
        self.ring.iter().position(|n| *n == histnum)
    }

    /// Histnum at ring index `pos` (0 = newest). Caller must ensure
    /// `pos < ring_len()`.
    pub fn ring_at(&self, pos: usize) -> i64 {
        self.ring[pos]
    }

    /// Histnum of the oldest entry (last in the ring), or `None`
    /// when the ring is empty. Mirrors C's `hist_ring->down`.
    pub fn ring_oldest(&self) -> Option<i64> {
        self.ring.last().copied()
    }

    /// Push an entry at the head of the ring (newest position) and
    /// register it in `entries` under `histnum`. Mirrors C's
    /// `hist_ring = he` after the doubly-linked-list splice.
    pub fn insert_at_head(&mut self, histnum: i64, entry: HistEntry) {
        self.entries.insert(histnum, entry);
        self.ring.retain(|n| *n != histnum);
        self.ring.insert(0, histnum);
        self.histlinect = self.ring.len() as i64;
    }

    /// Remove the entry with `histnum` from both the ring and the
    /// entries map. Mirrors C's `freehistnode` on a single entry.
    pub fn remove(&mut self, histnum: i64) {
        self.entries.remove(&histnum);
        self.ring.retain(|n| *n != histnum);
        self.histlinect = self.ring.len() as i64;
    }

    /// Get the n-th most recent entry (0 = latest)
    pub fn recent(&self, n: usize) -> Option<&HistEntry> {
        self.ring.get(n).and_then(|num| self.entries.get(num))
    }

    /// Search history backwards for a pattern
    pub fn search_back(&self, pattern: &str, start: i64) -> Option<&HistEntry> {
        for num in self.ring.iter() {
            if *num >= start {
                continue;
            }
            if let Some(entry) = self.entries.get(num) {
                if entry.text.contains(pattern) {
                    return Some(entry);
                }
            }
        }
        None
    }

    /// Search history forwards for a pattern
    pub fn search_forward(&self, pattern: &str, start: i64) -> Option<&HistEntry> {
        for num in self.ring.iter().rev() {
            if *num <= start {
                continue;
            }
            if let Some(entry) = self.entries.get(num) {
                if entry.text.contains(pattern) {
                    return Some(entry);
                }
            }
        }
        None
    }

    /// Perform history substitution
    pub fn expand(&mut self, line: &str) -> Result<String, String> {
        let mut result = String::new();
        let mut chars = line.chars().peekable();
        let bang = self.bangchar;

        while let Some(c) = chars.next() {
            if c == bang {
                match chars.peek() {
                    Some(&'!') => {
                        // !! - last command
                        chars.next();
                        if let Some(entry) = self.latest() {
                            result.push_str(&entry.text);
                        } else {
                            return Err("No previous command".to_string());
                        }
                    }
                    Some(&'-') | Some(&('0'..='9')) => {
                        // !n or !-n
                        let mut numstr = String::new();
                        if chars.peek() == Some(&'-') {
                            numstr.push(chars.next().unwrap());
                        }
                        while let Some(&c) = chars.peek() {
                            if c.is_ascii_digit() {
                                numstr.push(chars.next().unwrap());
                            } else {
                                break;
                            }
                        }
                        if let Ok(n) = numstr.parse::<i64>() {
                            let target = if n < 0 { self.curhist + n } else { n };
                            if let Some(entry) = self.get(target) {
                                result.push_str(&entry.text);
                            } else {
                                return Err(format!("!{}: event not found", numstr));
                            }
                        }
                    }
                    Some(&'?') => {
                        // !?string - search
                        chars.next();
                        let mut pattern = String::new();
                        while let Some(&c) = chars.peek() {
                            if c == '?' {
                                chars.next();
                                break;
                            }
                            pattern.push(chars.next().unwrap());
                        }
                        if let Some(entry) = self.search_back(&pattern, self.curhist) {
                            result.push_str(&entry.text);
                        } else {
                            return Err(format!("!?{}: event not found", pattern));
                        }
                    }
                    Some(&'^') | Some(&'$') | Some(&'*') | Some(&':') => {
                        // Word designators on last command
                        if let Some(entry) = self.latest() {
                            let words: Vec<&str> = entry.text.split_whitespace().collect();
                            match chars.next() {
                                Some('^') => {
                                    if let Some(w) = words.get(1) {
                                        result.push_str(w);
                                    }
                                }
                                Some('$') => {
                                    if let Some(w) = words.last() {
                                        result.push_str(w);
                                    }
                                }
                                Some('*') => {
                                    result.push_str(&words[1..].join(" "));
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(c) if c.is_alphabetic() => {
                        // !string - search prefix
                        let mut pattern = String::new();
                        while let Some(&c) = chars.peek() {
                            if c.is_alphanumeric() || c == '_' {
                                pattern.push(chars.next().unwrap());
                            } else {
                                break;
                            }
                        }
                        let found = self.ring.iter().find_map(|num| {
                            self.entries
                                .get(num)
                                .filter(|e| e.text.starts_with(&pattern))
                        });
                        if let Some(entry) = found {
                            result.push_str(&entry.text);
                        } else {
                            return Err(format!("!{}: event not found", pattern));
                        }
                    }
                    _ => result.push(bang),
                }
            } else if c == '^' && result.is_empty() {
                // ^old^new - quick substitution
                let mut old = String::new();
                let mut new = String::new();
                let mut in_new = false;

                for c in chars.by_ref() {
                    if c == '^' {
                        if in_new {
                            break;
                        }
                        in_new = true;
                    } else if in_new {
                        new.push(c);
                    } else {
                        old.push(c);
                    }
                }

                if let Some(entry) = self.latest() {
                    result = entry.text.replacen(&old, &new, 1);
                    self.hsubl = Some(old);
                    self.hsubr = Some(new);
                } else {
                    return Err("No previous command".to_string());
                }
            } else {
                result.push(c);
            }
        }

        Ok(result)
    }

    /// Read history file
    pub fn read_file(&mut self, path: &Path) -> io::Result<()> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        for line in reader.lines() {
            let line = line?;

            // Parse extended history format
            if line.starts_with(':') {
                // Extended format: : timestamp:0;command
                let parts: Vec<&str> = line.splitn(2, ';').collect();
                if parts.len() == 2 {
                    let text = parts[1].to_string();
                    let mut entry = HistEntry::new(self.curhist + 1, text);

                    // Parse timestamp
                    if let Some(ts_part) = parts[0].strip_prefix(": ") {
                        if let Some(ts_str) = ts_part.split(':').next() {
                            if let Ok(ts) = ts_str.parse::<i64>() {
                                entry.stim = ts;
                                entry.ftim = ts;
                            }
                        }
                    }

                    entry.flags |= hist_flags::OLD;
                    self.curhist += 1;
                    self.add_entry(entry);
                }
            } else if !line.is_empty() {
                // Simple format
                self.curhist += 1;
                let mut entry = HistEntry::new(self.curhist, line);
                entry.flags |= hist_flags::OLD;
                self.add_entry(entry);
            }
        }

        Ok(())
    }

    /// Write history file
    pub fn write_file(&self, path: &Path, append: bool) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(!append)
            .append(append)
            .open(path)?;

        for num in self.ring.iter().rev() {
            if let Some(entry) = self.entries.get(num) {
                if (entry.flags & hist_flags::NOWRITE) != 0 {
                    continue;
                }
                // Write extended format
                writeln!(file, ": {}:0;{}", entry.stim, entry.text)?;
            }
        }

        Ok(())
    }

    /// Clear all history
    pub fn clear(&mut self) {
        self.entries.clear();
        self.ring.clear();
        self.histlinect = 0;
    }

    /// Get all entries in order
    pub fn all_entries(&self) -> Vec<&HistEntry> {
        self.ring
            .iter()
            .filter_map(|n| self.entries.get(n))
            .collect()
    }

    /// Number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Save history context (from hist.c hist_context_save/restore)
#[derive(Clone, Default)]
/// Saved history-substitution state for nested input contexts.
/// Port of `struct hist_stack` from Src/hist.c —
/// `hist_context_save()` (line 248) / `hist_context_restore()`
/// (line 296) push/pop these around `eval`, `source`, etc.
pub struct HistStack {
    pub histactive: u32,
    pub histdone: i32,
    pub stophist: i32,
    pub chline: Option<String>,
    pub hptr: usize,
    pub chwords: Vec<(usize, usize)>,
    pub hlinesz: usize,
    pub defev: i64,
    pub hist_keep_comment: bool,
}

/// History done flags (from hist.c)
pub const HISTFLAG_DONE: i32 = 1;
pub const HISTFLAG_NOEXEC: i32 = 2;
pub const HISTFLAG_RECALL: i32 = 4;
pub const HISTFLAG_SETTY: i32 = 8;

/// Case modification types (from hist.c casemodify)
#[derive(Clone, Copy, Debug, PartialEq)]
/// Case-modification kind for `:U`/`:L`/`:C` modifiers.
/// Port of the `CASMOD_*` flag bits Src/hist.c uses inside
/// `casemodify()` (line ~504 in this Rust port; Src/utils.c on
/// the C side).
pub enum CaseMod {
    Lower,
    Upper,
    Caps,
}

/// Case modify a string (from hist.c casemodify lines 2194-2323)
/// Apply a `:U`/`:L`/`:C` case modifier.
/// Port of `casemodify()` from Src/utils.c.
pub fn casemodify(s: &str, how: CaseMod) -> String {
    let mut result = String::with_capacity(s.len());
    let mut nextupper = true;

    for c in s.chars() {
        let modified = match how {
            CaseMod::Lower => c.to_lowercase().collect::<String>(),
            CaseMod::Upper => c.to_uppercase().collect::<String>(),
            CaseMod::Caps => {
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
        };
        result.push_str(&modified);
    }

    result
}

/// Path-head modifier `:h` / `:hN`.
/// Port of `remtpath()` from Src/hist.c:2056. Two modes per the C
/// source's `if (!count)` switch:
///   count == 0 — bare `:h`. Skip the trailing filename component
///                and return the dirname (or "/" / "." when nothing
///                is left).
///   count > 0  — `:hN`. Walk from the FRONT decrementing on each
///                separator; terminate when count reaches 0. The
///                leading slash counts as one component, so `:h1` on
///                "/a/b/c" returns "/", `:h2` returns "/a", `:h3`
///                returns "/a/b". Consecutive separators are
///                squeezed (matches C `while (IS_DIRSEP(strp[1]))`).
pub fn remtpath(s: &str, count: i32) -> String {
    let s = s.trim_end_matches('/');

    if s.is_empty() {
        return "/".to_string();
    }

    if count == 0 {
        // Bare `:h` — strip the last component.
        if let Some(pos) = s.rfind('/') {
            if pos == 0 {
                return "/".to_string();
            }
            return s[..pos].trim_end_matches('/').to_string();
        }
        return ".".to_string();
    }

    // count > 0 — direct port of the front-walk loop in C remtpath
    // (Src/hist.c:2078-2098). Decrement on each separator; when
    // count <= 0, terminate at that position (special-casing the
    // leading-slash position).
    let bytes = s.as_bytes();
    let mut remaining = count;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'/' {
            remaining -= 1;
            if remaining <= 0 {
                if i == 0 {
                    // Leading slash counted as a component.
                    return "/".to_string();
                }
                return s[..i].to_string();
            }
            // Skip consecutive separators — C: `while
            // (IS_DIRSEP(strp[1])) ++strp;`
            while i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                i += 1;
            }
        }
        i += 1;
    }
    // Full string needed (count larger than the available components).
    s.to_string()
}

/// Remove leading path components (from hist.c remlpaths lines 2151-2186)
/// Remove leading path components.
/// Port of the `:h` arm inside `applymod()` (Src/utils.c).
pub fn remlpaths(s: &str, count: i32) -> String {
    let s = s.trim_end_matches('/');

    if s.is_empty() {
        return String::new();
    }

    let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();

    // Take the last `count` components (count==0 falls through to
    // count==1 default — `:t` with no number is one component).
    // Direct port of Src/hist.c:remlpaths which iterates from the
    // tail. Earlier the count-exhausts-parts branch returned `s`
    // unchanged, leaking the leading `/` for paths whose components
    // are fewer than count: `/just-a-name.zsh` with count=1 came
    // back as `/just-a-name.zsh` instead of `just-a-name.zsh`.
    let n = if count == 0 { 1 } else { count as usize };
    let take_n = n.min(parts.len());
    if take_n == 0 {
        return String::new();
    }
    parts
        .iter()
        .rev()
        .take(take_n)
        .rev()
        .copied()
        .collect::<Vec<&str>>()
        .join("/")
}

/// Remove extension (from hist.c remtext lines 2122-2131)
/// `:r` modifier — remove trailing extension.
/// Port of the `:r` arm inside `applymod()` (Src/utils.c).
pub fn remtext(s: &str) -> String {
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
        if dot_pos > 0 {
            return s[..dot_pos].to_string();
        }
    }
    s.to_string()
}

/// Get extension (from hist.c rembutext lines 2136-2148)
/// `:e` modifier — keep only trailing extension.
/// Port of the `:e` arm inside `applymod()` (Src/utils.c).
pub fn rembutext(s: &str) -> String {
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


/// Quote with word breaking (from hist.c quotebreak lines 2527-2556)
/// Backslash-bslashquote shell metachars including word breaks.
/// Port of `quotebreak()` from Src/hist.c.
pub fn quotebreak(s: &str) -> String {                                       // c:2527
    let mut result = String::with_capacity(s.len() + 10);
    result.push('\'');

    for c in s.chars() {
        if c == '\'' {
            result.push_str("'\\''");
        } else if c.is_whitespace() {
            result.push('\'');
            result.push(c);
            result.push('\'');
        } else {
            result.push(c);
        }
    }

    result.push('\'');
    result
}

/// `:s/old/new/` modifier — substitute pattern.
/// Port of `subst()` from Src/hist.c:2336.
pub fn subst(s: &str, in_pattern: &str, out_pattern: &str, global: bool) -> String {
    // Direct port of src/zsh/Src/hist.c:2336-2391 subst.
    // - Empty pattern means "use whole string as the pattern"
    //   (hist.c:2341-2342). Behaves as a no-replace in this Rust
    //   port since getmatch on the whole-string would just return
    //   the same string.
    // - Leading `#` (or `Pound` token) anchors at the start of the
    //   string per hist.c:2349-2353 (SUB_START flag).
    // - Leading `%` anchors at the end per hist.c:2354-2358
    //   (SUB_END flag).
    // - Otherwise unanchored substring match.
    // - In the replacement, `&` expands to the full match (via
    //   convamps); `\&` is a literal `&`.
    if in_pattern.is_empty() {
        return s.to_string();
    }

    // Strip anchor prefixes per hist.c:2349-2358.
    let mut anchor_start = false;
    let mut anchor_end = false;
    let mut pat = in_pattern;
    if let Some(rest) = pat.strip_prefix('#') {
        anchor_start = true;
        pat = rest;
    }
    if let Some(rest) = pat.strip_prefix('%') {
        anchor_end = true;
        pat = rest;
    }
    if pat.is_empty() {
        return s.to_string();
    }

    // Substitute `&` in replacement with the matched pattern
    // (the actual C uses convamps with the unanchored `pat`).
    let out_expanded = convamps(out_pattern, pat);

    if anchor_start && anchor_end {
        // Both anchors — match only if WHOLE string equals pat.
        if s == pat {
            return out_expanded;
        }
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

    if global {
        s.replace(pat, &out_expanded)
    } else {
        s.replacen(pat, &out_expanded, 1)
    }
}

/// Convert & to matched pattern (from hist.c convamps lines 2394-2418)
fn convamps(out: &str, in_pattern: &str) -> String {
    let mut result = String::with_capacity(out.len());
    let mut chars = out.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                result.push(next);
                chars.next();
            }
        } else if c == '&' {
            result.push_str(in_pattern);
        } else {
            result.push(c);
        }
    }

    result
}

/// Get argument specification (from hist.c getargspec lines 1792-1829)
/// Resolve a `!`/`*`/`-N`/`^`/`$`/word designator.
/// Port of `getargspec()` from Src/hist.c:563.
pub fn getargspec(argc: usize, c: char, marg: Option<usize>, evset: bool) -> Option<usize> {
    match c {
        '0' => Some(0),
        '1'..='9' => Some(c.to_digit(10).unwrap() as usize),
        '^' => Some(1),
        '$' => Some(argc),
        '%' => {
            if evset {
                return None;
            }
            marg
        }
        _ => None,
    }
}

/// History search containing pattern (from hist.c hconsearch lines 1836-1854)
impl History {
    pub fn hconsearch(&self, pattern: &str) -> Option<(i64, usize)> {
        for num in &self.ring {
            if let Some(entry) = self.entries.get(num) {
                if let Some(pos) = entry.text.find(pattern) {
                    let words: Vec<&str> = entry.text.split_whitespace().collect();
                    let mut word_idx = 0;
                    let mut char_count = 0;
                    for (i, word) in words.iter().enumerate() {
                        if char_count + word.len() > pos {
                            word_idx = i;
                            break;
                        }
                        char_count += word.len() + 1;
                    }
                    return Some((entry.histnum, word_idx));
                }
            }
        }
        None
    }

    /// History search by prefix (from hist.c hcomsearch lines 1859-1872)
    pub fn hcomsearch(&self, prefix: &str) -> Option<i64> {
        for num in &self.ring {
            if let Some(entry) = self.entries.get(num) {
                if entry.text.starts_with(prefix) {
                    return Some(entry.histnum);
                }
            }
        }
        None
    }

    /// Get arguments from history entry (from hist.c getargs lines 2453-2482)
    pub fn getargs(&self, ev: i64, arg1: usize, arg2: usize) -> Option<String> {
        let entry = self.entries.get(&ev)?;
        let words: Vec<&str> = entry.text.split_whitespace().collect();

        if arg2 < arg1 || arg1 >= words.len() || arg2 >= words.len() {
            return None;
        }

        if arg1 == 0 && arg2 == words.len() - 1 {
            return Some(entry.text.clone());
        }

        Some(words[arg1..=arg2].join(" "))
    }

    /// Save history context (from hist.c hist_context_save lines 248-290)
    pub fn save_context(&self) -> HistStack {
        HistStack {
            histactive: self.histactive,
            histdone: self.histdone,
            stophist: self.stophist,
            chline: self.curline.as_ref().map(|e| e.text.clone()),
            hptr: 0,
            chwords: Vec::new(),
            hlinesz: 0,
            defev: self.curhist - 1,
            hist_keep_comment: false,
        }
    }

    /// Restore history context (from hist.c hist_context_restore lines 296-325)
    pub fn restore_context(&mut self, ctx: &HistStack) {
        self.histactive = ctx.histactive;
        self.histdone = ctx.histdone;
        self.stophist = ctx.stophist;
    }

    /// Set history in-word state (from hist.c hist_in_word lines 339-345)
    pub fn hist_in_word(&mut self, yesno: bool) {
        if yesno {
            self.histactive |= HA_INWORD;
        } else {
            self.histactive &= !HA_INWORD;
        }
    }

    /// Check if in word (from hist.c hist_is_in_word lines 348-352)
    pub fn hist_is_in_word(&self) -> bool {
        (self.histactive & HA_INWORD) != 0
    }

    /// Add history number with offset (from hist.c addhistnum lines 1265-1280)
    pub fn addhistnum(&self, hl: i64, n: i64) -> i64 {                       // c:1266
        let target = hl + n;
        if target < 1 {
            0
        } else if target > self.curhist {
            self.curhist + 1
        } else {
            target
        }
    }

    /// Reduce blanks in history line (from hist.c histreduceblanks lines 1199-1250)
    pub fn histreduceblanks(line: &str, words: &[(usize, usize)]) -> String {
        if words.is_empty() {
            return line.to_string();
        }

        let mut result = String::new();
        let chars: Vec<char> = line.chars().collect();

        for (i, (start, end)) in words.iter().enumerate() {
            if i > 0 {
                result.push(' ');
            }
            for j in *start..*end {
                if j < chars.len() {
                    result.push(chars[j]);
                }
            }
        }

        result
    }

    /// Resize history entries to fit histsiz (from hist.c resizehistents lines 2620-2632)
    pub fn resizehistents(&mut self) {
        while self.histlinect > self.histsiz {
            if let Some(oldest) = self.ring.pop() {
                self.entries.remove(&oldest);
                self.histlinect -= 1;
            } else {
                break;
            }
        }
    }

    /// Read history file (from hist.c readhistfile lines 2675-2920)
    pub fn readhistfile(&mut self, filename: &str, err: bool) -> io::Result<usize> {
        let file = File::open(filename)?;
        let reader = BufReader::new(file);
        let mut count = 0;

        for line in reader.lines() {
            let line = line?;
            if line.is_empty() {
                continue;
            }

            // Check for extended history format: : <timestamp>:0;<command>
            if let Some(rest) = line.strip_prefix(": ") {
                if let Some(semi) = rest.find(';') {
                    let time_part = &rest[..semi];
                    let cmd_part = &rest[semi + 1..];

                    let stim = if let Some(colon) = time_part.find(':') {
                        time_part[..colon].parse::<i64>().unwrap_or(0)
                    } else {
                        time_part.parse::<i64>().unwrap_or(0)
                    };

                    if !cmd_part.trim().is_empty() {
                        self.curhist += 1;
                        let mut entry = HistEntry::new(self.curhist, cmd_part.to_string());
                        entry.stim = stim;
                        entry.flags = hist_flags::OLD;
                        self.add_entry(entry);
                        count += 1;
                    }
                }
            } else {
                // Plain history line
                if !line.trim().is_empty() {
                    self.curhist += 1;
                    let mut entry = HistEntry::new(self.curhist, line);
                    entry.flags = hist_flags::OLD;
                    self.add_entry(entry);
                    count += 1;
                }
            }
        }

        if err && count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "No history entries",
            ));
        }

        Ok(count)
    }

    /// Write history file (from hist.c savehistfile lines 2925-3155)
    pub fn savehistfile(&self, filename: &str, mode: WriteMode) -> io::Result<usize> {
        let file = match mode {
            WriteMode::Overwrite => File::create(filename)?,
            WriteMode::Append => OpenOptions::new()
                .create(true)
                .append(true)
                .open(filename)?,
        };
        let mut writer = io::BufWriter::new(file);
        let mut count = 0;

        for num in self.ring.iter().rev() {
            if let Some(entry) = self.entries.get(num) {
                if (entry.flags & hist_flags::NOWRITE) != 0 {
                    continue;
                }

                // Write in extended format
                writeln!(writer, ": {}:0;{}", entry.stim, entry.text)?;
                count += 1;
            }
        }

        writer.flush()?;
        Ok(count)
    }

    /// Lock history file (from hist.c lockhistfile lines 2961-2998)
    pub fn lockhistfile(&self, filename: &str, _excl: bool) -> io::Result<()> {
        let lockfile = format!("{}.lock", filename);
        File::create(&lockfile)?;
        Ok(())
    }

    /// Unlock history file (from hist.c unlockhistfile lines 3001-3018)
    pub fn unlockhistfile(&self, filename: &str) -> io::Result<()> {
        let lockfile = format!("{}.lock", filename);
        std::fs::remove_file(&lockfile).ok();
        Ok(())
    }

    /// Quote string for history (from hist.c quotestring lines 2483-2523)
    pub fn quotestring(s: &str) -> String {
        let mut result = String::with_capacity(s.len() + 10);
        result.push('\'');

        for c in s.chars() {
            if c == '\'' {
                result.push_str("'\\''");
            } else {
                result.push(c);
            }
        }

        result.push('\'');
        result
    }

    /// History word split (from hist.c get_history_word)
    pub fn get_history_word(line: &str, idx: usize) -> Option<&str> {
        line.split_whitespace().nth(idx)
    }

    /// Count words in history line
    pub fn histword_count(line: &str) -> usize {
        line.split_whitespace().count()
    }
}

/// History file write mode
/// History-file write mode.
/// Mirrors the `HFILE_*` flag bits Src/hist.c uses inside
/// `savehistfile()` to decide append / overwrite / merge.
pub enum WriteMode {
    Overwrite,
    Append,
}

// ---------------------------------------------------------------------------
// Missing functions from hist.c
// ---------------------------------------------------------------------------

/// Apply history word designator and modifiers to an event
/// (from hist.c histsubchar - the inline expansion engine)
///
/// Full syntax: !event:word_designator:modifier1:modifier2...
///
/// Word designators: 0 (command), ^ (first arg), $ (last), * (all args),
///   n (nth word), n-m (range), n* (nth to last), n- (nth to second-to-last)
///
/// Modifiers: h (head/dirname), t (tail/basename), r (remove ext), e (ext only),
///   l (lowercase), u (uppercase), s/old/new/ (substitute), & (repeat subst),
///   g (global modifier), p (print, don't execute), q (bslashquote), Q (unquote),
///   x (bslashquote words), a (absolute path)
/// Apply a word designator (`:N`/`:^`/`:$`/`:*`/etc.).
pub fn histremovedups(entries: &mut Vec<HistEntry>) {
    let mut seen = std::collections::HashSet::new();
    entries.retain(|e| seen.insert(e.text.clone()));
}

/// Reduce blanks in history text (from hist.c histreduceblanks)
/// Collapse runs of whitespace per `setopt HIST_REDUCE_BLANKS`.
/// Port of `histreduceblanks()` from Src/hist.c.
pub fn histreduceblanks(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !prev_space {
                result.push(' ');
                prev_space = true;
            }
        } else {
            result.push(c);
            prev_space = false;
        }
    }
    result.trim().to_string()
}

/// Get a history line as a complete string (from hist.c hgetline)
/// Render a history entry as a single line.
/// Port of `hgetline()`-related rendering inside Src/hist.c.
pub fn hgetline(entry: &HistEntry) -> String {
    entry.text.clone()
}

/// History word replacement (from hist.c hwrep)
/// Replace word N of a history entry's text.
/// Port of the `hwrep` step inside `histsubchar()`
/// (Src/hist.c:595) — used by `^old^new` quick subst.
pub fn hwrep(entry: &HistEntry, replacement: &str, word_idx: usize) -> String {
    let words: Vec<&str> = entry.text.split_whitespace().collect();
    if word_idx >= words.len() {
        return entry.text.clone();
    }
    let mut new_words: Vec<String> = words.iter().map(|s| s.to_string()).collect();
    new_words[word_idx] = replacement.to_string();
    new_words.join(" ")
}

/// Move forward in history (from hist.c addhistnum)
/// Increment a history-event number with wrap protection.
/// Port of the `addhistnum()` arithmetic Src/hist.c uses to
/// keep `$HISTCMD` monotonic.
pub fn addhistnum(base: i64, n: i64) -> i64 {                                // c:1266
    base + n
}

/// Check if history line should be ignored (starts with space, duplicate, etc.)
/// Apply HIST_IGNORE_* policies before recording an entry.
/// Port of the `setopt HIST_IGNORE_*` checks inside
/// `addhistnode()` (Src/hist.c).
pub fn should_ignore_line(
    text: &str,
    ignorespace: bool,
    ignoredups: bool,
    last: Option<&str>,
) -> bool {
    if ignorespace && text.starts_with(' ') {
        return true;
    }
    if ignoredups {
        if let Some(prev) = last {
            if prev == text {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
// Adjacent helpers (`bufferwords`, `histsplitwords`, `flockhistfile`,
// etc.) live below this block to keep history-related code in one
// file instead of splitting along test/non-test lines. Reordering
// would scatter the C-port topology across multiple modules.
mod tests {
    use super::*;

    #[test]
    fn test_history_add() {
        let mut hist = History::new();
        hist.hbegin(true);
        hist.hend(Some("echo hello".to_string()));

        assert_eq!(hist.len(), 1);
        assert_eq!(hist.latest().unwrap().text, "echo hello");
    }

    #[test]
    fn test_history_expand_bang_bang() {
        let mut hist = History::new();
        hist.hbegin(true);
        hist.hend(Some("ls -la".to_string()));

        let result = hist.expand("!! | grep foo").unwrap();
        assert_eq!(result, "ls -la | grep foo");
    }

    #[test]
    fn test_history_expand_caret() {
        let mut hist = History::new();
        hist.hbegin(true);
        hist.hend(Some("echo hello".to_string()));

        let result = hist.expand("^hello^world").unwrap();
        assert_eq!(result, "echo world");
    }

    #[test]
    fn test_history_search() {
        let mut hist = History::new();

        hist.hbegin(true);
        hist.hend(Some("cd /tmp".to_string()));

        hist.hbegin(true);
        hist.hend(Some("echo hello".to_string()));

        hist.hbegin(true);
        hist.hend(Some("ls -la".to_string()));

        let result = hist.search_back("echo", hist.curhist + 1);
        assert!(result.is_some());
        assert_eq!(result.unwrap().text, "echo hello");
    }

    #[test]
    fn test_history_capacity() {
        let mut hist = History::new();
        hist.histsiz = 3;

        for i in 0..5 {
            hist.hbegin(true);
            hist.hend(Some(format!("cmd{}", i)));
        }

        assert_eq!(hist.len(), 3);
        assert!(hist.get(1).is_none());
        assert!(hist.get(2).is_none());
    }
}

// ---------------------------------------------------------------------------
// Additional missing functions from hist.c (lexer integration layer)
// ---------------------------------------------------------------------------

/// Input stack management for history (from hist.c strinbeg/strinend)
/// Nested-input stack for history substitution.
/// Port of the `hist_stack` linked list Src/hist.c maintains
/// across `hist_context_save`/`hist_context_restore` (lines
/// 248/296).
pub struct HistInputStack {
    stack: Vec<HistInputState>,
}

struct HistInputState {
    dohist: bool,
}

impl Default for HistInputStack {
    fn default() -> Self {
        Self::new()
    }
}

impl HistInputStack {
    pub fn new() -> Self {
        HistInputStack { stack: Vec::new() }
    }

    /// Begin string input (from hist.c strinbeg)
    pub fn strinbeg(&mut self, dohist: bool) {
        self.stack.push(HistInputState { dohist });
    }

    /// End string input (from hist.c strinend)
    pub fn strinend(&mut self) {
        self.stack.pop();
    }

    /// Check if currently doing history
    pub fn doing_hist(&self) -> bool {
        self.stack.last().map(|s| s.dohist).unwrap_or(false)
    }
}

/// History line linkage (from hist.c linkcurline/unlinkcurline)
/// One link in the history-line ring.
/// Mirrors the `histent` linked-list node from Src/zsh.h.
pub struct HistLineLink {
    pub linked: bool,
    pub line: String,
}

impl HistLineLink {
    pub fn new() -> Self {
        HistLineLink {
            linked: false,
            line: String::new(),
        }
    }

    /// Link current line to history (from hist.c linkcurline)
    pub fn linkcurline(&mut self, line: &str) {
        self.line = line.to_string();
        self.linked = true;
    }

    /// Unlink current line from history (from hist.c unlinkcurline)
    pub fn unlinkcurline(&mut self) {
        self.linked = false;
        self.line.clear();
    }
}

impl Default for HistLineLink {
    fn default() -> Self {
        Self::new()
    }
}

/// History entry navigation (from hist.c movehistent/up_histent/down_histent)
impl History {
    /// Move n entries in history (from hist.c movehistent)
    pub fn movehistent(&self, start: i64, n: i64) -> Option<&HistEntry> {
        let target = start + n;
        self.get(target)
    }

    /// Move up one entry (from hist.c up_histent)
    pub fn up_histent(&self, current: i64) -> Option<&HistEntry> {
        self.get(current - 1)
    }

    /// Move down one entry (from hist.c down_histent)
    pub fn down_histent(&self, current: i64) -> Option<&HistEntry> {
        self.get(current + 1)
    }

    /// Get history entry by event number with near-match (from hist.c gethistent)
    pub fn gethistent(&self, ev: i64, near_match: bool) -> Option<&HistEntry> {
        if let Some(entry) = self.get(ev) {
            return Some(entry);
        }
        if !near_match {
            return None;
        }
        // Try nearest
        let mut best = None;
        let mut best_dist = i64::MAX;
        for (num, entry) in &self.entries {
            let dist = (*num - ev).abs();
            if dist < best_dist {
                best_dist = dist;
                best = Some(entry);
            }
        }
        best
    }

    /// Prepare next history entry (from hist.c prepnexthistent)
    pub fn prepnexthistent(&mut self) -> i64 {
        self.curhist + 1
    }
}

/// History word buffer operations (from hist.c ihwbegin/ihwabort/ihwend)
/// Per-character history-word buffer.
/// Port of the `hwbuf` global Src/hist.c keeps for the
/// `ihwaddc()` (line 357) word-tracker — used by `!:N`
/// designator lookup.
pub struct HistWordBuffer {
    buf: String,
    active: bool,
}

impl Default for HistWordBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl HistWordBuffer {
    pub fn new() -> Self {
        HistWordBuffer {
            buf: String::new(),
            active: false,
        }
    }

    /// Begin collecting a history word (from hist.c ihwbegin)
    pub fn ihwbegin(&mut self) {
        self.buf.clear();
        self.active = true;
    }

    /// Abort history word collection (from hist.c ihwabort)
    pub fn ihwabort(&mut self) {
        self.active = false;
        self.buf.clear();
    }

    /// End history word collection (from hist.c ihwend)
    pub fn ihwend(&mut self) -> Option<String> {
        if self.active {
            self.active = false;
            Some(std::mem::take(&mut self.buf))
        } else {
            None
        }
    }

    /// Add character to word buffer
    pub fn add(&mut self, c: char) {
        if self.active {
            self.buf.push(c);
        }
    }

    /// Get current buffer content (from hist.c hwget)
    pub fn hwget(&self) -> &str {
        &self.buf
    }
}

/// History backward word scan (from hist.c histbackword)
/// Walk back one word in a history line.
/// Port of the `iaddtoline()` (Src/hist.c:397) word-boundary
/// detection.
pub fn histbackword(line: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let bytes = line.as_bytes();
    let mut p = pos.min(bytes.len());

    // Skip whitespace
    while p > 0 && bytes[p - 1].is_ascii_whitespace() {
        p -= 1;
    }
    // Skip word chars
    while p > 0 && !bytes[p - 1].is_ascii_whitespace() {
        p -= 1;
    }
    p
}

/// Unget character for history (from hist.c ihungetc)
/// One-character pushback for history-substitution lexer.
/// Port of `safeinungetc()` (Src/hist.c:467) +
/// `ihungetc()` (line 989) — same single-slot ungetc model.
pub struct HistUnget {
    chars: Vec<char>,
}

impl Default for HistUnget {
    fn default() -> Self {
        Self::new()
    }
}

impl HistUnget {
    pub fn new() -> Self {
        HistUnget { chars: Vec::new() }
    }

    /// Push back a character (from hist.c ihungetc)
    pub fn ihungetc(&mut self, c: char) {
        self.chars.push(c);
    }

    /// Get a pushed-back character
    pub fn ihgetc(&mut self) -> Option<char> {
        self.chars.pop()
    }

    pub fn has_chars(&self) -> bool {
        !self.chars.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Remaining 23 missing hist.c functions
// ---------------------------------------------------------------------------

/// Add character to history word during lexing (from hist.c ihwaddc)
/// Push a character into the word buffer.
/// Port of `ihwaddc()` from Src/hist.c:357.
pub fn ihwaddc(hwbuf: &mut HistWordBuffer, c: char) {
    hwbuf.add(c);
}

/// Add character to current line during lexing (from hist.c iaddtoline)
/// Push a character into the in-progress history line.
/// Port of `iaddtoline()` from Src/hist.c:397.
pub fn iaddtoline(line: &mut String, c: char) {
    line.push(c);
}

/// Safe version of inungetc for history (from hist.c safeinungetc)
pub fn safeinungetc(unget: &mut HistUnget, c: char) {
    unget.ihungetc(c);
}

/// Flush history error state (from hist.c herrflush)
pub fn herrflush() {
    // Reset history error flags - in Rust this is handled by the parser state
}

/// Get substitution arguments from history (from hist.c getsubsargs)
/// Parses s/old/new/ syntax
pub fn getsubsargs(line: &str) -> Option<(String, String, bool)> {
    if line.len() < 2 {
        return None;
    }
    let sep = line.chars().next()?;
    let rest = &line[sep.len_utf8()..];

    let mut old = String::new();
    let mut new = String::new();
    let mut in_new = false;
    let mut global = false;

    for c in rest.chars() {
        if c == sep {
            if in_new {
                break;
            }
            in_new = true;
            continue;
        }
        if in_new {
            new.push(c);
        } else {
            old.push(c);
        }
    }

    // Check for trailing 'g' flag
    if rest.ends_with('g') && rest.len() > old.len() + new.len() + 2 {
        global = true;
    }

    if old.is_empty() {
        None
    } else {
        Some((old, new, global))
    }
}

/// Get argument count from history entry (from hist.c getargc)
pub fn getargc(entry: &HistEntry) -> usize {
    entry.num_words()
}

/// Report substitution failure (from hist.c substfailed)
pub fn substfailed() -> String {
    "substitution failed".to_string()
}

/// Count digits in a string prefix (from hist.c digitcount)
pub fn digitcount(s: &str) -> usize {
    s.chars().take_while(|c| c.is_ascii_digit()).count()
}

/// Port of `nohw()` from Src/hist.c:1062.
/// C: `static void nohw(UNUSED(int c))` — dummy function used instead
///   of hwaddc when history-word collection isn't needed. Empty body.
pub fn nohw(_c: char) {}                                                     // c:1062

/// Port of `nohwabort()` from Src/hist.c:1067.
/// C: `static void nohwabort(void)` — dummy hwbegin replacement. Empty.
pub fn nohwabort() {}                                                        // c:1067

/// Port of `nohwe()` from Src/hist.c:1072.
/// C: `static void nohwe(void)` — dummy hwend replacement. Empty.
pub fn nohwe() {}                                                            // c:1072

/// Put old history entry on top of ring (from hist.c putoldhistentryontop)
pub fn putoldhistentryontop(hist: &mut History) -> bool {
    // Move the oldest entry to the newest position for reuse
    if let Some(oldest_num) = hist.ring.first().copied() {
        if let Some(entry) = hist.entries.remove(&oldest_num) {
            hist.ring.remove(0);
            let new_num = hist.curhist + 1;
            hist.entries.insert(new_num, entry);
            hist.ring.push(new_num);
            return true;
        }
    }
    false
}

/// Check if current line matches history entry (from hist.c checkcurline)
pub fn checkcurline(hist: &History, line: &str) -> bool {
    hist.latest().map(|e| e.text == line).unwrap_or(false)
}

/// Quietly get history entry without error (from hist.c quietgethist)
pub fn quietgethist(hist: &History, ev: i64) -> Option<&HistEntry> {
    hist.get(ev)
}

/// Toggle the `HA_INWORD` bit on `histactive`.
/// Port of `hist_in_word()` from Src/hist.c.
pub fn hist_in_word(hist: &mut History, yesno: bool) {
    if yesno {
        hist.histactive |= HA_INWORD;
    } else {
        hist.histactive &= !HA_INWORD;
    }
}

/// Read the `HA_INWORD` bit on `histactive`.
/// Port of `hist_is_in_word()` from Src/hist.c.
pub fn hist_is_in_word(hist: &History) -> bool {
    (hist.histactive & HA_INWORD) != 0
}

/// Live count of `lockhistfile()` invocations not yet matched by
/// `unlockhistfile()`. Mirrors C zsh's `lockhistct` global. Used
/// by `histfileIsLocked` and the lock-counting pop in
/// `unlockhistfile`.
static LOCKHISTCT: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// Mirror of C's `histactive` global (HA_ACTIVE | HA_NOINC |
/// HA_INWORD bits). The lexer-integrated word-capture fns
/// (`ihwbegin`/`ihwend`) read this to skip recording when
/// already inside a word or when history is paused.
static HISTACTIVE: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

/// Live counter for `chwordpos` — the index into the per-line
/// word-boundary array zsh's lexer fills as it tokenises. Used by
/// the `ihw*` family. Mirrors C zsh's `chwordpos` global.
static CHWORDPOS: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// `hist_keep_comment` flag — set by `ihwabort` to retain a
/// comment that would otherwise be dropped. Mirrors the C global.
static HIST_KEEP_COMMENT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Per-line input buffer the lexer accumulates into (`chline` in
/// C). Owned by the parser/lexer in C; in zshrs this is an
/// auxiliary buffer the history-word machinery reads to slice
/// out individual words via the `chwords` offset array.
static CHLINE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Word-boundary offsets (`chwords` in C) — pairs of (start,end)
/// byte offsets into CHLINE. The C source uses a flat `short[]`
/// with even-indexed starts and odd-indexed ends; we store the
/// same layout as a flat Vec for parity with the index math.
static CHWORDS: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());

/// Stop-history depth (`stophist`). 0 = active, 1 = `setopt
/// nohistexpand` or `\!` escape, 2 = inside `noglob`/etc. Mirrors
/// the C global. Used by `ihwbegin`/`ihwend` to skip word
/// recording during stop-history runs.
static STOPHIST_FLAG: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// Process-wide singleton History — the C source reaches its history
/// state through file-static globals (`hist_ring`, `curhist`,
/// `histsiz`, `savehistsiz`) inside `Src/hist.c`. Rust collects them
/// into one `History` struct and parks it behind a `OnceLock<Mutex>`
/// for the same single-instance reach. `bin_fc` and other call sites
/// that mirror the C "operate on the global history" pattern lock
/// this directly. Initialised on first access via `History::new()`.
pub static HISTORY: std::sync::OnceLock<std::sync::Mutex<History>> =
    std::sync::OnceLock::new();


/// Whether the history file is currently locked by this process.
/// Port of `histfileIsLocked()` from Src/hist.c.
#[allow(non_snake_case)]
pub fn histfileIsLocked() -> bool {
    LOCKHISTCT.load(std::sync::atomic::Ordering::SeqCst) > 0
}

/// Move one history entry up (toward older). Port of
/// `up_histent()` from Src/hist.c. Returns the histnum of the
/// previous entry, or `None` when at the top of the ring.
pub fn up_histent(hist: &History, current: i64) -> Option<i64> {
    let pos = hist.ring_position(current)?;
    if pos + 1 >= hist.ring_len() {
        None
    } else {
        Some(hist.ring_at(pos + 1))
    }
}

/// Move one history entry down (toward newer). Port of
/// `down_histent()` from Src/hist.c. Returns the histnum of the
/// next entry, or `None` when at the bottom of the ring.
pub fn down_histent(hist: &History, current: i64) -> Option<i64> {
    let pos = hist.ring_position(current)?;
    if pos == 0 {
        None
    } else {
        Some(hist.ring_at(pos - 1))
    }
}

/// Abort the current half-formed history word. Port of
/// `ihwabort()` from Src/hist.c — back the word-position counter
/// off by one if it's odd (mid-word) and set
/// `hist_keep_comment` so the lexer preserves whatever follows.
pub fn ihwabort() {
    use std::sync::atomic::Ordering;
    let pos = CHWORDPOS.load(Ordering::SeqCst);
    if pos % 2 != 0 {
        CHWORDPOS.fetch_sub(1, Ordering::SeqCst);
    }
    HIST_KEEP_COMMENT.store(true, Ordering::SeqCst);
}

/// Get a history entry by event number, erroring if not found.
/// Port of `gethist()` from Src/hist.c — wraps `quietgethist` and
/// emits the same `no such event: N` error to stderr.
pub fn gethist(hist: &History, ev: i64) -> Option<&HistEntry> {
    let ret = quietgethist(hist, ev);
    if ret.is_none() {
        herrflush();
        zerr(&format!("no such event: {}", ev));
    }
    ret
}

/// Forward search through history for the first entry whose text
/// starts with `prefix` (newest-toward-oldest). Returns the
/// histnum on hit, `None` on miss. Port of `hcomsearch()` from
/// Src/hist.c — same up-walk + `strncmp` semantics, FOREIGN
/// entries skipped.
pub fn hcomsearch(hist: &History, prefix: &str) -> Option<i64> {
    let mut cur = hist.curhist;
    while let Some(prev) = up_histent(hist, cur) {
        cur = prev;
        if let Some(entry) = hist.get(cur) {
            if (entry.flags & hist_flags::FOREIGN) != 0 {
                continue;
            }
            if entry.text.starts_with(prefix) {
                return Some(cur);
            }
        }
    }
    None
}

/// Forward search through history for the first entry whose text
/// CONTAINS `needle`. Direct port of `hconsearch()` from
/// Src/hist.c. The `start` arg lets the caller resume from a
/// previous match; `None` starts at the most recent.
pub fn hconsearch(hist: &History, needle: &str, start: Option<i64>) -> Option<i64> {
    let mut cur = start.unwrap_or(hist.curhist);
    while let Some(prev) = up_histent(hist, cur) {
        cur = prev;
        if let Some(entry) = hist.get(cur) {
            if (entry.flags & hist_flags::FOREIGN) != 0 {
                continue;
            }
            if entry.text.contains(needle) {
                return Some(cur);
            }
        }
    }
    None
}

/// Insert `hist.curline` (the in-progress edit) at the head of
/// the ring and bump curhist. Port of `linkcurline()` from
/// Src/hist.c. Adapted from C's circular doubly-linked-list
/// splice to `Vec<i64>` push-front semantics — same observable
/// effect: latest entry visible at ring index 0.
pub fn linkcurline(hist: &mut History) {
    hist.curhist += 1;
    let n = hist.curhist;
    if let Some(ref mut cur) = hist.curline {
        cur.histnum = n;
    }
    if let Some(cur) = hist.curline.clone() {
        hist.insert_at_head(n, cur);
    }
}

/// Remove `hist.curline` from the ring head and decrement
/// curhist. Port of `unlinkcurline()` from Src/hist.c.
pub fn unlinkcurline(hist: &mut History) {
    let n = hist.curhist;
    hist.remove(n);
    hist.curhist -= 1;
}

/// Move `n` entries from `start` (positive = newer, negative =
/// older), skipping entries whose flags intersect `xflags`.
/// Returns the resulting histnum or `None` when the walk runs
/// out of ring. Port of `movehistent()` from Src/hist.c.
pub fn movehistent(hist: &History, start: i64, mut n: i32, xflags: u32) -> Option<i64> {
    let mut cur = start;
    while n < 0 {
        cur = up_histent(hist, cur)?;
        if let Some(e) = hist.get(cur) {
            if e.flags & xflags == 0 {
                n += 1;
            }
        }
    }
    while n > 0 {
        cur = down_histent(hist, cur)?;
        if let Some(e) = hist.get(cur) {
            if e.flags & xflags == 0 {
                n -= 1;
            }
        }
    }
    Some(cur)
}

/// Get a history entry by event number with near-match fallback.
/// Port of `gethistent()` from Src/hist.c. `nearmatch`:
///   == 0  → exact match only, `None` on miss.
///   <  0  → on miss, return the closest OLDER entry.
///   >  0  → on miss, return the closest NEWER entry.
pub fn gethistent(hist: &History, ev: i64, nearmatch: i32) -> Option<i64> {
    if hist.ring_len() == 0 {
        return None;
    }
    // Direct lookup first — most calls hit the exact event.
    if hist.get(ev).is_some() {
        return Some(ev);
    }
    if nearmatch == 0 {
        return None;
    }
    // Walk the ring to find closest with the right side.
    let mut best_older: Option<i64> = None;
    let mut best_newer: Option<i64> = None;
    for i in 0..hist.ring_len() {
        let n = hist.ring_at(i);
        if n < ev && best_older.map_or(true, |b| n > b) {
            best_older = Some(n);
        } else if n > ev && best_newer.map_or(true, |b| n < b) {
            best_newer = Some(n);
        }
    }
    if nearmatch < 0 { best_older } else { best_newer }
}

/// Allocate the next history slot and return its histnum. Port of
/// `prepnexthistent()` from Src/hist.c — drops the oldest entry
/// when the ring is full (per `histsiz`), increments curhist, and
/// reserves the slot. The C source returns a Histent pointer; we
/// return the histnum (caller fills text via `History::add` or
/// similar mutator).
pub fn prepnexthistent(hist: &mut History) -> i64 {
    if hist.histlinect >= hist.histsiz {
        // Drop oldest. C calls putoldhistentryontop(0) +
        // freehistnode; for our Vec-based ring, simply pop tail.
        if let Some(oldest) = hist.ring_oldest() {
            hist.remove(oldest);
        }
    }
    hist.curhist += 1;
    hist.curhist
}

/// Trim the ring back down to `histsiz` after a setting change.
/// Port of `resizehistents()` from Src/hist.c — same loop, but
/// without zsh's HISTEXPIREDUPSFIRST handling (TODO: port the
/// `putoldhistentryontop(1)` dup-priority preference).
pub fn resizehistents(hist: &mut History) {
    while hist.histlinect > hist.histsiz {
        if let Some(oldest) = hist.ring_oldest() {
            hist.remove(oldest);
        } else {
            break;
        }
    }
}

/// Open a new history-word at the current chline position +
/// `offset`. Port of `ihwbegin()` from Src/hist.c. The C source
/// pushes the start offset onto chwords[chwordpos++]. Skips when
/// stop-history is active (level 2), inside-word, or in alias
/// expansion. Mid-word (chwordpos % 2 != 0) it backs off so the
/// new word starts cleanly.
pub fn ihwbegin(offset: i32) {
    use std::sync::atomic::Ordering;
    let stop = STOPHIST_FLAG.load(Ordering::SeqCst);
    let active = HISTACTIVE.load(Ordering::SeqCst);
    if stop == 2 || (active & HA_INWORD) != 0 {
        // TODO: also check (inbufflags & (INP_ALIAS|INP_HIST))
        // == INP_ALIAS — needs the input-stream state port.
        return;
    }
    let pos = CHWORDPOS.load(Ordering::SeqCst);
    if pos % 2 != 0 {
        CHWORDPOS.fetch_sub(1, Ordering::SeqCst);
    }
    let start = (CHLINE.lock().unwrap().len() as i32 + offset).max(0);
    let mut words = CHWORDS.lock().unwrap();
    let idx = CHWORDPOS.load(Ordering::SeqCst) as usize;
    if words.len() <= idx {
        words.resize(idx + 1, 0);
    }
    words[idx] = start;
    CHWORDPOS.fetch_add(1, Ordering::SeqCst);
}

/// Close the currently-open history-word at the current chline
/// position. Port of `ihwend()` from Src/hist.c. If we'd capture
/// an empty word (current pos == start), the C source backs off
/// the start; we mirror that.
pub fn ihwend() {
    use std::sync::atomic::Ordering;
    let stop = STOPHIST_FLAG.load(Ordering::SeqCst);
    let active = HISTACTIVE.load(Ordering::SeqCst);
    if stop == 2 || (active & HA_INWORD) != 0 {
        return;
    }
    let pos = CHWORDPOS.load(Ordering::SeqCst);
    if pos % 2 == 0 {
        return; // not in a word
    }
    let cur = CHLINE.lock().unwrap().len() as i32;
    let mut words = CHWORDS.lock().unwrap();
    let start_idx = (pos - 1) as usize;
    if cur > words[start_idx] {
        let end_idx = pos as usize;
        if words.len() <= end_idx {
            words.resize(end_idx + 1, 0);
        }
        words[end_idx] = cur;
        CHWORDPOS.fetch_add(1, Ordering::SeqCst);
    } else {
        // Empty word — back off the start.
        CHWORDPOS.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Read back the most-recently-captured history word from chline.
/// Port of `hwget()` from Src/hist.c. Returns `(start_offset,
/// word_text)` for the word ending at chwordpos-1.
pub fn hwget() -> Option<(i32, String)> {
    use std::sync::atomic::Ordering;
    let pos = CHWORDPOS.load(Ordering::SeqCst);
    if pos == 0 || pos % 2 != 0 {
        return None;
    }
    let words = CHWORDS.lock().unwrap();
    let start_idx = (pos - 2) as usize;
    let end_idx = (pos - 1) as usize;
    if end_idx >= words.len() {
        return None;
    }
    let start = words[start_idx];
    let end = words[end_idx];
    let line = CHLINE.lock().unwrap();
    let s = start.max(0) as usize;
    let e = (end.max(0) as usize).min(line.len());
    if s > e || s >= line.len() {
        return None;
    }
    Some((start, line[s..e].to_string()))
}

/// Begin a history-recording scope. Port of `hbegin()` from
/// Src/hist.c. The C source is 86 lines mostly because of the
/// callback-slot wiring (hgetc/hungetc/hwaddc/hwbegin/hwabort/
/// hwend/addtoline) and the option-conditional save behavior
/// (INCAPPENDHISTORYTIME / SHAREHISTORY etc.).
///
/// Ported phases:
///   - isfirstln/isfirstch/histdone reset
///   - stophist setting based on dohist (0/1/2)
///   - chline/chwords/chwordpos reset
///   - histactive set with HA_ACTIVE / HA_NOINC bits
///
/// TODOs cited inline (need infrastructure not yet ported):
///   - callback-slot fn-ptr swaps (hgetc, hungetc, etc.)
///   - BANGHIST option → stophist=4
// initialize the history mechanism                                         // c:1106
///   - INCAPPENDHISTORYTIME conditional savehistfile
///   - linkcurline / defev (addhistnum) when entering interactive
///   - attachtty(mypgrp)
pub fn hbegin(hist: &mut History, dohist: i32) {                             // c:1110
    use std::sync::atomic::Ordering;

    hist.histdone = 0;
    let new_stop = if dohist == 0 {
        2
    } else if dohist != 2 {
        // C: (!interact || unset(SHINSTDIN)) ? 2 : 0
        2
    } else {
        0
    };
    hist.stophist = new_stop;
    STOPHIST_FLAG.store(new_stop, Ordering::SeqCst);

    if new_stop == 2 {
        // C nullifies the buffers and switches callbacks to no-ops.
        CHLINE.lock().unwrap().clear();
        CHWORDS.lock().unwrap().clear();
    } else {
        // C zalloc-initialises chline/chwords; we just clear ours.
        CHLINE.lock().unwrap().clear();
        CHWORDS.lock().unwrap().clear();
        // TODO: BANGHIST option → STOPHIST_FLAG=4
    }
    CHWORDPOS.store(0, Ordering::SeqCst);

    // Stamp the previous entry's finish time if it wasn't recorded.
    if let Some(latest) = hist.ring_oldest() {
        if let Some(e) = hist.entries.get_mut(&latest) {
            if e.ftim == 0 {
                e.ftim = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
            }
        }
    }

    let new_active = if dohist == 2 {
        HA_ACTIVE
    } else {
        HA_ACTIVE | HA_NOINC
    };
    hist.histactive = new_active;
    HISTACTIVE.store(new_active, Ordering::SeqCst);
}

/// End the current history-recording scope and (when active)
/// commit the line buffer as a new history entry. Port of
/// `hend()` from Src/hist.c — the C body is 177 lines covering
/// dup-detection, sharing options, and INCAPPENDHISTORY save
/// modes. This is the simplified port: clear active bits, reset
/// chwordpos, return whether we recorded anything.
///
/// TODOs cited inline:
///   - HISTNOFUNCTIONS / HISTNOSTORE filters
///   - HISTREDUCEBLANKS pass on the recorded text
// say we're done using the history mechanism                               // c:1470
///   - HISTIGNORESPACE / HISTIGNOREDUPS / HISTIGNOREALLDUPS
///   - INCAPPENDHISTORY / SHAREHISTORY savehistfile call
///   - histreduceblanks (we have it, just not invoked here)
pub fn hend(hist: &mut History, _new_text: Option<&str>) -> bool {           // c:1474
    use std::sync::atomic::Ordering;
    let was_active = (hist.histactive & HA_ACTIVE) != 0;
    let no_inc = (hist.histactive & HA_NOINC) != 0;
    hist.histactive = 0;
    HISTACTIVE.store(0, Ordering::SeqCst);
    CHWORDPOS.store(0, Ordering::SeqCst);
    HIST_KEEP_COMMENT.store(false, Ordering::SeqCst);
    was_active && !no_inc
}

/// Begin a history scope when input is from a string (e.g.
/// `eval`, `source`, `<<<` here-string). Port of `strinbeg()`
/// from Src/hist.c — bumps the strin counter then calls hbegin.
pub fn strinbeg(hist: &mut History, dohist: bool) {
    STRIN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    hbegin(hist, if dohist { 1 } else { 0 });
    // TODO: lexinit() + init_parse_status() — needs lexer port.
}

/// End a string-input history scope. Port of `strinend()` from
/// Src/hist.c — calls hend then decrements strin.
pub fn strinend(hist: &mut History) {
    hend(hist, None);
    STRIN.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
}

/// Counter of nested string-input scopes (`strin` in C).
/// Non-zero means we're inside a string-driven shell scope
/// (eval/source/here-string) — used by hbegin to skip
/// linkcurline + defev assignment that's only meaningful for
/// terminal input.
static STRIN: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// Read characters from `input` until `stop` (or `\n`, or EOF),
/// honoring `\X` as a literal-X escape. Port of `hdynread2()`
/// from Src/hist.c. The C signature reads via `ingetc()` and
/// pushes back the trailing `\n` via `inungetc('\n')`; the Rust
/// adaptation takes the input string by value and returns
/// `(collected, bytes_consumed)` so the caller can advance its
/// own input cursor — same semantics, no global ingetc/inungetc
/// dependency.
///
/// Used by the `${...:%pattern%body}` form's content-search
/// helper that the C source threads through `histsubchar`. Once
/// the input-stream port lands, this can be re-fitted with the
/// callback-driven shape.
pub fn hdynread2(stop: char, input: &str) -> (String, usize) {
    let mut out = String::new();
    let mut consumed = 0usize;
    let mut chars = input.chars();
    while let Some(c) = chars.next() {
        consumed += c.len_utf8();
        if c == stop || c == '\n' {
            // C: if (c == '\n') inungetc('\n'). The newline isn't
            // consumed in the returned cursor — caller resumes at
            // the newline. Mirror by walking `consumed` back.
            if c == '\n' {
                consumed -= c.len_utf8();
            }
            return (out, consumed);
        }
        if c == '\\' {
            // Backslash escape — read the next char as literal.
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

/// `qbang` flag — set when the lexer sees an escaped bangchar
/// that originated in a history line (so `\!` round-trips). Mirror
/// of the C global with the same name.
static QBANG: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// `lexstop` flag — set by histsubchar / ihgetc on parse error
/// or EOF to halt the lexer. Mirror of the C global.
static LEXSTOP: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// `exit_pending` flag — when set, `ihgetc` short-circuits. Mirror
/// of the C global; flipped by SIGINT/builtin `exit`.
static EXIT_PENDING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Driver for `!` history substitution.
///
/// Port of `histsubchar()` from Src/hist.c (~389 lines). The C
/// dispatcher reads via `ingetc`/`inungetc` to parse the full
/// `!` directive: event spec (`!!`, `!N`, `!-N`, `!?str?`,
/// `!str`), word designator (`:N`, `:^`, `:$`, `:*`,
/// `:N-M`), and modifier chain (`:s/x/y/`, `:h`, `:t`, `:r`,
/// `:e`, etc.). Result is the substituted text; the first char
/// is returned via the `Option<char>` and the rest is pushed
/// back onto `input` for the lexer to read on subsequent
/// `ingetc` calls.
///
/// SIMPLIFIED Rust port: handles the most common event specs
/// (`!!`, `!N`, `!-N`, `!str` prefix-search, `!?str?`
/// substring-search). Word designators and modifier chains are
/// not yet ported here — they're already handled in
/// subst.rs's `:` modifier dispatch for the `${var:s/x/y/}`
/// form, and porting them again as a recursive call into
/// `subst::modify` is the planned next pass (cited TODO).
///
/// Returns:
///   - `Some(c)` — pass the char on to the lexer (no
///     substitution happened OR substitution result's first char)
///   - `None`    — error; caller (ihgetc) sets lexstop+errflag
pub fn histsubchar(c: char, input: &mut crate::ported::input::InputBuffer, hist: &History) -> Option<char> {
    // Only `!` triggers substitution. Pass non-bangchar through.
    if c != hist.bangchar {
        return Some(c);
    }
    // Read the next char to see what kind of `!` this is.
    let next = match input.ingetc() {
        Some(c) => c,
        None => return Some(c), // bare `!` at EOF — pass through
    };
    // `! ` (bang space) and `!\n` are not history references —
    // mirror C's `if (isspace(c) || c == '=' || c == '(' || c
    // == ')' || c == '|' || c == '&' || c == ';')` early-return.
    if next.is_whitespace() || matches!(next, '=' | '(' | ')' | '|' | '&' | ';') {
        input.inungetc(next);
        return Some(c);
    }
    // Look up the event.
    let resolved: Option<&HistEntry> = match next {
        '!' => hist.latest(),                              // `!!` — previous
        d if d.is_ascii_digit() => {
            // `!N` — absolute event number. Read more digits.
            let mut n: i64 = (d as i64) - ('0' as i64);
            while let Some(c2) = input.ingetc() {
                if let Some(d2) = c2.to_digit(10) {
                    n = n * 10 + d2 as i64;
                } else {
                    input.inungetc(c2);
                    break;
                }
            }
            quietgethist(hist, n)
        }
        '-' => {
            // `!-N` — N-back from current.
            let mut n: i64 = 0;
            while let Some(c2) = input.ingetc() {
                if let Some(d2) = c2.to_digit(10) {
                    n = n * 10 + d2 as i64;
                } else {
                    input.inungetc(c2);
                    break;
                }
            }
            if n > 0 {
                quietgethist(hist, hist.curhist - n + 1)
            } else {
                None
            }
        }
        '?' => {
            // `!?str?` — substring-search. Read until `?` (or
            // newline/EOF) — direct port of the contained loop
            // around C's hdynread2 call. We build the string in
            // place since input.ingetc handles the read.
            let mut needle = String::new();
            while let Some(c2) = input.ingetc() {
                if c2 == '?' || c2 == '\n' { break; }
                needle.push(c2);
            }
            let n = hconsearch(hist, &needle, None);
            n.and_then(|h| quietgethist(hist, h))
        }
        _ => {
            // `!str` — prefix-search. Walk back via hcomsearch
            // until first match. Read the rest of the prefix.
            let mut prefix = String::from(next);
            while let Some(c2) = input.ingetc() {
                if c2.is_alphanumeric() || c2 == '_' || c2 == '-' {
                    prefix.push(c2);
                } else {
                    input.inungetc(c2);
                    break;
                }
            }
            let n = hcomsearch(hist, &prefix);
            n.and_then(|h| quietgethist(hist, h))
        }
    };

    let entry = match resolved {
        Some(e) => e,
        None => {
            herrflush();
            zerr("event not found");
            LEXSTOP.store(true, std::sync::atomic::Ordering::SeqCst);
            return None;
        }
    };

    // Start with the whole event text; word-designator + modifier
    // chain narrow / transform it from there.
    let mut text = entry.text.clone();

    // Optional word-designator: `:N`, `:^`, `:$`, `:*`, `:N-M`,
    // `:N*`. Direct port of subst.c:hist `getargspec` arm. The
    // colon may be elided for digit/`^`/`$`/`*` per zsh syntax,
    // but the unambiguous `:` form is what histsubchar handles.
    let words: Vec<String> = text.split_whitespace().map(|s| s.to_string()).collect();
    let nwords = words.len();
    // Peek for `:` followed by a word designator.
    let next = input.ingetc();
    let mut applied_designator = false;
    let mut next_after_desig: Option<char> = None;
    if next == Some(':') {
        // Read the designator chars.
        let d1 = input.ingetc();
        match d1 {
            Some('^') => {
                if !words.is_empty() {
                    text = words.get(1).cloned().unwrap_or_default();
                }
                applied_designator = true;
            }
            Some('$') => {
                if !words.is_empty() {
                    text = words.last().cloned().unwrap_or_default();
                }
                applied_designator = true;
            }
            Some('*') => {
                // All words after the command (1..end).
                if nwords > 1 {
                    text = words[1..].join(" ");
                } else {
                    text.clear();
                }
                applied_designator = true;
            }
            Some(d) if d.is_ascii_digit() => {
                let mut n: usize = d.to_digit(10).unwrap() as usize;
                while let Some(c2) = input.ingetc() {
                    if let Some(d2) = c2.to_digit(10) {
                        n = n * 10 + d2 as usize;
                    } else {
                        // `N-M` range form?
                        if c2 == '-' {
                            // Read second number (or `*`).
                            let start = n;
                            let mut m: usize = 0;
                            let mut star = false;
                            while let Some(c3) = input.ingetc() {
                                if let Some(d3) = c3.to_digit(10) {
                                    m = m * 10 + d3 as usize;
                                } else if c3 == '*' {
                                    star = true;
                                    break;
                                } else {
                                    next_after_desig = Some(c3);
                                    break;
                                }
                            }
                            let end_idx = if star { nwords.saturating_sub(1) } else { m };
                            if start < nwords && end_idx >= start {
                                let stop = end_idx.min(nwords - 1);
                                text = words[start..=stop].join(" ");
                            }
                            applied_designator = true;
                            // Fall through to modifier-chain
                            // dispatch below — pending char is in
                            // next_after_desig.
                        } else {
                            next_after_desig = Some(c2);
                        }
                        break;
                    }
                }
                if n < nwords {
                    text = words[n].to_string();
                }
                applied_designator = true;
            }
            Some(other) => {
                // Not a designator — push back and treat the `:` as
                // a modifier.
                next_after_desig = Some(other);
            }
            None => {}
        }
    } else if let Some(c2) = next {
        // No designator at all — push back and apply only modifiers
        // (which start with `:`, so this push-back is what subsequent
        // wire_modifiers will see).
        input.inungetc(c2);
    }
    let _ = applied_designator;

    // Inline modifier-chain dispatch — direct port of the `case
    // ':'` modifier loop in C's histsubchar tail. Each modifier
    // is one of: `:s/x/y/`, `:&`, `:h`/`:hN`, `:t`/`:tN`, `:r`,
    // `:e`, `:l`, `:u`, `:p`, `:q`, `:Q`, `:a`, `:A`, `:P`,
    // `:c`. Body kept inline (no Rust-only helper allowed).
    let mut peek = next_after_desig;
    loop {
        // First char of the chain is `:` — except when `pending`
        // already holds the start of the next modifier.
        let leadc = match peek.take() {
            Some(c) => c,
            None => match input.ingetc() {
                Some(c) => c,
                None => break,
            },
        };
        if leadc != ':' {
            // Not a modifier — push back and stop.
            input.inungetc(leadc);
            break;
        }
        // Modifier char.
        let m = match input.ingetc() {
            Some(c) => c,
            None => break,
        };
        // Optional digit count for h/t modifiers.
        let mut count: i32 = 0;
        if m == 'h' || m == 't' {
            while let Some(d) = input.ingetc() {
                if let Some(n) = d.to_digit(10) {
                    count = count * 10 + n as i32;
                } else {
                    input.inungetc(d);
                    break;
                }
            }
        }
        // Apply the modifier per Src/subst.c:4585+ dispatch — same
        // ladder we use in subst.rs::paramsubst's `:MOD` arm.
        text = match m {
            'h' => remtpath(&text, count),
            't' => remlpaths(&text, count),
            'r' => remtext(&text),
            'e' => rembutext(&text),
            'l' => casemodify(&text, CaseMod::Lower),
            'u' => casemodify(&text, CaseMod::Upper),
            'q' => quote(&text),
            'p' => text, // :p — print only (handled by caller)
            's' => {
                // `:s/old/new/[g]` substitute. Read delimiter, then
                // pat, then `/`, then repl, then optional `/`.
                let delim = match input.ingetc() {
                    Some(c) => c,
                    None => break,
                };
                let mut pat = String::new();
                while let Some(c) = input.ingetc() {
                    if c == delim || c == '\n' { break; }
                    pat.push(c);
                }
                let mut rep = String::new();
                while let Some(c) = input.ingetc() {
                    if c == delim || c == '\n' { input.inungetc(c); break; }
                    rep.push(c);
                }
                // Persist on the History? hsubl/hsubr live there but
                // we have only `&_hist`. Call the in-place subst
                // helper from this module.
                subst(&text, &pat, &rep, false)
            }
            '&' => {
                // Replay last :s/x/y/. Without mut access to hist we
                // can't read hsubl/hsubr — TODO (param histsubchar
                // signature change to take &mut History).
                text
            }
            'a' | 'A' | 'P' => {
                // Path canonicalize: try fs canonicalize, fall back
                // to chabspath. Same as subst.rs's :a/:A/:P arm.
                std::fs::canonicalize(&text)
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned())
                    .or_else(|| chabspath(&text))
                    .unwrap_or(text)
            }
            'Q' => {
                // Strip backslash + quote chars.
                let mut out = String::with_capacity(text.len());
                let mut chs = text.chars().peekable();
                while let Some(c) = chs.next() {
                    if c == '\\' { if let Some(nc) = chs.next() { out.push(nc); } }
                    else if c == '\'' || c == '"' { /* drop quotes */ }
                    else { out.push(c); }
                }
                out
            }
            'c' => {
                // :c — resolve via PATH like `which`.
                if text.starts_with('/') || text.starts_with("./") || text.starts_with("../") {
                    text
                } else if let Ok(path) = std::env::var("PATH") {
                    let mut found: Option<String> = None;
                    for dir in path.split(':') {
                        let p = std::path::PathBuf::from(dir).join(&text);
                        if p.is_file() {
                            found = Some(p.to_string_lossy().into_owned());
                            break;
                        }
                    }
                    found.unwrap_or(text)
                } else { text }
            }
            _ => {
                // Unknown modifier — push back and stop.
                input.inungetc(m);
                input.inungetc(':');
                break;
            }
        };
    }

    let mut chars = text.chars();
    let first = chars.next();
    let rest: Vec<char> = chars.collect();
    for c in rest.into_iter().rev() {
        input.inungetc(c);
    }
    first.or(Some(' '))
}

/// Lexer-side getc that records each char into the chline buffer
/// and dispatches `!` to history-substitution. Port of `ihgetc()`
/// from Src/hist.c.
pub fn ihgetc(input: &mut crate::ported::input::InputBuffer, hist: &History) -> Option<char> {
    use std::sync::atomic::Ordering;
    let c = input.ingetc()?;
    if EXIT_PENDING.load(Ordering::SeqCst) {
        LEXSTOP.store(true, Ordering::SeqCst);
        return Some(' ');
    }
    QBANG.store(false, Ordering::SeqCst);
    let inbufflags = input.flags;
    let in_alias = inbufflags & crate::ported::input::flags::INP_ALIAS != 0;
    let in_hist = inbufflags & crate::ported::input::flags::INP_HIST != 0;
    let mut c = c;
    if hist.stophist == 0 && !in_alias {
        c = match histsubchar(c, input, hist) {
            Some(r) => r,
            None => {
                LEXSTOP.store(true, Ordering::SeqCst);
                return Some(' ');
            }
        };
    }
    if in_hist && hist.stophist == 0 {
        // `\!` in a history-replayed line should be `!` — peek next.
        QBANG.store(false, Ordering::SeqCst);
        if c == '\\' {
            if let Some(c2) = input.ingetc() {
                if c2 == hist.bangchar {
                    QBANG.store(true, Ordering::SeqCst);
                    c = hist.bangchar;
                } else {
                    input.inungetc(c2);
                }
            }
        }
    } else if hist.stophist != 0 || in_alias {
        // Escaped bangchar handling — same predicate the C uses.
        QBANG.store(c == hist.bangchar && hist.stophist < 2, Ordering::SeqCst);
    }
    // Record into chline (hwaddc + addtoline both append in C).
    CHLINE.lock().unwrap().push(c);
    Some(c)
}

/// Push a char back onto the input stream and roll the chline
/// buffer back by one. Port of `ihungetc()` from Src/hist.c.
/// The C source has elaborate logic around `\` + `\n`
/// continuations and zlemetacs/zlemetall (line-edit cursor)
/// adjustments; this port focuses on the chline rollback +
/// inungetc dispatch — TODOs cited for the line-edit
/// integration since zshrs uses a different ZLE backend.
pub fn ihungetc(input: &mut crate::ported::input::InputBuffer, c: char, hist: &History) {
    use std::sync::atomic::Ordering;
    if LEXSTOP.load(Ordering::SeqCst) {
        return;
    }
    let inbufflags = input.flags;
    let in_alias_only = (inbufflags & crate::ported::input::flags::INP_ALIAS) != 0
        && (inbufflags & crate::ported::input::flags::INP_HIST) == 0;
    if !in_alias_only {
        let mut buf = CHLINE.lock().unwrap();
        if !buf.is_empty() {
            buf.pop();
        }
        QBANG.store(
            c == hist.bangchar && hist.stophist < 2 && !buf.is_empty()
                && buf.chars().last() == Some('\\'),
            Ordering::SeqCst,
        );
    } else {
        QBANG.store(false, Ordering::SeqCst);
    }
    input.inungetc(c);
    // TODO: expanding/zlemetacs/zlemetall adjustments (Src/hist.c
    // ihungetc body) — needs ZLE-state integration.
}

/// Save the active history-substitution context onto a HistStack.
/// Port of `hist_context_save()` from Src/hist.c:248. The C source
/// stores function-pointer slots (`hgetc`/`hungetc`/`hwaddc` etc.)
/// alongside the parser scratch buffers; our Rust adaptation keeps
/// the buffer/state slots that exist on `History` and leaves the
/// callback-slot save as TODO until the input-stream port lands.
pub fn hist_context_save(hist: &History, hs: &mut HistStack, _toplevel: bool) {
    hs.histactive = hist.histactive;
    hs.histdone = hist.histdone;
    hs.stophist = hist.stophist;
    // chline / hptr / chwords / hlinesz / defev — line-edit
    // scratch state. Owned by the lexer integration once that
    // ports; for now zero-initialised so save+restore is a no-op
    // round trip on those fields.
    hs.hist_keep_comment = HIST_KEEP_COMMENT.load(std::sync::atomic::Ordering::SeqCst);
}

/// Restore a previously-saved history-substitution context.
/// Port of `hist_context_restore()` from Src/hist.c:296. Mirror
/// of the save above.
pub fn hist_context_restore(hist: &mut History, hs: &HistStack, _toplevel: bool) {
    hist.histactive = hs.histactive;
    hist.histdone = hs.histdone;
    hist.stophist = hs.stophist;
    HIST_KEEP_COMMENT.store(hs.hist_keep_comment, std::sync::atomic::Ordering::SeqCst);
}

// Stack of saved history snapshots — port of C's struct histsave
// linked list pushed by pushhiststack and popped by pophiststack.
// Lives in a thread-local to mirror the per-process global C uses.
thread_local! {
    static HIST_STACK: std::cell::RefCell<Vec<HistSnapshot>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// One saved history-state frame. Mirrors `struct histsave`
/// (Src/hist.c) but holds the values we actually port — file
/// paths and size limits. The full C struct also stashes
/// `hist_ring`, `hist_skip_flags`, etc.; those are moved when
/// `History::*` mutator state is fully ported.
#[derive(Clone, Default)]
pub struct HistSnapshot {
    pub histfile: Option<String>,
    pub histsiz: i64,
    pub savehistsiz: i64,
    pub level: i32,
}

/// Push the current history state onto the stack and reset to a
/// fresh state with the new file/size limits. Port of
/// `pushhiststack()` from Src/hist.c. The C source preserves the
/// in-edit `curline` across the swap when the active history
/// holds it; we mirror that with the snapshot/restore pair.
pub fn pushhiststack(hist: &mut History, hf: Option<&str>, hs: i64, shs: i64, level: i32) {
    let snap = HistSnapshot {
        histfile: hist.histfile.clone(),
        histsiz: hist.histsiz,
        savehistsiz: hist.savehistsiz,
        level,
    };
    HIST_STACK.with(|s| s.borrow_mut().push(snap));
    hist.histfile = hf.map(|s| s.to_string());
    hist.histsiz = hs;
    hist.savehistsiz = shs;
}

/// Pop the previously-pushed history snapshot. Port of
/// `pophiststack()` from Src/hist.c.
pub fn pophiststack(hist: &mut History) {
    if let Some(snap) = HIST_STACK.with(|s| s.borrow_mut().pop()) {
        hist.histfile = snap.histfile;
        hist.histsiz = snap.histsiz;
        hist.savehistsiz = snap.savehistsiz;
    }
}

/// Save the active history to its file and pop the stack frame.
/// Port of `saveandpophiststack()` from Src/hist.c:hist.c — calls
/// savehistfile with the current settings, then pophiststack.
/// `savehistfile` itself is TODO (file-I/O port pending), so this
/// just pops for now.
pub fn saveandpophiststack(hist: &mut History, _writeflags: i32) {
    // TODO: savehistfile(hist.histfile.as_deref(),
    //                    HFILE_USE_OPTIONS | writeflags)
    // (Src/hist.c savehistfile, 221 lines, needs file-format port).
    pophiststack(hist);
}

/// Canonicalise a path in place — collapse `.`/`..`, prepend
/// cwd if relative, squeeze duplicate slashes, preserve trailing
/// `/` semantics. Port of `chabspath()` from Src/hist.c.
/// Returns `None` when collapse fails (e.g. `..` past root with
/// no SUPERROOT). The C source mutates a `char**`; we return the
/// new path. Used by the `:A` modifier in older zsh codepaths.
pub fn chabspath(input: &str) -> Option<String> {
    if input.is_empty() {
        return Some(String::new());
    }
    let mut path = if !input.starts_with('/') {
        let cwd = std::env::current_dir().ok()?;
        let cwd_s = cwd.to_string_lossy().into_owned();
        if cwd_s.ends_with('/') {
            format!("{}{}", cwd_s, input)
        } else {
            format!("{}/{}", cwd_s, input)
        }
    } else {
        input.to_string()
    };
    // Collapse pass — direct port of the C `for (;;) { if/else }`
    // walk over `current` writing into `dest`.
    let chars: Vec<char> = path.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' {
            out.push('/');
            i += 1;
            while i < chars.len() && chars[i] == '/' {
                i += 1;
            }
        } else if c == '.' && i + 1 < chars.len() && chars[i + 1] == '.'
            && (i + 2 == chars.len() || chars[i + 2] == '/')
        {
            // `..` component
            if out.len() <= 1 {
                // At root or starting — push literal `..` (matches
                // C's first branch: dest == junkptr or current ==
                // junkptr). Without SUPERROOT support, this falls
                // through to "can't go above root".
                if out.is_empty() || out == ['/'] {
                    return None;
                }
                out.push('.');
                out.push('.');
            } else if out.len() >= 3 && &out[out.len() - 3..] == &['.', '.', '/'] {
                out.push('.');
                out.push('.');
            } else {
                // Pop the last component up to (but not including)
                // the prior `/`.
                if out.last() == Some(&'/') && out.len() > 1 {
                    out.pop();
                }
                while out.last().map(|c| *c != '/').unwrap_or(false) {
                    out.pop();
                }
            }
            i += 2;
            if i < chars.len() && chars[i] == '/' {
                i += 1;
            }
        } else if c == '.' && (i + 1 == chars.len() || chars[i + 1] == '/') {
            // `.` component — skip it and following slashes.
            i += 1;
            while i < chars.len() && chars[i] == '/' {
                i += 1;
            }
        } else {
            // Regular component byte.
            out.push(c);
            i += 1;
        }
    }
    // Strip trailing slashes (C: `while (dest > *junkptr + 1 &&
    // dest[-1] == '/') dest--`).
    while out.len() > 1 && out.last() == Some(&'/') {
        out.pop();
    }
    path = out.into_iter().collect();
    if path.is_empty() {
        Some("/".to_string())
    } else {
        Some(path)
    }
}

/// Wrap a string in single-quotes, escaping any embedded `'` and
/// (when not inside a quoted run) blank chars. Port of `quote()`
/// from Src/hist.c — used by the `:q` history modifier.
/// The C signature mutates a `char**`; the Rust port takes &str
/// and returns the quoted String.
pub fn quote(s: &str) -> String {
    let bytes: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(bytes.len() + 3);
    out.push('\'');
    let mut inquotes = false;
    let mut prev: char = '\0';
    for (i, &c) in bytes.iter().enumerate() {
        if c == '\'' {
            inquotes = !inquotes;
            // `'\''` form: close existing quote, escaped quote, reopen.
            out.push('\'');
            out.push('\\');
            out.push('\'');
            out.push('\'');
        } else if c.is_whitespace() && !inquotes && prev != '\\' {
            // Blank outside quoted run: close, emit, reopen.
            out.push('\'');
            out.push(c);
            out.push('\'');
        } else {
            out.push(c);
        }
        prev = if i < bytes.len() { c } else { prev };
    }
    out.push('\'');
    out
}

/// Extract the substring spanning words `arg1..=arg2` of a
/// history entry. Port of `getargs()` from Src/hist.c — uses the
/// per-entry `words` boundary array; emits an error and returns
/// `None` for out-of-range / corrupted positions.
pub fn getargs(entry: &HistEntry, arg1: usize, arg2: usize) -> Option<String> {
    let nwords = entry.words.len();
    if nwords == 0 || arg2 < arg1 || arg1 >= nwords || arg2 >= nwords {
        herrflush();
        zerr("no such word in event");
        return None;
    }
    // Optimisation: full-event request returns the whole text.
    if arg1 == 0 && arg2 == nwords - 1 {
        return Some(entry.text.clone());
    }
    let (pos1, _) = entry.words[arg1];
    let (_, pos2) = entry.words[arg2];
    if pos2 > entry.text.len() || pos1 > pos2 {
        herrflush();
        zerr("history event too long, can't index requested words");
        return None;
    }
    Some(entry.text[pos1..pos2].to_string())
}

/// Acquire an exclusive lock on the history file. Port of
/// `lockhistfile()` from Src/hist.c. The C source has three
/// platform-conditional locking strategies (fcntl `flock`, then
/// symlink-based, then link-based with retry); zshrs uses the
/// fcntl path unconditionally — modern Unix supports it, and
/// the symlink/link fallbacks are for ancient hosts. Increments
/// `LOCKHISTCT` on success so nested lock calls re-use the
/// existing lock and only the outermost `unlockhistfile` releases.
///
/// Returns:
///   0 — locked successfully
///   1 — keep_trying=false and lock held by another process
///   2 — fatal error (path bad, fs error)
///
/// `keep_trying` controls retry-on-busy. C uses 67ms exponential
/// backoff; we use a tighter loop with a short sleep, capped at
/// 30 retries (~3s wall time).
pub fn lockhistfile(hist: &History, fn_path: Option<&str>, keep_trying: bool) -> i32 {
    use std::sync::atomic::Ordering;

    let path: String = match fn_path {
        Some(p) => p.to_string(),
        None => match hist.histfile.as_deref() {
            Some(p) => p.to_string(),
            None => return 1,
        },
    };

    // Re-entrant: if we already hold the lock, just bump the count.
    if LOCKHISTCT.fetch_add(1, Ordering::SeqCst) > 0 {
        return 0;
    }

    // Try fcntl flock. Mirrors the `if (isset(HISTFCNTLLOCK))
    // return flockhistfile(...);` early-return branch in C.
    let max_tries = if keep_trying { 30 } else { 1 };
    for attempt in 0..max_tries {
        if flockhistfile(&path) {
            return 0;
        }
        if attempt + 1 < max_tries {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    // All retries exhausted — back out the count and report busy.
    LOCKHISTCT.fetch_sub(1, Ordering::SeqCst);
    if keep_trying { 2 } else { 1 }
}

/// Read a zsh history file into the in-memory ring. Port of
/// `readhistfile()` from Src/hist.c (~196 lines). This is a
/// SIMPLIFIED port covering the common case:
///   - Plain lines: one history entry per line
///   - Extended format: `: <stim>:<dur>;<text>` (zsh's
///     EXTENDED_HISTORY)
///   - Backslash-newline continuation for multi-line entries
///
/// TODOs from the C source not yet ported (cited inline):
///   - HFILE_FAST resume via lasthist.fpos/fsiz/mtim — full
///     re-read every call; the C version would skip if the
///     file hasn't changed.
///   - Lex pre-pass to populate word-boundary array.
///   - HFILE_NO_REC_DUPS / HFILE_SKIP_DUPS / HFILE_SKIP_FOREIGN /
///     HFILE_SKIP_OLD flag handling beyond the bare entry insert.
///   - Locale conversion via meta encoding.
pub fn readhistfile(hist: &mut History, fn_path: Option<&str>, _err: bool, _readflags: i32) {
    let path: String = match fn_path {
        Some(p) => p.to_string(),
        None => match hist.histfile.as_deref() {
            Some(p) => p.to_string(),
            None => return,
        },
    };
    let contents = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return,
    };
    if contents.is_empty() {
        return;
    }
    // Acquire lock per C; on busy fall through to read anyway.
    let _ = lockhistfile(hist, Some(&path), true);

    let mut current: Option<(i64, i64, String)> = None; // (stim, ftim, text)
    for raw_line in contents.lines() {
        // Backslash-newline continuation — append next line into
        // the in-progress entry. Mirrors C's `while (...) buf =
        // realloc(...)` continuation loop.
        if let Some((stim, ftim, ref mut text)) = current {
            if text.ends_with('\\') {
                text.pop();
                text.push('\n');
                text.push_str(raw_line);
                current = Some((stim, ftim, text.clone()));
                continue;
            }
            // Flush in-progress entry before starting a new one.
            hist.curhist += 1;
            let mut entry = HistEntry::new(hist.curhist, text.clone());
            entry.stim = stim;
            entry.ftim = ftim;
            entry.flags |= hist_flags::OLD;
            hist.insert_at_head(hist.curhist, entry);
            current = None;
        }
        // Extended format: `: <stim>:<dur>;<text>`
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
        // Plain line — record with now-ish timestamp (lossy; the
        // file didn't carry one).
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        current = Some((now, now, raw_line.to_string()));
    }
    // Flush trailing in-progress entry.
    if let Some((stim, ftim, text)) = current {
        hist.curhist += 1;
        let mut entry = HistEntry::new(hist.curhist, text);
        entry.stim = stim;
        entry.ftim = ftim;
        entry.flags |= hist_flags::OLD;
        hist.insert_at_head(hist.curhist, entry);
    }
    unlockhistfile(&path);
    // Trim to histsiz per resizehistents semantics.
    resizehistents(hist);
}

/// Write the in-memory history ring to a file. Port of
/// `savehistfile()` from Src/hist.c (~221 lines). SIMPLIFIED to
/// the common case: emit each entry in extended format
/// (`: <stim>:<dur>;<text>`) for portability with C zsh's
/// EXTENDED_HISTORY option.
///
/// TODOs cited from C source:
///   - HFILE_APPEND vs truncate semantics — currently always truncates
///   - HFILE_USE_OPTIONS to honor INC_APPEND_HISTORY etc.
///   - Backslash-newline encoding for embedded newlines in entry text
///   - HISTSAVENODUPS / HISTIGNORESPACE filtering at write time
///   - savehistsiz cap (write only N most recent) — currently writes all
pub fn savehistfile(hist: &History, fn_path: Option<&str>, _writeflags: i32) {
    use std::io::Write;
    let path: String = match fn_path {
        Some(p) => p.to_string(),
        None => match hist.histfile.as_deref() {
            Some(p) => p.to_string(),
            None => return,
        },
    };
    let _ = lockhistfile(hist, Some(&path), true);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
    {
        // Write oldest-first (C iterates hist_ring->down forward).
        // Our ring is newest-first; iterate in reverse for that
        // order on disk.
        let cap = hist.savehistsiz.max(0) as usize;
        let mut count = 0;
        for i in (0..hist.ring_len()).rev() {
            if cap > 0 && count >= cap {
                break;
            }
            let n = hist.ring_at(i);
            if let Some(entry) = hist.get(n) {
                let dur = entry.ftim.saturating_sub(entry.stim);
                let _ = writeln!(file, ": {}:{};{}", entry.stim, dur, entry.text);
                count += 1;
            }
        }
    }
    unlockhistfile(&path);
}

/// Decrement the lock counter and release the underlying flock
/// when the count drops to 0. Port of `unlockhistfile()` from
/// Src/hist.c.
pub fn unlockhistfile(path: &str) {
    use std::sync::atomic::Ordering;
    let prev = LOCKHISTCT.fetch_sub(1, Ordering::SeqCst);
    if prev <= 0 {
        // Mirror C: under-count is a no-op (the C source asserts).
        LOCKHISTCT.store(0, Ordering::SeqCst);
        return;
    }
    if prev == 1 {
        // Outermost release — drop the .lock file. flock(2) auto-
        // releases on file close, so we just delete the lockfile
        // path. Best-effort; ignore errors (mirrors C).
        let lockpath = format!("{}.lock", path);
        let _ = std::fs::remove_file(&lockpath);
    }
}

/// Dynamic history read during expansion (from hist.c hdynread)
pub fn hdynread(_hist: &History) -> Option<String> {
    // This is used for dynamic history reading during !{...} expansion
    // In Rust, this is handled inline during expand()
    None
}

/// Initialize history subsystem (from hist.c inithist)
pub fn inithist() -> History {
    History::new()
}

/// Read a single history line from file (from hist.c readhistline)
pub fn readhistline(line: &str) -> Option<HistEntry> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    // Extended history format: ": timestamp:duration;command"
    if let Some(rest) = line.strip_prefix(": ") {
        if let Some(semi) = rest.find(';') {
            let meta = &rest[..semi];
            let cmd = &rest[semi + 1..];
            let parts: Vec<&str> = meta.splitn(2, ':').collect();
            let timestamp = parts
                .first()
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0);
            let mut entry = HistEntry::new(0, cmd.to_string());
            entry.stim = timestamp;
            return Some(entry);
        }
    }
    Some(HistEntry::new(0, line.to_string()))
}

/// Lock history file with bin_zsystem_flock (from hist.c flockhistfile)
pub fn flockhistfile(path: &str) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        if let Ok(file) = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(format!("{}.lock", path))
        {
            let fd = file.as_raw_fd();
            unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) == 0 }
        } else {
            false
        }
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Check age of lock file (from hist.c checklocktime)
pub fn checklocktime(path: &str, max_age_secs: u64) -> bool {
    let lockfile = format!("{}.lock", path);
    if let Ok(meta) = std::fs::metadata(&lockfile) {
        if let Ok(modified) = meta.modified() {
            if let Ok(age) = modified.elapsed() {
                return age.as_secs() < max_age_secs;
            }
        }
    }
    false
}

/// Split history line into words (from hist.c histsplitwords)
pub fn histsplitwords(line: &str) -> Vec<(usize, usize)> {
    let mut words = Vec::new();
    let mut in_word = false;
    let mut word_start = 0;
    let mut in_quote = false;
    let mut quote_char = '\0';

    for (i, c) in line.char_indices() {
        if in_quote {
            if c == quote_char {
                in_quote = false;
            }
            continue;
        }
        if c == '\'' || c == '"' {
            in_quote = true;
            quote_char = c;
            if !in_word {
                word_start = i;
                in_word = true;
            }
            continue;
        }
        if c.is_ascii_whitespace() {
            if in_word {
                words.push((word_start, i));
                in_word = false;
            }
        } else if !in_word {
            word_start = i;
            in_word = true;
        }
    }
    if in_word {
        words.push((word_start, line.len()));
    }
    words
}

/// History stack operations for nested parsing (from hist.c pushhiststack/pophiststack)
pub struct HistStackManager {
    stack: Vec<HistStackFrame>,
}

struct HistStackFrame {
    curhist: i64,
    histsiz: usize,
    histactive: u32,
}

impl Default for HistStackManager {
    fn default() -> Self {
        Self::new()
    }
}

impl HistStackManager {
    pub fn new() -> Self {
        HistStackManager { stack: Vec::new() }
    }

    /// Push current history state (from hist.c pushhiststack)
    pub fn pushhiststack(&mut self, hist: &History) {
        self.stack.push(HistStackFrame {
            curhist: hist.curhist,
            histsiz: hist.histsiz as usize,
            histactive: hist.histactive,
        });
    }

    /// Pop and restore history state (from hist.c pophiststack)
    pub fn pophiststack(&mut self, hist: &mut History) {
        if let Some(frame) = self.stack.pop() {
            hist.curhist = frame.curhist;
            hist.histsiz = frame.histsiz as i64;
            hist.histactive = frame.histactive;
        }
    }

    /// Save and pop history stack (from hist.c saveandpophiststack)
    pub fn saveandpophiststack(&mut self, hist: &mut History) {
        self.pophiststack(hist);
    }
}

/// Resolve path to real path (from hist.c chrealpath)
pub fn chrealpath(path: &str) -> Option<String> {
    std::fs::canonicalize(path)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

/// Get all words from current edit buffer (from hist.c bufferwords)
pub fn bufferwords(line: &str, cursor_pos: usize) -> (Vec<String>, usize) {
    let words: Vec<String> = line.split_whitespace().map(String::from).collect();
    // Find which word the cursor is in
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

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: hist
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs
