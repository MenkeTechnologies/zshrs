//! Filename generation (globbing) for zshrs
//!
//! Direct port from zsh/Src/glob.c
//!
//! Supports:
//! - Basic glob patterns (*, ?, [...])
//! - Extended glob patterns (#, ##, ~, ^)
//! - Recursive globbing (**/*)
//! - Glob qualifiers (., /, @, etc.)
//! - Brace expansion ({a,b,c}, {1..10})
//! - Sorting and filtering matches

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fs::{self, Metadata};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Sort specifier flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobSort {
    Name,
    Depth,
    Size,
    Atime,
    Mtime,
    Ctime,
    Links,
    None,
    Exec(usize), // index into exec sort strings
}

/// Sort order
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

/// A single sort specification
#[derive(Debug, Clone)]
pub struct SortSpec {
    pub sort_type: GlobSort,
    pub order: SortOrder,
    pub follow_links: bool,
}

/// Time units for qualifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeUnit {
    Seconds,
    Minutes,
    Hours,
    Days,
    Weeks,
    Months,
}

/// Size units for qualifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeUnit {
    Bytes,
    PosixBlocks,
    Kilobytes,
    Megabytes,
    Gigabytes,
    Terabytes,
}

/// Range comparison
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeOp {
    Less,
    Equal,
    Greater,
}

/// A glob qualifier function
#[derive(Debug, Clone)]
pub enum Qualifier {
    /// File type qualifiers
    IsRegular,
    IsDirectory,
    IsSymlink,
    IsSocket,
    IsFifo,
    IsBlockDev,
    IsCharDev,
    IsDevice,
    IsExecutable,

    /// Permission qualifiers
    Readable,
    Writable,
    Executable,
    WorldReadable,
    WorldWritable,
    WorldExecutable,
    GroupReadable,
    GroupWritable,
    GroupExecutable,
    Setuid,
    Setgid,
    Sticky,

    /// Ownership qualifiers
    OwnedByEuid,
    OwnedByEgid,
    OwnedByUid(u32),
    OwnedByGid(u32),

    /// Numeric qualifiers with range
    Size {
        value: u64,
        unit: SizeUnit,
        op: RangeOp,
    },
    Links {
        value: u64,
        op: RangeOp,
    },
    Atime {
        value: i64,
        unit: TimeUnit,
        op: RangeOp,
    },
    Mtime {
        value: i64,
        unit: TimeUnit,
        op: RangeOp,
    },
    Ctime {
        value: i64,
        unit: TimeUnit,
        op: RangeOp,
    },

    /// Mode specification
    Mode {
        yes: u32,
        no: u32,
    },

    /// Device number
    Device(u64),

    /// Non-empty directory
    NonEmptyDir,

    /// Shell evaluation
    Eval(String),
}

/// A glob match with metadata for sorting
#[derive(Debug, Clone)]
pub struct GlobMatch {
    pub name: String,
    pub path: PathBuf,
    pub size: u64,
    pub atime: i64,
    pub mtime: i64,
    pub ctime: i64,
    pub links: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub dev: u64,
    pub ino: u64,
    // For symlink targets (when following)
    pub target_size: u64,
    pub target_atime: i64,
    pub target_mtime: i64,
    pub target_ctime: i64,
    pub target_links: u64,
    // For exec sort strings
    pub sort_strings: Vec<String>,
}

impl GlobMatch {
    pub fn from_path(path: &Path) -> Option<Self> {
        let meta = fs::symlink_metadata(path).ok()?;
        let name = path.file_name()?.to_string_lossy().to_string();

        let (target_size, target_atime, target_mtime, target_ctime, target_links) =
            if meta.file_type().is_symlink() {
                if let Ok(target_meta) = fs::metadata(path) {
                    (
                        target_meta.size(),
                        target_meta.atime(),
                        target_meta.mtime(),
                        target_meta.ctime(),
                        target_meta.nlink(),
                    )
                } else {
                    (
                        meta.size(),
                        meta.atime(),
                        meta.mtime(),
                        meta.ctime(),
                        meta.nlink(),
                    )
                }
            } else {
                (
                    meta.size(),
                    meta.atime(),
                    meta.mtime(),
                    meta.ctime(),
                    meta.nlink(),
                )
            };

        Some(GlobMatch {
            name,
            path: path.to_path_buf(),
            size: meta.size(),
            atime: meta.atime(),
            mtime: meta.mtime(),
            ctime: meta.ctime(),
            links: meta.nlink(),
            mode: meta.mode(),
            uid: meta.uid(),
            gid: meta.gid(),
            dev: meta.dev(),
            ino: meta.ino(),
            target_size,
            target_atime,
            target_mtime,
            target_ctime,
            target_links,
            sort_strings: Vec::new(),
        })
    }

    pub fn compare(&self, other: &Self, specs: &[SortSpec], numeric_sort: bool) -> Ordering {
        for spec in specs {
            let cmp = match spec.sort_type {
                GlobSort::Name => {
                    if numeric_sort {
                        numeric_string_cmp(&self.name, &other.name)
                    } else {
                        self.name.cmp(&other.name)
                    }
                }
                GlobSort::Depth => {
                    let self_depth = self.path.components().count();
                    let other_depth = other.path.components().count();
                    self_depth.cmp(&other_depth)
                }
                GlobSort::Size => {
                    if spec.follow_links {
                        self.target_size.cmp(&other.target_size)
                    } else {
                        self.size.cmp(&other.size)
                    }
                }
                GlobSort::Atime => {
                    if spec.follow_links {
                        other.target_atime.cmp(&self.target_atime)
                    } else {
                        other.atime.cmp(&self.atime)
                    }
                }
                GlobSort::Mtime => {
                    if spec.follow_links {
                        other.target_mtime.cmp(&self.target_mtime)
                    } else {
                        other.mtime.cmp(&self.mtime)
                    }
                }
                GlobSort::Ctime => {
                    if spec.follow_links {
                        other.target_ctime.cmp(&self.target_ctime)
                    } else {
                        other.ctime.cmp(&self.ctime)
                    }
                }
                GlobSort::Links => {
                    if spec.follow_links {
                        other.target_links.cmp(&self.target_links)
                    } else {
                        other.links.cmp(&self.links)
                    }
                }
                GlobSort::None => Ordering::Equal,
                GlobSort::Exec(idx) => {
                    let a = self.sort_strings.get(idx).map(|s| s.as_str()).unwrap_or("");
                    let b = other
                        .sort_strings
                        .get(idx)
                        .map(|s| s.as_str())
                        .unwrap_or("");
                    if numeric_sort {
                        numeric_string_cmp(a, b)
                    } else {
                        a.cmp(b)
                    }
                }
            };

            if cmp != Ordering::Equal {
                return match spec.order {
                    SortOrder::Ascending => cmp,
                    SortOrder::Descending => cmp.reverse(),
                };
            }
        }
        Ordering::Equal
    }
}

/// Numeric string comparison (for numeric glob sort)
fn numeric_string_cmp(a: &str, b: &str) -> Ordering {
    let mut ai = a.chars().peekable();
    let mut bi = b.chars().peekable();

    loop {
        match (ai.peek(), bi.peek()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(&ac), Some(&bc)) => {
                if ac.is_ascii_digit() && bc.is_ascii_digit() {
                    // Compare numeric segments
                    let mut an = String::new();
                    let mut bn = String::new();
                    while let Some(&c) = ai.peek() {
                        if c.is_ascii_digit() {
                            an.push(c);
                            ai.next();
                        } else {
                            break;
                        }
                    }
                    while let Some(&c) = bi.peek() {
                        if c.is_ascii_digit() {
                            bn.push(c);
                            bi.next();
                        } else {
                            break;
                        }
                    }
                    let av: u64 = an.parse().unwrap_or(0);
                    let bv: u64 = bn.parse().unwrap_or(0);
                    match av.cmp(&bv) {
                        Ordering::Equal => continue,
                        other => return other,
                    }
                } else {
                    match ac.cmp(&bc) {
                        Ordering::Equal => {
                            ai.next();
                            bi.next();
                        }
                        other => return other,
                    }
                }
            }
        }
    }
}

/// Glob options
#[derive(Debug, Clone, Default)]
pub struct GlobOptions {
    pub null_glob: bool,
    pub mark_dirs: bool,
    pub no_glob_dots: bool,
    pub list_types: bool,
    pub numeric_sort: bool,
    pub follow_links: bool,
    pub extended_glob: bool,
    pub case_glob: bool,
    pub glob_star_short: bool,
    pub bare_glob_qual: bool,
    pub brace_ccl: bool,
}

/// Parsed glob qualifier set
#[derive(Debug, Clone, Default)]
pub struct QualifierSet {
    pub qualifiers: Vec<Qualifier>,
    pub alternatives: Vec<Vec<Qualifier>>,
    pub negated: bool,
    pub follow_links: bool,
    pub sorts: Vec<SortSpec>,
    pub first: Option<i32>,
    pub last: Option<i32>,
    pub colon_mods: Option<String>,
    pub pre_words: Vec<String>,
    pub post_words: Vec<String>,
}

