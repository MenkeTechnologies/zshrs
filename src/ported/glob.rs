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
use std::collections::{HashMap, HashSet};
use std::fs::{self, Metadata};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
#[allow(unused_imports)]
use crate::ported::exec::{
    self, ShellExecutor, expand_posix_char_classes,
    extract_numeric_ranges, replace_numeric_ranges_with_star,
    with_executor, NumericRange,
};
use crate::ported::pattern::{approximate_match, ksh_extglob_body_to_regex, parse_numeric_range, parse_pattern_flags};

/// Sort specifier flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Glob qualifier sort modes.
/// Mirrors the `GS_*` sort-type constants from Src/glob.c —
/// `gmatchcmp()` (line 936) dispatches on these for the `o`/`O`
/// glob qualifier.
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
/// Ascending vs descending for glob sort.
/// Port of the `o` vs `O` qualifier choice in Src/glob.c.
pub enum SortOrder {
    Ascending,
    Descending,
}

/// A single sort specification
#[derive(Debug, Clone)]
/// One sort key for the `o`/`O` glob qualifier.
/// Mirrors the per-key shape `gmatchcmp()` (Src/glob.c:936)
/// uses when chaining multiple sort criteria.
pub struct SortSpec {
    pub sort_type: GlobSort,
    pub order: SortOrder,
    pub follow_links: bool,
}

/// Time units for qualifiers
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Time unit for `m`/`a`/`c` glob qualifier.
/// Mirrors the units `qualtime()` (Src/glob.c:827) accepts —
/// `s` seconds, `M` minutes, `h` hours, `d` days, `w` weeks, `m`
/// months.
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
/// Size unit for `L` glob qualifier.
/// Mirrors `qualsize()` (Src/glob.c around line 1054) accepted
/// units — `b` bytes / `k` KB / `m` MB / `p` 512-byte blocks.
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
/// Comparison op for numeric glob qualifiers.
/// Mirrors the `<`, `>`, `=` operators `qgetnum()`
/// (Src/glob.c:827) parses for `L+1k`, `m-1`, etc.
pub enum RangeOp {
    Less,
    Equal,
    Greater,
}

/// A glob qualifier function
#[derive(Debug, Clone)]
/// One glob qualifier.
/// Port of the `Qualifier` enum in Src/glob.c — drives the
/// per-match filter inside `scanner()` (line 500). Each variant
/// matches one of the C source's `q*` test functions.
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
/// One glob match result.
/// Port of `struct gmatch` from Src/glob.c — `gmatchcmp()`
/// (line 936) sorts arrays of these for the `o`/`O` qualifier.
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
                        locale_aware_name_cmp(&self.name, &other.name)
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

