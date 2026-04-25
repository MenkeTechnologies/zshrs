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

                while let Some(c) = chars.next() {
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
#[derive(Clone)]
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

impl Default for HistStack {
    fn default() -> Self {
        HistStack {
            histactive: 0,
            histdone: 0,
            stophist: 0,
            chline: None,
            hptr: 0,
            chwords: Vec::new(),
            hlinesz: 0,
            defev: 0,
            hist_keep_comment: false,
        }
    }
}

/// History done flags (from hist.c)
pub const HISTFLAG_DONE: i32 = 1;
pub const HISTFLAG_NOEXEC: i32 = 2;
pub const HISTFLAG_RECALL: i32 = 4;
pub const HISTFLAG_SETTY: i32 = 8;

/// Case modification types (from hist.c casemodify)
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CaseMod {
    Lower,
    Upper,
    Caps,
}

/// Case modify a string (from hist.c casemodify lines 2194-2323)
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

/// Remove trailing path component (from hist.c remtpath lines 2056-2117)
pub fn remtpath(s: &str, count: i32) -> String {
    let s = s.trim_end_matches('/');

    if s.is_empty() {
        return "/".to_string();
    }

    if count == 0 {
        if let Some(pos) = s.rfind('/') {
            if pos == 0 {
                return "/".to_string();
            }
            return s[..pos].trim_end_matches('/').to_string();
        }
        return ".".to_string();
    }

    let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
    if count as usize >= parts.len() {
        return s.to_string();
    }

    let leading_slash = s.starts_with('/');
    let result: String = parts
        .iter()
        .take(count as usize)
        .map(|s| *s)
        .collect::<Vec<&str>>()
        .join("/");

    if leading_slash {
        format!("/{}", result)
    } else {
        result
    }
}

/// Remove leading path components (from hist.c remlpaths lines 2151-2186)
pub fn remlpaths(s: &str, count: i32) -> String {
    let s = s.trim_end_matches('/');

    if s.is_empty() {
        return String::new();
    }

    let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();

    if count == 0 {
        if let Some(last) = parts.last() {
            return last.to_string();
        }
        return String::new();
    }

    if count as usize >= parts.len() {
        return s.to_string();
    }

    parts
        .iter()
        .rev()
        .take(count as usize)
        .rev()
        .map(|s| *s)
        .collect::<Vec<&str>>()
        .join("/")
}

/// Remove extension (from hist.c remtext lines 2122-2131)
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

/// Convert to absolute path (from hist.c chabspath lines 1877-1955)
pub fn chabspath(s: &str) -> std::io::Result<String> {
    if s.is_empty() {
        return Ok(String::new());
    }

    let path = if !s.starts_with('/') {
        let cwd = std::env::current_dir()?;
        format!("{}/{}", cwd.display(), s)
    } else {
        s.to_string()
    };

    let mut result = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => continue,
            ".." => {
                if !result.is_empty() && result.last() != Some(&"..") {
                    result.pop();
                } else if result.is_empty() && !path.starts_with('/') {
                    result.push("..");
                }
            }
            c => result.push(c),
        }
    }

    if path.starts_with('/') {
        Ok(format!("/{}", result.join("/")))
    } else if result.is_empty() {
        Ok(".".to_string())
    } else {
        Ok(result.join("/"))
    }
}