/// Main glob state
pub struct GlobState {
    pub options: GlobOptions,
    pub matches: Vec<GlobMatch>,
    pub qualifiers: Option<QualifierSet>,
    pathbuf: String,
    pathpos: usize,
}

impl GlobState {
    pub fn new(options: GlobOptions) -> Self {
        GlobState {
            options,
            matches: Vec::new(),
            qualifiers: None,
            pathbuf: String::with_capacity(4096),
            pathpos: 0,
        }
    }

    /// Main entry point: expand a glob pattern
    pub fn glob(&mut self, pattern: &str) -> Vec<String> {
        self.matches.clear();
        self.pathbuf.clear();
        self.pathpos = 0;

        // Check if globbing is enabled and pattern has wildcards
        if !has_wildcards(pattern) {
            return vec![pattern.to_string()];
        }

        // Parse qualifiers if present
        let (pat, quals) = self.parse_qualifiers(pattern);
        self.qualifiers = quals;

        // Parse the pattern into components
        if let Some(complist) = self.parse_pattern(&pat) {
            // Handle absolute vs relative paths
            if pat.starts_with('/') {
                self.pathbuf.push('/');
                self.pathpos = 1;
            }

            // Do the actual globbing
            self.scanner(&complist, 0);
        }

        // Sort results
        self.sort_matches();

        // Apply subscript selection
        self.apply_selection();

        // Extract filenames
        let mut results: Vec<String> = self
            .matches
            .iter()
            .map(|m| {
                let mut s = m.path.to_string_lossy().to_string();
                if self.options.mark_dirs || self.options.list_types {
                    if let Ok(meta) = fs::symlink_metadata(&m.path) {
                        let ch = file_type_char(meta.mode());
                        if self.options.list_types || (self.options.mark_dirs && ch == '/') {
                            s.push(ch);
                        }
                    }
                }
                s
            })
            .collect();

        // Handle no matches
        if results.is_empty() && !self.options.null_glob {
            results.push(pattern.to_string());
        }

        results
    }

