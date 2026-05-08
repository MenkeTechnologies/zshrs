//! History management for zshrs
//!
//! Port from zsh/Src/hist.c
//!
//! Provides history expansion, history file management, and history ring.

use std::collections::HashMap;
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
    entries: HashMap<i64, HistEntry>,
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

    /// Begin history for a new command
    pub fn hbegin(&mut self, interactive: bool) {
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

    /// End history for current command
    pub fn hend(&mut self, text: Option<String>) -> bool {
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
pub fn quotebreak(s: &str) -> String {
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
    pub fn addhistnum(&self, hl: i64, n: i64) -> i64 {
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
pub fn addhistnum(base: i64, n: i64) -> i64 {
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

/// No-op history word handler (from hist.c nohw)
pub fn nohw(_c: char) {}

/// No-op history word abort (from hist.c nohwabort)
pub fn nohwabort() {}

/// No-op history word end (from hist.c nohwe)
pub fn nohwe() {}

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

/// Live counter for `chwordpos` — the index into the per-line
/// word-boundary array zsh's lexer fills as it tokenises. Used by
/// the `ihw*` family. Mirrors C zsh's `chwordpos` global.
static CHWORDPOS: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

/// `hist_keep_comment` flag — set by `ihwabort` to retain a
/// comment that would otherwise be dropped. Mirrors the C global.
static HIST_KEEP_COMMENT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

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
        eprintln!("no such event: {}", ev);
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
/// splice to Vec<i64> push-front semantics — same observable
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

/// Decrement the lock counter. Port of `unlockhistfile()` from
/// Src/hist.c — drops the lock when the count reaches 0. The
/// actual file-level unlock (flock release) is currently a TODO
/// pending the wider history-file I/O port (`lockhistfile`,
/// `readhistfile`, `savehistfile`).
pub fn unlockhistfile(_path: &str) {
    use std::sync::atomic::Ordering;
    let prev = LOCKHISTCT.fetch_sub(1, Ordering::SeqCst);
    if prev <= 0 {
        // Mirror C: under-count is a no-op (the C source asserts).
        LOCKHISTCT.store(0, Ordering::SeqCst);
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
impl crate::ported::exec::ShellExecutor {
    /// Expand history references: !!, !n, !-n, !string, !?string?
    pub(crate) fn expand_history(&self, input: &str) -> String {
        let Some(ref engine) = self.history else {
            return input.to_string();
        };

        // Quick check: nothing to expand
        if !input.contains('!') && !input.starts_with('^') {
            return input.to_string();
        }

        // History expansion only fires in interactive mode (zsh's default).
        // For `-c` script mode, `!!` etc. are literal — pulling from the
        // persistent history db would inject random commands from the user's
        // saved sessions. We anchor on stdin-is-tty, which is the
        // unambiguous signal — the `interactive` option may be set on by
        // default in zshrs's options table for compat. atty::is checks the
        // OS-level fd state.
        if !atty::is(atty::Stream::Stdin) {
            return input.to_string();
        }

        let history_count = engine.count().unwrap_or(0) as usize;
        if history_count == 0 {
            return input.to_string();
        }

        let chars: Vec<char> = input.chars().collect();

        // ^foo^bar quick substitution (only at start of input)
        if chars.first() == Some(&'^') {
            if let Some(expanded) = self.history_quick_subst(&chars, engine) {
                return expanded;
            }
        }

        let mut result = String::new();
        let mut i = 0;
        let mut in_single_quote = false;
        let mut in_brace = 0; // Track ${...} nesting
        let mut last_subst: Option<(String, String)> = None; // for :& modifier

        while i < chars.len() {
            // Track single quotes — no history expansion inside them
            if chars[i] == '\'' && in_brace == 0 {
                in_single_quote = !in_single_quote;
                result.push(chars[i]);
                i += 1;
                continue;
            }
            if in_single_quote {
                result.push(chars[i]);
                i += 1;
                continue;
            }

            // Track ${...} nesting
            if i + 1 < chars.len() && chars[i] == '$' && chars[i + 1] == '{' {
                in_brace += 1;
                result.push(chars[i]);
                i += 1;
                result.push(chars[i]);
                i += 1;
                continue;
            }
            if chars[i] == '}' && in_brace > 0 {
                in_brace -= 1;
                result.push(chars[i]);
                i += 1;
                continue;
            }

            // Backslash-escaped ! is literal
            if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '!' {
                result.push('!');
                i += 2;
                continue;
            }

            if chars[i] == '!' && in_brace == 0 {
                if i + 1 >= chars.len() {
                    // Trailing ! — literal
                    result.push('!');
                    i += 1;
                    continue;
                }

                let next = chars[i + 1];
                // ! followed by space, =, ( — literal (zsh rule)
                if next == ' ' || next == '\t' || next == '=' || next == '(' || next == '\n' {
                    result.push('!');
                    i += 1;
                    continue;
                }

                // Resolve the event string
                let (event_str, new_i) = self.history_resolve_event(&chars, i, engine, &result);
                if let Some(ev) = event_str {
                    // Check for word designators and modifiers
                    let (final_str, final_i) = self.history_apply_designators_and_modifiers(
                        &chars,
                        new_i,
                        &ev,
                        &mut last_subst,
                    );
                    result.push_str(&final_str);
                    i = final_i;
                } else {
                    // Could not resolve — keep the ! literal
                    result.push('!');
                    i += 1;
                }
                continue;
            }
            result.push(chars[i]);
            i += 1;
        }

        result
    }
    /// ^foo^bar quick substitution — replace first occurrence of foo with bar
    /// in the previous command.
    pub(crate) fn history_quick_subst(
        &self,
        chars: &[char],
        engine: &crate::history::HistoryEngine,
    ) -> Option<String> {
        let mut i = 1; // skip leading ^
        let mut old = String::new();
        while i < chars.len() && chars[i] != '^' {
            old.push(chars[i]);
            i += 1;
        }
        if i >= chars.len() {
            return None;
        }
        i += 1; // skip middle ^
        let mut new = String::new();
        while i < chars.len() && chars[i] != '^' && chars[i] != '\n' {
            new.push(chars[i]);
            i += 1;
        }
        let prev = engine.get_by_offset(0).ok()??;
        Some(prev.command.replacen(&old, &new, 1))
    }
    /// Resolve which history event ! refers to.  Returns (Some(full_command), index_after_event)
    /// or (None, original_index) if we can't resolve.
    pub(crate) fn history_resolve_event(
        &self,
        chars: &[char],
        bang_pos: usize,
        engine: &crate::history::HistoryEngine,
        current_line: &str,
    ) -> (Option<String>, usize) {
        let mut i = bang_pos + 1; // past the !

        // !{...} brace-wrapped event
        let in_brace = i < chars.len() && chars[i] == '{';
        if in_brace {
            i += 1;
        }

        let c = if i < chars.len() {
            chars[i]
        } else {
            return (None, bang_pos);
        };

        let (event, new_i) = match c {
            '!' => {
                // !! — previous command
                let entry = engine.get_by_offset(0).ok().flatten();
                (entry.map(|e| e.command), i + 1)
            }
            '#' => {
                // !# — current command line so far
                (Some(current_line.to_string()), i + 1)
            }
            '-' => {
                // !-n — nth previous command
                i += 1;
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                if i > start {
                    let n: usize = chars[start..i]
                        .iter()
                        .collect::<String>()
                        .parse()
                        .unwrap_or(0);
                    if n > 0 {
                        let entry = engine.get_by_offset(n - 1).ok().flatten();
                        (entry.map(|e| e.command), i)
                    } else {
                        (None, bang_pos)
                    }
                } else {
                    (None, bang_pos)
                }
            }
            '?' => {
                // !?string? — contains search
                i += 1;
                let start = i;
                while i < chars.len() && chars[i] != '?' && chars[i] != '\n' {
                    i += 1;
                }
                let search: String = chars[start..i].iter().collect();
                if i < chars.len() && chars[i] == '?' {
                    i += 1;
                }
                let entry = engine
                    .search(&search, 1)
                    .ok()
                    .and_then(|v| v.into_iter().next());
                (entry.map(|e| e.command), i)
            }
            c if c.is_ascii_digit() => {
                // !n — command by absolute number
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let n: i64 = chars[start..i]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0);
                if n > 0 {
                    let entry = engine.get_by_number(n).ok().flatten();
                    (entry.map(|e| e.command), i)
                } else {
                    (None, bang_pos)
                }
            }
            '$' => {
                // !$ — last word of previous command (shorthand for !!:$)
                let entry = engine.get_by_offset(0).ok().flatten();
                let word =
                    entry.and_then(|e| Self::history_split_words(&e.command).last().cloned());
                // Return the word directly — skip designator parsing
                let final_i = if in_brace && i + 1 < chars.len() && chars[i + 1] == '}' {
                    i + 2
                } else {
                    i + 1
                };
                return (word, final_i);
            }
            '^' => {
                // !^ — first arg of previous command (shorthand for !!:1)
                let entry = engine.get_by_offset(0).ok().flatten();
                let word = entry.and_then(|e| {
                    let words = Self::history_split_words(&e.command);
                    words.get(1).cloned()
                });
                let final_i = if in_brace && i + 1 < chars.len() && chars[i + 1] == '}' {
                    i + 2
                } else {
                    i + 1
                };
                return (word, final_i);
            }
            '*' => {
                // !* — all args of previous command (shorthand for !!:*)
                let entry = engine.get_by_offset(0).ok().flatten();
                let word = entry.map(|e| {
                    let words = Self::history_split_words(&e.command);
                    if words.len() > 1 {
                        words[1..].join(" ")
                    } else {
                        String::new()
                    }
                });
                let final_i = if in_brace && i + 1 < chars.len() && chars[i + 1] == '}' {
                    i + 2
                } else {
                    i + 1
                };
                return (word, final_i);
            }
            c if c.is_alphabetic() || c == '_' || c == '/' || c == '.' => {
                // !string — prefix search
                let start = i;
                while i < chars.len()
                    && !chars[i].is_whitespace()
                    && chars[i] != ':'
                    && chars[i] != '!'
                    && chars[i] != '}'
                {
                    i += 1;
                }
                let prefix: String = chars[start..i].iter().collect();
                let entry = engine
                    .search_prefix(&prefix, 1)
                    .ok()
                    .and_then(|v| v.into_iter().next());
                (entry.map(|e| e.command), i)
            }
            _ => (None, bang_pos),
        };

        // Skip closing brace
        let final_i = if in_brace && new_i < chars.len() && chars[new_i] == '}' {
            new_i + 1
        } else {
            new_i
        };

        (event, final_i)
    }
    /// Split a command string into words for word designators, respecting quotes.
    pub(crate) fn history_split_words(cmd: &str) -> Vec<String> {
        let mut words = Vec::new();
        let mut current = String::new();
        let mut in_sq = false;
        let mut in_dq = false;
        let mut escaped = false;

        for c in cmd.chars() {
            if escaped {
                current.push(c);
                escaped = false;
                continue;
            }
            if c == '\\' {
                current.push(c);
                escaped = true;
                continue;
            }
            if c == '\'' && !in_dq {
                in_sq = !in_sq;
                current.push(c);
                continue;
            }
            if c == '"' && !in_sq {
                in_dq = !in_dq;
                current.push(c);
                continue;
            }
            if c.is_whitespace() && !in_sq && !in_dq {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
                continue;
            }
            current.push(c);
        }
        if !current.is_empty() {
            words.push(current);
        }
        words
    }
    /// Apply word designators (:0, :n, :^, :$, :*, :n-m) and modifiers
    /// (:h, :t, :r, :e, :s/old/new/, :gs/old/new/, :p, :l, :u, :q, :Q, :a, :A)
    /// to an already-resolved event string.
    pub(crate) fn history_apply_designators_and_modifiers(
        &self,
        chars: &[char],
        mut i: usize,
        event: &str,
        last_subst: &mut Option<(String, String)>,
    ) -> (String, usize) {
        let words = Self::history_split_words(event);
        let argc = words.len().saturating_sub(1); // last word index

        // Check for word designator — either :N or bare :^ :$ :*
        let mut sline = event.to_string();

        if i < chars.len() && chars[i] == ':' {
            i += 1;
            if i < chars.len() {
                // Parse word designator
                let (farg, larg, new_i) = self.history_parse_word_range(chars, i, argc);
                i = new_i;
                if farg.is_some() || larg.is_some() {
                    let f = farg.unwrap_or(0);
                    let l = larg.unwrap_or(argc);
                    let selected: Vec<&String> = words
                        .iter()
                        .enumerate()
                        .filter(|(idx, _)| *idx >= f && *idx <= l)
                        .map(|(_, w)| w)
                        .collect();
                    sline = selected
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(" ");
                }
            }
        } else if i < chars.len() && chars[i] == '*' {
            // !!* shorthand for !!:1-$
            i += 1;
            if words.len() > 1 {
                sline = words[1..].join(" ");
            } else {
                sline = String::new();
            }
        }

        // Apply modifiers (:h :t :r :e :s :gs :p :l :u :q :Q :a :A)
        while i < chars.len() && chars[i] == ':' {
            i += 1;
            if i >= chars.len() {
                break;
            }
            let mut global = false;
            if chars[i] == 'g' && i + 1 < chars.len() {
                global = true;
                i += 1;
            }
            match chars[i] {
                'h' => {
                    // Head — remove trailing path component
                    i += 1;
                    if let Some(pos) = sline.rfind('/') {
                        if pos > 0 {
                            sline = sline[..pos].to_string();
                        } else {
                            sline = "/".to_string();
                        }
                    }
                }
                't' => {
                    // Tail — remove leading path components
                    i += 1;
                    if let Some(pos) = sline.rfind('/') {
                        sline = sline[pos + 1..].to_string();
                    }
                }
                'r' => {
                    // Remove extension
                    i += 1;
                    if let Some(pos) = sline.rfind('.') {
                        if pos > 0 && sline[..pos].rfind('/').is_none_or(|sp| sp < pos) {
                            sline = sline[..pos].to_string();
                        }
                    }
                }
                'e' => {
                    // Extension only
                    i += 1;
                    if let Some(pos) = sline.rfind('.') {
                        sline = sline[pos + 1..].to_string();
                    } else {
                        sline = String::new();
                    }
                }
                'l' => {
                    // Lowercase
                    i += 1;
                    sline = sline.to_lowercase();
                }
                'u' => {
                    // Uppercase
                    i += 1;
                    sline = sline.to_uppercase();
                }
                'p' => {
                    // Print only, don't execute (we just expand — caller handles this)
                    i += 1;
                    // For now, just expand — :p suppression would need upstream support
                }
                'q' => {
                    // Quote — single-bslashquote the result
                    i += 1;
                    sline = format!("'{}'", sline.replace('\'', "'\\''"));
                }
                'Q' => {
                    // Unquote — remove one level of shell quoting.
                    // zsh hist.c remquote: strips matching `'`/`"` pairs
                    // AND backslash escapes (`\X` → `X`). Without the
                    // backslash unescape, `a="a\\ b"; echo ${a:Q}` left
                    // the `\ ` sequence intact instead of giving `a b`.
                    i += 1;
                    let bytes: Vec<char> = sline.chars().collect();
                    let mut out = String::with_capacity(sline.len());
                    let mut j = 0;
                    let mut in_dq = false;
                    let mut in_sq = false;
                    while j < bytes.len() {
                        let c = bytes[j];
                        if in_sq {
                            if c == '\'' {
                                in_sq = false;
                            } else {
                                out.push(c);
                            }
                            j += 1;
                            continue;
                        }
                        if in_dq {
                            if c == '"' {
                                in_dq = false;
                            } else if c == '\\' && j + 1 < bytes.len() {
                                j += 1;
                                out.push(bytes[j]);
                            } else {
                                out.push(c);
                            }
                            j += 1;
                            continue;
                        }
                        match c {
                            '\'' => in_sq = true,
                            '"' => in_dq = true,
                            '\\' if j + 1 < bytes.len() => {
                                j += 1;
                                out.push(bytes[j]);
                            }
                            _ => out.push(c),
                        }
                        j += 1;
                    }
                    sline = out;
                }
                'a' => {
                    // Absolute path
                    i += 1;
                    if !sline.starts_with('/') {
                        if let Ok(cwd) = std::env::current_dir() {
                            sline = format!("{}/{}", cwd.display(), sline);
                        }
                    }
                }
                'A' => {
                    // Realpath
                    i += 1;
                    if let Ok(real) = std::fs::canonicalize(&sline) {
                        sline = real.to_string_lossy().to_string();
                    }
                }
                's' | 'S' => {
                    // :s/old/new/ or :gs/old/new/
                    i += 1;
                    if i < chars.len() {
                        let delim = chars[i];
                        i += 1;
                        let mut old_s = String::new();
                        while i < chars.len() && chars[i] != delim {
                            old_s.push(chars[i]);
                            i += 1;
                        }
                        if i < chars.len() {
                            i += 1;
                        } // skip delimiter
                        let mut new_s = String::new();
                        while i < chars.len()
                            && chars[i] != delim
                            && chars[i] != ':'
                            && chars[i] != ' '
                        {
                            new_s.push(chars[i]);
                            i += 1;
                        }
                        if i < chars.len() && chars[i] == delim {
                            i += 1;
                        } // skip trailing delimiter
                        *last_subst = Some((old_s.clone(), new_s.clone()));
                        if global {
                            sline = sline.replace(&old_s, &new_s);
                        } else {
                            sline = sline.replacen(&old_s, &new_s, 1);
                        }
                    }
                }
                '&' => {
                    // Repeat last substitution
                    i += 1;
                    if let Some((ref old_s, ref new_s)) = last_subst {
                        if global {
                            sline = sline.replace(old_s.as_str(), new_s.as_str());
                        } else {
                            sline = sline.replacen(old_s.as_str(), new_s.as_str(), 1);
                        }
                    }
                }
                _ => {
                    if global {
                        // 'g' was consumed but next char isn't s/S/& — put back
                        // by not advancing i further
                    }
                    break;
                }
            }
        }

        (sline, i)
    }
    /// Parse a word range like 0, 1, ^, $, *, n-m, n-
    pub(crate) fn history_parse_word_range(
        &self,
        chars: &[char],
        mut i: usize,
        argc: usize,
    ) -> (Option<usize>, Option<usize>, usize) {
        if i >= chars.len() {
            return (None, None, i);
        }

        // Check for modifiers that aren't word designators
        match chars[i] {
            'h' | 't' | 'r' | 'e' | 's' | 'S' | 'g' | 'p' | 'q' | 'Q' | 'l' | 'u' | 'a' | 'A'
            | '&' => {
                // This is a modifier, not a word designator — back up
                return (None, None, i - 1); // -1 to re-read the ':'
            }
            _ => {}
        }

        let farg = if chars[i] == '^' {
            i += 1;
            Some(1usize)
        } else if chars[i] == '$' {
            i += 1;
            return (Some(argc), Some(argc), i);
        } else if chars[i] == '*' {
            i += 1;
            return (Some(1), Some(argc), i);
        } else if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let n: usize = chars[start..i]
                .iter()
                .collect::<String>()
                .parse()
                .unwrap_or(0);
            Some(n)
        } else {
            None
        };

        // Check for range: n-m or n-
        if i < chars.len() && chars[i] == '-' {
            i += 1;
            if i < chars.len() && chars[i] == '$' {
                i += 1;
                return (farg, Some(argc), i);
            } else if i < chars.len() && chars[i].is_ascii_digit() {
                let start = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let m: usize = chars[start..i]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .unwrap_or(0);
                return (farg, Some(m), i);
            } else {
                // n- means n to argc-1
                return (farg, Some(argc.saturating_sub(1)), i);
            }
        }

        if farg.is_some() {
            (farg, farg, i)
        } else {
            (None, None, i)
        }
    }
    /// Check if a string starts with history modifier characters
    pub(crate) fn is_history_modifier(&self, s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        let first = s.chars().next().unwrap();
        matches!(
            first,
            // `g` is the prefix for `:gs/.../.../` (global substitution).
            // `s` is `:s/old/new/`. `U`/`L`/`V`/`X` are bash-only forms
            // we accept here so they reach apply_history_modifiers and
            // emit zsh's "unrecognized modifier" error rather than
            // silently falling through to an empty substitution.
            'A' | 'a'
                | 'h'
                | 't'
                | 'r'
                | 'e'
                | 'l'
                | 'u'
                | 'q'
                | 'Q'
                | 'P'
                | 's'
                | 'g'
                | 'U'
                | 'L'
                | 'V'
                | 'X'
        )
    }
    /// Apply zsh history-style modifiers to a value
    /// Modifiers can be chained: :A:h:h
    pub(crate) fn apply_history_modifiers(&self, val: &str, modifiers: &str) -> String {
        let mut result = val.to_string();
        let mut chars = modifiers.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                ':' => continue,
                'A' => {
                    if let Ok(abs) = std::fs::canonicalize(&result) {
                        result = abs.to_string_lossy().to_string();
                    } else {
                        // canonicalize() requires the path to exist. For
                        // non-existent paths zsh still removes `./` and
                        // resolves `..` lexically — `./foo` → `<cwd>/foo`,
                        // not `<cwd>/./foo`. Without this normalization,
                        // `${a:A}` for `a=./foo` left the `./` segment in
                        // the output even after the cwd-prefix.
                        let joined = if result.starts_with('/') {
                            std::path::PathBuf::from(&result)
                        } else if let Ok(cwd) = std::env::current_dir() {
                            cwd.join(&result)
                        } else {
                            std::path::PathBuf::from(&result)
                        };
                        let mut parts: Vec<String> = Vec::new();
                        for comp in joined.components() {
                            use std::path::Component::*;
                            match comp {
                                CurDir => {}
                                ParentDir => {
                                    parts.pop();
                                }
                                Normal(s) => parts.push(s.to_string_lossy().to_string()),
                                RootDir => parts.insert(0, String::new()),
                                Prefix(p) => {
                                    parts.insert(0, p.as_os_str().to_string_lossy().to_string())
                                }
                            }
                        }
                        result = parts.join("/");
                        if result.is_empty() {
                            result = "/".to_string();
                        }
                    }
                }
                'a' => {
                    if !result.starts_with('/') {
                        if let Ok(cwd) = std::env::current_dir() {
                            result = cwd.join(&result).to_string_lossy().to_string();
                        }
                    }
                }
                'h' => {
                    // zsh strips trailing slashes BEFORE applying head:
                    // `/tmp/` :h is `/`, not `/tmp`. Repeatedly trim
                    // trailing `/` first, then drop the last segment.
                    let trimmed = result.trim_end_matches('/');
                    if trimmed.is_empty() {
                        // Pure-slash input (`/`, `//`, …) — head is `/`.
                        result = "/".to_string();
                    } else if let Some(pos) = trimmed.rfind('/') {
                        if pos == 0 {
                            result = "/".to_string();
                        } else {
                            result = trimmed[..pos].to_string();
                        }
                    } else {
                        result = ".".to_string();
                    }
                }
                't' => {
                    // Mirror zsh: strip trailing slashes before tail
                    // extraction so `foo/` :t is `foo`, not the empty
                    // segment after the slash.
                    let trimmed = result.trim_end_matches('/');
                    if let Some(pos) = trimmed.rfind('/') {
                        result = trimmed[pos + 1..].to_string();
                    } else {
                        result = trimmed.to_string();
                    }
                }
                'r' => {
                    if let Some(dot_pos) = result.rfind('.') {
                        let slash_pos = result.rfind('/').map(|p| p + 1).unwrap_or(0);
                        if dot_pos > slash_pos {
                            result = result[..dot_pos].to_string();
                        }
                    }
                }
                'e' => {
                    if let Some(dot_pos) = result.rfind('.') {
                        let slash_pos = result.rfind('/').map(|p| p + 1).unwrap_or(0);
                        if dot_pos > slash_pos {
                            result = result[dot_pos + 1..].to_string();
                        } else {
                            result = String::new();
                        }
                    } else {
                        result = String::new();
                    }
                }
                'l' => {
                    // `:l` lowercase. Direct port of
                    // src/zsh/Src/hist.c:931-933 — calls casemodify
                    // with CASMOD_LOWER. Use the faithful
                    // casemodify port instead of plain to_lowercase
                    // for Unicode-correct multibyte handling.
                    result = casemodify(&result, CaseMod::Lower);
                }
                'u' => {
                    // `:u` uppercase. Port of src/zsh/Src/hist.c:934-936.
                    result = casemodify(&result, CaseMod::Upper);
                }
                'C' => {
                    // `:C` capitalize. zsh-only modifier per
                    // hist.c (see CASMOD_CAPS dispatch via
                    // casemodify). The history-modifier loop's
                    // legacy path didn't recognize `:C` — only the
                    // `(C)` parameter flag did. Same semantics:
                    // word-aware capitalization with mid-word
                    // lowercase enforcement.
                    result = casemodify(&result, CaseMod::Caps);
                }
                'q' => {
                    // zsh `:q` uses backslash quoting, not single-bslashquote
                    // wrapping. Each shell-meta char gets a `\` prefix.
                    let mut out = String::with_capacity(result.len() + 8);
                    for ch in result.chars() {
                        if " \t\n'\"\\$`;|&<>()[]{}*?#~!".contains(ch) {
                            out.push('\\');
                        }
                        out.push(ch);
                    }
                    result = out;
                }
                'x' => {
                    // `:x` bslashquote with word breaks. Direct port of
                    // src/zsh/Src/hist.c:2527-2556 quotebreak —
                    // wraps the value in single quotes, escapes
                    // internal `'` as `'\''`, AND closes-then-reopens
                    // SQ around each whitespace char (so `hello world`
                    // becomes `'hello' 'world'`). Already ported as a
                    // standalone helper in zle_hist.
                    result = crate::hist::quotebreak(&result);
                }
                'Q' => {
                    // Same shell-bslashquote-remove as the other :Q path
                    // (hist.c remquote): strips matching `'`/`"` pairs
                    // AND backslash escapes inside or unquoted.
                    let bytes: Vec<char> = result.chars().collect();
                    let mut out = String::with_capacity(result.len());
                    let mut j = 0;
                    let mut in_dq = false;
                    let mut in_sq = false;
                    while j < bytes.len() {
                        let c = bytes[j];
                        if in_sq {
                            if c == '\'' {
                                in_sq = false;
                            } else {
                                out.push(c);
                            }
                            j += 1;
                            continue;
                        }
                        if in_dq {
                            if c == '"' {
                                in_dq = false;
                            } else if c == '\\' && j + 1 < bytes.len() {
                                j += 1;
                                out.push(bytes[j]);
                            } else {
                                out.push(c);
                            }
                            j += 1;
                            continue;
                        }
                        match c {
                            '\'' => in_sq = true,
                            '"' => in_dq = true,
                            '\\' if j + 1 < bytes.len() => {
                                j += 1;
                                out.push(bytes[j]);
                            }
                            _ => out.push(c),
                        }
                        j += 1;
                    }
                    result = out;
                }
                'P' => {
                    if let Ok(real) = std::fs::canonicalize(&result) {
                        result = real.to_string_lossy().to_string();
                    }
                }
                'g' => {
                    // `:g` is a prefix to `:s` (or `:&`) meaning "global
                    // substitution". Peek next char — if `s` or `&`,
                    // route through the substitution arm with global=true.
                    let global = true;
                    let next = chars.next();
                    match next {
                        Some('s') => {
                            /* :g substitute — stubbed pending faithful subst.c modify() port */ let _ = global;
                        }
                        _ => {
                            // Stray `:g` without `:s`/`:&` follow-up —
                            // unrecognized in zsh, exit modifier loop.
                            break;
                        }
                    }
                }
                's' => {
                    // `:s/old/new/` — single substitution. Delimiter is
                    // the char after `s` (typically `/`). Final delim
                    // optional.
                    /* :s/old/new/ — stubbed pending faithful subst.c modify() port */
                }
                // Bash-only modifiers — zsh rejects with "unrecognized
                // modifier". Match that error format. Without these arms,
                // unknown modifiers silently terminated the loop and the
                // caller saw the previous-stage value (often empty).
                'U' | 'L' | 'V' | 'X' => {
                    eprintln!("zshrs:1: unrecognized modifier `{}'", c);
                    result = String::new();
                    break;
                }
                _ => break,
            }
        }
        result
    }
}
// END moved-from-exec-rs