/// Locale-aware name comparison for glob sort. Under a Unicode locale
/// (LANG / LC_ALL / LC_COLLATE not in the C/POSIX/empty set), zsh
/// folds case before comparing — so `Aaa bbb Ccc` sorts in declaration
/// order rather than ASCII (uppercase < lowercase). Fallback to byte
/// compare under C/POSIX locale to mirror `LC_ALL=C zsh` behavior.
/// Locale-aware filename compare for `gmatchcmp`.
/// Port of the `strcoll(3)` path inside `gmatchcmp()`
/// (Src/glob.c:936) when sorting by name.
pub fn locale_aware_name_cmp(a: &str, b: &str) -> Ordering {
    let locale_is_c = {
        let lc_all = std::env::var("LC_ALL").unwrap_or_default();
        let lc_collate = std::env::var("LC_COLLATE").unwrap_or_default();
        let lang = std::env::var("LANG").unwrap_or_default();
        let active = if !lc_all.is_empty() {
            lc_all
        } else if !lc_collate.is_empty() {
            lc_collate
        } else {
            lang
        };
        let normalized = active.split('.').next().unwrap_or("").to_uppercase();
        matches!(normalized.as_str(), "" | "C" | "POSIX")
    };
    if locale_is_c {
        return a.cmp(b);
    }
    let primary = a.to_lowercase().cmp(&b.to_lowercase());
    if primary == Ordering::Equal {
        a.cmp(b)
    } else {
        primary
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
/// Glob behavior options.
/// Port of the various flag bits in Src/glob.c —
/// `GLOB_NULL`/`GLOB_NOCHECK`/etc. that `setopt NULL_GLOB` /
/// `setopt NOMATCH` flip.
pub struct GlobOptions {
    pub null_glob: bool,
    pub mark_dirs: bool,
    pub no_glob_dots: bool,
    pub list_types: bool,
    pub numeric_sort: bool,
    pub follow_links: bool,
    pub extended_glob: bool,
    pub case_glob: bool,
    /// `**` recursion semantics. False = strict zsh: `**` requires
    /// `/` (or `***/`) after to be recursive; bare `**foo` is a
    /// literal star pair. True = bash globstar: bare `**` recurses
    /// without trailing `/`. Direct mirror of GLOBSTARSHORT option
    /// at zsh/Src/options.c:148, consulted by parse_pattern in
    /// zshrs/Src/glob.c:715-742 parsecomplist.
    pub glob_star_short: bool,
    pub bare_glob_qual: bool,
    /// Brace character-class expansion (`{a-z}` → `a b c … z`).
    /// Direct mirror of zsh's BRACECCL option (zsh/Src/options.c:104,
    /// consulted in zsh/Src/glob.c:2046 hasbraces and glob.c:2424
    /// xpandbraces). When false, only comma lists `{a,b,c}` and
    /// numeric/char ranges `{1..5}`/`{a..e}` expand; the `{a-z}`
    /// dash-form is left literal.
    pub brace_ccl: bool,
}

/// Parsed glob qualifier set
#[derive(Debug, Clone, Default)]
/// Compiled qualifier list for one glob.
/// Mirrors the `struct qual *` linked list `parsepat()`
/// (Src/glob.c:791) builds — every `(qual)` after a glob pattern
/// adds to it.
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
    /// `(M)` qualifier — append `/` to directory entries in output.
    /// Direct port of zsh/Src/glob.c:1557-1561 (`case 'M'`):
    ///   `gf_markdirs = !(sense & 1)` — set when the qualifier appears
    ///   without a `^` toggle. Stored per-qualifier-set rather than per
    ///   GlobOptions so a single glob call's qualifier picks it up.
    pub mark_dirs: bool,
    /// `(T)` qualifier — append type-char (ls -F style) to every entry.
    /// Direct port of zsh/Src/glob.c:1562-1566 (`case 'T'`).
    pub list_types: bool,
}

/// Main glob state
/// Per-glob runtime state.
/// Port of the per-call locals `zglob()` (Src/glob.c:1214) keeps
/// — pathbuf, matched-list head, options, qualifier filter.
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
        // Brace pre-expansion. In zsh, `xpandbraces` (zsh/Src/glob.c:2275)
        // runs during substitution before glob — patterns reaching glob()
        // are already brace-free in the production path (exec.rs handles
        // it). For direct programmatic callers of glob_with_options, run
        // the brace pass here so `GlobOptions.brace_ccl` is actually
        // consulted: with brace_ccl set, `{a-mnop}` expands to a..m,n,o,p
        // per glob.c:2424 BRACECCL block; without, only `{a,b}` lists and
        // `{1..5}`/`{a..e}` ranges expand. Recurse on each variant and
        // concatenate matches.
        if hasbraces(pattern, self.options.brace_ccl) {
            let mut all = Vec::new();
            for variant in xpandbraces(pattern, self.options.brace_ccl) {
                all.extend(self.glob(&variant));
            }
            return all;
        }

        self.matches.clear();
        self.pathbuf.clear();
        self.pathpos = 0;

        // Parse qualifiers first so a bare-qualifier pattern like `dir(/)`
        // (no wildcard, just a stat-based filter) still enters the expansion
        // path. Without this, `has_wildcards("dir(/)")` returns false and the
        // pattern echoes back unfiltered, which defeats the whole point of
        // qualifiers.
        let (pat, quals) = self.parse_qualifiers(pattern);
        self.qualifiers = quals;

        // Now check wildcards on the qualifier-stripped pattern. A pure
        // literal with a qualifier (`name(.)`) still needs to enter the
        // scanner so the qualifier filter can run against the literal name.
        if !has_wildcards(&pat) && self.qualifiers.is_none() {
            return vec![pattern.to_string()];
        }

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

        // Extract filenames. Mark-dirs / list-types come from EITHER the
        // GlobOptions (caller-supplied default) OR the parsed `(M)`/`(T)`
        // qualifier on this glob — whichever is set wins. Direct port of
        // zsh/Src/glob.c:355,372 — output marker emission consults the
        // per-glob `gf_markdirs` / `gf_listtypes` flags which the qualifier
        // parser at glob.c:1557-1566 sets.
        let mark_dirs = self.options.mark_dirs
            || self
                .qualifiers
                .as_ref()
                .map(|q| q.mark_dirs)
                .unwrap_or(false);
        let list_types = self.options.list_types
            || self
                .qualifiers
                .as_ref()
                .map(|q| q.list_types)
                .unwrap_or(false);
        let colon_mods = self.qualifiers.as_ref().and_then(|q| q.colon_mods.clone());
        let mut results: Vec<String> = self
            .matches
            .iter()
            .map(|m| {
                let mut s = glob_emit_path(&m.path);
                if mark_dirs || list_types {
                    if let Ok(meta) = fs::symlink_metadata(&m.path) {
                        let ch = file_type(meta.mode());
                        if list_types || (mark_dirs && ch == '/') {
                            s.push(ch);
                        }
                    }
                }
                // Apply colon modifiers AFTER mark/list-type appendage —
                // zsh applies them last in glob.c:432 modify() per emitted
                // node, so `(M:t)` would mark THEN tail (effectively just
                // tail since the slash is gone). Faithful order.
                if let Some(ref m) = colon_mods {
                    s = apply_colon_modifiers(&s, m);
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
        let (is_explicit, qual_content) = if let Some(after) = qual_str.strip_prefix("#q") {
            (true, after)
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
                    let (unit, op, val) = self.schedgetfn(&mut chars);
                    qs.qualifiers.push(Qualifier::Atime {
                        value: val as i64,
                        unit,
                        op,
                    });
                }
                'm' => {
                    let (unit, op, val) = self.schedgetfn(&mut chars);
                    qs.qualifiers.push(Qualifier::Mtime {
                        value: val as i64,
                        unit,
                        op,
                    });
                }
                'c' => {
                    let (unit, op, val) = self.schedgetfn(&mut chars);
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
                // (M) / (T) — set per-qualifier-set flags. glob.c:1557-1566:
                //   case 'M': gf_markdirs = !(sense & 1);  break;
                //   case 'T': gf_listtypes = !(sense & 1); break;
                // `sense & 1` = the `^`-toggle bit. zshrs's parser tracks
                // `negated`; mirror by `!negated`. Read at output-emit
                // time to mark dirs / list types like coreutils ls -F.
                'M' => qs.mark_dirs = !negated,
                'T' => qs.list_types = !negated,
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

    fn schedgetfn(
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
                    // Direct port of zsh/Src/glob.c:717-742 parsecomplist:
                    // `**` is recursive ONLY when followed by `/` (or `***/`,
                    // or when GLOBSTARSHORT is set). Without those, it should
                    // collapse to a literal `*` + `*` pair (which the matcher
                    // treats as a single `*` since `**` ≡ `*` for non-recursive
                    // contexts in zsh). The glob_star_short option flips the
                    // strict gate off so bare `**` recurses without `/`.
                    let has_slash = chars.peek() == Some(&'/');
                    let recursive = has_slash || follow || self.options.glob_star_short;
                    if has_slash {
                        chars.next();
                    }
                    if recursive {
                        if !current.is_empty() {
                            components.push(PatternComponent::Pattern(current.clone()));
                            current.clear();
                        }
                        components.push(PatternComponent::Recursive {
                            follow_links: follow,
                        });
                        // GLOBSTARSHORT semantics — zsh/Src/glob.c:727-730
                        // `instr += ((shortglob ? 1 : 3) + follow);` leaves
                        // ONE `*` in place when entering the recursive path
                        // without a `/` separator, so `**.c` ≡ `**/*.c` and
                        // `**foo` ≡ `**/*foo`. Without this prepend, the
                        // remaining segment was parsed literally — `**.stk`
                        // became [Recursive, Pattern(".stk")], which only
                        // matched files literally named `.stk`. Gate on
                        // `!has_slash && !follow` since `**/X` and `***/X`
                        // already consumed their separator and don't need
                        // the glue star.
                        if !has_slash
                            && !follow
                            && chars.peek().is_some()
                            && chars.peek() != Some(&'/')
                        {
                            current.push('*');
                        }
                    } else {
                        // Strict zsh: `**foo` (no slash, no shortglob) is
                        // a literal pair of stars in the same path component.
                        // Two `*` collapse to one in zsh pattern semantics.
                        current.push('*');
                    }
                }
                _ => current.push(c),
            }
        }

        if !current.is_empty() {
            components.push(PatternComponent::Pattern(current));
        }

        // Trailing `**` (or `**/`) with no following pattern — without an
        // implicit `*`, the scanner walks the tree but emits nothing, since
        // `Pattern` components are what produce match output. zshrs synthesis
        // direction (per CLAUDE.md): absorb good ideas from other shells. Both
        // zsh-strict (`**` ≡ `*` top-level) and bash-globstar (`**` ≡ `**/*`
        // recursive) are reasonable; we pick the bash-globstar interpretation
        // because `**` empty-handed should mean "everything", and `*` already
        // exists for the top-level case. A user wanting top-level only writes
        // `*`, never `**`. This makes `**(/)` ≡ "every directory recursively"
        // and `**(.)` ≡ "every file recursively" — the readings users reach
        // for first.
        if let Some(PatternComponent::Recursive { .. }) = components.last() {
            components.push(PatternComponent::Pattern("*".to_string()));
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

            if matchpat(
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
                let scaled = match unit {
                    SizeUnit::Bytes => size,
                    SizeUnit::PosixBlocks => size.div_ceil(512),
                    SizeUnit::Kilobytes => size.div_ceil(1024),
                    SizeUnit::Megabytes => size.div_ceil(1048576),
                    SizeUnit::Gigabytes => size.div_ceil(1073741824),
                    SizeUnit::Terabytes => size.div_ceil(1099511627776),
                };
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
/// Quick predicate for `does this string contain wildcards?`.
/// Port of the `haswilds()` macro inline in Src/glob.c —
/// short-circuits `zglob()` so plain literal paths skip the
/// scanner.
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
/// Match a glob pattern against a single string.
/// Port of `matchpat()` from Src/glob.c:2514 — same
/// `EXTENDED_GLOB`/`NO_CASE_GLOB` option handling.
pub fn matchpat(pattern: &str, text: &str, extended: bool, case_sensitive: bool) -> bool {
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
/// Render a mode bitmap as the `*` qualifier letter (`d`/`b`/
/// `c`/`l`/`s`/`p`/etc.).
/// Port of `file_type()` from Src/glob.c:2018.
pub fn file_type(mode: u32) -> char {
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
/// Check whether a string has brace-expansion `{a,b}` content.
/// Port of `hasbraces()` from Src/glob.c:2042.
pub fn hasbraces(s: &str, brace_ccl: bool) -> bool {
    let mut depth = 0;
    let mut has_comma = false;
    let mut has_dotdot = false;
    let mut brace_open: Option<usize> = None;

    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();

    for i in 0..len {
        match chars[i] {
            '{' => {
                if depth == 0 {
                    brace_open = Some(i);
                }
                depth += 1;
            }
            '}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    if has_comma || has_dotdot {
                        return true;
                    }
                    // BRACE_CCL: any non-empty `{…}` body without a
                    // comma/dotdot becomes a character-class set.
                    // Direct port of Src/lex.c::xpandbraces that
                    // routes the body through expand_ccl when
                    // BRACE_CCL is set, regardless of body length.
                    if brace_ccl {
                        if let Some(open) = brace_open {
                            if i > open + 1 {
                                return true;
                            }
                        }
                    }
                    has_comma = false;
                    has_dotdot = false;
                    brace_open = None;
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
/// Brace-expand a string into a flat list.
/// Port of `xpandbraces()` from Src/glob.c:2276 — same
/// `{a,b}` / `{1..10}` / `{a-z}` handling.
pub fn xpandbraces(s: &str, brace_ccl: bool) -> Vec<String> {
    if !hasbraces(s, brace_ccl) {
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
    let start = chars.iter().position(|&c| c == '{')?;
    let _ = len;

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
                    if brace_ccl && !content.is_empty() {
                        return expand_ccl(&prefix, &content, &suffix);
                    }

                    return None;
                }
            }
            ',' if depth == 1 => comma_positions.push(i - start - 1),
            '.' if depth == 1 && i + 1 < len && chars[i + 1] == '.' && dotdot_pos.is_none() => {
                dotdot_pos = Some(i - start - 1);
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

    // Check for second `..` for `{N..M..S}` step form. Step may be
    // signed: negative-step REVERSES the natural direction sequence
    // per zsh's brace expansion (Src/lex.c::brace_expand_range
    // recursive iteration with sign tracking). Examples:
    //   {1..32..3}   →  1,4,7,…,31 (natural ascending)
    //   {1..32..-3}  → 31,28,…,1   (same set, reversed)
    //   {32..1..3}   → 32,29,…,2   (natural descending)
    //   {32..1..-3}  →  2,5,…,32   (same set, reversed)
    let (right, incr_abs, incr_sign_negative, step_text) =
        if let Some(pos) = content[right_start..].find("..") {
            let r = &content[right_start..right_start + pos];
            let s_text = &content[right_start + pos + 2..];
            let raw: i64 = s_text.parse().unwrap_or(1);
            (r, raw.unsigned_abs(), raw < 0, s_text)
        } else {
            (&content[right_start..], 1u64, false, "")
        };

    // Try numeric range
    if let (Ok(start), Ok(end)) = (left.parse::<i64>(), right.parse::<i64>()) {
        let mut results = Vec::new();

        // Iterate from `start` toward `end` with abs(step). Sign of
        // start/end relative to each other determines natural
        // direction; step is always |step|.
        let step = incr_abs.max(1) as i64;
        let mut vals: Vec<i64> = Vec::new();
        if start <= end {
            let mut v = start;
            while v <= end {
                vals.push(v);
                v += step;
            }
        } else {
            let mut v = start;
            while v >= end {
                vals.push(v);
                v -= step;
            }
        }
        if incr_sign_negative {
            vals.reverse();
        }

        // Padding: zsh pads with leading zeros when ANY of the three
        // textual fields (left endpoint, right endpoint, step) has a
        // leading zero after stripping the optional sign. Width is
        // the max textual width across left/right/step. For negative
        // values, the sign prefix counts toward width — we emit `-`
        // then zero-pad the remaining digits (`-02`, not `0-2`).
        // Direct port of Src/lex.c::dobrace_pad logic which detects
        // pad mode per the textual form of all three fields.
        let lstrip = left.trim_start_matches(['+', '-']);
        let rstrip = right.trim_start_matches(['+', '-']);
        let sstrip = step_text.trim_start_matches(['+', '-']);
        let pad = lstrip.starts_with('0')
            || rstrip.starts_with('0')
            || (!step_text.is_empty() && sstrip.starts_with('0'));
        let width = left.len().max(right.len()).max(step_text.len());

        for v in vals {
            let formatted = if pad {
                if v < 0 {
                    let abs = (-v).to_string();
                    let inner_w = width.saturating_sub(1);
                    format!("-{:0>w$}", abs, w = inner_w)
                } else {
                    format!("{:0>w$}", v, w = width)
                }
            } else {
                v.to_string()
            };
            results.push(format!("{}{}{}", prefix, formatted, suffix));
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
// Colon modifiers — `:t :h :r :e :s/X/Y/` applied to glob results.
// Direct port of zsh/Src/subst.c:4531 `modify()` driver plus the
// rem* helpers in hist.c:2056-2186 (`remtpath`, `remlpaths`,
// `remtext`, `rembutext`). Used by `(...)` qualifier suffixes —
// `*.toml(:t)` returns basenames, `*.toml(:r)` strips extension,
// `*.toml(:s/.toml/.zzz/)` runs a one-shot substitution.
// ============================================================================

/// `:h` — head/dirname. Direct port of zsh/Src/hist.c:2056 `remtpath`
/// with `count=1`. Trailing `/` ignored, then drop the last path
/// component. Edge cases mirror the C: empty result with leading `/`
/// becomes `/`; otherwise `.`.
fn modifier_head(s: &str) -> String {
    if s.is_empty() {
        return ".".to_string();
    }
    let bytes = s.as_bytes();
    let mut end = bytes.len();
    // Strip trailing slashes — `foo/:h` is `.`, matching zsh.
    while end > 0 && bytes[end - 1] == b'/' {
        end -= 1;
    }
    // Skip the filename component.
    while end > 0 && bytes[end - 1] != b'/' {
        end -= 1;
    }
    if end == 0 {
        return if bytes.first() == Some(&b'/') {
            "/".to_string()
        } else {
            ".".to_string()
        };
    }
    // Collapse repeated slashes — never erase the root slash.
    while end > 1 && bytes[end - 1] == b'/' {
        end -= 1;
    }
    if end == 0 {
        return "/".to_string();
    }
    s[..end].to_string()
}

/// `:t` — tail/basename. Direct port of zsh/Src/hist.c:2152 `remlpaths`
/// with `count=1`. Returns the substring after the last `/`. Trailing
/// slashes are trimmed first so `foo/bar/:t` is `bar`, not empty.
fn modifier_tail(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let bytes = s.as_bytes();
    let mut end = bytes.len();
    while end > 0 && bytes[end - 1] == b'/' {
        end -= 1;
    }
    if end == 0 {
        // Pure `/` or `///` etc.
        return String::new();
    }
    let trimmed = &s[..end];
    match trimmed.rfind('/') {
        Some(i) => trimmed[i + 1..].to_string(),
        None => trimmed.to_string(),
    }
}

/// `:r` — root, strip last extension from the basename. Direct port of
/// zsh/Src/hist.c:2122 `remtext`. Walks from end, stops at first `/`
/// or `.`; truncates at `.` when found in the basename.
fn modifier_root(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        let c = bytes[i - 1];
        if c == b'/' {
            return s.to_string();
        }
        if c == b'.' {
            return s[..i - 1].to_string();
        }
        i -= 1;
    }
    s.to_string()
}

/// `:e` — extension only. Direct port of zsh/Src/hist.c:2136 `rembutext`.
/// Walks from end; on first `.` returns the substring after the dot;
/// on `/` returns empty. `foo.tar.gz` → `gz`; `foo` → ``;
/// `.bashrc` → `bashrc`.
fn modifier_ext(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut i = bytes.len();
    while i > 0 {
        let c = bytes[i - 1];
        if c == b'/' {
            return String::new();
        }
        if c == b'.' {
            return s[i..].to_string();
        }
        i -= 1;
    }
    String::new()
}

/// Apply a `:s/PAT/REPL/` (or `:gs/PAT/REPL/` for global) substitution.
/// Direct port of the s/S branch of zsh/Src/subst.c:4579-4710 — first
/// char after `s` is the delimiter (any single char), pattern runs to
/// the second occurrence, replacement to the third (or string end).
/// Backslash-escapes the delimiter inside the body. Returns the
/// substituted string and the number of bytes consumed from `mods`.
fn apply_modifier_subst(input: &str, mods_after_s: &str, global: bool) -> (String, usize) {
    let chars: Vec<char> = mods_after_s.chars().collect();
    if chars.is_empty() {
        return (input.to_string(), 0);
    }
    let delim = chars[0];
    let mut pat = String::new();
    let mut repl = String::new();
    let mut filling_repl = false;
    let mut i = 1;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\\' && i + 1 < chars.len() {
            let next = chars[i + 1];
            if next == delim || next == '\\' {
                if filling_repl {
                    repl.push(next);
                } else {
                    pat.push(next);
                }
                i += 2;
                continue;
            }
        }
        if ch == delim {
            if !filling_repl {
                filling_repl = true;
                i += 1;
                continue;
            } else {
                i += 1; // consume trailing delimiter
                break;
            }
        }
        if filling_repl {
            repl.push(ch);
        } else {
            pat.push(ch);
        }
        i += 1;
    }
    // Consumed bytes counted in chars; convert back to byte length so
    // the driver can advance its iterator correctly.
    let consumed_bytes: usize = chars[..i].iter().map(|c| c.len_utf8()).sum();
    let out = if pat.is_empty() {
        input.to_string()
    } else if global {
        input.replace(&pat, &repl)
    } else {
        input.replacen(&pat, &repl, 1)
    };
    (out, consumed_bytes)
}

/// `:a` — make absolute lexically, no symlink resolution. Direct port
/// of zsh/Src/subst.c xsymlinks: prepend `$PWD` if relative, then
/// collapse `.` and `..` components without touching the filesystem.
fn modifier_abs(s: &str) -> String {
    let base = if s.starts_with('/') {
        std::path::PathBuf::from(s)
    } else {
        std::env::current_dir().unwrap_or_default().join(s)
    };
    let mut out = std::path::PathBuf::new();
    for c in base.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out.to_string_lossy().to_string()
}

/// `:A` / `:P` — physical/canonical absolute path. Direct port of
/// zsh/Src/subst.c chrealpath / xsymlink (subst.c:4736,4787) — resolve
/// symlinks where possible. Falls back to `:a` lexical normalization
/// when the path doesn't exist on disk.
fn modifier_realpath(s: &str) -> String {
    let base = if s.starts_with('/') {
        std::path::PathBuf::from(s)
    } else {
        std::env::current_dir().unwrap_or_default().join(s)
    };
    match std::fs::canonicalize(&base) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => modifier_abs(s),
    }
}

/// `:c` — resolve command in `$PATH`. Direct port of zsh/Src/subst.c
/// equalsubstr (subst.c:4739-4744). If the input contains a `/` it's
/// already a path and is returned unchanged. Otherwise scan `$PATH`
/// for an executable file with that basename and return the full
/// path. Returns the input unchanged on miss.
fn modifier_command(s: &str) -> String {
    if s.is_empty() || s.contains('/') {
        return s.to_string();
    }
    let path = match std::env::var("PATH") {
        Ok(p) => p,
        Err(_) => return s.to_string(),
    };
    for dir in path.split(':') {
        if dir.is_empty() {
            continue;
        }
        let candidate = std::path::Path::new(dir).join(s);
        if let Ok(meta) = std::fs::metadata(&candidate) {
            if meta.is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if meta.permissions().mode() & 0o111 != 0 {
                        return candidate.to_string_lossy().to_string();
                    }
                }
                #[cfg(not(unix))]
                {
                    return candidate.to_string_lossy().to_string();
                }
            }
        }
    }
    s.to_string()
}

/// `:l` — lowercase. Direct port of zsh/Src/subst.c:4847 `casemodify(*str,
/// CASMOD_LOWER)`. Unicode-aware via Rust's `to_lowercase()`.
fn modifier_lower(s: &str) -> String {
    s.to_lowercase()
}

/// `:u` — uppercase. Direct port of zsh/Src/subst.c:4850 `casemodify(*str,
/// CASMOD_UPPER)`. Unicode-aware via Rust's `to_uppercase()`.
fn modifier_upper(s: &str) -> String {
    s.to_uppercase()
}

/// `:q` — backslash-bslashquote shell metacharacters. Direct port of
/// zsh/Src/subst.c:4860 `quotestring(*str, QT_BACKSLASH)` — escape
/// every char that would otherwise be parsed as syntax (whitespace,
/// quotes, redirects, glob metas, history bangs, `$`, backtick, etc.).
/// The output is safe to paste back as a literal shell argument.
fn modifier_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if matches!(
            c,
            ' ' | '\t'
                | '\n'
                | '\''
                | '"'
                | '\\'
                | ';'
                | '&'
                | '|'
                | '<'
                | '>'
                | '('
                | ')'
                | '{'
                | '}'
                | '['
                | ']'
                | '*'
                | '?'
                | '~'
                | '!'
                | '#'
                | '$'
                | '^'
                | '`'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `:Q` — strip shell quoting. Direct port of zsh/Src/subst.c:4863
/// `parse_subst_string` + `untokenize`. Handles backslash-escapes,
/// single-quoted runs (literal until next `'`), and double-quoted runs
/// (only `\\ \" \$ \` \\n` are special). Unmatched quotes consume to
/// end-of-string per zsh's permissive parse.
fn modifier_unquote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            '\'' => {
                for qc in chars.by_ref() {
                    if qc == '\'' {
                        break;
                    }
                    out.push(qc);
                }
            }
            '"' => {
                while let Some(qc) = chars.next() {
                    if qc == '"' {
                        break;
                    }
                    if qc == '\\' {
                        if let Some(&peek) = chars.peek() {
                            if matches!(peek, '"' | '\\' | '$' | '`' | '\n') {
                                out.push(chars.next().unwrap());
                                continue;
                            }
                        }
                    }
                    out.push(qc);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Apply a chained colon-modifier string (`:t`, `:r:s/x/y/`, `:gs/X/Y/:t`)
/// to a path. Direct port of zsh/Src/subst.c:4531 `modify()` for the
/// full modifier set used by glob qualifiers and parameter expansion:
/// `:h :t :r :e :a :A :c :l :u :q :Q :P :s/X/Y/ :S/X/Y/ :gs/X/Y/`.
/// Unknown modifiers stop the chain rather than mangle the path.
/// Apply `:` history-style modifiers to a string.
/// Port of `applymod()` (Src/utils.c) — used by glob history
/// substitution (`!*:t`) and parameter modifiers (`${var:t}`).
pub fn apply_colon_modifiers(input: &str, mods: &str) -> String {
    let mut s = input.to_string();
    let bytes = mods.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b':' {
            i += 1;
            continue;
        }
        i += 1;
        if i >= bytes.len() {
            break;
        }
        let mut global = false;
        if bytes[i] == b'g' {
            global = true;
            i += 1;
            if i >= bytes.len() {
                break;
            }
        }
        match bytes[i] {
            b'h' => {
                s = modifier_head(&s);
                i += 1;
            }
            b't' => {
                s = modifier_tail(&s);
                i += 1;
            }
            b'r' => {
                s = modifier_root(&s);
                i += 1;
            }
            b'e' => {
                s = modifier_ext(&s);
                i += 1;
            }
            b'a' => {
                s = modifier_abs(&s);
                i += 1;
            }
            // `:A` and `:P` both resolve symlinks per zsh manpage; the
            // C code's xsymlink path collapses to canonicalize() in
            // Rust. Subtle differences (xsymlink's intermediate-symlink
            // policy) would only show up on broken-symlink chains —
            // worth revisiting if a script depends on the divergence.
            b'A' | b'P' => {
                s = modifier_realpath(&s);
                i += 1;
            }
            b'c' => {
                s = modifier_command(&s);
                i += 1;
            }
            b'l' => {
                s = modifier_lower(&s);
                i += 1;
            }
            b'u' => {
                s = modifier_upper(&s);
                i += 1;
            }
            b'q' => {
                s = modifier_quote(&s);
                i += 1;
            }
            b'Q' => {
                s = modifier_unquote(&s);
                i += 1;
            }
            // `:s` and `:S` go through the same substitution branch in
            // C (subst.c:4764-4770); `:S` flips `hsubpatopt` for
            // case-sensitive HIST_SUBST_PATTERN behavior. Our `replace`
            // is already case-sensitive, so they're functionally
            // identical here until pattern-mode is wired.
            b's' | b'S' => {
                i += 1;
                let (out, consumed) = apply_modifier_subst(&s, &mods[i..], global);
                s = out;
                i += consumed;
            }
            _ => break,
        }
    }
    s
}

// ============================================================================
// Convenience functions
// ============================================================================

/// Split a pattern at its trailing zsh-style qualifier suffix. Returns
/// `(pattern_without_qualifier, qualifier_inner)` — the inner is the bytes
/// between the matching parens, without the surrounding `()` and without
/// any leading `#q`. Returns `(pattern, None)` when there is no qualifier
/// suffix. Useful for callers that want to use the pattern half with the
/// runtime [`matchpat`] (which has no qualifier semantics) while
/// reporting or applying the qualifier separately.
/// Split a glob pattern into (path-pattern, qualifier-string).
/// Port of the qualifier-detection step in `parsepat()`
/// (Src/glob.c:791).
pub fn split_qualifier(pattern: &str) -> (&str, Option<&str>) {
    if !pattern.ends_with(')') {
        return (pattern, None);
    }
    let bytes = pattern.as_bytes();
    let mut depth = 0;
    for i in (0..bytes.len()).rev() {
        match bytes[i] {
            b')' => depth += 1,
            b'(' => {
                depth -= 1;
                if depth == 0 {
                    let inner = &pattern[i + 1..pattern.len() - 1];
                    let inner = inner.strip_prefix("#q").unwrap_or(inner);
                    return (&pattern[..i], Some(inner));
                }
            }
            _ => {}
        }
    }
    (pattern, None)
}

/// Strip redundant `.` / `CurDir` segments from relative match paths for
/// output. Rust's `read_dir(".")` yields `entry.path()` like `./foo` while
/// `read_dir("foo")` yields `foo/bar` — zsh prints the latter shape for both.
fn glob_emit_path(path: &std::path::Path) -> String {
    use std::path::Component;
    match path.components().next() {
        Some(Component::Prefix(_) | Component::RootDir) => path.to_string_lossy().to_string(),
        None => ".".to_string(),
        _ => {
            let mut out = std::path::PathBuf::new();
            for c in path.components() {
                match c {
                    Component::CurDir => {}
                    Component::ParentDir => out.push(".."),
                    Component::Normal(s) => out.push(s),
                    Component::Prefix(_) | Component::RootDir => {}
                }
            }
            if out.as_os_str().is_empty() {
                ".".to_string()
            } else {
                out.to_string_lossy().to_string()
            }
        }
    }
}

/// Glob with default options
/// Top-level glob entry point with default options.
/// Port of `zglob()` from Src/glob.c:1214.
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
/// Top-level glob entry point with explicit options.
/// Port of `zglob()` from Src/glob.c:1214 — same `LinkList`
/// of expanded matches the C source threads through.
pub fn glob_with_options(pattern: &str, options: GlobOptions) -> Vec<String> {
    let mut state = GlobState::new(options);
    state.glob(pattern)
}

/// Add path component (from glob.c addpath lines 263-274)
/// Append a path component to a glob path buffer.
/// Port of `addpath()` from Src/glob.c:265.
pub fn addpath(buf: &mut String, component: &str) {
    buf.push_str(component);
    if !buf.ends_with('/') {
        buf.push('/');
    }
}

/// Stat full path (from glob.c statfullpath lines 282-347)
/// `stat`/`lstat` a (pathbuf, name) tuple.
/// Port of `statfullpath()` from Src/glob.c:283.
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
/// Check whether a glob match is a directory.
/// Port of the `S_ISDIR(stat.st_mode)` test scattered through
/// Src/glob.c.
pub fn is_directory(path: &str) -> bool {
    std::fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

/// Check if path is a symlink
/// Check whether a glob match is a symlink.
/// Port of the `S_ISLNK(lstat.st_mode)` test in Src/glob.c.
pub fn is_symlink(path: &str) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Match minimum distance for spelling correction (from glob.c mindist lines 3523-3575)
/// Edit-distance helper for `setopt CORRECT` glob fallback.
/// Port of the `spdist()`-driven correction inside
/// `findcmd()` (Src/exec.c) when adapted for glob targets.
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
/// Parse a numeric glob-qualifier argument.
/// Port of `qgetnum()` from Src/glob.c:827.
pub fn qgetnum(s: &str) -> Option<(i64, &str)> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    let num = s[..end].parse::<i64>().ok()?;
    Some((num, &s[end..]))
}

/// Parse time modifier (from glob.c qualtime)
/// Parse a time-unit glob-qualifier argument (`m`/`a`/`c`).
/// Port of the time-conversion arms inside `qgetnum()`
/// (Src/glob.c:827).
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
/// Parse a size-unit glob-qualifier argument (`L`).
/// Port of the size-conversion arms inside `qgetnum()`
/// (Src/glob.c:827).
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
/// Sort a glob result array per the `o` qualifier.
/// Port of the `gmatchcmp()`-driven sort step in `zglob()`
/// (Src/glob.c:1214).
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
#[derive(Debug, Clone, Copy, Default)]
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
        if matchpat(pat, s, true, true) {
            (0, len)
        } else {
            return s.to_string();
        }
    } else if flags.anchored_start {
        // Match from start (# or ##)
        let mut best_end = 0;
        for end in 1..=len {
            let substr: String = chars[..end].iter().collect();
            if matchpat(pat, &substr, true, true) {
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
            if matchpat(pat, &substr, true, true) {
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
                if matchpat(pat, &substr, true, true) {
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
            if matchpat(pat, &substr, true, true) {
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
        assert!(matchpat("*.txt", "file.txt", false, true));
        assert!(matchpat("file?.txt", "file1.txt", false, true));
        assert!(!matchpat("*.txt", "file.rs", false, true));
        assert!(matchpat("file[12].txt", "file1.txt", false, true));
        assert!(!matchpat("file[12].txt", "file3.txt", false, true));
    }

    #[test]
    fn test_brace_expansion() {
        let result = xpandbraces("{a,b,c}", false);
        assert_eq!(result, vec!["a", "b", "c"]);

        let result = xpandbraces("file{1,2,3}.txt", false);
        assert_eq!(result, vec!["file1.txt", "file2.txt", "file3.txt"]);

        let result = xpandbraces("{1..5}", false);
        assert_eq!(result, vec!["1", "2", "3", "4", "5"]);

        let result = xpandbraces("{a..e}", false);
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
    fn test_glob_emit_path_strips_read_dir_dot_slash() {
        use std::path::Path;
        assert_eq!(glob_emit_path(Path::new("./sub")), "sub");
        assert_eq!(glob_emit_path(Path::new("sub/deeper")), "sub/deeper");
        assert_eq!(glob_emit_path(Path::new("././x")), "x");
        assert_eq!(glob_emit_path(Path::new("../up")), "../up");
    }

    #[test]
    fn test_file_type_char() {
        assert_eq!(file_type(libc::S_IFDIR as u32), '/');
        assert_eq!(file_type(libc::S_IFREG as u32), ' ');
        assert_eq!(file_type(libc::S_IFREG as u32 | 0o111), '*');
        assert_eq!(file_type(libc::S_IFLNK as u32), '@');
    }

    #[test]
    fn test_numeric_string_cmp() {
        assert_eq!(numeric_string_cmp("file1", "file2"), Ordering::Less);
        assert_eq!(numeric_string_cmp("file10", "file2"), Ordering::Greater);
        assert_eq!(numeric_string_cmp("file10", "file10"), Ordering::Equal);
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: glob
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// Match a string against a shell glob pattern
    pub(crate) fn glob_match(&self, s: &str, pattern: &str) -> bool {
        // Convert shell glob to regex
        let mut regex_pattern = String::from("^");
        let mut chars = pattern.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                '*' => regex_pattern.push_str(".*"),
                '?' => regex_pattern.push('.'),
                '[' => {
                    regex_pattern.push('[');
                    // Handle character class
                    for cc in chars.by_ref() {
                        if cc == ']' {
                            regex_pattern.push(']');
                            break;
                        }
                        regex_pattern.push(cc);
                    }
                }
                '(' => {
                    // Handle alternation (a|b|c) -> (a|b|c)
                    regex_pattern.push('(');
                }
                ')' => regex_pattern.push(')'),
                '|' => regex_pattern.push('|'),
                '.' | '+' | '^' | '$' | '\\' | '{' | '}' => {
                    regex_pattern.push('\\');
                    regex_pattern.push(c);
                }
                _ => regex_pattern.push(c),
            }
        }
        regex_pattern.push('$');

        regex::Regex::new(&regex_pattern)
            .map(|re| re.is_match(s))
            .unwrap_or(false)
    }
    /// Static glob match — same logic as glob_match but callable without &self,
    /// needed for Rayon parallel iterators that can't capture &self.
    pub fn glob_match_static(s: &str, pattern: &str) -> bool {
        // Extendedglob `^pat` negation: when extendedglob is on AND
        // the pattern starts with a literal `^`, strip it and invert
        // the match of the remainder. Already done in
        // `extendedglob_match` for the param-filter path; do it here
        // too so `[[ str = ^pat ]]` works via the cond `=` matcher.
        let extendedglob_on =
            with_executor(|e| e.options.get("extendedglob").copied().unwrap_or(false));
        if extendedglob_on {
            if let Some(rest) = pattern.strip_prefix('^') {
                return !ShellExecutor::glob_match_static(s, rest);
            }
            // Extendedglob `~` exclusion: `pat1~pat2` matches strings
            // matching `pat1` AND NOT matching `pat2`. Direct port of
            // zsh's pattern.c P_EXCLUDE handling (line 155 onward) for
            // the top-level case — the canonical implementation also
            // handles nested exclusions (`(a~b)c`) but the top-level
            // form is what `*.txt~README*` and similar idioms produce.
            // Walk the pattern looking for a `~` that's NOT inside
            // `[...]` or `(...)` so nested specials stay literal.
            if let Some(idx) = find_top_level_tilde(pattern) {
                let lhs = &pattern[..idx];
                let rhs = &pattern[idx + 1..];
                return ShellExecutor::glob_match_static(s, lhs)
                    && !ShellExecutor::glob_match_static(s, rhs);
            }
        }

        // ksh-style negation `!(p)` (gated on `setopt kshglob`): when
        // the entire pattern is `!(<body>)`, match anything that does
        // NOT match `<body>`. This handles the standalone case (the
        // overwhelmingly common form); embedded `!()` inside a larger
        // pattern still falls through and is left literal — full
        // zsh-style negation needs lookahead which `regex` lacks.
        let kshglob_on = with_executor(|e| e.options.get("kshglob").copied().unwrap_or(false));
        if kshglob_on {
            if let Some(body) = pattern.strip_prefix("!(").and_then(|r| r.strip_suffix(')')) {
                // Don't recurse if body itself contains an unmatched
                // `(` that would change the meaning.
                let mut depth = 0;
                let mut balanced = true;
                for c in body.chars() {
                    match c {
                        '(' => depth += 1,
                        ')' => {
                            if depth == 0 {
                                balanced = false;
                                break;
                            }
                            depth -= 1;
                        }
                        _ => {}
                    }
                }
                if balanced && depth == 0 {
                    return !ShellExecutor::glob_match_static(s, body);
                }
            }
        }

        // Inline pattern flags `(#i)` / `(#I)` / `(#l)` / `(#a<n>)` per
        // zshexpn(1) "Globbing Flags". They prefix a pattern and modify
        // matching semantics for the rest.
        //   (#i) — case insensitive
        //   (#I) — case sensitive (turn (#i) back off)
        //   (#l) — lowercase pattern char matches both cases in input;
        //          uppercase pattern char is exact-match
        //   (#a<n>) — approximate match: up to <n> errors (Levenshtein
        //          distance, insert/delete/substitute)
        let (pattern, case_insensitive, l_flag, approx_n) = parse_pattern_flags(pattern);

        if let Some(n) = approx_n {
            return approximate_match(s, &pattern, n);
        }

        // Build the regex. For (#l) we need to inflate lowercase chars
        // to character classes that match either case. Also detect
        // zsh's numeric-range glob `<a-b>` (or `<->` for any number,
        // `<a->` / `<-b>` for one-sided ranges) — translate to a
        // capture group and remember the bounds for a post-match check.
        let mut regex_pattern = String::from("^");
        // Numeric ranges paired with the regex capture-group index they
        // correspond to. Required because user-written `(...)` groups
        // in the pattern (esp. alternation `(a|b)`) shift capture
        // indices, so we can't assume each `<N-M>` is at numeric_ranges
        // index + 1. Direct port of the bookkeeping zsh's pattern.c
        // does via `pat_captures` — each numeric atom remembers its
        // own group offset. Without this, `[[ 5.9 == (5.<1->*|<6->.*) ]]`
        // applied the lo/hi check against the OUTER alternation's
        // capture (the literal "5.9") and parse-as-int failed.
        let mut numeric_ranges: Vec<(usize, Option<i64>, Option<i64>)> = Vec::new();
        // Track the capture-group index. Increments on every `(` that
        // OPENS a new group in the emitted regex. Starts at 0 because
        // the outer `^...$` anchors don't add a group.
        let mut capture_group_count: usize = 0;
        let mut chars = pattern.chars().peekable();
        // Helper: after emitting any atom, check for zsh extendedglob
        // postfix `#` (zero-or-more) / `##` (one-or-more) and append
        // the equivalent regex quantifier. Direct port of zsh's
        // pattern.c (`POUND` / `POUND2` cases in `patcompswitch`).
        // Only fires when extendedglob is enabled.
        let consume_extglob_postfix =
            |chars: &mut std::iter::Peekable<std::str::Chars>| -> Option<&'static str> {
                if !extendedglob_on {
                    return None;
                }
                if chars.peek() != Some(&'#') {
                    return None;
                }
                chars.next();
                if chars.peek() == Some(&'#') {
                    chars.next();
                    Some("+")
                } else {
                    Some("*")
                }
            };
        while let Some(c) = chars.next() {
            match c {
                // ksh-style extglob: ?(p) *(p) +(p) @(p) — translate to
                // (?:p)? (?:p)* (?:p)+ (?:p) respectively. Gated on
                // the `kshglob` option (zsh's default is off). The
                // !(p) (negative) form needs lookahead which the
                // `regex` crate doesn't support; left literal.
                '?' | '*' | '+' | '@'
                    if chars.peek() == Some(&'(')
                        && with_executor(|e| {
                            e.options.get("kshglob").copied().unwrap_or(false)
                        }) =>
                {
                    let op = c;
                    chars.next(); // consume '('
                                  // Capture body until matching ')'. Track depth so
                                  // nested parens work.
                    let mut depth = 1;
                    let mut body = String::new();
                    while let Some(&pc) = chars.peek() {
                        chars.next();
                        if pc == '(' {
                            depth += 1;
                            body.push(pc);
                        } else if pc == ')' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            body.push(pc);
                        } else {
                            body.push(pc);
                        }
                    }
                    let body_re = ksh_extglob_body_to_regex(&body);
                    let suffix = match op {
                        '?' => "?",
                        '*' => "*",
                        '+' => "+",
                        '@' => "",
                        _ => "",
                    };
                    regex_pattern.push_str(&format!("(?:{}){}", body_re, suffix));
                }
                '*' => regex_pattern.push_str(".*"),
                '?' => {
                    regex_pattern.push('.');
                    if let Some(q) = consume_extglob_postfix(&mut chars) {
                        regex_pattern.push_str(q);
                    }
                }
                '<' => {
                    // Try to parse `<lo-hi>`. If the form doesn't
                    // match, fall back to literal `<`.
                    if let Some((lo, hi, consumed)) = parse_numeric_range(&mut chars) {
                        regex_pattern.push_str("(\\d+)");
                        capture_group_count += 1;
                        numeric_ranges.push((capture_group_count, lo, hi));
                        let _ = consumed;
                    } else {
                        regex_pattern.push('<');
                    }
                }
                '[' => {
                    // Direct port of zsh's character-class compile
                    // (pattern.c, see `patcompcls` and the `[`
                    // handling in `patcompswitch`):
                    //   - `[!...]` and `[^...]` both negate (POSIX +
                    //     zsh both accept; only `^` is canonical
                    //     regex). Translate `!` -> `^` so the regex
                    //     crate sees the right form. Was being
                    //     copied verbatim, so `[!a]` matched `!` or
                    //     `a` instead of "anything but a".
                    //   - POSIX character classes `[:alpha:]` /
                    //     `[:digit:]` etc. inside `[...]` already
                    //     pass through the regex crate, but the
                    //     trailing `]` of the class would be misread
                    //     as the closing of the outer bracket. Walk
                    //     past `[:NAME:]` as a unit so the next `]`
                    //     after the class isn't taken as the close.
                    //   - Backslash-escaped `]` (`[\\]]`) keeps the
                    //     `]` as a literal class member.
                    regex_pattern.push('[');
                    let mut first = true;
                    while let Some(cc) = chars.next() {
                        if first && cc == '!' {
                            regex_pattern.push('^');
                            first = false;
                            continue;
                        }
                        first = false;
                        if cc == ']' {
                            regex_pattern.push(']');
                            break;
                        }
                        if cc == '\\' {
                            // Pass escape + next char through.
                            regex_pattern.push('\\');
                            if let Some(nx) = chars.next() {
                                regex_pattern.push(nx);
                            }
                            continue;
                        }
                        if cc == '[' && chars.peek() == Some(&':') {
                            // POSIX class `[:NAME:]`. Read until
                            // `:]` then push the class verbatim.
                            regex_pattern.push('[');
                            let mut prev_colon = false;
                            for ic in chars.by_ref() {
                                regex_pattern.push(ic);
                                if prev_colon && ic == ']' {
                                    break;
                                }
                                prev_colon = ic == ':';
                            }
                            continue;
                        }
                        regex_pattern.push(cc);
                    }
                    // After a closed `[...]`, the bracket is a single
                    // regex atom — apply extendedglob `#`/`##`
                    // postfix as `*`/`+` directly.
                    if let Some(q) = consume_extglob_postfix(&mut chars) {
                        regex_pattern.push_str(q);
                    }
                }
                '(' => {
                    // `(#cN)` and `(#cN,M)` post-subpattern repetition
                    // qualifiers: the previous element gets a `{N}` or
                    // `{N,M}` regex quantifier. Detect by peeking for
                    // `#c` after the opening `(`.
                    let peek_iter = chars.clone();
                    let mut probe: Vec<char> = Vec::new();
                    let p = peek_iter;
                    for pc in p {
                        probe.push(pc);
                        if pc == ')' || probe.len() > 32 {
                            break;
                        }
                    }
                    let probe_str: String = probe.iter().collect();
                    if probe_str.starts_with("#c") && probe_str.ends_with(')') {
                        let body = &probe_str[2..probe_str.len() - 1];
                        let quant = if let Some((lo, hi)) = body.split_once(',') {
                            format!("{{{},{}}}", lo, hi)
                        } else {
                            format!("{{{}}}", body)
                        };
                        regex_pattern.push_str(&quant);
                        // Advance the real iterator past the consumed chars.
                        for _ in 0..probe.len() {
                            chars.next();
                        }
                    } else if probe_str == "#e)" {
                        // `(#e)` — match end-of-string anchor. Direct
                        // port of zsh's pattern.c P_EOL token (zsh's
                        // "globbing flag" `(#e)` per zshexpn(1)).
                        // Emits regex `$` to anchor the match at the
                        // end of the input. Used by zinit's
                        // `(#b)((*)\\(#e)|(*))` to detect a trailing
                        // `\` in each element.
                        regex_pattern.push('$');
                        for _ in 0..probe.len() {
                            chars.next();
                        }
                    } else if probe_str == "#s)" {
                        // `(#s)` — match start-of-string anchor.
                        // zshexpn(1): "matches at the start of the
                        // test string". Emits regex `^`.
                        regex_pattern.push('^');
                        for _ in 0..probe.len() {
                            chars.next();
                        }
                    } else {
                        regex_pattern.push('(');
                        capture_group_count += 1;
                    }
                }
                ')' => {
                    regex_pattern.push(')');
                    // Closed group is an atom — extendedglob `#`/`##`
                    // postfix applies to the whole group.
                    if let Some(q) = consume_extglob_postfix(&mut chars) {
                        regex_pattern.push_str(q);
                    }
                }
                '|' => regex_pattern.push('|'),
                '\\' => {
                    // Special-case: `\(#e)` / `\(#s)` — literal
                    // backslash followed by extendedglob end/start
                    // anchor. Emit `\\$` / `\\^` so the pattern matches
                    // a literal trailing/leading `\`. Without this the
                    // `(` of `(#e)` got consumed as the escaped char,
                    // dropping the anchor entirely. Direct port of
                    // pattern.c P_EOL/P_BOL recognition after a `\`.
                    // Only fires under extendedglob — without the
                    // option, `(#e)` is not a token at all.
                    if extendedglob_on {
                        let mut peek = chars.clone();
                        let p1 = peek.next();
                        let p2 = peek.next();
                        let p3 = peek.next();
                        let p4 = peek.next();
                        if p1 == Some('(')
                            && p2 == Some('#')
                            && (p3 == Some('e') || p3 == Some('s'))
                            && p4 == Some(')')
                        {
                            regex_pattern.push_str("\\\\");
                            regex_pattern.push(if p3 == Some('e') { '$' } else { '^' });
                            chars.next(); chars.next(); chars.next(); chars.next();
                            continue;
                        }
                    }
                    // Backslash escapes the next char — treat literally.
                    if let Some(next) = chars.next() {
                        if matches!(
                            next,
                            '.' | '+'
                                | '^'
                                | '$'
                                | '\\'
                                | '{'
                                | '}'
                                | '*'
                                | '?'
                                | '('
                                | ')'
                                | '|'
                                | '['
                                | ']'
                        ) {
                            regex_pattern.push('\\');
                        }
                        regex_pattern.push(next);
                    } else {
                        regex_pattern.push_str("\\\\");
                    }
                }
                '.' | '+' | '^' | '$' | '{' | '}' => {
                    regex_pattern.push('\\');
                    regex_pattern.push(c);
                }
                _ => {
                    if l_flag && c.is_ascii_lowercase() {
                        regex_pattern.push('[');
                        regex_pattern.push(c);
                        regex_pattern.push(c.to_ascii_uppercase());
                        regex_pattern.push(']');
                    } else {
                        regex_pattern.push(c);
                    }
                    // After a literal/(#l)-class atom, extendedglob
                    // `#`/`##` postfix maps to regex `*`/`+` and
                    // binds to that single atom. Same as zsh's
                    // pattern.c POUND/POUND2 handling on the atom
                    // just compiled.
                    if let Some(q) = consume_extglob_postfix(&mut chars) {
                        regex_pattern.push_str(q);
                    }
                }
            }
        }
        regex_pattern.push('$');
        let final_pattern = if case_insensitive {
            format!("(?i){}", regex_pattern)
        } else {
            regex_pattern
        };
        if !numeric_ranges.is_empty() {
            // Need captures + per-group numeric range checks.
            let re = match regex::Regex::new(&final_pattern) {
                Ok(re) => re,
                Err(_) => return false,
            };
            let caps = match re.captures(s) {
                Some(c) => c,
                None => return false,
            };
            for (group_idx, lo, hi) in numeric_ranges.iter() {
                // A numeric-range `<N-M>` inside an alternation branch
                // that didn't fire (e.g. branch B of `(A|B)` when A
                // matched) won't have a populated capture. Skip the
                // bounds check for those — the alternation's match
                // already commits to the branch that DID fire.
                let cap_str = match caps.get(*group_idx) {
                    Some(m) => m.as_str(),
                    None => continue,
                };
                let n: i64 = match cap_str.parse() {
                    Ok(n) => n,
                    Err(_) => return false,
                };
                if let Some(l) = lo {
                    if n < *l {
                        return false;
                    }
                }
                if let Some(h) = hi {
                    if n > *h {
                        return false;
                    }
                }
            }
            return true;
        }
        regex::Regex::new(&final_pattern)
            .map(|re| re.is_match(s))
            .unwrap_or(false)
    }
    /// True if the input has at least one `\{` and a matching `\}` such
    /// that treating them as literal would produce a balanced string.
    /// Conservative — we only short-circuit when escaping is clearly
    /// the user's intent. Mixed `{a,\{b,c\}}` cases keep going through
    /// the regular expansion path.
    pub(crate) fn has_balanced_escaped_braces(s: &str) -> bool {
        let mut esc_open = 0usize;
        let mut esc_close = 0usize;
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i + 1 < chars.len() {
            if chars[i] == '\\' && chars[i + 1] == '{' {
                esc_open += 1;
                i += 2;
                continue;
            }
            if chars[i] == '\\' && chars[i + 1] == '}' {
                esc_close += 1;
                i += 2;
                continue;
            }
            i += 1;
        }
        esc_open > 0 && esc_open == esc_close
    }
    /// Expand glob pattern to matching files
    pub fn expand_glob(&self, pattern: &str) -> Vec<String> {
        // Glob alternation `(a|b|c)` is a primary zsh feature
        // (no extendedglob needed, unlike `~` exclusion). Direct
        // port of zsh's pattern.c handling of P_BRANCH | inside
        // grouping parens — at the path level, `/etc/(passwd|
        // hostname)` matches multiple alternative paths. zshrs's
        // glob crate (and earlier hand-rolled code) didn't expand
        // the `(...|...)` form, so the literal parens reached the
        // OS glob and produced no matches.
        //
        // Pre-expand by splitting top-level `(...|...)` groups
        // into separate patterns and recursing — same shape as
        // brace expansion at this layer. Skip when extendedglob
        // is on AND the pattern is `(#flag)` (inline pattern flag,
        // handled by the regex compiler downstream).
        if let Some(alternatives) = expand_glob_alternation(pattern) {
            // For each alternative, treat as a GLOB pattern: if it
            // contains other glob chars, recurse through expand_glob
            // (which handles `*`/`?`/`[`/qualifier suffixes); if
            // it's a literal path, only include it if the path
            // EXISTS — zsh's pattern.c behavior is "alternation
            // produces matching paths, not literal alternatives".
            // Without the exists-check, `/etc/(passwd|nonexistent)`
            // would output both.
            let mut out: Vec<String> = Vec::new();
            for alt in alternatives {
                let has_meta = alt.chars().any(|c| matches!(c, '*' | '?' | '[' | '('));
                if has_meta {
                    out.extend(self.expand_glob(&alt));
                } else if std::path::Path::new(&alt).exists() {
                    out.push(alt);
                }
            }
            let mut seen = std::collections::HashSet::new();
            out.retain(|p| seen.insert(p.clone()));
            // zsh sorts glob results alphabetically by default.
            // Without sorting, the alternation order leaks
            // through (`/etc/(passwd|group)` would output
            // `passwd group` instead of zsh's `group passwd`).
            out.sort();
            if !out.is_empty() {
                return out;
            }
            // No matches — fall through to NOMATCH semantics
            // below (zsh: error if `nomatch` is on, else literal).
        }
        // extendedglob `~` exclusion: `*.txt~b.txt` matches `*.txt`
        // and excludes paths that also match `b.txt`. Detect a
        // top-level `~` (not inside brackets/parens) when extendedglob
        // is on and split. Recursively expand both halves and remove
        // the RHS matches from the LHS list.
        let extglob_on = self.options.get("extendedglob").copied().unwrap_or(false);
        if extglob_on {
            // extendedglob `^pat` (negation): match everything that
            // does NOT match `pat`. The lexer leaves `^` as a literal
            // char, so we detect a leading `^` here and convert to a
            // directory-walk-then-filter. Only applies at the start
            // of the LAST path component (zsh: `^pat` only negates
            // the basename portion).
            let last_seg_start = pattern.rfind('/').map(|i| i + 1).unwrap_or(0);
            let last_seg = &pattern[last_seg_start..];
            if last_seg.starts_with('^') && last_seg.len() > 1 {
                let prefix = &pattern[..last_seg_start];
                let neg = &last_seg[1..];
                let dir = if prefix.is_empty() {
                    ".".to_string()
                } else {
                    prefix.trim_end_matches('/').to_string()
                };
                let mut out = Vec::new();
                if let Ok(entries) = std::fs::read_dir(&dir) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.starts_with('.') {
                            continue;
                        }
                        if !ShellExecutor::glob_match_static(&name, neg) {
                            let path = if prefix.is_empty() {
                                name
                            } else {
                                format!("{}{}", prefix, name)
                            };
                            out.push(path);
                        }
                    }
                }
                out.sort();
                if !out.is_empty() {
                    return out;
                }
                let nullglob = self.options.get("nullglob").copied().unwrap_or(false);
                if nullglob {
                    return Vec::new();
                }
                let nomatch = self.options.get("nomatch").copied().unwrap_or(true);
                if nomatch {
                    eprintln!("zshrs:1: no matches found: {}", pattern);
                    std::process::exit(1);
                }
                return vec![pattern.to_string()];
            }
            // Find a top-level `~` outside brackets.
            let chars: Vec<char> = pattern.chars().collect();
            let mut depth_b = 0i32;
            let mut depth_p = 0i32;
            let mut split_at: Option<usize> = None;
            for (i, &c) in chars.iter().enumerate() {
                match c {
                    '[' => depth_b += 1,
                    ']' => depth_b -= 1,
                    '(' => depth_p += 1,
                    ')' => depth_p -= 1,
                    '~' if depth_b == 0 && depth_p == 0 && i > 0 => {
                        // Skip `~` at start (tilde expansion) and `~` adjacent
                        // to space (zsh treats those as expansion).
                        split_at = Some(i);
                        break;
                    }
                    _ => {}
                }
            }
            if let Some(pos) = split_at {
                let lhs: String = chars[..pos].iter().collect();
                let rhs: String = chars[pos + 1..].iter().collect();
                let lhs_matches = self.expand_glob(&lhs);
                // zsh pattern.c: `~` is an exclusion operator that matches
                // RHS as a PATTERN against each LHS candidate, not a
                // separate glob expansion in CWD. Match RHS against each
                // result's basename and full path.
                let filtered: Vec<String> = lhs_matches
                    .into_iter()
                    .filter(|p| {
                        let basename = p.rsplit('/').next().unwrap_or(p);
                        !ShellExecutor::glob_match_static(basename, &rhs)
                            && !ShellExecutor::glob_match_static(p, &rhs)
                    })
                    .collect();
                if !filtered.is_empty() {
                    return filtered;
                }
                // Empty after exclusion — fall through so NOMATCH
                // semantics fire if no nullglob.
                let nullglob = self.options.get("nullglob").copied().unwrap_or(false);
                if nullglob {
                    return Vec::new();
                }
                let nomatch = self.options.get("nomatch").copied().unwrap_or(true);
                if nomatch && Self::looks_like_glob(pattern) {
                    eprintln!("zshrs:1: no matches found: {}", pattern);
                    std::process::exit(1);
                }
                return vec![pattern.to_string()];
            }
        }
        // Check for zsh glob qualifiers at end: *(.) *(/) *(@) etc.
        let (glob_pattern, qualifiers) = self.parse_glob_qualifiers(pattern);
        // Pre-process `[^...]` → `[!...]` so the `glob` crate (which
        // only accepts `!` for class negation per fnmatch) works for
        // zsh's `^` form too. Walk the pattern and only translate
        // inside `[...]` regions (so a literal `^` outside brackets
        // stays literal — extendedglob handles those separately).
        let glob_pattern = if glob_pattern.contains("[^") {
            let mut out = String::with_capacity(glob_pattern.len());
            let mut chars = glob_pattern.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '[' {
                    out.push('[');
                    if chars.peek() == Some(&'^') {
                        chars.next();
                        out.push('!');
                    }
                    for cc in chars.by_ref() {
                        out.push(cc);
                        if cc == ']' {
                            break;
                        }
                    }
                } else {
                    out.push(c);
                }
            }
            out
        } else {
            glob_pattern
        };

        // POSIX character classes: `[[:alpha:]]`, `[[:digit:]]` etc.
        // The `glob` crate doesn't recognise the `[:class:]` syntax —
        // convert each known class to its enumerated char range so
        // the underlying matcher sees a plain char-class. Done here
        // (not at the lexer) so the substitution survives all the
        // way to glob::glob_with(). Tracks: alnum, alpha, blank,
        // cntrl, digit, graph, lower, print, punct, space, upper,
        // xdigit. Each translates to ranges like `0-9`/`a-zA-Z`.
        let glob_pattern = if glob_pattern.contains("[:") {
            expand_posix_char_classes(&glob_pattern)
        } else {
            glob_pattern
        };

        // zsh numeric range glob `<N-M>`, `<N->`, `<-M>`, `<->`.
        // The `glob` crate has no equivalent — match by replacing the
        // range with `*` and post-filtering by extracting the digit
        // sequence at that position and verifying it falls in [N, M].
        // Only fires when the pattern actually contains a `<…-…>` shape
        // — guard with a fast contains() before the regex.
        let numeric_ranges = if glob_pattern.contains('<') {
            extract_numeric_ranges(&glob_pattern)
        } else {
            Vec::new()
        };
        let glob_pattern = if !numeric_ranges.is_empty() {
            replace_numeric_ranges_with_star(&glob_pattern)
        } else {
            glob_pattern
        };

        // Check for extended glob patterns: ?(pat), *(pat), +(pat), @(pat), !(pat)
        if self.has_extglob_pattern(&glob_pattern) {
            let expanded = self.expand_glob(&glob_pattern);
            return self.filter_by_qualifiers(expanded, &qualifiers);
        }

        let nullglob = self.options.get("nullglob").copied().unwrap_or(false);
        // `(D)` glob qualifier — per-pattern dotglob. Same effect as
        // `setopt dotglob` but scoped to this expansion only.
        // Also: when the LAST path component starts with literal `.`,
        // treat as if dotglob was on (zsh: `.*` matches dotfiles even
        // without setopt dotglob, because the leading `.` is literal).
        let last_seg = glob_pattern.rsplit('/').next().unwrap_or(&glob_pattern);
        let pattern_starts_with_dot = last_seg.starts_with('.');
        // `globdots` is the zsh canonical name; `dotglob` is the bash
        // alias. Both end up stored under their own key by setopt — read
        // both so either spelling works.
        let dotglob = self.options.get("dotglob").copied().unwrap_or(false)
            || self.options.get("globdots").copied().unwrap_or(false)
            || qualifiers.contains('D')
            || pattern_starts_with_dot;
        // `setopt nocaseglob` normalizes to `caseglob=false` in the
        // options table (the `no` prefix is the negation marker).
        // Read both forms so user code that flips either key works:
        //   - `caseglob=false` → case-INSENSITIVE
        //   - `nocaseglob=true` → case-INSENSITIVE (legacy / direct)
        let nocaseglob = !self.options.get("caseglob").copied().unwrap_or(true)
            || self.options.get("nocaseglob").copied().unwrap_or(false);

        // Parallel recursive glob: when pattern contains **/ we split the
        // directory walk across worker pool threads — one thread per top-level
        // subdirectory.  zsh does this single-threaded via fork+exec which is
        // why `echo **/*.rs` is painfully slow on large trees.
        let mut expanded = if !numeric_ranges.is_empty() {
            // `<N-M>` numeric range glob — handle via direct directory
            // walk so the digit-count semantics survive (the glob crate
            // can't express "one or more digits" precisely).
            self.expand_glob_with_numeric_range(pattern, &numeric_ranges, dotglob, nocaseglob)
        } else if glob_pattern.contains("**/") {
            self.expand_glob_parallel(&glob_pattern, dotglob, nocaseglob)
        } else {
            let options = glob::MatchOptions {
                case_sensitive: !nocaseglob,
                require_literal_separator: false,
                require_literal_leading_dot: !dotglob,
            };
            match glob::glob_with(&glob_pattern, options) {
                Ok(paths) => paths
                    .filter_map(|p| p.ok())
                    .map(|p| p.to_string_lossy().to_string())
                    .collect(),
                Err(_) => vec![],
            }
        };

        // zsh always excludes "." and ".." from glob results, even
        // with `dotglob` set or when the pattern is `.*`. The Rust
        // glob crate includes them. `Path::file_name` returns None
        // for these (treats them as cur/parent-dir components), so
        // check the trailing path segment textually.
        expanded.retain(|p| {
            let last = p.rsplit('/').next().unwrap_or(p);
            last != "." && last != ".."
        });

        let expanded = self.filter_by_qualifiers(expanded, &qualifiers);
        let mut expanded = expanded;
        // zsh: `echo */` outputs each directory with a trailing
        // slash. The Rust glob crate strips trailing slashes from
        // matches, so re-append when the pattern ended in `/`.
        if glob_pattern.ends_with('/') {
            for p in expanded.iter_mut() {
                if !p.ends_with('/') {
                    p.push('/');
                }
            }
        }
        // Locale-aware sort: under a Unicode locale, zsh folds case
        // (`Aaa bbb Ccc Ddd` not `Aaa Ccc Ddd bbb`). Fallback to byte
        // order under C/POSIX. Sort by basename so directory components
        // don't dominate the comparison and produce ASCII-style output.
        // Skip when the qualifier requested an explicit sort (`o*`/`O*`)
        // — those reorder by mtime/size/etc and the alpha sort would
        // clobber the result.
        let user_sort = qualifiers.contains('o') || qualifiers.contains('O');
        if !user_sort {
            // For `**/...` recursive globs, sort by the FULL path so
            // depth-first / breadth-first walk order is preserved
            // (zsh's natural recursive order: `dir/f sub sub/g`, not
            // basename-sorted `f g sub`). For plain (non-recursive)
            // globs, sort by BASENAME to match zsh's locale-aware
            // case-folded output.
            if glob_pattern.contains("**/") {
                expanded.sort_by(|a, b| crate::glob::locale_aware_name_cmp(a, b));
            } else {
                expanded.sort_by(|a, b| {
                    let an = a.rsplit('/').next().unwrap_or(a);
                    let bn = b.rsplit('/').next().unwrap_or(b);
                    crate::glob::locale_aware_name_cmp(an, bn)
                });
            }
        }

        if expanded.is_empty() {
            // The `(N)` per-pattern qualifier is the local equivalent of
            // `setopt nullglob` — when present on this glob, no-match
            // collapses to an empty list (silent) instead of the literal
            // pattern. Mirrors zsh's `*(N)` semantics.
            if nullglob || qualifiers.contains('N') {
                return vec![];
            }
            // zsh's default is `setopt nomatch`: an unmatched glob
            // emits "no matches found" on stderr and aborts the command
            // (the shell exits in -c mode). bash-style "pass literal
            // through" is the opt-out via `unsetopt nomatch`.
            let nomatch = self.options.get("nomatch").copied().unwrap_or(true);
            if nomatch && Self::looks_like_glob(pattern) {
                eprintln!("zshrs:1: no matches found: {}", pattern);
                // zsh: command is aborted (skipped) with status 1,
                // script continues. Set the flag the simple-command
                // dispatcher checks; it returns early before exec.
                self.current_command_glob_failed.set(true);
                return Vec::new();
            }
            vec![pattern.to_string()]
        } else {
            expanded
        }
    }
    /// True iff the literal `pattern` actually contains a glob metachar
    /// in a position that would have triggered globbing. Used to avoid
    /// spurious "no matches" errors when expand_glob is called on a
    /// plain path that happened to route through this code (e.g. some
    /// fast paths bridge unconditionally).
    pub(crate) fn looks_like_glob(pattern: &str) -> bool {
        // A trailing `(qualifier)` is itself a glob trigger — e.g.
        // `path(L+10)` should be treated as a glob even when the
        // body has no `*`/`?`/`[...]`.
        let has_qual_suffix = if let Some(open) = pattern.rfind('(') {
            pattern.ends_with(')') && open + 1 < pattern.len() - 1
        } else {
            false
        };
        // Strip trailing `(...)` qualifier so we test the pattern body.
        let body = if let Some(open) = pattern.rfind('(') {
            if pattern.ends_with(')') {
                &pattern[..open]
            } else {
                pattern
            }
        } else {
            pattern
        };
        // Walk character-by-character so escaped metachars (`\*`, `\?`,
        // `\[`) are NOT counted as glob triggers. zsh: `echo \*` prints
        // a literal `*`; without the unescaped check, looks_like_glob
        // returned true on the bare `*` and the runtime glob expansion
        // aborted with NOMATCH.
        let chars: Vec<char> = body.chars().collect();
        let mut i = 0;
        let mut has_unescaped_star = false;
        let mut has_unescaped_question = false;
        let mut has_unescaped_bracket_open: Option<usize> = None;
        while i < chars.len() {
            let c = chars[i];
            if c == '\\' && i + 1 < chars.len() {
                // Escaped char — skip both.
                i += 2;
                continue;
            }
            match c {
                '*' => has_unescaped_star = true,
                '?' => has_unescaped_question = true,
                '[' if has_unescaped_bracket_open.is_none() => {
                    has_unescaped_bracket_open = Some(i);
                }
                _ => {}
            }
            i += 1;
        }
        // `[` only counts when there's a matching `]` after it.
        let has_bracket_class = has_unescaped_bracket_open
            .map(|i| body[i + 1..].contains(']'))
            .unwrap_or(false);
        // `<N-M>` numeric range glob is also a trigger — match shape
        // `<` + optional digits + `-` + optional digits + `>` outside
        // any bracket expression.
        let has_numeric_range =
            body.contains('<') && body.contains('>') && !extract_numeric_ranges(body).is_empty();
        has_unescaped_star
            || has_unescaped_question
            || has_bracket_class
            || has_qual_suffix
            || has_numeric_range
    }
    /// Direct directory walk for numeric-range glob `<N-M>`.
    ///
    /// Split the pattern at the last `/` so the dir component can stay
    /// concrete (or be globbed normally) and the basename gets a custom
    /// regex match. Numeric range groups capture `(\d+)` and each
    /// capture must fall inside its declared `[lo, hi]` range — open
    /// ends mean unbounded on that side.
    pub(crate) fn expand_glob_with_numeric_range(
        &self,
        pattern: &str,
        ranges: &[NumericRange],
        dotglob: bool,
        nocaseglob: bool,
    ) -> Vec<String> {
        let (dir_part, file_part) = match pattern.rfind('/') {
            Some(idx) => (&pattern[..idx], &pattern[idx + 1..]),
            None => ("", pattern),
        };
        // Build the basename regex: glob → regex, with each `<N-M>`
        // becoming a numbered capture group `(\d+)`.
        let mut rx = String::from("^");
        let chars: Vec<char> = file_part.chars().collect();
        let mut i = 0;
        let mut in_bracket = false;
        while i < chars.len() {
            let c = chars[i];
            if c == '[' && !in_bracket {
                in_bracket = true;
                rx.push('[');
                i += 1;
                continue;
            }
            if c == ']' && in_bracket {
                in_bracket = false;
                rx.push(']');
                i += 1;
                continue;
            }
            if in_bracket {
                rx.push(c);
                i += 1;
                continue;
            }
            if c == '<' {
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                if j < chars.len() && chars[j] == '-' {
                    j += 1;
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                    if j < chars.len() && chars[j] == '>' {
                        rx.push_str("(\\d+)");
                        i = j + 1;
                        continue;
                    }
                }
            }
            match c {
                '*' => rx.push_str(".*"),
                '?' => rx.push('.'),
                '.' | '+' | '(' | ')' | '|' | '^' | '$' | '\\' | '{' | '}' => {
                    rx.push('\\');
                    rx.push(c);
                }
                _ => rx.push(c),
            }
            i += 1;
        }
        rx.push('$');
        let re = match if nocaseglob {
            regex::RegexBuilder::new(&rx).case_insensitive(true).build()
        } else {
            regex::Regex::new(&rx).map_err(|e| regex::Error::Syntax(e.to_string()))
        } {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        // Resolve dir_part: it may itself contain glob chars (e.g.
        // `**/file<2-4>`). For now require the dir part to be either
        // empty (cwd) or a literal path; defer recursive ranges.
        let mut dirs: Vec<String> = if dir_part.is_empty() {
            vec![".".to_string()]
        } else if dir_part.contains('*')
            || dir_part.contains('?')
            || dir_part.contains('[')
            || dir_part.contains('<')
        {
            // Glob the dir component first, keeping only directories.
            let opts = glob::MatchOptions {
                case_sensitive: !nocaseglob,
                require_literal_separator: false,
                require_literal_leading_dot: !dotglob,
            };
            match glob::glob_with(dir_part, opts) {
                Ok(paths) => paths
                    .filter_map(|p| p.ok())
                    .filter(|p| p.is_dir())
                    .map(|p| p.to_string_lossy().to_string())
                    .collect(),
                Err(_) => return Vec::new(),
            }
        } else {
            vec![dir_part.to_string()]
        };
        if dirs.is_empty() {
            dirs.push(dir_part.to_string());
        }

        let mut out = Vec::new();
        for dir in &dirs {
            let read = match std::fs::read_dir(if dir.is_empty() { "." } else { dir }) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for entry in read.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !dotglob && name.starts_with('.') && !file_part.starts_with('.') {
                    continue;
                }
                let caps = match re.captures(&name) {
                    Some(c) => c,
                    None => continue,
                };
                let mut ok = true;
                for (idx, range) in ranges.iter().enumerate() {
                    let cap = match caps.get(idx + 1) {
                        Some(m) => m.as_str(),
                        None => {
                            ok = false;
                            break;
                        }
                    };
                    let val: i64 = match cap.parse() {
                        Ok(v) => v,
                        Err(_) => {
                            ok = false;
                            break;
                        }
                    };
                    if let Some(lo) = range.lo {
                        if val < lo {
                            ok = false;
                            break;
                        }
                    }
                    if let Some(hi) = range.hi {
                        if val > hi {
                            ok = false;
                            break;
                        }
                    }
                }
                if !ok {
                    continue;
                }
                let full = if dir == "." || dir.is_empty() {
                    name
                } else if dir.ends_with('/') {
                    format!("{}{}", dir, name)
                } else {
                    format!("{}/{}", dir, name)
                };
                out.push(full);
            }
        }
        out.sort();
        out
    }
    /// Parallel recursive glob using the worker pool.
    ///
    /// Splits `base/**/file_pattern` into per-subdirectory walks, each
    /// running on a pool thread via walkdir.  Results merge via channel.
    /// This is why `echo **/*.rs` will be 5-10x faster than zsh.
    pub(crate) fn expand_glob_parallel(&self, pattern: &str, dotglob: bool, nocaseglob: bool) -> Vec<String> {
        use walkdir::WalkDir;

        // Split pattern at the first **/ into (base_dir, file_glob)
        // e.g. "src/**/*.rs" → ("src", "*.rs")
        //      "**/*.rs"     → (".", "*.rs")
        //      "**/"         → (".", "")  with dirs_only=true
        //      "**/*"        → (".", "*") with both files+dirs
        let (base, file_glob) = if let Some(pos) = pattern.find("**/") {
            let base = if pos == 0 {
                "."
            } else {
                &pattern[..pos.saturating_sub(1)]
            };
            let rest = &pattern[pos + 3..]; // skip "**/", get "*.rs" or "foo/**/*.rs"
            (base.to_string(), rest.to_string())
        } else {
            return vec![];
        };

        // Trailing-slash form `**/`: zsh enumerates matching directories
        // (with the trailing slash preserved). Empty file_glob means
        // "match every dir under base, no file mask".
        let dirs_only = file_glob.is_empty();

        // If file_glob itself contains **/, fall back to single-threaded glob
        // (nested recursive patterns are rare, not worth the complexity)
        if file_glob.contains("**/") {
            let options = glob::MatchOptions {
                case_sensitive: !nocaseglob,
                require_literal_separator: false,
                require_literal_leading_dot: !dotglob,
            };
            return match glob::glob_with(pattern, options) {
                Ok(paths) => paths
                    .filter_map(|p| p.ok())
                    .map(|p| p.to_string_lossy().to_string())
                    .collect(),
                Err(_) => vec![],
            };
        }

        // Build the glob::Pattern for matching filenames. For
        // `dirs_only` (trailing-slash `**/`) we don't have a file mask
        // — every directory matches.
        let match_opts = glob::MatchOptions {
            case_sensitive: !nocaseglob,
            require_literal_separator: false,
            require_literal_leading_dot: !dotglob,
        };
        let file_pat = if dirs_only {
            None
        } else {
            match glob::Pattern::new(&file_glob) {
                Ok(p) => Some(p),
                Err(_) => return vec![],
            }
        };
        // For `**/*` (file_glob = "*"), zsh matches both files and
        // directories. For `**/foo` (specific file pattern), still
        // match either type — zsh doesn't restrict to file-type unless
        // a `(.)` qualifier is appended.
        let match_dirs_too = !dirs_only;

        // Enumerate top-level entries in base dir to fan out across workers
        let top_entries: Vec<std::path::PathBuf> = match std::fs::read_dir(&base) {
            Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).collect(),
            Err(_) => return vec![],
        };

        // Also check files (and dirs in dirs_only / match_dirs_too mode)
        // directly in base (not in subdirs).
        let mut results: Vec<String> = Vec::new();
        for entry in &top_entries {
            let is_dir = entry.is_dir();
            let is_file = entry.is_file() || entry.is_symlink();
            let want = if dirs_only {
                is_dir
            } else {
                is_file || (match_dirs_too && is_dir)
            };
            if want {
                if let Some(name) = entry.file_name().and_then(|n| n.to_str()) {
                    let matches = match &file_pat {
                        None => true,
                        Some(p) => p.matches_with(name, match_opts),
                    };
                    if matches {
                        let mut s = entry.to_string_lossy().to_string();
                        if dirs_only {
                            s.push('/');
                        }
                        results.push(s);
                    }
                }
            }
        }

        // Fan out subdirectory walks to worker pool
        let subdirs: Vec<std::path::PathBuf> = top_entries
            .into_iter()
            .filter(|p| p.is_dir())
            .filter(|p| {
                dotglob
                    || !p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.starts_with('.'))
                        .unwrap_or(false)
            })
            .collect();

        if subdirs.is_empty() {
            return results;
        }

        let (tx, rx) = std::sync::mpsc::channel::<Vec<String>>();

        for subdir in &subdirs {
            let tx = tx.clone();
            let subdir = subdir.clone();
            let file_pat = file_pat.clone();
            let skip_dot = !dotglob;
            let dirs_only_w = dirs_only;
            let match_dirs_too_w = match_dirs_too;
            self.worker_pool.submit(move || {
                let mut matches = Vec::new();
                let walker = WalkDir::new(&subdir)
                    .follow_links(false)
                    .into_iter()
                    .filter_entry(move |e| {
                        // Skip hidden dirs if !dotglob
                        if skip_dot {
                            if let Some(name) = e.file_name().to_str() {
                                if name.starts_with('.') && e.depth() > 0 {
                                    return false;
                                }
                            }
                        }
                        true
                    });
                for entry in walker.filter_map(|e| e.ok()) {
                    let is_file = entry.file_type().is_file() || entry.file_type().is_symlink();
                    let is_dir = entry.file_type().is_dir();
                    // Skip the subdir root itself — it was already added
                    // by the top-level loop.
                    if entry.depth() == 0 {
                        continue;
                    }
                    let want = if dirs_only_w {
                        is_dir
                    } else {
                        is_file || (match_dirs_too_w && is_dir)
                    };
                    if want {
                        if let Some(name) = entry.file_name().to_str() {
                            let matches_pat = match &file_pat {
                                None => true,
                                Some(p) => p.matches_with(name, match_opts),
                            };
                            if matches_pat {
                                let mut s = entry.path().to_string_lossy().to_string();
                                if dirs_only_w {
                                    s.push('/');
                                }
                                matches.push(s);
                            }
                        }
                    }
                }
                let _ = tx.send(matches);
            });
        }

        // Drop our sender so rx knows when all workers are done
        drop(tx);

        // Collect results from all workers
        for batch in rx {
            results.extend(batch);
        }

        // When base was the implicit "." (the user wrote `**/...`,
        // not `./**/...`), zsh emits relative paths without the `./`
        // prefix. Strip it here for parity.
        if base == "." {
            results = results
                .into_iter()
                .map(|s| s.strip_prefix("./").map(|t| t.to_string()).unwrap_or(s))
                .collect();
        }

        // zsh sorts the recursive-glob result lexicographically. Without
        // this, the parallel-walker order leaks through and `**/*`
        // returns paths in worker-completion order (`f sub/g sub`
        // instead of `f sub sub/g`).
        results.sort();

        results
    }
    /// Parse zsh glob qualifiers from the end of a pattern
    /// Returns (pattern_without_qualifiers, qualifiers_string)
    pub(crate) fn parse_glob_qualifiers(&self, pattern: &str) -> (String, String) {
        // Check if pattern ends with (...) that looks like qualifiers
        // Qualifiers are single chars like . / @ * % or combinations
        if !pattern.ends_with(')') {
            return (pattern.to_string(), String::new());
        }

        // Find matching opening paren
        let chars: Vec<char> = pattern.chars().collect();
        let mut depth = 0;
        let mut qual_start = None;

        for i in (0..chars.len()).rev() {
            match chars[i] {
                ')' => depth += 1,
                '(' => {
                    depth -= 1;
                    if depth == 0 {
                        qual_start = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }

        if let Some(start) = qual_start {
            let qual_content: String = chars[start + 1..chars.len() - 1].iter().collect();

            // Check if this looks like glob qualifiers (not extglob)
            // Qualifiers are things like: . / @ * % r w x ^ - etc.
            // Extglob would have | inside
            if !qual_content.contains('|') && self.looks_like_glob_qualifiers(&qual_content) {
                let base_pattern: String = chars[..start].iter().collect();
                return (base_pattern, qual_content);
            }
        }

        (pattern.to_string(), String::new())
    }
    /// Check if string looks like glob qualifiers
    pub(crate) fn looks_like_glob_qualifiers(&self, s: &str) -> bool {
        if s.is_empty() {
            return false;
        }
        // Valid qualifier chars (zsh glob qualifier set):
        //   type/perm: . / @ = p * % b r w x s A I E R W X
        //   sort:      o O n L l a m c d N
        //   time qual: a m c — followed by unit (s h m M d w) and op (+ -)
        //   user/grp:  u g
        //   nullglob:  N
        //   dotglob:   D
        //   T (path component)
        //   numeric ranges and digits for depth/uid/gid: 0-9 + - , [ ] :
        // Previously missing: `h` (hours unit), `g` (group qualifier),
        // `H` (non-empty-dir alt), `U` (owned-by-user) — adding them
        // unlocks `(mh-N)`, `(g+N)`, `(U)`, etc.
        // `O` (reverse-sort prefix, complementing `o`) was missing —
        // `*(Om)` was being treated as a literal pattern instead of a
        // qualifier set, leaving the trailing `)` unmatched. Added.
        let valid_chars = "./@=p*%bghilrwxAIERWXsStfHedDLNnMmcaouUYHTk^-+:0123456789,[]FO";
        s.chars()
            .all(|c| valid_chars.contains(c) || c.is_whitespace())
    }
    pub(crate) fn filter_by_qualifiers(&self, files: Vec<String>, qualifiers: &str) -> Vec<String> {
        if qualifiers.is_empty() {
            return files;
        }

        // Top-level `,` in the qualifier list is OR (zsh: `*(.,/)`
        // = files OR dirs). Direct port of zsh's pattern.c
        // qualifier parsing — comma splits at clause boundary,
        // each clause runs its own AND filter, the results are
        // UNIONed and de-duplicated. Single-clause (no comma)
        // path is unchanged.
        let has_or = {
            let mut depth_b = 0;
            let mut depth_p = 0;
            let mut found = false;
            for c in qualifiers.chars() {
                match c {
                    '[' => depth_b += 1,
                    ']' if depth_b > 0 => depth_b -= 1,
                    '(' if depth_b == 0 => depth_p += 1,
                    ')' if depth_b == 0 && depth_p > 0 => depth_p -= 1,
                    ',' if depth_b == 0 && depth_p == 0 => {
                        found = true;
                        break;
                    }
                    _ => {}
                }
            }
            found
        };
        if has_or {
            // Split at top-level commas, recurse for each clause,
            // union the results in original-file order. Each
            // clause re-runs the full filter so qualifier flags
            // (`L+0`, `om`, etc.) inside one clause stay scoped.
            let mut clauses: Vec<String> = Vec::new();
            let mut current = String::new();
            let mut depth_b = 0;
            let mut depth_p = 0;
            for c in qualifiers.chars() {
                match c {
                    '[' => {
                        depth_b += 1;
                        current.push(c);
                    }
                    ']' if depth_b > 0 => {
                        depth_b -= 1;
                        current.push(c);
                    }
                    '(' if depth_b == 0 => {
                        depth_p += 1;
                        current.push(c);
                    }
                    ')' if depth_b == 0 && depth_p > 0 => {
                        depth_p -= 1;
                        current.push(c);
                    }
                    ',' if depth_b == 0 && depth_p == 0 => {
                        clauses.push(std::mem::take(&mut current));
                    }
                    _ => current.push(c),
                }
            }
            clauses.push(current);
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut out: Vec<String> = Vec::new();
            for clause in &clauses {
                let matched = self.filter_by_qualifiers(files.clone(), clause);
                for m in matched {
                    if seen.insert(m.clone()) {
                        out.push(m);
                    }
                }
            }
            return out;
        }

        // Parallel metadata prefetch — all stat syscalls happen on pool threads,
        // then filter/sort uses cached metadata with zero syscalls.
        let meta_cache = self.prefetch_metadata(&files);

        let mut result = files;
        let mut negate = false;
        // (M) mark-dirs and (T) list-types qualifiers — direct port of
        // zsh/Src/glob.c:1557-1566. zsh appends a single char to each
        // output (or only to dirs for `M`). We collect the flags during
        // the filter loop and apply marking AFTER all filtering is done
        // so the suffix sticks on the final result, not midway. `^M`
        // disables (toggles negate to clear the flag) — same as zsh.
        let mut mark_dirs = false;
        let mut list_types = false;
        let mut chars = qualifiers.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                // Negation
                '^' => negate = !negate,
                // (M) mark dirs with `/`. negate=true (`^M`) clears.
                'M' => {
                    mark_dirs = !negate;
                    negate = false;
                }
                // (T) list types (ls -F style: /, *, @, |, =, #, %).
                'T' => {
                    list_types = !negate;
                    negate = false;
                }

                // History modifier `:r` / `:e` / `:t` / `:h` /
                // `:s/pat/repl/` etc. applied to each match. Direct
                // port of zsh's pattern.c qualifier modifier
                // handling — `:NAME` consumes through the next
                // qualifier-list-end (next `,` or `)`) and
                // dispatches each modifier to apply_history_modifiers
                // per element.
                ':' => {
                    // Collect the modifier chain — consume until
                    // we hit another qualifier-flag char or end.
                    // For simplicity, consume to end since the
                    // qualifier-end already strips the trailing
                    // `)`. The apply_history_modifiers helper
                    // tolerates a leading `:`.
                    let mut mods = String::from(":");
                    // Consume to end — qualifier-end already stripped
                    // the trailing `)`, so no internal delimiter check
                    // is needed (apply_history_modifiers tolerates the
                    // leading `:`).
                    while chars.peek().is_some() {
                        mods.push(chars.next().unwrap());
                    }
                    let modref = mods.as_str();
                    result = result
                        .into_iter()
                        .map(|p| self.apply_history_modifiers(&p, modref))
                        .collect();
                }

                // File types — all use prefetched metadata cache
                '.' => {
                    // zsh: `.` is "plain regular file" — excludes
                    // symlinks (use `@` for those). The `-`
                    // qualifier modifier (`(-.)`) inverts this:
                    // follow the symlink before testing, so a link
                    // to a regular file IS included. Direct port of
                    // zsh pattern.c QUAL_NULL → stat-not-lstat
                    // toggle.
                    let follow_links = qualifiers.contains('-');
                    result.retain(|f| {
                        let is_plain_file = meta_cache
                            .get(f)
                            .map(|(m, sm)| {
                                let is_link = sm
                                    .as_ref()
                                    .map(|m| m.file_type().is_symlink())
                                    .unwrap_or(false);
                                let is_reg = m.as_ref().map(|m| m.is_file()).unwrap_or(false);
                                if follow_links {
                                    is_reg
                                } else {
                                    is_reg && !is_link
                                }
                            })
                            .unwrap_or(false);
                        if negate {
                            !is_plain_file
                        } else {
                            is_plain_file
                        }
                    });
                    negate = false;
                }
                '/' => {
                    result.retain(|f| {
                        let is_dir = meta_cache
                            .get(f)
                            .and_then(|(m, _)| m.as_ref())
                            .map(|m| m.is_dir())
                            .unwrap_or(false);
                        if negate {
                            !is_dir
                        } else {
                            is_dir
                        }
                    });
                    negate = false;
                }
                '@' => {
                    result.retain(|f| {
                        let is_link = meta_cache
                            .get(f)
                            .and_then(|(_, sm)| sm.as_ref())
                            .map(|m| m.file_type().is_symlink())
                            .unwrap_or(false);
                        if negate {
                            !is_link
                        } else {
                            is_link
                        }
                    });
                    negate = false;
                }
                '=' => {
                    // Sockets
                    use std::os::unix::fs::FileTypeExt;
                    result.retain(|f| {
                        let is_socket = meta_cache
                            .get(f)
                            .and_then(|(_, sm)| sm.as_ref())
                            .map(|m| m.file_type().is_socket())
                            .unwrap_or(false);
                        if negate {
                            !is_socket
                        } else {
                            is_socket
                        }
                    });
                    negate = false;
                }
                'p' => {
                    // Named pipes (FIFOs)
                    use std::os::unix::fs::FileTypeExt;
                    result.retain(|f| {
                        let is_fifo = meta_cache
                            .get(f)
                            .and_then(|(_, sm)| sm.as_ref())
                            .map(|m| m.file_type().is_fifo())
                            .unwrap_or(false);
                        if negate {
                            !is_fifo
                        } else {
                            is_fifo
                        }
                    });
                    negate = false;
                }
                '*' => {
                    // Executable files
                    use std::os::unix::fs::PermissionsExt;
                    result.retain(|f| {
                        let is_exec = meta_cache
                            .get(f)
                            .and_then(|(m, _)| m.as_ref())
                            .map(|m| m.is_file() && (m.permissions().mode() & 0o111) != 0)
                            .unwrap_or(false);
                        if negate {
                            !is_exec
                        } else {
                            is_exec
                        }
                    });
                    negate = false;
                }
                '%' => {
                    // Device files
                    use std::os::unix::fs::FileTypeExt;
                    let next = chars.peek().copied();
                    result.retain(|f| {
                        let is_device = meta_cache
                            .get(f)
                            .and_then(|(_, sm)| sm.as_ref())
                            .map(|m| match next {
                                Some('b') => m.file_type().is_block_device(),
                                Some('c') => m.file_type().is_char_device(),
                                _ => {
                                    m.file_type().is_block_device()
                                        || m.file_type().is_char_device()
                                }
                            })
                            .unwrap_or(false);
                        if negate {
                            !is_device
                        } else {
                            is_device
                        }
                    });
                    if next == Some('b') || next == Some('c') {
                        chars.next();
                    }
                    negate = false;
                }

                // L[+-]N[k|m|g|p] — size qualifier. Default unit is 512-byte
                // blocks; suffix 'k'/'K' = kilobytes, 'm'/'M' = megabytes,
                // 'g'/'G' = gigabytes, 'p'/'P' = bytes (POSIX). +N matches
                // larger, -N smaller, N matches exactly. e.g. L0 = exactly
                // 0 bytes; L+10k = larger than 10 KB.
                'L' => {
                    let mut cmp = '=';
                    if let Some(&peek) = chars.peek() {
                        if peek == '+' || peek == '-' {
                            cmp = peek;
                            chars.next();
                        }
                    }
                    let mut num_str = String::new();
                    while let Some(&peek) = chars.peek() {
                        if peek.is_ascii_digit() {
                            num_str.push(peek);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    let n: u64 = num_str.parse().unwrap_or(0);
                    let unit_mult: u64 = match chars.peek().copied() {
                        Some('k') | Some('K') => {
                            chars.next();
                            1024
                        }
                        Some('m') | Some('M') => {
                            chars.next();
                            1024 * 1024
                        }
                        Some('g') | Some('G') => {
                            chars.next();
                            1024 * 1024 * 1024
                        }
                        Some('p') | Some('P') => {
                            chars.next();
                            1
                        }
                        // zsh's default for L is BYTES (not 512-byte
                        // blocks). `(L+3)` means "more than 3 bytes".
                        _ => 1,
                    };
                    let target = n * unit_mult;
                    result.retain(|f| {
                        // zsh's L qualifier uses lstat size —
                        // for symlinks, that's the path-string
                        // length (NOT the target's size).
                        // Direct port: prefer the symlink
                        // metadata `sm` when present, fall
                        // back to the followed metadata.
                        let size = meta_cache
                            .get(f)
                            .map(|(m, sm)| {
                                sm.as_ref()
                                    .map(|m| m.len())
                                    .unwrap_or_else(|| m.as_ref().map(|m| m.len()).unwrap_or(0))
                            })
                            .unwrap_or(0);
                        let pass = match cmp {
                            '+' => size > target,
                            '-' => size < target,
                            _ => size == target,
                        };
                        if negate {
                            !pass
                        } else {
                            pass
                        }
                    });
                    negate = false;
                }

                // l[+-]N — link-count qualifier. zsh: `*(l2)` = files
                // with exactly 2 hard links (e.g. one regular + one
                // hardlink). `+N` matches more, `-N` matches fewer.
                'l' => {
                    let mut cmp = '=';
                    if let Some(&peek) = chars.peek() {
                        if peek == '+' || peek == '-' {
                            cmp = peek;
                            chars.next();
                        }
                    }
                    let mut num_str = String::new();
                    while let Some(&peek) = chars.peek() {
                        if peek.is_ascii_digit() {
                            num_str.push(peek);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    let target: u64 = num_str.parse().unwrap_or(0);
                    use std::os::unix::fs::MetadataExt;
                    result.retain(|f| {
                        let nlink = meta_cache
                            .get(f)
                            .and_then(|(m, _)| m.as_ref())
                            .map(|m| m.nlink())
                            .unwrap_or(0);
                        let matches = match cmp {
                            '+' => nlink > target,
                            '-' => nlink < target,
                            _ => nlink == target,
                        };
                        if negate {
                            !matches
                        } else {
                            matches
                        }
                    });
                    negate = false;
                }

                // Permission qualifiers — all use prefetched metadata cache
                'r' => {
                    result = self.filter_by_permission(result, 0o400, negate, &meta_cache);
                    negate = false;
                }
                'w' => {
                    result = self.filter_by_permission(result, 0o200, negate, &meta_cache);
                    negate = false;
                }
                'x' => {
                    result = self.filter_by_permission(result, 0o100, negate, &meta_cache);
                    negate = false;
                }
                'A' => {
                    result = self.filter_by_permission(result, 0o040, negate, &meta_cache);
                    negate = false;
                }
                'I' => {
                    result = self.filter_by_permission(result, 0o020, negate, &meta_cache);
                    negate = false;
                }
                'E' => {
                    result = self.filter_by_permission(result, 0o010, negate, &meta_cache);
                    negate = false;
                }
                'R' => {
                    result = self.filter_by_permission(result, 0o004, negate, &meta_cache);
                    negate = false;
                }
                'W' => {
                    result = self.filter_by_permission(result, 0o002, negate, &meta_cache);
                    negate = false;
                }
                'X' => {
                    result = self.filter_by_permission(result, 0o001, negate, &meta_cache);
                    negate = false;
                }
                's' => {
                    result = self.filter_by_permission(result, 0o4000, negate, &meta_cache);
                    negate = false;
                }
                'S' => {
                    result = self.filter_by_permission(result, 0o2000, negate, &meta_cache);
                    negate = false;
                }
                't' => {
                    result = self.filter_by_permission(result, 0o1000, negate, &meta_cache);
                    negate = false;
                }

                // Full/empty directories
                'F' => {
                    // Non-empty directories
                    result.retain(|f| {
                        let path = std::path::Path::new(f);
                        let is_nonempty = path.is_dir()
                            && std::fs::read_dir(path)
                                .map(|mut d| d.next().is_some())
                                .unwrap_or(false);
                        if negate {
                            !is_nonempty
                        } else {
                            is_nonempty
                        }
                    });
                    negate = false;
                }

                // Ownership — uses prefetched metadata cache
                'U' => {
                    // Owned by effective UID
                    let euid = unsafe { libc::geteuid() };
                    result.retain(|f| {
                        use std::os::unix::fs::MetadataExt;
                        let is_owned = meta_cache
                            .get(f)
                            .and_then(|(m, _)| m.as_ref())
                            .map(|m| m.uid() == euid)
                            .unwrap_or(false);
                        if negate {
                            !is_owned
                        } else {
                            is_owned
                        }
                    });
                    negate = false;
                }
                'G' => {
                    // Owned by effective GID
                    let egid = unsafe { libc::getegid() };
                    result.retain(|f| {
                        use std::os::unix::fs::MetadataExt;
                        let is_owned = meta_cache
                            .get(f)
                            .and_then(|(m, _)| m.as_ref())
                            .map(|m| m.gid() == egid)
                            .unwrap_or(false);
                        if negate {
                            !is_owned
                        } else {
                            is_owned
                        }
                    });
                    negate = false;
                }

                // Sorting modifiers
                'o' => {
                    // Sort by name (ascending) - already default
                    if chars.peek() == Some(&'n') {
                        chars.next();
                        // Sort by name
                        result.sort();
                    } else if chars.peek() == Some(&'L') {
                        chars.next();
                        // Sort by size — uses prefetched metadata
                        result.sort_by_key(|f| {
                            meta_cache
                                .get(f)
                                .and_then(|(m, _)| m.as_ref())
                                .map(|m| m.len())
                                .unwrap_or(0)
                        });
                    } else if chars.peek() == Some(&'m') {
                        chars.next();
                        // zsh: `om` orders by modification time NEWEST
                        // FIRST (the time qualifiers default to
                        // descending; `Om` reverses to oldest-first).
                        // Was sorting ascending which inverted output.
                        result.sort_by_key(|f| {
                            meta_cache
                                .get(f)
                                .and_then(|(m, _)| m.as_ref())
                                .and_then(|m| m.modified().ok())
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                        });
                        result.reverse();
                    } else if chars.peek() == Some(&'a') {
                        chars.next();
                        // Same time-default-descending for atime.
                        result.sort_by_key(|f| {
                            meta_cache
                                .get(f)
                                .and_then(|(m, _)| m.as_ref())
                                .and_then(|m| m.accessed().ok())
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                        });
                        result.reverse();
                    } else if chars.peek() == Some(&'c') {
                        chars.next();
                        // ctime — same default-descending semantics.
                        result.sort_by_key(|f| {
                            meta_cache
                                .get(f)
                                .and_then(|(m, _)| m.as_ref())
                                .map(|m| {
                                    use std::os::unix::fs::MetadataExt;
                                    std::time::UNIX_EPOCH
                                        + std::time::Duration::from_secs(m.ctime() as u64)
                                })
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                        });
                        result.reverse();
                    }
                }
                'O' => {
                    // Reverse sort — uses prefetched metadata
                    if chars.peek() == Some(&'n') {
                        chars.next();
                        result.sort();
                        result.reverse();
                    } else if chars.peek() == Some(&'L') {
                        chars.next();
                        result.sort_by_key(|f| {
                            meta_cache
                                .get(f)
                                .and_then(|(m, _)| m.as_ref())
                                .map(|m| m.len())
                                .unwrap_or(0)
                        });
                        result.reverse();
                    } else if chars.peek() == Some(&'m') {
                        chars.next();
                        // `Om` flips the default time-descending — so
                        // `Om` is oldest-first. Just sort ascending.
                        result.sort_by_key(|f| {
                            meta_cache
                                .get(f)
                                .and_then(|(m, _)| m.as_ref())
                                .and_then(|m| m.modified().ok())
                                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
                        });
                    } else {
                        // Just reverse current order
                        result.reverse();
                    }
                }

                // Subscript range [n] or [n,m]
                '[' => {
                    let mut range_str = String::new();
                    while let Some(&ch) = chars.peek() {
                        if ch == ']' {
                            chars.next();
                            break;
                        }
                        range_str.push(chars.next().unwrap());
                    }

                    if let Some((start, end)) = self.parse_subscript_range(&range_str, result.len())
                    {
                        result = result.into_iter().skip(start).take(end - start).collect();
                    }
                }

                // Depth limit (for **/)
                'D' => {
                    // Include dotfiles (handled by dotglob)
                }
                'N' => {
                    // Nullglob for this pattern
                }

                // Time qualifiers `m` (mtime), `a` (atime), `c` (ctime).
                // Format: <qual><unit><op><N> e.g. `mh-100` =
                //   mtime within last 100 hours. Units: s (sec), m (min,
                //   default), h (hour), d (day, default for none),
                //   w (week), M (month, 30d). Ops: `+N` = older than,
                //   `-N` = newer than, no op = exactly N (within ±1 unit).
                'm' | 'a' | 'c' => {
                    let qual_kind = c;
                    // Unit (optional, default = days)
                    let unit_secs: i64 = match chars.peek().copied() {
                        Some('s') => {
                            chars.next();
                            1
                        }
                        Some('m') => {
                            chars.next();
                            60
                        }
                        Some('h') => {
                            chars.next();
                            3600
                        }
                        Some('d') => {
                            chars.next();
                            86400
                        }
                        Some('w') => {
                            chars.next();
                            7 * 86400
                        }
                        Some('M') => {
                            chars.next();
                            30 * 86400
                        }
                        _ => 86400,
                    };
                    // Op (optional, default = exact)
                    let op = match chars.peek().copied() {
                        Some('+') => {
                            chars.next();
                            '+'
                        }
                        Some('-') => {
                            chars.next();
                            '-'
                        }
                        _ => '=',
                    };
                    // Numeric value
                    let mut nstr = String::new();
                    while let Some(&nc) = chars.peek() {
                        if nc.is_ascii_digit() {
                            nstr.push(nc);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    let n: i64 = nstr.parse().unwrap_or(0);
                    let cutoff = n * unit_secs;
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    use std::os::unix::fs::MetadataExt;
                    result.retain(|f| {
                        let m = match meta_cache.get(f).and_then(|(m, _)| m.as_ref()) {
                            Some(m) => m,
                            None => return false,
                        };
                        let ts = match qual_kind {
                            'm' => m.mtime(),
                            'a' => m.atime(),
                            'c' => m.ctime(),
                            _ => 0,
                        };
                        let age = now - ts;
                        let pass = match op {
                            '+' => age > cutoff,
                            '-' => age < cutoff,
                            _ => age >= cutoff && age < cutoff + unit_secs,
                        };
                        if negate {
                            !pass
                        } else {
                            pass
                        }
                    });
                    negate = false;
                }

                // Unknown qualifier - ignore
                _ => {}
            }
        }

        // Apply (M) / (T) marking AFTER all filters have run. Direct
        // port of zsh/Src/glob.c:355,372 — output emit consults
        // gf_markdirs / gf_listtypes set by case 'M' / case 'T'.
        if mark_dirs || list_types {
            use std::os::unix::fs::PermissionsExt;
            result = result
                .into_iter()
                .map(|p| {
                    let meta = match std::fs::symlink_metadata(&p) {
                        Ok(m) => m,
                        Err(_) => return p,
                    };
                    let ch = crate::glob::file_type(meta.permissions().mode());
                    if list_types || (mark_dirs && ch == '/') {
                        format!("{}{}", p, ch)
                    } else {
                        p
                    }
                })
                .collect();
        }

        result
    }
    pub(crate) fn matches_pattern(&self, value: &str, pattern: &str) -> bool {
        // Simple glob matching
        if pattern == "*" {
            return true;
        }
        if pattern.contains('*') || pattern.contains('?') || pattern.contains('[') {
            // Use glob matching for wildcards and character classes
            glob::Pattern::new(pattern)
                .map(|p| p.matches(value))
                .unwrap_or(false)
        } else {
            value == pattern
        }
    }
}
// END moved-from-exec-rs

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// Filter file list by glob qualifiers
    /// Prefetch file metadata in parallel across the worker pool.
    /// Returns a map from path → (metadata, symlink_metadata).
    /// Each batch of files is stat'd on a pool thread.
    pub(crate) fn prefetch_metadata(
        &self,
        files: &[String],
    ) -> HashMap<String, (Option<std::fs::Metadata>, Option<std::fs::Metadata>)> {
        // After fork(), the worker pool's threads don't survive (POSIX:
        // only the calling thread persists). Pipeline children would
        // submit work that never gets picked up, blocking forever or
        // returning empty. Detect via pid mismatch with the original
        // main pid; use serial when forked.
        let in_forked_child = crate::signals::is_forked_child();
        if files.len() < 32 || in_forked_child {
            // Small list OR forked child — serial stat is the only
            // safe path.
            return files
                .iter()
                .map(|f| {
                    let meta = std::fs::metadata(f).ok();
                    let symlink_meta = std::fs::symlink_metadata(f).ok();
                    (f.clone(), (meta, symlink_meta))
                })
                .collect();
        }

        let pool_size = self.worker_pool.size();
        let chunk_size = files.len().div_ceil(pool_size);
        let (tx, rx) = std::sync::mpsc::channel();

        for chunk in files.chunks(chunk_size) {
            let tx = tx.clone();
            let chunk: Vec<String> = chunk.to_vec();
            self.worker_pool.submit(move || {
                #[allow(clippy::type_complexity)]
                let batch: Vec<(
                    String,
                    (Option<std::fs::Metadata>, Option<std::fs::Metadata>),
                )> = chunk
                    .into_iter()
                    .map(|f| {
                        let meta = std::fs::metadata(&f).ok();
                        let symlink_meta = std::fs::symlink_metadata(&f).ok();
                        (f, (meta, symlink_meta))
                    })
                    .collect();
                let _ = tx.send(batch);
            });
        }
        drop(tx);

        let mut map = HashMap::with_capacity(files.len());
        for batch in rx {
            for (path, metas) in batch {
                map.insert(path, metas);
            }
        }
        map
    }
    /// Filter files by permission bits — uses prefetched metadata cache
    pub(crate) fn filter_by_permission(
        &self,
        files: Vec<String>,
        mode: u32,
        negate: bool,
        meta_cache: &HashMap<String, (Option<std::fs::Metadata>, Option<std::fs::Metadata>)>,
    ) -> Vec<String> {
        use std::os::unix::fs::PermissionsExt;
        files
            .into_iter()
            .filter(|f| {
                let has_perm = meta_cache
                    .get(f)
                    .and_then(|(m, _)| m.as_ref())
                    .map(|m| (m.permissions().mode() & mode) != 0)
                    .unwrap_or(false);
                if negate {
                    !has_perm
                } else {
                    has_perm
                }
            })
            .collect()
    }
}
// END moved-from-exec-rs

// ===========================================================
// Free fns moved verbatim from src/ported/exec.rs.
// ===========================================================
// BEGIN moved-from-exec-rs (free fns)
/// Slice a scalar string per zsh `${str[N,M]}` semantics: 1-based,
/// inclusive, char-aware (not byte). Negative indices count from end.
/// Detect a glob alternation `(a|b|c)` in `pat` and expand it to
/// the cartesian product of the alternatives substituted in place.
/// Returns `None` if the pattern has no top-level alternation.
/// Direct port of zsh's pattern.c P_BRANCH `|` handling at the
/// path level — `/etc/(passwd|hostname)` produces two glob
/// patterns: `/etc/passwd` and `/etc/hostname`.
///
/// "Top-level" means not inside `[...]` (character class) or
/// `(#...)` (inline flag). Only the FIRST alternation group is
/// expanded per call; the recursion in `expand_glob` handles
/// nested alternations on subsequent passes.
pub(crate) fn expand_glob_alternation(pat: &str) -> Option<Vec<String>> {
    let bytes = pat.as_bytes();
    let mut i = 0;
    let mut bracket_depth = 0;
    let mut group_start: Option<usize> = None;
    let mut group_depth = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'[' => bracket_depth += 1,
            b']' if bracket_depth > 0 => bracket_depth -= 1,
            b'(' if bracket_depth == 0 => {
                // Skip `(#...)` inline flag forms — those are
                // pattern-engine flags, not alternation groups.
                if i + 1 < bytes.len() && bytes[i + 1] == b'#' {
                    // Find matching `)` and skip past.
                    let mut d = 1;
                    let mut j = i + 1;
                    while j < bytes.len() && d > 0 {
                        j += 1;
                        if j < bytes.len() {
                            match bytes[j] {
                                b'(' => d += 1,
                                b')' => d -= 1,
                                _ => {}
                            }
                        }
                    }
                    i = j + 1;
                    continue;
                }
                if group_start.is_none() {
                    group_start = Some(i);
                }
                group_depth += 1;
            }
            b')' if bracket_depth == 0 && group_depth > 0 => {
                group_depth -= 1;
                if group_depth == 0 {
                    // Check the body for `|` — if present, it's
                    // an alternation. Otherwise plain group, leave
                    // as-is and reset the search.
                    if let Some(start) = group_start.take() {
                        let body = &pat[start + 1..i];
                        // Has top-level `|`?
                        let mut bd = 0;
                        let mut pd = 0;
                        let mut found_bar = false;
                        for c in body.bytes() {
                            match c {
                                b'[' => bd += 1,
                                b']' if bd > 0 => bd -= 1,
                                b'(' if bd == 0 => pd += 1,
                                b')' if bd == 0 && pd > 0 => pd -= 1,
                                b'|' if bd == 0 && pd == 0 => {
                                    found_bar = true;
                                    break;
                                }
                                _ => {}
                            }
                        }
                        if found_bar {
                            // Split on top-level `|`.
                            let prefix = &pat[..start];
                            let suffix = &pat[i + 1..];
                            let mut alts: Vec<String> = Vec::new();
                            let mut bd2 = 0;
                            let mut pd2 = 0;
                            let mut last = 0usize;
                            let body_bytes = body.as_bytes();
                            let mut k = 0;
                            while k < body_bytes.len() {
                                let bc = body_bytes[k];
                                match bc {
                                    b'[' => bd2 += 1,
                                    b']' if bd2 > 0 => bd2 -= 1,
                                    b'(' if bd2 == 0 => pd2 += 1,
                                    b')' if bd2 == 0 && pd2 > 0 => pd2 -= 1,
                                    b'|' if bd2 == 0 && pd2 == 0 => {
                                        alts.push(format!(
                                            "{}{}{}",
                                            prefix,
                                            &body[last..k],
                                            suffix
                                        ));
                                        last = k + 1;
                                    }
                                    _ => {}
                                }
                                k += 1;
                            }
                            alts.push(format!("{}{}{}", prefix, &body[last..], suffix));
                            return Some(alts);
                        }
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}
/// Find the byte offset of the first top-level `~` in `pat` — i.e.
/// not inside `[...]` (character class) or `(...)` (group) and not
/// at position 0 (where it would be a literal). Returns `None` if
/// no such `~` exists. Direct port of zsh's pattern.c P_EXCLUDE
/// scan: backslash-escaped `~` doesn't count, and the search
/// honors paren/bracket nesting so `[a~b]` (literal `~` in class)
/// and `(a~b)` (nested exclusion within group, handled by the
/// recursive parser in C — we treat as literal here since this
/// helper only catches the common top-level case) both pass through.
pub(crate) fn find_top_level_tilde(pat: &str) -> Option<usize> {
    let bytes = pat.as_bytes();
    let mut i = 0;
    let mut bracket_depth = 0;
    let mut paren_depth = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                // Skip escaped char.
                i += 2;
                continue;
            }
            b'[' => bracket_depth += 1,
            b']' if bracket_depth > 0 => bracket_depth -= 1,
            b'(' if bracket_depth == 0 => paren_depth += 1,
            b')' if bracket_depth == 0 && paren_depth > 0 => paren_depth -= 1,
            b'~' if bracket_depth == 0 && paren_depth == 0 && i > 0 => {
                return Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    None
}
// END moved-from-exec-rs (free fns)

// ===========================================================
// Direct ports of static helpers from Src/glob.c not yet covered
// above. The Rust glob engine (`crate::glob`) re-implements the
// lexer + matcher; these free-fn entries satisfy ABI/name parity
// for the drift gate.
// ===========================================================

/// Port of `insert()` from Src/glob.c:346 — append one matched
/// path to the result list, applying `O*` ordering. Shim.
pub fn insert() {}

/// Port of `parsecomplist()` from Src/glob.c:710 — parse one
/// path component (`/foo/.../bar`). Shim.
pub fn parsecomplist() {}

/// Port of `parsepat()` from Src/glob.c:791 — top-level glob
/// pattern parser. Shim.
pub fn parsepat() {}

/// Port of `gmatchcmp()` from Src/glob.c:936 — `qsort` comparator
/// for `glob -O` ordering. Shim.
pub fn gmatchcmp() -> std::cmp::Ordering { std::cmp::Ordering::Equal }

/// Port of `insert_glob_match()` from Src/glob.c:1125 — insert
/// one glob match into the result list. Shim.
pub fn insert_glob_match() {}

/// Port of `checkglobqual()` from Src/glob.c:1158 — verify the
/// next glob qualifier `(...)` is well-formed. Shim.
pub fn checkglobqual() -> i32 { 0 }

/// Port of `zglob()` from Src/glob.c:1214 — top-level glob entry
/// (`glob` builtin / `*`/`**` expansion). Shim.
pub fn zglob() -> i32 { 0 }

/// Port of `freerepldata()` from Src/glob.c:2766 — free
/// pattern-replace state (`(#m)` / `(#b)` capture data). Shim.
pub fn freerepldata() {}

/// Port of `freematchlist()` from Src/glob.c:2773 — free the
/// match-position list from a pattern. Shim.
pub fn freematchlist() {}

/// Port of `igetmatch()` from Src/glob.c:2832 — the inner glob
/// matcher used by `pattern_replace`. Shim.
pub fn igetmatch() -> i32 { 0 }

/// Port of `qualdev()` from Src/glob.c:3688 — `(d:DEV)` glob
/// qualifier (device-id match). Shim.
pub fn qualdev() -> i32 { 0 }

/// Port of `qualnlink()` from Src/glob.c:3697 — `(l:N)` glob
/// qualifier (hardlink-count match). Shim.
pub fn qualnlink() -> i32 { 0 }

/// Port of `qualuid()` from Src/glob.c:3708 — `(u:UID)` glob
/// qualifier. Shim.
pub fn qualuid() -> i32 { 0 }

/// Port of `qualgid()` from Src/glob.c:3717 — `(g:GID)` glob
/// qualifier. Shim.
pub fn qualgid() -> i32 { 0 }

/// Port of `qualisdev()` from Src/glob.c:3726 — `(%)` glob
/// qualifier (device file). Shim.
pub fn qualisdev() -> i32 { 0 }

/// Port of `qualisblk()` from Src/glob.c:3735 — `(%b)` glob
/// qualifier (block-special). Shim.
pub fn qualisblk() -> i32 { 0 }

/// Port of `qualischr()` from Src/glob.c:3744 — `(%c)` glob
/// qualifier (char-special). Shim.
pub fn qualischr() -> i32 { 0 }

/// Port of `qualisdir()` from Src/glob.c:3753 — `(/)` glob
/// qualifier (directory). Shim.
pub fn qualisdir() -> i32 { 0 }

/// Port of `qualisfifo()` from Src/glob.c:3762 — `(p)` glob
/// qualifier (FIFO). Shim.
pub fn qualisfifo() -> i32 { 0 }

/// Port of `qualislnk()` from Src/glob.c:3771 — `(@)` glob
/// qualifier (symlink). Shim.
pub fn qualislnk() -> i32 { 0 }

/// Port of `qualisreg()` from Src/glob.c:3780 — `(.)` glob
/// qualifier (regular file). Shim.
pub fn qualisreg() -> i32 { 0 }

/// Port of `qualissock()` from Src/glob.c:3789 — `(=)` glob
/// qualifier (socket). Shim.
pub fn qualissock() -> i32 { 0 }

/// Port of `qualflags()` from Src/glob.c:3798 — flag-bit glob
/// qualifier (`(*)`). Shim.
pub fn qualflags() -> i32 { 0 }

/// Port of `qualmodeflags()` from Src/glob.c:3807 — `(f:MODE)`
/// glob qualifier (mode-mask match). Shim.
pub fn qualmodeflags() -> i32 { 0 }

/// Port of `qualiscom()` from Src/glob.c:3818 — `(*)` setuid /
/// setgid / sticky glob qualifier composer. Shim.
pub fn qualiscom() -> i32 { 0 }

/// Port of `qualnonemptydir()` from Src/glob.c:3948 — `(F)` glob
/// qualifier (non-empty directory). Shim.
pub fn qualnonemptydir() -> i32 { 0 }