    fn parse_qualifiers(&self, pattern: &str) -> (String, Option<QualifierSet>) {
        if !pattern.ends_with(')') {
            return (pattern.to_string(), None);
        }

        // Find matching open paren
        let bytes = pattern.as_bytes();
        let mut depth = 0;
        let mut qual_start = None;

        for i in (0..bytes.len()).rev() {
            match bytes[i] {
                b')' => depth += 1,
                b'(' => {
                    depth -= 1;
                    if depth == 0 {
                        qual_start = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }

        let start = match qual_start {
            Some(s) => s,
            None => return (pattern.to_string(), None),
        };

        // Check for (#q...) explicit qualifier syntax
        let qual_str = &pattern[start + 1..pattern.len() - 1];
        let (is_explicit, qual_content) = if qual_str.starts_with("#q") {
            (true, &qual_str[2..])
        } else if self.options.bare_glob_qual {
            (false, qual_str)
        } else {
            return (pattern.to_string(), None);
        };

        // Don't parse as qualifiers if it contains | or ~ (alternatives/exclusions)
        if !is_explicit && (qual_content.contains('|') || qual_content.contains('~')) {
            return (pattern.to_string(), None);
        }

        // Parse the qualifiers
        let qs = self.parse_qualifier_string(qual_content);
        (pattern[..start].to_string(), Some(qs))
    }

    fn parse_qualifier_string(&self, s: &str) -> QualifierSet {
        let mut qs = QualifierSet::default();
        let mut chars = s.chars().peekable();
        let mut negated = false;
        let mut follow = false;

        while let Some(c) = chars.next() {
            match c {
                '^' => negated = !negated,
                '-' => follow = !follow,
                ',' => {
                    // Start new alternative
                    if !qs.qualifiers.is_empty() {
                        qs.alternatives.push(std::mem::take(&mut qs.qualifiers));
                    }
                    negated = false;
                    follow = false;
                }
                ':' => {
                    // Colon modifiers - rest of string
                    let rest: String = chars.collect();
                    qs.colon_mods = Some(format!(":{}", rest));
                    break;
                }
                // File type qualifiers
                '/' => qs.qualifiers.push(Qualifier::IsDirectory),
                '.' => qs.qualifiers.push(Qualifier::IsRegular),
                '@' => qs.qualifiers.push(Qualifier::IsSymlink),
                '=' => qs.qualifiers.push(Qualifier::IsSocket),
                'p' => qs.qualifiers.push(Qualifier::IsFifo),
                '%' => match chars.peek() {
                    Some('b') => {
                        chars.next();
                        qs.qualifiers.push(Qualifier::IsBlockDev);
                    }
                    Some('c') => {
                        chars.next();
                        qs.qualifiers.push(Qualifier::IsCharDev);
                    }
                    _ => qs.qualifiers.push(Qualifier::IsDevice),
                },
                '*' => qs.qualifiers.push(Qualifier::IsExecutable),
                // Permission qualifiers
                'r' => qs.qualifiers.push(Qualifier::Readable),
                'w' => qs.qualifiers.push(Qualifier::Writable),
                'x' => qs.qualifiers.push(Qualifier::Executable),
                'R' => qs.qualifiers.push(Qualifier::WorldReadable),
                'W' => qs.qualifiers.push(Qualifier::WorldWritable),
                'X' => qs.qualifiers.push(Qualifier::WorldExecutable),
                'A' => qs.qualifiers.push(Qualifier::GroupReadable),
                'I' => qs.qualifiers.push(Qualifier::GroupWritable),
                'E' => qs.qualifiers.push(Qualifier::GroupExecutable),
                's' => qs.qualifiers.push(Qualifier::Setuid),
                'S' => qs.qualifiers.push(Qualifier::Setgid),
                't' => qs.qualifiers.push(Qualifier::Sticky),
                // Ownership
                'U' => qs.qualifiers.push(Qualifier::OwnedByEuid),
                'G' => qs.qualifiers.push(Qualifier::OwnedByEgid),
                'u' => {
                    let uid = self.parse_uid_gid(&mut chars);
                    qs.qualifiers.push(Qualifier::OwnedByUid(uid));
                }
                'g' => {
                    let gid = self.parse_uid_gid(&mut chars);
                    qs.qualifiers.push(Qualifier::OwnedByGid(gid));
                }
                // Size
                'L' => {
                    let (unit, op, val) = self.parse_size_spec(&mut chars);
                    qs.qualifiers.push(Qualifier::Size {
                        value: val,
                        unit,
                        op,
                    });
                }
                // Link count
                'l' => {
                    let (op, val) = self.parse_range_spec(&mut chars);
                    qs.qualifiers.push(Qualifier::Links { value: val, op });
                }
                // Times
                'a' => {
                    let (unit, op, val) = self.parse_time_spec(&mut chars);
                    qs.qualifiers.push(Qualifier::Atime {
                        value: val as i64,
                        unit,
                        op,
                    });
                }
                'm' => {
                    let (unit, op, val) = self.parse_time_spec(&mut chars);
                    qs.qualifiers.push(Qualifier::Mtime {
                        value: val as i64,
                        unit,
                        op,
                    });
                }
                'c' => {
                    let (unit, op, val) = self.parse_time_spec(&mut chars);
                    qs.qualifiers.push(Qualifier::Ctime {
                        value: val as i64,
                        unit,
                        op,
                    });
                }
                // Sort
                'o' | 'O' => {
                    let desc = c == 'O';
                    if let Some(&sc) = chars.peek() {
                        let sort_type = match sc {
                            'n' => {
                                chars.next();
                                GlobSort::Name
                            }
                            'L' => {
                                chars.next();
                                GlobSort::Size
                            }
                            'l' => {
                                chars.next();
                                GlobSort::Links
                            }
                            'a' => {
                                chars.next();
                                GlobSort::Atime
                            }
                            'm' => {
                                chars.next();
                                GlobSort::Mtime
                            }
                            'c' => {
                                chars.next();
                                GlobSort::Ctime
                            }
                            'd' => {
                                chars.next();
                                GlobSort::Depth
                            }
                            'N' => {
                                chars.next();
                                GlobSort::None
                            }
                            _ => GlobSort::Name,
                        };
                        qs.sorts.push(SortSpec {
                            sort_type,
                            order: if desc {
                                SortOrder::Descending
                            } else {
                                SortOrder::Ascending
                            },
                            follow_links: follow,
                        });
                    }
                }
                // Flags
                'N' => { /* nullglob handled elsewhere */ }
                'D' => { /* dotglob handled elsewhere */ }
                'n' => { /* numsort handled elsewhere */ }
                'M' => { /* markdirs handled elsewhere */ }
                'T' => { /* listtypes handled elsewhere */ }
                'F' => qs.qualifiers.push(Qualifier::NonEmptyDir),
                // Subscript
                '[' => {
                    let (first, last) = self.parse_subscript(&mut chars);
                    qs.first = first;
                    qs.last = last;
                }
                _ => {}
            }
        }

        if !qs.qualifiers.is_empty() {
            qs.alternatives.push(std::mem::take(&mut qs.qualifiers));
        }

        qs.negated = negated;
        qs.follow_links = follow;
        qs
    }

    fn parse_uid_gid(&self, chars: &mut std::iter::Peekable<std::str::Chars>) -> u32 {
        // Check for numeric or delimited string
        if chars.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            let mut num = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_ascii_digit() {
                    num.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            num.parse().unwrap_or(0)
        } else {
            // Delimited name - skip for now
            0
        }
    }

    fn parse_size_spec(
        &self,
        chars: &mut std::iter::Peekable<std::str::Chars>,
    ) -> (SizeUnit, RangeOp, u64) {
        let unit = match chars.peek() {
            Some('p') | Some('P') => {
                chars.next();
                SizeUnit::PosixBlocks
            }
            Some('k') | Some('K') => {
                chars.next();
                SizeUnit::Kilobytes
            }
            Some('m') | Some('M') => {
                chars.next();
                SizeUnit::Megabytes
            }
            Some('g') | Some('G') => {
                chars.next();
                SizeUnit::Gigabytes
            }
            Some('t') | Some('T') => {
                chars.next();
                SizeUnit::Terabytes
            }
            _ => SizeUnit::Bytes,
        };
        let (op, val) = self.parse_range_spec(chars);
        (unit, op, val)
    }

    fn parse_time_spec(
        &self,
        chars: &mut std::iter::Peekable<std::str::Chars>,
    ) -> (TimeUnit, RangeOp, u64) {
        let unit = match chars.peek() {
            Some('s') => {
                chars.next();
                TimeUnit::Seconds
            }
            Some('m') => {
                chars.next();
                TimeUnit::Minutes
            }
            Some('h') => {
                chars.next();
                TimeUnit::Hours
            }
            Some('d') => {
                chars.next();
                TimeUnit::Days
            }
            Some('w') => {
                chars.next();
                TimeUnit::Weeks
            }
            Some('M') => {
                chars.next();
                TimeUnit::Months
            }
            _ => TimeUnit::Days,
        };
        let (op, val) = self.parse_range_spec(chars);
        (unit, op, val)
    }

    fn parse_range_spec(&self, chars: &mut std::iter::Peekable<std::str::Chars>) -> (RangeOp, u64) {
        let op = match chars.peek() {
            Some('+') => {
                chars.next();
                RangeOp::Greater
            }
            Some('-') => {
                chars.next();
                RangeOp::Less
            }
            _ => RangeOp::Equal,
        };
        let mut num = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() {
                num.push(c);
                chars.next();
            } else {
                break;
            }
        }
        let val = num.parse().unwrap_or(0);
        (op, val)
    }

    fn parse_subscript(
        &self,
        chars: &mut std::iter::Peekable<std::str::Chars>,
    ) -> (Option<i32>, Option<i32>) {
        let mut first_str = String::new();
        let mut last_str = String::new();
        let mut in_last = false;

        while let Some(&c) = chars.peek() {
            chars.next();
            if c == ']' {
                break;
            } else if c == ',' {
                in_last = true;
            } else if in_last {
                last_str.push(c);
            } else {
                first_str.push(c);
            }
        }

        let first = first_str.parse().ok();
        let last = if in_last {
            last_str.parse().ok()
        } else {
            first
        };
        (first, last)
    }

    fn parse_pattern(&self, pattern: &str) -> Option<Vec<PatternComponent>> {
        let mut components = Vec::new();
        let mut current = String::new();
        let mut chars = pattern.chars().peekable();
        let mut in_bracket = false;

        // Skip leading slash for absolute paths
        if chars.peek() == Some(&'/') {
            chars.next();
        }

        while let Some(c) = chars.next() {
            match c {
                '/' if !in_bracket => {
                    if !current.is_empty() {
                        components.push(PatternComponent::Pattern(current.clone()));
                        current.clear();
                    }
                }
                '[' => {
                    in_bracket = true;
                    current.push(c);
                }
                ']' => {
                    in_bracket = false;
                    current.push(c);
                }
                '*' if !in_bracket && chars.peek() == Some(&'*') => {
                    chars.next();
                    // Check for ***
                    let follow = chars.peek() == Some(&'*');
                    if follow {
                        chars.next();
                    }
                    // Skip trailing /
                    if chars.peek() == Some(&'/') {
                        chars.next();
                    }
                    if !current.is_empty() {
                        components.push(PatternComponent::Pattern(current.clone()));
                        current.clear();
                    }
                    components.push(PatternComponent::Recursive {
                        follow_links: follow,
                    });
                }
                _ => current.push(c),
            }
        }

        if !current.is_empty() {
            components.push(PatternComponent::Pattern(current));
        }

        if components.is_empty() {
            None
        } else {
            Some(components)
        }
    }

    fn scanner(&mut self, components: &[PatternComponent], depth: usize) {
        if components.is_empty() {
            return;
        }

        let base_path = if self.pathbuf.is_empty() {
            ".".to_string()
        } else {
            self.pathbuf.clone()
        };

        match &components[0] {
            PatternComponent::Pattern(pat) => {
                self.scan_pattern(&base_path, pat, &components[1..], depth);
            }
            PatternComponent::Recursive { follow_links } => {
                // Match zero directories first
                self.scanner(&components[1..], depth);
                // Then recurse into subdirectories
                self.scan_recursive(&base_path, &components[1..], *follow_links, depth);
            }
        }
    }

    fn scan_pattern(&mut self, base: &str, pattern: &str, rest: &[PatternComponent], depth: usize) {
        let dir = match fs::read_dir(base) {
            Ok(d) => d,
            Err(_) => return,
        };

        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();

            // Skip hidden files unless pattern starts with .
            if self.options.no_glob_dots && name.starts_with('.') && !pattern.starts_with('.') {
                continue;
            }

            if pattern_match(
                pattern,
                &name,
                self.options.extended_glob,
                self.options.case_glob,
            ) {
                let path = entry.path();

                if rest.is_empty() {
                    // Final component - add to matches if qualifiers pass
                    if self.check_qualifiers(&path) {
                        if let Some(m) = GlobMatch::from_path(&path) {
                            self.matches.push(m);
                        }
                    }
                } else {
                    // More components to match - must be a directory
                    if path.is_dir() {
                        let old_pos = self.pathbuf.len();
                        if !self.pathbuf.is_empty() && !self.pathbuf.ends_with('/') {
                            self.pathbuf.push('/');
                        }
                        self.pathbuf.push_str(&name);
                        self.scanner(rest, depth + 1);
                        self.pathbuf.truncate(old_pos);
                    }
                }
            }
        }
    }

    fn scan_recursive(
        &mut self,
        base: &str,
        rest: &[PatternComponent],
        follow_links: bool,
        depth: usize,
    ) {
        let dir = match fs::read_dir(base) {
            Ok(d) => d,
            Err(_) => return,
        };

        for entry in dir.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();

            // Skip hidden files
            if self.options.no_glob_dots && name.starts_with('.') {
                continue;
            }

            let path = entry.path();
            let is_dir = if follow_links {
                path.is_dir()
            } else {
                entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
            };

            if is_dir {
                let old_pos = self.pathbuf.len();
                if !self.pathbuf.is_empty() && !self.pathbuf.ends_with('/') {
                    self.pathbuf.push('/');
                }
                self.pathbuf.push_str(&name);

                // Try matching rest from this directory
                self.scanner(rest, depth + 1);

                // Continue recursing
                self.scan_recursive(&self.pathbuf.clone(), rest, follow_links, depth + 1);

                self.pathbuf.truncate(old_pos);
            }
        }
    }

    fn check_qualifiers(&self, path: &Path) -> bool {
        let qs = match &self.qualifiers {
            Some(q) => q,
            None => return true,
        };

        if qs.alternatives.is_empty() {
            return true;
        }

        let meta = match if qs.follow_links {
            fs::metadata(path)
        } else {
            fs::symlink_metadata(path)
        } {
            Ok(m) => m,
            Err(_) => return false,
        };

        // Check each alternative (OR)
        for alt in &qs.alternatives {
            if self.check_qualifier_list(alt, path, &meta) {
                return !qs.negated;
            }
        }

        qs.negated
    }

    fn check_qualifier_list(&self, quals: &[Qualifier], path: &Path, meta: &Metadata) -> bool {
        for q in quals {
            if !self.check_single_qualifier(q, path, meta) {
                return false;
            }
        }
        true
    }