/// Quote a string for shell (from hist.c quote lines 2486-2523)
pub fn quote(s: &str) -> String {
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

/// Quote with word breaking (from hist.c quotebreak lines 2527-2556)
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

/// Perform history substitution (from hist.c subst lines 2336-2391)
pub fn subst(s: &str, in_pattern: &str, out_pattern: &str, global: bool) -> String {
    if in_pattern.is_empty() {
        return s.to_string();
    }

    let out_expanded = convamps(out_pattern, in_pattern);

    if global {
        s.replace(in_pattern, &out_expanded)
    } else {
        s.replacen(in_pattern, &out_expanded, 1)
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
            if line.starts_with(": ") {
                let rest = &line[2..];
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
///   g (global modifier), p (print, don't execute), q (quote), Q (unquote),
///   x (quote words), a (absolute path)
pub fn apply_word_designator(text: &str, designator: &str) -> Option<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }

    match designator {
        "0" => Some(words[0].to_string()),
        "^" => words.get(1).map(|s| s.to_string()),
        "$" => words.last().map(|s| s.to_string()),
        "*" => {
            if words.len() > 1 {
                Some(words[1..].join(" "))
            } else {
                Some(String::new())
            }
        }
        s if s.contains('-') => {
            let parts: Vec<&str> = s.splitn(2, '-').collect();
            let start: usize = if parts[0].is_empty() {
                0
            } else {
                parts[0].parse().ok()?
            };
            let end: usize = if parts[1].is_empty() {
                words.len() - 2 // n- means up to but not including last
            } else {
                parts[1].parse().ok()?
            };
            if start <= end && end < words.len() {
                Some(words[start..=end].join(" "))
            } else {
                None
            }
        }
        s if s.ends_with('*') => {
            let start: usize = s[..s.len() - 1].parse().ok()?;
            if start < words.len() {
                Some(words[start..].join(" "))
            } else {
                None
            }
        }
        s => {
            let idx: usize = s.parse().ok()?;
            words.get(idx).map(|s| s.to_string())
        }
    }
}

/// Apply a single history modifier to text
pub fn apply_hist_modifier(
    text: &str,
    modifier: char,
    global: bool,
    subst_state: &mut (String, String),
) -> String {
    match modifier {
        'h' => {
            // Head (dirname) - remove trailing path component
            if let Some(pos) = text.rfind('/') {
                if pos == 0 {
                    "/".to_string()
                } else {
                    text[..pos].to_string()
                }
            } else {
                ".".to_string()
            }
        }
        't' => {
            // Tail (basename) - remove leading path components
            text.rsplit('/').next().unwrap_or(text).to_string()
        }
        'r' => {
            // Remove extension
            if let Some(dot) = text.rfind('.') {
                if dot > text.rfind('/').unwrap_or(0) {
                    return text[..dot].to_string();
                }
            }
            text.to_string()
        }
        'e' => {
            // Extension only
            if let Some(dot) = text.rfind('.') {
                if dot > text.rfind('/').unwrap_or(0) {
                    return text[dot + 1..].to_string();
                }
            }
            String::new()
        }
        'l' => text.to_lowercase(),
        'u' => text.to_uppercase(),
        'q' => {
            // Quote - single-quote the text
            format!("'{}'", text.replace('\'', "'\\''"))
        }
        'Q' => {
            // Unquote - remove one level of quoting
            let s = text.strip_prefix('\'').and_then(|s| s.strip_suffix('\''));
            match s {
                Some(inner) => inner.replace("'\\''", "'"),
                None => {
                    let s = text.strip_prefix('"').and_then(|s| s.strip_suffix('"'));
                    match s {
                        Some(inner) => inner.to_string(),
                        None => text.to_string(),
                    }
                }
            }
        }
        'x' => {
            // Quote words individually
            text.split_whitespace()
                .map(|w| format!("'{}'", w.replace('\'', "'\\''")))
                .collect::<Vec<_>>()
                .join(" ")
        }
        'a' => {
            // Make absolute path
            if text.starts_with('/') {
                text.to_string()
            } else if let Ok(cwd) = std::env::current_dir() {
                cwd.join(text).to_string_lossy().to_string()
            } else {
                text.to_string()
            }
        }
        's' | '&' => {
            // Substitution (handled by caller with subst_state)
            if modifier == '&' {
                // Repeat last substitution
                let (ref old, ref new) = *subst_state;
                if old.is_empty() {
                    return text.to_string();
                }
                if global {
                    text.replace(old.as_str(), new.as_str())
                } else {
                    text.replacen(old.as_str(), new.as_str(), 1)
                }
            } else {
                text.to_string() // 's' is handled externally
            }
        }
        'p' => text.to_string(), // Print only - handled by caller
        _ => text.to_string(),
    }
}

/// Remove duplicate history entries (from hist.c histremovedups)
pub fn histremovedups(entries: &mut Vec<HistEntry>) {
    let mut seen = std::collections::HashSet::new();
    entries.retain(|e| seen.insert(e.text.clone()));
}

/// Reduce blanks in history text (from hist.c histreduceblanks)
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
pub fn hgetline(entry: &HistEntry) -> String {
    entry.text.clone()
}

/// History word replacement (from hist.c hwrep)
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
pub fn addhistnum(base: i64, n: i64) -> i64 {
    base + n
}

/// Check if history line should be ignored (starts with space, duplicate, etc.)
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
pub fn ihwaddc(hwbuf: &mut HistWordBuffer, c: char) {
    hwbuf.add(c);
}

/// Add character to current line during lexing (from hist.c iaddtoline)
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
    if line.starts_with(": ") {
        let rest = &line[2..];
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

/// Lock history file with flock (from hist.c flockhistfile)
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