    fn check_single_qualifier(&self, qual: &Qualifier, path: &Path, meta: &Metadata) -> bool {
        let mode = meta.mode();
        let ft = meta.file_type();

        match qual {
            Qualifier::IsRegular => ft.is_file(),
            Qualifier::IsDirectory => ft.is_dir(),
            Qualifier::IsSymlink => ft.is_symlink(),
            Qualifier::IsSocket => mode & libc::S_IFMT as u32 == libc::S_IFSOCK as u32,
            Qualifier::IsFifo => mode & libc::S_IFMT as u32 == libc::S_IFIFO as u32,
            Qualifier::IsBlockDev => mode & libc::S_IFMT as u32 == libc::S_IFBLK as u32,
            Qualifier::IsCharDev => mode & libc::S_IFMT as u32 == libc::S_IFCHR as u32,
            Qualifier::IsDevice => {
                let fmt = mode & libc::S_IFMT as u32;
                fmt == libc::S_IFBLK as u32 || fmt == libc::S_IFCHR as u32
            }
            Qualifier::IsExecutable => ft.is_file() && (mode & 0o111 != 0),
            Qualifier::Readable => mode & 0o400 != 0,
            Qualifier::Writable => mode & 0o200 != 0,
            Qualifier::Executable => mode & 0o100 != 0,
            Qualifier::WorldReadable => mode & 0o004 != 0,
            Qualifier::WorldWritable => mode & 0o002 != 0,
            Qualifier::WorldExecutable => mode & 0o001 != 0,
            Qualifier::GroupReadable => mode & 0o040 != 0,
            Qualifier::GroupWritable => mode & 0o020 != 0,
            Qualifier::GroupExecutable => mode & 0o010 != 0,
            Qualifier::Setuid => mode & libc::S_ISUID as u32 != 0,
            Qualifier::Setgid => mode & libc::S_ISGID as u32 != 0,
            Qualifier::Sticky => mode & libc::S_ISVTX as u32 != 0,
            Qualifier::OwnedByEuid => meta.uid() == unsafe { libc::geteuid() },
            Qualifier::OwnedByEgid => meta.gid() == unsafe { libc::getegid() },
            Qualifier::OwnedByUid(uid) => meta.uid() == *uid,
            Qualifier::OwnedByGid(gid) => meta.gid() == *gid,
            Qualifier::Size { value, unit, op } => {
                let size = meta.size();
                let scaled = scale_size(size, *unit);
                compare_range(scaled, *value, *op)
            }
            Qualifier::Links { value, op } => compare_range(meta.nlink(), *value, *op),
            Qualifier::Atime { value, unit, op } => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;
                let diff = now - meta.atime();
                let scaled = scale_time(diff, *unit);
                compare_range(scaled as u64, *value as u64, *op)
            }
            Qualifier::Mtime { value, unit, op } => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;
                let diff = now - meta.mtime();
                let scaled = scale_time(diff, *unit);
                compare_range(scaled as u64, *value as u64, *op)
            }
            Qualifier::Ctime { value, unit, op } => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64;
                let diff = now - meta.ctime();
                let scaled = scale_time(diff, *unit);
                compare_range(scaled as u64, *value as u64, *op)
            }
            Qualifier::Mode { yes, no } => {
                let m = mode & 0o7777;
                (m & yes) == *yes && (m & no) == 0
            }
            Qualifier::Device(dev) => meta.dev() == *dev,
            Qualifier::NonEmptyDir => {
                if !ft.is_dir() {
                    return false;
                }
                if let Ok(mut entries) = fs::read_dir(path) {
                    entries.any(|e| {
                        e.ok()
                            .map(|e| {
                                let name = e.file_name();
                                name != "." && name != ".."
                            })
                            .unwrap_or(false)
                    })
                } else {
                    false
                }
            }
            Qualifier::Eval(_) => true, // Would need shell integration
        }
    }

    fn sort_matches(&mut self) {
        let specs = self
            .qualifiers
            .as_ref()
            .map(|q| q.sorts.clone())
            .unwrap_or_else(|| {
                vec![SortSpec {
                    sort_type: GlobSort::Name,
                    order: SortOrder::Ascending,
                    follow_links: false,
                }]
            });

        if specs.iter().any(|s| s.sort_type == GlobSort::None) {
            return;
        }

        let numeric = self.options.numeric_sort;
        self.matches.sort_by(|a, b| a.compare(b, &specs, numeric));
    }

    fn apply_selection(&mut self) {
        let (first, last) = match &self.qualifiers {
            Some(q) => (q.first, q.last),
            None => return,
        };

        let len = self.matches.len() as i32;
        if len == 0 {
            return;
        }

        let start = match first {
            Some(f) if f < 0 => (len + f).max(0) as usize,
            Some(f) => (f - 1).max(0) as usize,
            None => 0,
        };

        let end = match last {
            Some(l) if l < 0 => (len + l + 1).max(0) as usize,
            Some(l) => l.min(len) as usize,
            None => len as usize,
        };

        if start < end && start < self.matches.len() {
            self.matches = self.matches[start..end.min(self.matches.len())].to_vec();
        } else {
            self.matches.clear();
        }
    }
}

/// Pattern component
#[derive(Debug, Clone)]
enum PatternComponent {
    Pattern(String),
    Recursive { follow_links: bool },
}

/// Check if string has glob wildcards
pub fn has_wildcards(s: &str) -> bool {
    let mut in_bracket = false;
    let mut escape = false;

    for c in s.chars() {
        if escape {
            escape = false;
            continue;
        }
        match c {
            '\\' => escape = true,
            '[' => {
                in_bracket = true;
                return true; // brackets themselves are wildcards
            }
            ']' => in_bracket = false,
            '*' | '?' if !in_bracket => return true,
            '#' | '^' | '~' if !in_bracket => return true,
            _ => {}
        }
    }
    false
}

/// Simple glob pattern matching
pub fn pattern_match(pattern: &str, text: &str, extended: bool, case_sensitive: bool) -> bool {
    let pat = if case_sensitive {
        pattern.to_string()
    } else {
        pattern.to_lowercase()
    };
    let txt = if case_sensitive {
        text.to_string()
    } else {
        text.to_lowercase()
    };

    glob_match_impl(&pat, &txt, extended)
}

fn glob_match_impl(pattern: &str, text: &str, extended: bool) -> bool {
    let mut pi = pattern.chars().peekable();
    let mut ti = text.chars().peekable();

    while let Some(pc) = pi.next() {
        match pc {
            '*' => {
                // ** is handled at higher level
                if pi.peek().is_none() {
                    return true; // * at end matches everything
                }
                // Try matching rest of pattern from each position
                let rest: String = pi.collect();
                let mut pos = 0;
                for (i, _) in text
                    .char_indices()
                    .skip(ti.clone().count().saturating_sub(text.len()))
                {
                    if i >= pos {
                        if glob_match_impl(&rest, &text[i..], extended) {
                            return true;
                        }
                        pos = i + 1;
                    }
                }
                // Also try matching at end
                return glob_match_impl(&rest, "", extended);
            }
            '?' => {
                if ti.next().is_none() {
                    return false;
                }
            }
            '[' => {
                let tc = match ti.next() {
                    Some(c) => c,
                    None => return false,
                };
                if !match_bracket_expr(&mut pi, tc) {
                    return false;
                }
            }
            '#' if extended => {
                // Zero or more of previous - simplified
                continue;
            }
            '^' if extended => {
                // Negation - simplified
                continue;
            }
            '~' if extended => {
                // Exclusion - simplified
                continue;
            }
            '\\' => {
                let escaped = pi.next();
                let tc = ti.next();
                if escaped != tc {
                    return false;
                }
            }
            _ => {
                if ti.next() != Some(pc) {
                    return false;
                }
            }
        }
    }

    ti.peek().is_none()
}

fn match_bracket_expr(pi: &mut std::iter::Peekable<std::str::Chars>, tc: char) -> bool {
    let mut chars_in_class = Vec::new();
    let mut negate = false;
    let mut first = true;

    while let Some(c) = pi.next() {
        if first && (c == '!' || c == '^') {
            negate = true;
            first = false;
            continue;
        }
        first = false;

        if c == ']' && !chars_in_class.is_empty() {
            break;
        }

        if pi.peek() == Some(&'-') {
            pi.next();
            if let Some(&end) = pi.peek() {
                if end != ']' {
                    pi.next();
                    for ch in c..=end {
                        chars_in_class.push(ch);
                    }
                    continue;
                }
            }
            // '-' at end is literal
            chars_in_class.push(c);
            chars_in_class.push('-');
            continue;
        }

        chars_in_class.push(c);
    }

    let matched = chars_in_class.contains(&tc);
    if negate {
        !matched
    } else {
        matched
    }
}

/// File type character for -F style listing
pub fn file_type_char(mode: u32) -> char {
    let fmt = mode & libc::S_IFMT as u32;
    if fmt == libc::S_IFBLK as u32 {
        '#'
    } else if fmt == libc::S_IFCHR as u32 {
        '%'
    } else if fmt == libc::S_IFDIR as u32 {
        '/'
    } else if fmt == libc::S_IFIFO as u32 {
        '|'
    } else if fmt == libc::S_IFLNK as u32 {
        '@'
    } else if fmt == libc::S_IFREG as u32 {
        if mode & 0o111 != 0 {
            '*'
        } else {
            ' '
        }
    } else if fmt == libc::S_IFSOCK as u32 {
        '='
    } else {
        '?'
    }
}

fn scale_size(bytes: u64, unit: SizeUnit) -> u64 {
    match unit {
        SizeUnit::Bytes => bytes,
        SizeUnit::PosixBlocks => (bytes + 511) / 512,
        SizeUnit::Kilobytes => (bytes + 1023) / 1024,
        SizeUnit::Megabytes => (bytes + 1048575) / 1048576,
        SizeUnit::Gigabytes => (bytes + 1073741823) / 1073741824,
        SizeUnit::Terabytes => (bytes + 1099511627775) / 1099511627776,
    }
}

fn scale_time(secs: i64, unit: TimeUnit) -> i64 {
    match unit {
        TimeUnit::Seconds => secs,
        TimeUnit::Minutes => secs / 60,
        TimeUnit::Hours => secs / 3600,
        TimeUnit::Days => secs / 86400,
        TimeUnit::Weeks => secs / 604800,
        TimeUnit::Months => secs / 2592000,
    }
}

fn compare_range(value: u64, target: u64, op: RangeOp) -> bool {
    match op {
        RangeOp::Less => value < target,
        RangeOp::Equal => value == target,
        RangeOp::Greater => value > target,
    }
}

// ============================================================================
// Brace expansion
// ============================================================================

/// Check if string has brace expansion
pub fn has_braces(s: &str, brace_ccl: bool) -> bool {
    let mut depth = 0;
    let mut has_comma = false;
    let mut has_dotdot = false;

    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();

    for i in 0..len {
        match chars[i] {
            '{' => {
                if brace_ccl && depth == 0 {
                    // Check for {a-z} style
                    if i + 2 < len && chars[i + 2] == '}' {
                        return true;
                    }
                }
                depth += 1;
            }
            '}' => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 && (has_comma || has_dotdot) {
                        return true;
                    }
                }
            }
            ',' if depth == 1 => has_comma = true,
            '.' if depth == 1 && i + 1 < len && chars[i + 1] == '.' => has_dotdot = true,
            _ => {}
        }
    }

    false
}

/// Expand braces in a string
pub fn expand_braces(s: &str, brace_ccl: bool) -> Vec<String> {
    if !has_braces(s, brace_ccl) {
        return vec![s.to_string()];
    }

    let mut results = vec![s.to_string()];
    let mut changed = true;

    while changed {
        changed = false;
        let mut new_results = Vec::new();

        for item in &results {
            if let Some(expanded) = expand_single_brace(item, brace_ccl) {
                new_results.extend(expanded);
                changed = true;
            } else {
                new_results.push(item.clone());
            }
        }

        results = new_results;
    }

    results
}

fn expand_single_brace(s: &str, brace_ccl: bool) -> Option<Vec<String>> {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();

    // Find the first brace
    let mut brace_start = None;
    for i in 0..len {
        if chars[i] == '{' {
            brace_start = Some(i);
            break;
        }
    }

    let start = brace_start?;

    // Find matching close brace and contents
    let mut depth = 1;
    let mut comma_positions = Vec::new();
    let mut dotdot_pos = None;

    for i in (start + 1)..len {
        match chars[i] {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    let prefix: String = chars[..start].iter().collect();
                    let suffix: String = chars[i + 1..].iter().collect();
                    let content: String = chars[start + 1..i].iter().collect();

                    // Check for range expansion
                    if let Some(dp) = dotdot_pos {
                        if comma_positions.is_empty() {
                            return expand_range(&prefix, &content, dp, &suffix);
                        }
                    }

                    // Comma expansion
                    if !comma_positions.is_empty() {
                        return expand_comma(&prefix, &content, &comma_positions, &suffix);
                    }

                    // brace_ccl expansion
                    if brace_ccl && content.len() > 0 {
                        return expand_ccl(&prefix, &content, &suffix);
                    }

                    return None;
                }
            }
            ',' if depth == 1 => comma_positions.push(i - start - 1),
            '.' if depth == 1 && i + 1 < len && chars[i + 1] == '.' => {
                if dotdot_pos.is_none() {
                    dotdot_pos = Some(i - start - 1);
                }
            }
            _ => {}
        }
    }

    None
}

fn expand_range(
    prefix: &str,
    content: &str,
    dotdot_pos: usize,
    suffix: &str,
) -> Option<Vec<String>> {
    let left = &content[..dotdot_pos];
    let right_start = dotdot_pos + 2;

    // Check for second ..
    let (right, incr) = if let Some(pos) = content[right_start..].find("..") {
        let r = &content[right_start..right_start + pos];
        let i: i64 = content[right_start + pos + 2..].parse().unwrap_or(1);
        (r, i.abs() as u64)
    } else {
        (&content[right_start..], 1u64)
    };

    // Try numeric range
    if let (Ok(start), Ok(end)) = (left.parse::<i64>(), right.parse::<i64>()) {
        let mut results = Vec::new();
        let (start, end, reverse) = if start <= end {
            (start, end, false)
        } else {
            (end, start, true)
        };

        // Determine padding width
        let width = left.len().max(right.len());
        let pad = left.starts_with('0') || right.starts_with('0');

        let mut vals: Vec<i64> = (start..=end).step_by(incr as usize).collect();
        if reverse {
            vals.reverse();
        }

        for v in vals {
            let s = if pad {
                format!("{}{:0>width$}{}", prefix, v, suffix, width = width)
            } else {
                format!("{}{}{}", prefix, v, suffix)
            };
            results.push(s);
        }
        return Some(results);
    }

    // Try character range
    if left.len() == 1 && right.len() == 1 {
        let start = left.chars().next()?;
        let end = right.chars().next()?;
        let (start, end, reverse) = if start <= end {
            (start, end, false)
        } else {
            (end, start, true)
        };

        let mut results = Vec::new();
        let mut chars: Vec<char> = (start..=end).collect();
        if reverse {
            chars.reverse();
        }

        for c in chars {
            results.push(format!("{}{}{}", prefix, c, suffix));
        }
        return Some(results);
    }

    None
}

fn expand_comma(
    prefix: &str,
    content: &str,
    positions: &[usize],
    suffix: &str,
) -> Option<Vec<String>> {
    let mut results = Vec::new();
    let mut last = 0;

    for &pos in positions {
        let part = &content[last..pos];
        results.push(format!("{}{}{}", prefix, part, suffix));
        last = pos + 1;
    }
    results.push(format!("{}{}{}", prefix, &content[last..], suffix));

    Some(results)
}

fn expand_ccl(prefix: &str, content: &str, suffix: &str) -> Option<Vec<String>> {
    let mut chars_set = HashSet::new();
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if i + 2 < chars.len() && chars[i + 1] == '-' {
            let start = chars[i];
            let end = chars[i + 2];
            for c in start..=end {
                chars_set.insert(c);
            }
            i += 3;
        } else {
            chars_set.insert(chars[i]);
            i += 1;
        }
    }

    let mut results: Vec<String> = chars_set
        .iter()
        .map(|c| format!("{}{}{}", prefix, c, suffix))
        .collect();
    results.sort();
    Some(results)
}

// ============================================================================
// Convenience functions
// ============================================================================

/// Glob with default options
pub fn glob(pattern: &str) -> Vec<String> {
    let mut state = GlobState::new(GlobOptions {
        null_glob: false,
        mark_dirs: false,
        no_glob_dots: true,
        list_types: false,
        numeric_sort: false,
        follow_links: false,
        extended_glob: true,
        case_glob: true,
        glob_star_short: false,
        bare_glob_qual: true,
        brace_ccl: false,
    });
    state.glob(pattern)
}

/// Glob with custom options
pub fn glob_with_options(pattern: &str, options: GlobOptions) -> Vec<String> {
    let mut state = GlobState::new(options);
    state.glob(pattern)
}

/// Add path component (from glob.c addpath lines 263-274)
pub fn addpath(buf: &mut String, component: &str) {
    buf.push_str(component);
    if !buf.ends_with('/') {
        buf.push('/');
    }
}

/// Stat full path (from glob.c statfullpath lines 282-347)
pub fn statfullpath(pathbuf: &str, name: &str, follow: bool) -> Option<std::fs::Metadata> {
    let full = if name.is_empty() {
        if pathbuf.is_empty() {
            ".".to_string()
        } else {
            pathbuf.to_string()
        }
    } else {
        format!("{}{}", pathbuf, name)
    };

    if follow {
        std::fs::metadata(&full).ok()
    } else {
        std::fs::symlink_metadata(&full).ok()
    }
}

/// Check if path is a directory (from glob.c)
pub fn is_directory(path: &str) -> bool {
    std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

/// Check if path is a symlink
pub fn is_symlink(path: &str) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Match minimum distance for spelling correction (from glob.c mindist lines 3523-3575)
pub fn mindist(dir: &str, name: &str, best: &mut String, exact: bool) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return usize::MAX;
    };

    let mut min_dist = usize::MAX;

    for entry in entries.flatten() {
        let entry_name = entry.file_name().to_string_lossy().to_string();
        if exact && entry_name == name {
            *best = entry_name;
            return 0;
        }

        let dist = crate::utils::spdist(name, &entry_name, min_dist);
        if dist < min_dist {
            min_dist = dist;
            *best = entry_name.clone();
        }
    }

    min_dist
}

/// Parse qualifier (from glob.c qgetnum)
pub fn qgetnum(s: &str) -> Option<(i64, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    let num = s[..end].parse::<i64>().ok()?;
    Some((num, &s[end..]))
}

/// Parse time modifier (from glob.c qualtime)
pub fn qualtime(s: &str, units: char) -> Option<(i64, &str)> {
    let (mut num, rest) = qgetnum(s)?;

    match units {
        'h' => num *= 3600,
        'd' => num *= 86400,
        'w' => num *= 604800,
        'M' => num *= 2592000,
        _ => {}
    }

    Some((num, rest))
}

/// Parse size modifier (from glob.c qualsize)
pub fn qualsize(s: &str, units: char) -> Option<(i64, &str)> {
    let (mut num, rest) = qgetnum(s)?;

    match units {
        'k' | 'K' => num *= 1024,
        'm' | 'M' => num *= 1024 * 1024,
        'g' | 'G' => num *= 1024 * 1024 * 1024,
        't' | 'T' => num *= 1024 * 1024 * 1024 * 1024,
        'p' | 'P' => num *= 512,
        _ => {}
    }

    Some((num, rest))
}

/// Sort glob matches by type (from glob.c gmatchcmp lines 3595-3680)
pub fn sort_matches_by_type(matches: &mut [String], sort_type: GlobSort, reverse: bool) {
    match sort_type {
        GlobSort::Name => {
            matches.sort();
        }
        GlobSort::Size => {
            matches.sort_by(|a, b| {
                let size_a = std::fs::metadata(a).map(|m| m.len()).unwrap_or(0);
                let size_b = std::fs::metadata(b).map(|m| m.len()).unwrap_or(0);
                size_a.cmp(&size_b)
            });
        }
        GlobSort::Mtime => {
            matches.sort_by(|a, b| {
                let time_a = std::fs::metadata(a).and_then(|m| m.modified()).ok();
                let time_b = std::fs::metadata(b).and_then(|m| m.modified()).ok();
                time_a.cmp(&time_b)
            });
        }
        GlobSort::Atime => {
            matches.sort_by(|a, b| {
                let time_a = std::fs::metadata(a).and_then(|m| m.accessed()).ok();
                let time_b = std::fs::metadata(b).and_then(|m| m.accessed()).ok();
                time_a.cmp(&time_b)
            });
        }
        GlobSort::Depth => {
            matches.sort_by(|a, b| {
                let depth_a = a.matches('/').count();
                let depth_b = b.matches('/').count();
                depth_a.cmp(&depth_b)
            });
        }
        GlobSort::Links => {
            matches.sort_by(|a, b| {
                let links_a = std::fs::metadata(a).map(|m| m.nlink()).unwrap_or(0);
                let links_b = std::fs::metadata(b).map(|m| m.nlink()).unwrap_or(0);
                links_a.cmp(&links_b)
            });
        }
        _ => {}
    }

    if reverse {
        matches.reverse();
    }
}

/// File qualifier test functions (from glob.c qual* functions)
pub mod qualifiers {
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;

    pub fn is_regular(path: &str) -> bool {
        std::fs::metadata(path)
            .map(|m| m.is_file())
            .unwrap_or(false)
    }

    pub fn is_directory(path: &str) -> bool {
        std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
    }

    pub fn is_symlink(path: &str) -> bool {
        std::fs::symlink_metadata(path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    }

    pub fn is_fifo(path: &str) -> bool {
        std::fs::metadata(path)
            .map(|m| (m.mode() & libc::S_IFMT as u32) == libc::S_IFIFO as u32)
            .unwrap_or(false)
    }

    pub fn is_socket(path: &str) -> bool {
        std::fs::metadata(path)
            .map(|m| (m.mode() & libc::S_IFMT as u32) == libc::S_IFSOCK as u32)
            .unwrap_or(false)
    }

    pub fn is_block_device(path: &str) -> bool {
        std::fs::metadata(path)
            .map(|m| (m.mode() & libc::S_IFMT as u32) == libc::S_IFBLK as u32)
            .unwrap_or(false)
    }

    pub fn is_char_device(path: &str) -> bool {
        std::fs::metadata(path)
            .map(|m| (m.mode() & libc::S_IFMT as u32) == libc::S_IFCHR as u32)
            .unwrap_or(false)
    }

    pub fn is_setuid(path: &str) -> bool {
        std::fs::metadata(path)
            .map(|m| (m.mode() & libc::S_ISUID as u32) != 0)
            .unwrap_or(false)
    }

    pub fn is_setgid(path: &str) -> bool {
        std::fs::metadata(path)
            .map(|m| (m.mode() & libc::S_ISGID as u32) != 0)
            .unwrap_or(false)
    }

    pub fn is_sticky(path: &str) -> bool {
        std::fs::metadata(path)
            .map(|m| (m.mode() & libc::S_ISVTX as u32) != 0)
            .unwrap_or(false)
    }

    pub fn is_readable(path: &str) -> bool {
        std::fs::metadata(path).is_ok() && std::fs::File::open(path).is_ok()
    }

    pub fn is_writable(path: &str) -> bool {
        std::fs::OpenOptions::new().write(true).open(path).is_ok()
    }

    pub fn is_executable(path: &str) -> bool {
        std::fs::metadata(path)
            .map(|m| (m.mode() & 0o111) != 0)
            .unwrap_or(false)
    }

    pub fn size_matches(path: &str, size: u64, cmp: std::cmp::Ordering) -> bool {
        std::fs::metadata(path)
            .map(|m| m.len().cmp(&size) == cmp)
            .unwrap_or(false)
    }

    pub fn mtime_matches(path: &str, secs: i64, cmp: std::cmp::Ordering) -> bool {
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .map(|t| {
                let elapsed = t.elapsed().map(|d| d.as_secs() as i64).unwrap_or(0);
                elapsed.cmp(&secs) == cmp
            })
            .unwrap_or(false)
    }

    pub fn uid_matches(path: &str, uid: u32) -> bool {
        std::fs::metadata(path)
            .map(|m| m.uid() == uid)
            .unwrap_or(false)
    }

    pub fn gid_matches(path: &str, gid: u32) -> bool {
        std::fs::metadata(path)
            .map(|m| m.gid() == gid)
            .unwrap_or(false)
    }

    pub fn nlinks_matches(path: &str, nlinks: u64, cmp: std::cmp::Ordering) -> bool {
        std::fs::metadata(path)
            .map(|m| m.nlink().cmp(&nlinks) == cmp)
            .unwrap_or(false)
    }

    /// Check if file is an executable command (from glob.c qualiscom)
    pub fn is_command(path: &str) -> bool {
        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => return false,
        };

        if !meta.is_file() {
            return false;
        }

        // Check if executable
        let mode = meta.mode();
        if mode & 0o111 == 0 {
            return false;
        }

        // Check if in PATH would make it a command
        // For now just check executable bit
        true
    }
}

// ============================================================================
// Pattern matching with replacement (from glob.c getmatch family)
// ============================================================================

/// Match flags for getmatch
#[derive(Debug, Clone, Copy)]
pub struct MatchFlags {
    /// Match at start
    pub anchored_start: bool,
    /// Match at end
    pub anchored_end: bool,
    /// Shortest match
    pub shortest: bool,
    /// Subexpression matching
    pub subexpr: bool,
}

impl Default for MatchFlags {
    fn default() -> Self {
        MatchFlags {
            anchored_start: false,
            anchored_end: false,
            shortest: false,
            subexpr: false,
        }
    }
}

/// Internal match data
#[derive(Debug, Clone)]
pub struct MatchData {
    pub str: String,
    pub pattern: String,
    pub match_start: usize,
    pub match_end: usize,
    pub replacement: Option<String>,
}

/// Get match return value (from glob.c get_match_ret lines 2338-2420)
pub fn get_match_ret(data: &MatchData, start: usize, end: usize) -> String {
    if start >= end || start >= data.str.len() {
        return String::new();
    }

    let end = end.min(data.str.len());
    data.str[start..end].to_string()
}

/// Compile pattern and get match info (from glob.c compgetmatch lines 2430-2510)
pub fn compgetmatch(pat: &str) -> Option<(String, MatchFlags)> {
    let mut flags = MatchFlags::default();
    let mut pattern = pat.to_string();

    // Check for anchors
    if pattern.starts_with('#') {
        flags.anchored_start = true;
        pattern = pattern[1..].to_string();
    }
    if pattern.starts_with("##") {
        flags.anchored_start = true;
        flags.shortest = false;
        pattern = pattern[2..].to_string();
    }
    if pattern.ends_with('%') {
        flags.anchored_end = true;
        pattern.pop();
    }
    if pattern.ends_with("%%") {
        flags.anchored_end = true;
        flags.shortest = false;
        pattern.truncate(pattern.len().saturating_sub(2));
    }

    Some((pattern, flags))
}

/// Get pattern match with optional replacement (from glob.c getmatch lines 2520-2680)
///
/// This implements ${var#pat}, ${var##pat}, ${var%pat}, ${var%%pat},
/// ${var/pat/repl}, ${var//pat/repl}
pub fn getmatch(s: &str, pat: &str, flags: MatchFlags, n: i32, replstr: Option<&str>) -> String {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();

    if len == 0 {
        return s.to_string();
    }

    // Find match
    let (match_start, match_end) = if flags.anchored_start && flags.anchored_end {
        // Full match
        if pattern_match(pat, s, true, true) {
            (0, len)
        } else {
            return s.to_string();
        }
    } else if flags.anchored_start {
        // Match from start (# or ##)
        let mut best_end = 0;
        for end in 1..=len {
            let substr: String = chars[..end].iter().collect();
            if pattern_match(pat, &substr, true, true) {
                if flags.shortest {
                    return match replstr {
                        Some(r) => format!("{}{}", r, chars[end..].iter().collect::<String>()),
                        None => chars[end..].iter().collect(),
                    };
                }
                best_end = end;
            }
        }
        if best_end > 0 {
            (0, best_end)
        } else {
            return s.to_string();
        }
    } else if flags.anchored_end {
        // Match from end (% or %%)
        let mut best_start = len;
        for start in (0..len).rev() {
            let substr: String = chars[start..].iter().collect();
            if pattern_match(pat, &substr, true, true) {
                if flags.shortest {
                    return match replstr {
                        Some(r) => format!("{}{}", chars[..start].iter().collect::<String>(), r),
                        None => chars[..start].iter().collect(),
                    };
                }
                best_start = start;
            }
        }
        if best_start < len {
            (best_start, len)
        } else {
            return s.to_string();
        }
    } else {
        // Floating match (/ or //)
        for start in 0..len {
            for end in (start + 1)..=len {
                let substr: String = chars[start..end].iter().collect();
                if pattern_match(pat, &substr, true, true) {
                    let prefix: String = chars[..start].iter().collect();
                    let suffix: String = chars[end..].iter().collect();
                    return match replstr {
                        Some(r) => format!("{}{}{}", prefix, r, suffix),
                        None => format!("{}{}", prefix, suffix),
                    };
                }
            }
        }
        return s.to_string();
    };

    // Apply replacement
    let prefix: String = chars[..match_start].iter().collect();
    let suffix: String = chars[match_end..].iter().collect();

    match replstr {
        Some(r) => format!("{}{}{}", prefix, r, suffix),
        None => format!("{}{}", prefix, suffix),
    }
}

/// Get match for array elements (from glob.c getmatcharr lines 2690-2750)
pub fn getmatcharr(
    arr: &[String],
    pat: &str,
    flags: MatchFlags,
    n: i32,
    replstr: Option<&str>,
) -> Vec<String> {
    arr.iter()
        .map(|s| getmatch(s, pat, flags, n, replstr))
        .collect()
}

/// Get match list for global replacement (from glob.c getmatchlist lines 2760-2850)
pub fn getmatchlist(s: &str, pat: &str) -> Vec<(usize, usize)> {
    let mut matches = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();

    let mut pos = 0;
    while pos < len {
        for end in (pos + 1)..=len {
            let substr: String = chars[pos..end].iter().collect();
            if pattern_match(pat, &substr, true, true) {
                matches.push((pos, end));
                pos = end;
                break;
            }
        }
        if matches.last().map(|&(_, e)| e) != Some(pos) {
            pos += 1;
        }
    }

    matches
}

/// Set pattern start offset (from glob.c set_pat_start)
pub fn set_pat_start(pattern: &str, offset: usize) -> String {
    if offset == 0 || offset >= pattern.len() {
        return pattern.to_string();
    }
    pattern[offset..].to_string()
}

/// Set pattern end (from glob.c set_pat_end)
pub fn set_pat_end(pattern: &str, end: usize) -> String {
    if end >= pattern.len() {
        return pattern.to_string();
    }
    pattern[..end].to_string()
}

// ============================================================================
// Tokenization (from glob.c tokenize family)
// ============================================================================

/// Token types for glob tokenization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobToken {
    Literal(char),
    Star,         // *
    Question,     // ?
    BracketOpen,  // [
    BracketClose, // ]
    ParenOpen,    // (
    ParenClose,   // )
    Pipe,         // |
    Hash,         // # (extended)
    Tilde,        // ~ (extended)
    Caret,        // ^ (extended)
    BraceOpen,    // {
    BraceClose,   // }
    Comma,        // , (in braces)
    Range,        // .. (in braces)
}

/// Tokenize a glob pattern (from glob.c tokenize lines 3100-3180)
pub fn tokenize(s: &str) -> Vec<GlobToken> {
    let mut tokens = Vec::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        let token = match c {
            '\\' => {
                // Escaped character
                if let Some(next) = chars.next() {
                    GlobToken::Literal(next)
                } else {
                    GlobToken::Literal('\\')
                }
            }
            '*' => GlobToken::Star,
            '?' => GlobToken::Question,
            '[' => GlobToken::BracketOpen,
            ']' => GlobToken::BracketClose,
            '(' => GlobToken::ParenOpen,
            ')' => GlobToken::ParenClose,
            '|' => GlobToken::Pipe,
            '#' => GlobToken::Hash,
            '~' => GlobToken::Tilde,
            '^' => GlobToken::Caret,
            '{' => GlobToken::BraceOpen,
            '}' => GlobToken::BraceClose,
            ',' => GlobToken::Comma,
            '.' if chars.peek() == Some(&'.') => {
                chars.next();
                GlobToken::Range
            }
            _ => GlobToken::Literal(c),
        };
        tokens.push(token);
    }

    tokens
}

/// Tokenize for shell (from glob.c shtokenize lines 3190-3250)
/// Handles shell-specific quoting
pub fn shtokenize(s: &str) -> Vec<GlobToken> {
    let mut tokens = Vec::new();
    let mut chars = s.chars().peekable();
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while let Some(c) = chars.next() {
        if in_single_quote {
            if c == '\'' {
                in_single_quote = false;
            } else {
                tokens.push(GlobToken::Literal(c));
            }
            continue;
        }

        if in_double_quote {
            if c == '"' {
                in_double_quote = false;
            } else if c == '\\' {
                if let Some(next) = chars.next() {
                    tokens.push(GlobToken::Literal(next));
                }
            } else {
                tokens.push(GlobToken::Literal(c));
            }
            continue;
        }

        match c {
            '\'' => in_single_quote = true,
            '"' => in_double_quote = true,
            '\\' => {
                if let Some(next) = chars.next() {
                    tokens.push(GlobToken::Literal(next));
                }
            }
            '*' => tokens.push(GlobToken::Star),
            '?' => tokens.push(GlobToken::Question),
            '[' => tokens.push(GlobToken::BracketOpen),
            ']' => tokens.push(GlobToken::BracketClose),
            _ => tokens.push(GlobToken::Literal(c)),
        }
    }

    tokens
}

/// Tokenize with zsh-specific flags (from glob.c zshtokenize lines 3260-3380)
pub fn zshtokenize(s: &str, extended_glob: bool, sh_glob: bool) -> Vec<GlobToken> {
    let mut tokens = Vec::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        let token = match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    GlobToken::Literal(next)
                } else {
                    GlobToken::Literal('\\')
                }
            }
            '*' => GlobToken::Star,
            '?' => GlobToken::Question,
            '[' => GlobToken::BracketOpen,
            ']' => GlobToken::BracketClose,
            '#' if extended_glob => GlobToken::Hash,
            '^' if extended_glob => GlobToken::Caret,
            '~' if extended_glob => GlobToken::Tilde,
            '(' if extended_glob => GlobToken::ParenOpen,
            ')' if extended_glob => GlobToken::ParenClose,
            '|' if extended_glob => GlobToken::Pipe,
            '{' if !sh_glob => GlobToken::BraceOpen,
            '}' if !sh_glob => GlobToken::BraceClose,
            ',' if !sh_glob => GlobToken::Comma,
            _ => GlobToken::Literal(c),
        };
        tokens.push(token);
    }

    tokens
}

/// Remove null arguments from token list (from glob.c remnulargs lines 3390-3420)
pub fn remnulargs(tokens: &mut Vec<GlobToken>) {
    tokens.retain(|t| {
        if let GlobToken::Literal(c) = t {
            *c != '\0'
        } else {
            true
        }
    });
}

// ============================================================================
// Mode specification parsing (from glob.c qgetmodespec)
// ============================================================================

/// Parsed mode specification
#[derive(Debug, Clone, Copy, Default)]
pub struct ModeSpec {
    pub who: u32,  // u, g, o, a masks
    pub op: char,  // +, -, =
    pub perm: u32, // r, w, x, s, t masks
}

/// Parse mode specification like chmod (from glob.c qgetmodespec lines 790-920)
/// Examples: u+x, go-w, a=r, 755
pub fn qgetmodespec(s: &str) -> Option<(ModeSpec, &str)> {
    let mut chars = s.chars().peekable();
    let mut spec = ModeSpec::default();

    // Check for octal mode
    if chars.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        let mut mode_str = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() && c < '8' {
                mode_str.push(c);
                chars.next();
            } else {
                break;
            }
        }
        if let Ok(mode) = u32::from_str_radix(&mode_str, 8) {
            spec.perm = mode;
            spec.op = '=';
            spec.who = 0o7777;
            let rest_pos = s.len() - chars.collect::<String>().len();
            return Some((spec, &s[rest_pos..]));
        }
        return None;
    }

    // Parse symbolic mode
    // Who: u, g, o, a
    let mut who = 0u32;
    while let Some(&c) = chars.peek() {
        match c {
            'u' => {
                who |= 0o4700;
                chars.next();
            }
            'g' => {
                who |= 0o2070;
                chars.next();
            }
            'o' => {
                who |= 0o1007;
                chars.next();
            }
            'a' => {
                who |= 0o7777;
                chars.next();
            }
            _ => break,
        }
    }
    if who == 0 {
        who = 0o7777; // Default to all
    }
    spec.who = who;

    // Op: +, -, =
    spec.op = match chars.next() {
        Some('+') => '+',
        Some('-') => '-',
        Some('=') => '=',
        _ => return None,
    };

    // Perm: r, w, x, X, s, t
    let mut perm = 0u32;
    while let Some(&c) = chars.peek() {
        match c {
            'r' => {
                perm |= 0o444;
                chars.next();
            }
            'w' => {
                perm |= 0o222;
                chars.next();
            }
            'x' => {
                perm |= 0o111;
                chars.next();
            }
            'X' => {
                perm |= 0o111;
                chars.next();
            } // Conditional execute
            's' => {
                perm |= 0o6000;
                chars.next();
            }
            't' => {
                perm |= 0o1000;
                chars.next();
            }
            _ => break,
        }
    }
    spec.perm = perm & who;

    let rest_pos = s.len() - chars.collect::<String>().len();
    Some((spec, &s[rest_pos..]))
}

/// Apply mode spec to existing mode
pub fn apply_modespec(mode: u32, spec: &ModeSpec) -> u32 {
    match spec.op {
        '+' => mode | spec.perm,
        '-' => mode & !spec.perm,
        '=' => (mode & !spec.who) | spec.perm,
        _ => mode,
    }
}

// ============================================================================
// Brace char range parsing (from glob.c bracechardots)
// ============================================================================

/// Parse character range in braces like {a..z} (from glob.c bracechardots lines 1780-1850)
pub fn bracechardots(s: &str) -> Option<(char, char, i32)> {
    let chars: Vec<char> = s.chars().collect();

    // Must be at least "a..b"
    if chars.len() < 4 {
        return None;
    }

    // Find ..
    let dotdot_pos = s.find("..")?;
    if dotdot_pos == 0 {
        return None;
    }

    let left = &s[..dotdot_pos];
    let right = &s[dotdot_pos + 2..];

    // Check for increment
    let (end_str, incr) = if let Some(pos) = right.find("..") {
        let end = &right[..pos];
        let inc: i32 = right[pos + 2..].parse().unwrap_or(1);
        (end, inc)
    } else {
        (right, 1)
    };

    // Single character range
    if left.chars().count() == 1 && end_str.chars().count() == 1 {
        let c1 = left.chars().next()?;
        let c2 = end_str.chars().next()?;
        return Some((c1, c2, incr));
    }

    None
}

// ============================================================================
// Redirect expansion (from glob.c xpandredir)
// ============================================================================

/// Redirect types
#[derive(Debug, Clone)]
pub struct Redirect {
    pub fd: i32,
    pub target: String,
    pub rtype: RedirectType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectType {
    Read,      // <
    Write,     // >
    Append,    // >>
    ReadWrite, // <>
    Clobber,   // >|
    Here,      // <<
    HereStr,   // <<<
    Dup,       // >&, <&
    Pipe,      // |
}

/// Expand redirections with glob patterns (from glob.c xpandredir lines 1690-1770)
pub fn xpandredir(redir: &Redirect, options: &GlobOptions) -> Vec<Redirect> {
    // Check if target has wildcards
    if !has_wildcards(&redir.target) {
        return vec![redir.clone()];
    }

    // Glob expand the target
    let mut state = GlobState::new(options.clone());
    let matches = state.glob(&redir.target);

    if matches.is_empty() {
        return vec![redir.clone()];
    }

    // For redirections, we usually only want one match
    if matches.len() > 1 {
        // Ambiguous redirect - return original
        return vec![redir.clone()];
    }

    vec![Redirect {
        fd: redir.fd,
        target: matches[0].clone(),
        rtype: redir.rtype,
    }]
}

// ============================================================================
// Exec string for sorting (from glob.c glob_exec_string)
// ============================================================================

/// Execute a command and capture output for sorting (from glob.c glob_exec_string lines 920-1020)
/// This is used for the `e` glob qualifier: *(e:'cmd':)
pub fn glob_exec_string(cmd: &str, filename: &str) -> Option<String> {
    use std::process::Command;

    // Replace $REPLY or {} with filename
    let cmd = cmd.replace("$REPLY", filename).replace("{}", filename);

    let output = Command::new("sh").arg("-c").arg(&cmd).output().ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Execute a qualifier expression (from glob.c qualsheval full impl)
pub fn qualsheval(filename: &str, expr: &str) -> bool {
    use std::process::Command;

    // Set REPLY to filename and evaluate expression
    let script = format!("REPLY='{}'; {}", filename.replace("'", "'\\''"), expr);

    Command::new("sh")
        .arg("-c")
        .arg(&script)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use tempfile::TempDir;

    fn setup_test_dir() -> TempDir {
        let dir = TempDir::new().unwrap();
        let base = dir.path();

        // Create test files
        File::create(base.join("file1.txt")).unwrap();
        File::create(base.join("file2.txt")).unwrap();
        File::create(base.join("file3.rs")).unwrap();
        File::create(base.join(".hidden")).unwrap();

        // Create subdirectory
        fs::create_dir(base.join("subdir")).unwrap();
        File::create(base.join("subdir/nested.txt")).unwrap();

        dir
    }

    #[test]
    fn test_has_wildcards() {
        assert!(has_wildcards("*.txt"));
        assert!(has_wildcards("file?.txt"));
        assert!(has_wildcards("file[12].txt"));
        assert!(!has_wildcards("file.txt"));
        assert!(!has_wildcards("path/to/file.txt"));
    }

    #[test]
    fn test_pattern_match() {
        assert!(pattern_match("*.txt", "file.txt", false, true));
        assert!(pattern_match("file?.txt", "file1.txt", false, true));
        assert!(!pattern_match("*.txt", "file.rs", false, true));
        assert!(pattern_match("file[12].txt", "file1.txt", false, true));
        assert!(!pattern_match("file[12].txt", "file3.txt", false, true));
    }

    #[test]
    fn test_brace_expansion() {
        let result = expand_braces("{a,b,c}", false);
        assert_eq!(result, vec!["a", "b", "c"]);

        let result = expand_braces("file{1,2,3}.txt", false);
        assert_eq!(result, vec!["file1.txt", "file2.txt", "file3.txt"]);

        let result = expand_braces("{1..5}", false);
        assert_eq!(result, vec!["1", "2", "3", "4", "5"]);

        let result = expand_braces("{a..e}", false);
        assert_eq!(result, vec!["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn test_glob_simple() {
        let dir = setup_test_dir();
        let pattern = format!("{}/*.txt", dir.path().display());

        let mut state = GlobState::new(GlobOptions::default());
        let results = state.glob(&pattern);

        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|s| s.ends_with("file1.txt")));
        assert!(results.iter().any(|s| s.ends_with("file2.txt")));
    }

    #[test]
    fn test_glob_hidden() {
        let dir = setup_test_dir();
        let pattern = format!("{}/*", dir.path().display());

        // With no_glob_dots = true (default)
        let mut state = GlobState::new(GlobOptions {
            no_glob_dots: true,
            ..Default::default()
        });
        let results = state.glob(&pattern);
        assert!(!results.iter().any(|s| s.contains(".hidden")));

        // With no_glob_dots = false
        let mut state = GlobState::new(GlobOptions {
            no_glob_dots: false,
            ..Default::default()
        });
        let results = state.glob(&pattern);
        assert!(results.iter().any(|s| s.contains(".hidden")));
    }

    #[test]
    fn test_file_type_char() {
        assert_eq!(file_type_char(libc::S_IFDIR as u32), '/');
        assert_eq!(file_type_char(libc::S_IFREG as u32), ' ');
        assert_eq!(file_type_char(libc::S_IFREG as u32 | 0o111), '*');
        assert_eq!(file_type_char(libc::S_IFLNK as u32), '@');
    }

    #[test]
    fn test_numeric_string_cmp() {
        assert_eq!(numeric_string_cmp("file1", "file2"), Ordering::Less);
        assert_eq!(numeric_string_cmp("file10", "file2"), Ordering::Greater);
        assert_eq!(numeric_string_cmp("file10", "file10"), Ordering::Equal);
    }
}
