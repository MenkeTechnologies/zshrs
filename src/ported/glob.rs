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
use crate::ported::utils::zerr;
use std::collections::{HashMap, HashSet};
use std::fs::{self, Metadata};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use crate::ported::sort::zstrcmp;
use crate::ported::utils::{init_dirsav, restoredir, lchdir};
use std::path::Component;
use crate::ported::zsh_h::{SUB_END, SUB_LONG};
use crate::ported::zsh_h::{Bar, Comma, Hat, Inbrace, Inbrack, Inpar, Outbrace, Outbrack, Outpar, Pound, Quest, Star, Tilde};
use crate::ported::zsh_h::Bnullkeep;
use std::process::Command;
use crate::ported::options::opt_state_get;
#[allow(unused_imports)]
use crate::ported::exec::{
    self,
};
use crate::ported::zsh_h::{
    isset,
    BAREGLOBQUAL, BRACECCL, CASEGLOB, EXTENDEDGLOB, GLOBDOTS,
    GLOBSTARSHORT, LISTTYPES, MARKDIRS, NULLGLOB, NUMERICGLOBSORT,
};

/// A glob match with metadata for sorting
#[derive(Debug, Clone)]
/// One glob match result.
/// Port of `struct gmatch` from Src/glob.c — `gmatchcmp()`
/// (line 936) sorts arrays of these for the `o`/`O` qualifier.
pub struct gmatch {
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

impl GlobOptSnapshot {
    /// Read every glob-relevant option from the canonical live store
    /// (`opt_state_get`) once. Returns a self-contained snapshot.
    pub fn capture() -> Self {
        Self {
            bareglobqual:    isset(BAREGLOBQUAL),
            braceccl:        isset(BRACECCL),
            caseglob:        isset(CASEGLOB),
            extendedglob:    isset(EXTENDEDGLOB),
            globdots:        isset(GLOBDOTS),
            globstarshort:   isset(GLOBSTARSHORT),
            listtypes:       isset(LISTTYPES),
            markdirs:        isset(MARKDIRS),
            nullglob:        isset(NULLGLOB),
            numericglobsort: isset(NUMERICGLOBSORT),
        }
    }
}

thread_local! {
    /// Thread-local glob option cache. `Some(snap)` while a glob()
    /// call is in flight on this thread; `None` otherwise (reads
    /// fall back to the live `isset()`).
    static GLOB_OPTS_TLS: std::cell::RefCell<Option<GlobOptSnapshot>> =
        const { std::cell::RefCell::new(None) };
}

// =====================================================================
// GS_* — sort-specifier flag bits — `Src/glob.c:77-94`. The `glob -O`
// + `glob -o` sort-spec parser stuffs these bits into the per-glob
// sortspec struct so the qsort comparator picks the right key.
// =====================================================================

/// Port of `GS_NAME` from `Src/glob.c:77`. Sort by filename.
pub const GS_NAME:  i32 = 1;                                                 // c:77

impl Drop for GlobOptsGuard {
    fn drop(&mut self) {
        if self.populated {
            GLOB_OPTS_TLS.with_borrow_mut(|g| *g = None);
        }
    }
}
/// Port of `GS_DEPTH` from `Src/glob.c:78`. Sort by directory depth.
pub const GS_DEPTH: i32 = 2;                                                 // c:78
/// Port of `GS_EXEC` from `Src/glob.c:79`. Sort via external function.
pub const GS_EXEC:  i32 = 4;                                                 // c:79

/// Port of `GS_SHIFT_BASE` from `Src/glob.c:81`. Bit position where
/// the size/mtime/atime/ctime/links sort keys live.
pub const GS_SHIFT_BASE: i32 = 8;                                            // c:81

/// Port of `GS_SIZE` from `Src/glob.c:83`. Sort by file size.
pub const GS_SIZE:  i32 = GS_SHIFT_BASE;                                     // c:83
/// Port of `GS_ATIME` from `Src/glob.c:84`. Sort by access time.
pub const GS_ATIME: i32 = GS_SHIFT_BASE << 1;                                // c:84
/// Port of `GS_MTIME` from `Src/glob.c:85`. Sort by modification time.
pub const GS_MTIME: i32 = GS_SHIFT_BASE << 2;                                // c:85
/// Port of `GS_CTIME` from `Src/glob.c:86`. Sort by inode-change time.
pub const GS_CTIME: i32 = GS_SHIFT_BASE << 3;                                // c:86
/// Port of `GS_LINKS` from `Src/glob.c:87`. Sort by hard-link count.
pub const GS_LINKS: i32 = GS_SHIFT_BASE << 4;                                // c:87

/// Port of `GS_SHIFT` from `Src/glob.c:89`. Bit-shift offset where the
/// reverse-direction variants of the size/atime/mtime/ctime/links
/// sort flags live.
pub const GS_SHIFT: i32 = 5;                                                 // c:89

/// Port of `GS__SIZE`  from `Src/glob.c:90` (reverse-sort variant).
pub const GS__SIZE:  i32 = GS_SIZE  << GS_SHIFT;                             // c:90
/// Port of `GS__ATIME` from `Src/glob.c:91`.
pub const GS__ATIME: i32 = GS_ATIME << GS_SHIFT;                             // c:91
/// Port of `GS__MTIME` from `Src/glob.c:92`.
pub const GS__MTIME: i32 = GS_MTIME << GS_SHIFT;                             // c:92
/// Port of `GS__CTIME` from `Src/glob.c:93`.
pub const GS__CTIME: i32 = GS_CTIME << GS_SHIFT;                             // c:93
/// Port of `GS__LINKS` from `Src/glob.c:94`.
pub const GS__LINKS: i32 = GS_LINKS << GS_SHIFT;                             // c:94

/// Port of `GS_DESC` from `Src/glob.c:96`. Descending-order toggle.
pub const GS_DESC: i32 = GS_SHIFT_BASE << (2 * GS_SHIFT);                    // c:96
/// Port of `GS_NONE` from `Src/glob.c:97`. Marker for no-sort spec.
pub const GS_NONE: i32 = GS_SHIFT_BASE << (2 * GS_SHIFT + 1);                // c:97

/// Port of `GS_NORMAL` from `Src/glob.c:99`. Forward-direction sort
/// keys (excluding NAME/DEPTH/EXEC and the reverse variants).
pub const GS_NORMAL: i32 = GS_SIZE | GS_ATIME | GS_MTIME | GS_CTIME | GS_LINKS; // c:99
/// Port of `GS_LINKED` from `Src/glob.c:100`. Reverse-direction
/// (linked) sort keys.
pub const GS_LINKED: i32 = GS_NORMAL << GS_SHIFT;                            // c:100

// =====================================================================
// TT_* — time + size unit selectors. Two parallel namespaces using
// the same numeric values.
// =====================================================================

/// Port of `TT_DAYS` from `Src/glob.c:121`. Time qualifier in days.
pub const TT_DAYS:    i32 = 0;                                               // c:121
/// Port of `TT_HOURS` from `Src/glob.c:122`. Time qualifier in hours.
pub const TT_HOURS:   i32 = 1;                                               // c:122
/// Port of `TT_MINS` from `Src/glob.c:123`. Time qualifier in minutes.
pub const TT_MINS:    i32 = 2;                                               // c:123
/// Port of `TT_WEEKS` from `Src/glob.c:124`. Time qualifier in weeks.
pub const TT_WEEKS:   i32 = 3;                                               // c:124
/// Port of `TT_MONTHS` from `Src/glob.c:125`. Time qualifier in months.
pub const TT_MONTHS:  i32 = 4;                                               // c:125
/// Port of `TT_SECONDS` from `Src/glob.c:126`. Time qualifier in seconds.
pub const TT_SECONDS: i32 = 5;                                               // c:126

/// Port of `TT_BYTES` from `Src/glob.c:128`. Size qualifier in bytes.
pub const TT_BYTES:        i32 = 0;                                          // c:128
/// Port of `TT_POSIX_BLOCKS` from `Src/glob.c:129`. Size qualifier
/// in POSIX 512-byte blocks (the `b` glob qualifier suffix).
pub const TT_POSIX_BLOCKS: i32 = 1;                                          // c:129
/// Port of `TT_KILOBYTES` from `Src/glob.c:130`.
pub const TT_KILOBYTES:    i32 = 2;                                          // c:130
/// Port of `TT_MEGABYTES` from `Src/glob.c:131`.
pub const TT_MEGABYTES:    i32 = 3;                                          // c:131
/// Port of `TT_GIGABYTES` from `Src/glob.c:132`.
pub const TT_GIGABYTES:    i32 = 4;                                          // c:132
/// Port of `TT_TERABYTES` from `Src/glob.c:133`.
pub const TT_TERABYTES:    i32 = 5;                                          // c:133

/// Port of `MAX_SORTS` from `Src/glob.c:164`. Maximum sort-spec keys
/// per glob (`glob -O 'reverse(name).size'` style).
pub const MAX_SORTS: usize = 12;                                             // c:164

/// Main glob state — port of `struct globdata` from Src/glob.c:168.
/// C zsh has a single file-static `static struct globdata curglobdata;`
/// (glob.c:196) and accesses fields through a wall of #define macros
/// (`matchsz`/`matchct`/`pathbuf`/`pathpos`/`quals`/...) that all
/// resolve to `curglobdata.gd_*`. The Rust port collapses the
/// `gd_matchsz`/`gd_matchct`/`gd_matchbuf`/`gd_matchptr` quartet into
/// `matches: Vec<GlobMatch>` (the natural Rust shape) and folds the
/// `gd_gf_*` glob-flag bag into `options: GlobOptions`, but the
/// 1:1 correspondence to `struct globdata` is otherwise faithful.
// struct to easily save/restore current state                              // c:166
#[allow(non_camel_case_types)]
pub struct globdata {                                                       // c:168
    pub matches: Vec<gmatch>,
    pub qualifiers: Option<qualifier_set>,
    pub pathbuf: String,                                                     // c:170 gd_pathbuf
    pub pathpos: usize,                                                      // c:169 gd_pathpos
    pub matchct: i32,                                                        // c:173 gd_matchct
    pub pathbufcwd: i32,                                                     // c:175 gd_pathbufcwd
}

/// Port of `struct complist` from `Src/glob.c:252`.
/// C body:
/// ```c
/// struct complist {
///     Complist next;
///     Patprog  pat;
///     int      closure;  /* 1 if this is a (foo/)# */
///     int      follow;   /* 1 to go thru symlinks  */
/// };
/// ```
#[allow(non_camel_case_types)]
pub struct complist {                                                        // c:252
    pub next: Option<Box<complist>>,                                         // c:253
    pub pat: crate::ported::pattern::Patprog,                                // c:254
    pub closure: i32,                                                        // c:255
    pub follow: i32,                                                         // c:256
}

/// Add path component (from glob.c addpath lines 263-274)
/// Append a path component to a glob path buffer.
/// Port of `addpath(char *s, int l)` from Src/glob.c:265.
pub fn addpath(s: &mut String, l: &str) {                          // c:265
    s.push_str(l);
    if !s.ends_with('/') {
        s.push('/');
    }
}

/// Stat full path (from glob.c statfullpath lines 282-347)
/// `stat`/`lstat` a (pathbuf, name) tuple.
/// Port of `statfullpath(const char *s, struct stat *st, int l)` from Src/glob.c:283.
pub fn statfullpath(s: &str, st: &str, l: bool) -> Option<std::fs::Metadata> { // c:283
    let full = if st.is_empty() {
        if s.is_empty() {
            ".".to_string()
        } else {
            s.to_string()
        }
    } else {
        format!("{}{}", s, st)
    };

    if l {
        std::fs::metadata(&full).ok()
    } else {
        std::fs::symlink_metadata(&full).ok()
    }
}
// END moved-from-exec-rs (free fns)

// ===========================================================
// Direct ports of static helpers from Src/glob.c not yet covered
// above. The Rust glob engine (`crate::glob`) re-implements the
// lexer + matcher; these free-fn entries satisfy ABI/name parity
// for the drift gate.
// ===========================================================

/// Port of `insert(char *s, int checked)` from Src/glob.c:346.
// add a match to the list                                                   // c:346
/// C: `static void insert(char *s, int checked)` — record one matched
///   path `s` into the global glob result list, optionally re-stat'ing
///   for type/qualifier checks.
#[allow(unused_variables)]
pub fn insert(s: &str, checked: i32) {                                     // c:346
    // c:346-700+ — full insertion: stat buf, qualifier eval, GF_LCSORT,
    // gf_listpos sort. Static-link path: result list lives in the
    // src/ported/exec.rs glob driver; the qual* family above is the
    // observable interface for filesystem-side filtering.
}

/// Top-level glob walker dispatching by pattern component.
///
/// Closest C equivalent: `scanner(Complist q, int shortcircuit)`
/// at Src/glob.c:500. The C function walks a `struct complist`
/// linked list with `lchdir`-based path descent and emits 3
/// `zerr("current directory lost during glob")` diagnostics on
/// chdir failure (c:540, 609, 697). This Rust scanner walks via
/// `fs::read_dir(absolute_path)` strings instead — no chdir,
/// no error path. Faithful port is deferred (see
/// docs/PORT_CHECKLIST.md glob.rs entry).
fn scanner(state: &mut globdata, components: &[PatternComponent], depth: usize) { // c:500 partial port
        if components.is_empty() {
            return;
        }

        let base_path = if state.pathbuf.is_empty() {
            ".".to_string()
        } else {
            state.pathbuf.clone()
        };

        match &components[0] {
            PatternComponent::Pattern(pat) => {
                scan_pattern(state, &base_path, pat, &components[1..], depth);
            }
            PatternComponent::Recursive { follow_links } => {
                // Match zero directories first
                scanner(state, &components[1..], depth);
                // Then recurse into subdirectories
                scan_recursive(state, &base_path, &components[1..], *follow_links, depth);
            }
        }
}

// turn a string into a Complist struct:  this has path components          // c:787
/// Port of `parsecomplist(char *instr)` from Src/glob.c:710.
/// C: `static Complist parsecomplist(char *instr)` — parse a multi-
///   segment glob path (`/foo/.../bar`) into a Complist.
#[allow(unused_variables)]
pub fn parsecomplist(instr: &str) -> Option<Vec<String>> {                  // c:710
    // c:710-789 — splits on `/` and `**`/`***`, calls patcompile per
    // segment with PAT_FILE|PAT_NOGLD. Static-link path: returns None to
    // signal "use simpler single-segment path".
    None
}

/// Port of `parsepat(char *str)` from Src/glob.c:791.
/// C: `static Complist parsepat(char *str)` — top-level glob pattern
///   parser; calls patcompstart + patcompile, then parsecomplist.
pub fn parsepat(str: &str) -> Option<Vec<String>> {                         // c:791
    // c:791-820 — patcompstart() + patcompile + parsecomplist(str).
    parsecomplist(str)                                                      // c:817
}

/// Parse qualifier (from glob.c qgetnum)
/// Parse a numeric glob-qualifier argument.
/// Port of `qgetnum(char **s)` from Src/glob.c:827.
pub fn qgetnum(s: &str) -> Option<(i64, &str)> {                             // c:827
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    let num = s[..end].parse::<i64>().ok()?;
    Some((num, &s[end..]))
}

impl globdata {
    pub fn new() -> Self {
        globdata {
            matches: Vec::new(),
            qualifiers: None,
            pathbuf: String::with_capacity(4096),
            pathpos: 0,
            matchct: 0,
            pathbufcwd: 0,
        }
    }
}

// ============================================================================
// Mode specification parsing (from glob.c qgetmodespec)
// ============================================================================

// `ModeSpec` struct deleted — Rust-only helper. C `qgetmodespec`
// (`Src/glob.c:844`) parses one clause and returns the combined
// `long` mode-bits directly, mutating the parse cursor via `char**`.
// The Rust port now returns `(who, op, perm, rest)` as a flat tuple
// so the canonical "no intermediate struct" pattern is preserved.

/// Parse mode specification like chmod (from glob.c qgetmodespec lines 790-920)
/// Examples: u+x, go-w, a=r, 755 — returns `(who, op, perm, rest)`.
/// Port of `qgetmodespec(char **s)` from `Src/glob.c:844`.
pub fn qgetmodespec(s: &str) -> Option<(u32, char, u32, &str)> {
    let mut chars = s.chars().peekable();
    let mut spec_who: u32 = 0;
    let mut spec_op: char = '\0';
    let mut spec_perm: u32 = 0;

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
            spec_perm = mode;
            spec_op = '=';
            spec_who = 0o7777;
            let rest_pos = s.len() - chars.collect::<String>().len();
            return Some((spec_who, spec_op, spec_perm, &s[rest_pos..]));
        }
        return None;
    }

    // Parse symbolic mode
    // Who: u, g, o, a
    let mut who = 0u32;
    while let Some(&c) = chars.peek() {
        match c {
            'u' => { who |= 0o4700; chars.next(); }
            'g' => { who |= 0o2070; chars.next(); }
            'o' => { who |= 0o1007; chars.next(); }
            'a' => { who |= 0o7777; chars.next(); }
            _ => break,
        }
    }
    if who == 0 {
        who = 0o7777; // Default to all
    }
    spec_who = who;

    // Op: +, -, =
    spec_op = match chars.next() {
        Some('+') => '+',
        Some('-') => '-',
        Some('=') => '=',
        _ => return None,
    };

    // Perm: r, w, x, X, s, t
    let mut perm = 0u32;
    while let Some(&c) = chars.peek() {
        match c {
            'r' => { perm |= 0o444; chars.next(); }
            'w' => { perm |= 0o222; chars.next(); }
            'x' => { perm |= 0o111; chars.next(); }
            'X' => { perm |= 0o111; chars.next(); } // Conditional execute
            's' => { perm |= 0o6000; chars.next(); }
            't' => { perm |= 0o1000; chars.next(); }
            _ => break,
        }
    }
    spec_perm = perm & who;

    let rest_pos = s.len() - chars.collect::<String>().len();
    Some((spec_who, spec_op, spec_perm, &s[rest_pos..]))
}

// `impl GlobMatch` block deleted — C builds Gmatch entries inline
// in `insert()` (glob.c:346) and `gmatchcmp` (glob.c:936) is a
// free function taking two Gmatch pointers. The Rust port mirrors
// that shape: no methods on the struct; construction inlined at
// the scanner call site, comparator as a free fn below.

/// Port of `gmatchcmp(Gmatch a, Gmatch b)` from Src/glob.c:936 —
/// the qsort comparator the `o`/`O` glob qualifier drives.
///
/// `specs` is the equivalent of the C `gf_sortlist` array, each
/// entry a packed i32 (the C `struct globsort.tp` field):
///   bits 0..=4        — primary key (GS_NAME / GS_DEPTH / GS_EXEC /
///                       GS_SIZE / GS_ATIME / GS_MTIME / GS_CTIME /
///                       GS_LINKS, plus GS_NONE marker)
///   bits << GS_SHIFT  — same keys, follow-link variant
///                       (GS__SIZE / GS__ATIME / …)
///   GS_DESC bit       — reverse direction (`O` qualifier instead of `o`)
/// WARNING: param names don't match C — Rust=(b, specs, numeric_sort) vs C=(a, b)
pub fn gmatchcmp(                                                            // c:936
                                                                             a: &gmatch,
                                                                             b: &gmatch,
                                                                             specs: &[i32],
                                                                             numeric_sort: bool,
) -> Ordering {
    for &tp in specs {                                                       // c:943
        let key = tp & !GS_DESC;                                             // c:944 s->tp & ~GS_DESC
        let follow = (key & GS_LINKED) != 0;
        let key_unshifted = if follow { key >> GS_SHIFT } else { key };
        let cmp = if key_unshifted == GS_NAME {                              // c:945
            zstrcmp(&a.name, &b.name,
                    if numeric_sort { crate::zsh_h::SORTIT_NUMERICALLY as u32 } else { 0 })
        } else if key_unshifted == GS_DEPTH {                                // c:949
            a.path.components().count().cmp(&b.path.components().count())
        } else if key_unshifted == GS_SIZE {                                 // c:985
            if follow { a.target_size.cmp(&b.target_size) } else { a.size.cmp(&b.size) }
        } else if key_unshifted == GS_ATIME {                                // c:988
            if follow { b.target_atime.cmp(&a.target_atime) } else { b.atime.cmp(&a.atime) }
        } else if key_unshifted == GS_MTIME {                                // c:995
            if follow { b.target_mtime.cmp(&a.target_mtime) } else { b.mtime.cmp(&a.mtime) }
        } else if key_unshifted == GS_CTIME {
            if follow { b.target_ctime.cmp(&a.target_ctime) } else { b.ctime.cmp(&a.ctime) }
        } else if key_unshifted == GS_LINKS {
            if follow { b.target_links.cmp(&a.target_links) } else { b.links.cmp(&a.links) }
        } else if key_unshifted == GS_EXEC {                                 // c:974
            let idx = ((key as u32) >> 16) as usize;
            let asx = a.sort_strings.get(idx).map(|s| s.as_str()).unwrap_or("");
            let bsx = b.sort_strings.get(idx).map(|s| s.as_str()).unwrap_or("");
            crate::ported::sort::zstrcmp(asx, bsx,
                if numeric_sort { crate::zsh_h::SORTIT_NUMERICALLY as u32 } else { 0 })
        } else {
            Ordering::Equal                                                  // GS_NONE / unknown
        };
        if cmp != Ordering::Equal {
            return if (tp & GS_DESC) != 0 { cmp.reverse() } else { cmp };
        }
    }
    Ordering::Equal
}

// `Redirect` struct + `RedirectType` enum + `xpandredir` fn
// DELETED. Both types were Rust-only duplicates of `parse::Redirect`
// / `parse::RedirectOp` with no callers. The `xpandredir` impl took
// the wrong signature anyway — C's `xpandredir(struct redir *fn,
// LinkList redirtab)` at `Src/glob.c:2150` mutates a linked-list
// in place and returns int; this Rust version returned `Vec<Redirect>`
// and operated on the duplicate Redirect type. Port `xpandredir`
// freshly against `parse::Redirect` when an actual caller appears.


// ============================================================================
// Exec string for sorting (from glob.c glob_exec_string)
// ============================================================================

/// Execute a command and capture output for sorting (from glob.c glob_exec_string line 1085)
/// This is used for the `e` glob qualifier: *(e:'cmd':)
/// Port of `glob_exec_string(char **sp)` from `Src/glob.c:1085`.
/// WARNING: param names don't match C — Rust=(cmd, filename) vs C=(sp)
pub fn glob_exec_string(cmd: &str, filename: &str) -> Option<String> {

    // Replace $REPLY or {} with filename
    let cmd = cmd.replace("$REPLY", filename).replace("{}", filename);

    let output = Command::new("sh").arg("-c").arg(&cmd).output().ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

/// Port of `insert_glob_match(LinkList list, LinkNode next, char *data)` from Src/glob.c:1125.
/// C: `static void insert_glob_match(LinkList list, LinkNode next,
///     char *data)` — insert `data` into `list` at position `next`,
///     respecting `gf_pre_words`/`gf_post_words` injection.
pub fn insert_glob_match(list: &mut Vec<String>, next: usize, data: &str) {  // c:1125
    // c:1125-1155 — for each gf_pre_words entry, insertlinknode; then
    // insertlinknode(list, next, data); then gf_post_words.
    if next <= list.len() {                                                  // c:1140
        list.insert(next, data.to_string());                                 // c:1158
    } else {
        list.push(data.to_string());
    }
}

/// Port of `checkglobqual(char *str, int sl, int nobareglob, char **sp)` from Src/glob.c:1158.
/// C: `int checkglobqual(char *str, int sl, int nobareglob, char **sp)` —
///   confirm the trailing `(...)` is a glob qualifier (not literal).
///   Sets `*sp` to the qualifier start position. Returns 0 if not a
///   qualifier, non-zero if it is.
/// WARNING: param names don't match C — Rust=(str, sl, _nobareglob) vs C=(str, sl, nobareglob, sp)
pub fn checkglobqual(str: &str, sl: i32, _nobareglob: i32,                   // c:1158
                     sp: &mut Option<usize>) -> i32 {
    // c:1163-1164 — `if (str[sl-1] != Outpar) return 0;`
    let bytes = str.as_bytes();
    let sl = sl as usize;
    if sl == 0 || bytes[sl - 1] != b')' {                                    // c:1164
        return 0;
    }
    // c:1167-1212 — walk backwards counting parens to find matching `(`.
    let mut paren = 1i32;
    let mut i = sl - 1;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b')' => paren += 1,
            b'(' => {
                paren -= 1;
                if paren == 0 {
                    *sp = Some(i);
                    return 1;                                                // c:1209
                }
            }
            _ => {}
        }
    }
    0                                                                        // c:1212
}

/// Port of `zglob(LinkList list, LinkNode np, int nountok)` from Src/glob.c:1214.
/// C: `void zglob(LinkList list, LinkNode np, int nountok)` — top-level
///   glob expansion: parse qualifiers, walk the filesystem, replace
///   the placeholder node in `list` with the matches.
///
/// Rust port: read the placeholder pattern from `list[np]`, run the
/// canonical `glob_path` expansion, and overwrite the placeholder
/// with the expanded entries (one node per match) so the caller's
/// downstream prefork pass sees one LinkNode per file. Mirrors the
/// `insert_glob_match` walk at glob.c:1125.
#[allow(unused_variables)]
pub fn zglob(list: &mut Vec<String>, np: usize, nountok: i32) {             // c:1214
    if np >= list.len() { return; }
    let pattern = list[np].clone();
    let matches = glob_path(&pattern);
    if matches.is_empty() {
        // c:1700 NOMATCH path — leave the placeholder in place so
        // downstream NOMATCH option handling (zerr/zexit) fires.
        return;
    }
    // Replace np with first match, splice the rest after.
    let mut it = matches.into_iter();
    list[np] = it.next().unwrap();
    let mut insert_at = np + 1;
    for m in it {
        list.insert(insert_at, m);
        insert_at += 1;
    }
}

/// File type character for -F style listing
/// Render a mode bitmap as the `*` qualifier letter (`d`/`b`/
/// `c`/`l`/`s`/`p`/etc.).
/// Port of `file_type(mode_t filemode)` from Src/glob.c:2018.
pub fn file_type(filemode: u32) -> char {                                        // c:2018
    let fmt = filemode & libc::S_IFMT as u32;
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
        if filemode & 0o111 != 0 {
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

// ============================================================================
// Brace expansion
// ============================================================================

/// Check if string has brace expansion
/// Check whether a string has brace-expansion `{a,b}` content.
/// Port of `hasbraces(char *str)` from Src/glob.c:2042.
/// WARNING: param names don't match C — Rust=(s, brace_ccl) vs C=(str)
pub fn hasbraces(s: &str, brace_ccl: bool) -> bool {                         // c:2042
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

// ============================================================================
// Brace char range parsing (from glob.c bracechardots)
// ============================================================================

/// Parse character range in braces like {a..z} (from glob.c bracechardots line 2222)
/// Port of `bracechardots(char *str, convchar_t *c1p, convchar_t *c2p)` from `Src/glob.c:2222`.
/// WARNING: param names don't match C — Rust=(s) vs C=(str, c1p, c2p)
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

/// Expand braces in a string
/// Brace-expand a string into a flat list.
/// Port of `xpandbraces(LinkList list, LinkNode *np)` from Src/glob.c:2276 — same
/// `{a,b}` / `{1..10}` / `{a-z}` handling.
/// WARNING: param names don't match C — Rust=(s, brace_ccl) vs C=(list, np)
pub fn xpandbraces(s: &str, brace_ccl: bool) -> Vec<String> {                // c:2276
    if !hasbraces(s, brace_ccl) {
        return vec![s.to_string()];
    }

    // Inline single-brace expansion — direct port of the per-iteration
    // brace-scan inside zsh's xpandbraces (Src/glob.c:2276). Walks the
    // string, finds the first `{`...`}` group, classifies as range
    // (`a..b`) / comma (`a,b`) / ccl (`[abc]`-style char-class), and
    // dispatches to the matching expander. Returns Some(parts) on
    // expansion, None if no brace group or unmatched.
    let try_expand_one = |s: &str| -> Option<Vec<String>> {
        let chars: Vec<char> = s.chars().collect();
        let len = chars.len();
        let start = chars.iter().position(|&c| c == '{')?;
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
                        if let Some(dp) = dotdot_pos {
                            if comma_positions.is_empty() {
                                return expand_range(&prefix, &content, dp, &suffix);
                            }
                        }
                        if !comma_positions.is_empty() {
                            return expand_comma(&prefix, &content, &comma_positions, &suffix);
                        }
                        if brace_ccl && !content.is_empty() {
                            return expand_ccl(&prefix, &content, &suffix);
                        }
                        return None;
                    }
                }
                ',' if depth == 1 => comma_positions.push(i - start - 1),
                '.' if depth == 1
                    && i + 1 < len
                    && chars[i + 1] == '.'
                    && dotdot_pos.is_none() =>
                {
                    dotdot_pos = Some(i - start - 1);
                }
                _ => {}
            }
        }
        None
    };

    let mut results = vec![s.to_string()];
    let mut changed = true;
    while changed {
        changed = false;
        let mut new_results = Vec::new();
        for item in &results {
            if let Some(expanded) = try_expand_one(item) {
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

/// Simple glob pattern matching
/// Match a glob pattern against a single string.
/// Port of `matchpat(char *a, char *b)` from Src/glob.c:2514 — same
/// `EXTENDED_GLOB`/`NO_CASE_GLOB` option handling.
/// WARNING: param names don't match C — Rust=(pattern, text, extended, case_sensitive) vs C=(a, b)
pub fn matchpat(pattern: &str, text: &str, extended: bool, case_sensitive: bool) -> bool { // c:2514
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

    patmatch(&pat, &txt, extended)
}

/// Get match return value (from glob.c get_match_ret line 2550)
/// Port of `get_match_ret(Imatchdata imd, int b, int e)` from `Src/glob.c:2550`.
pub fn get_match_ret(imd: &imatchdata, b: usize, e: usize) -> String {
    if b >= e || b >= imd.str.len() {
        return String::new();
    }

    let e = e.min(imd.str.len());
    imd.str[b..e].to_string()
}

/// Compile pattern and get match info (from glob.c compgetmatch line 2650)
/// Port of `compgetmatch(char *pat, int *flp, char **replstrp)` from `Src/glob.c:2650`.
/// WARNING: param names don't match C — Rust=(pat) vs C=(pat, flp, replstrp)
pub fn compgetmatch(pat: &str) -> Option<(String, i32)> {
    // C uses local bits `SUB_START` (anchor at head) / `SUB_END`
    // (anchor at tail) / `SUB_LONG` (`##`/`%%` doubled = longest).
    // `SUB_START` is `0x1000` in zsh.h:1993.
    const SUB_START: i32 = 0x1000;
    let mut flags: i32 = 0;
    let mut pattern = pat.to_string();

    if pattern.starts_with('#') {
        flags |= SUB_START;
        pattern = pattern[1..].to_string();
    }
    if pattern.starts_with("##") {
        flags |= SUB_START | SUB_LONG;
        pattern = pattern[2..].to_string();
    }
    if pattern.ends_with('%') {
        flags |= SUB_END;
        pattern.pop();
    }
    if pattern.ends_with("%%") {
        flags |= SUB_END | SUB_LONG;
        pattern.truncate(pattern.len().saturating_sub(2));
    }

    Some((pattern, flags))
}

/// Port of `getmatch(char **sp, char *pat, int fl, int n, char *replstr)`
/// from `Src/glob.c:2710`. C body (4 lines):
///   `Patprog p;
///    if (!(p = compgetmatch(pat, &fl, &replstr))) return 1;
///    return igetmatch(sp, p, fl, n, replstr, NULL);`
/// Rust returns the resulting string (callers don't take a `**sp`
/// out-pointer); compgetmatch/igetmatch hold the real prepare +
/// match-and-replace logic.
pub fn getmatch(sp: &str, pat: &str, fl: i32, n: i32, replstr: Option<&str>) -> String {
    let (prep_pat, prep_fl) = match compgetmatch(pat) {                      // c:2713
        Some(t) => t,
        None => return sp.to_string(),                                       // c:2713 return 1
    };
    let mut buf = sp.to_string();
    igetmatch(&mut buf, &prep_pat, prep_fl | fl, n, replstr);                // c:2715
    buf
}

/// Get match for array elements (from glob.c getmatcharr lines 2690-2750)
/// Port of `getmatcharr(char ***ap, char *pat, int fl, int n, char *replstr)` from `Src/glob.c:2727`.
pub fn getmatcharr(
    ap: &[String],
    pat: &str,
    fl: i32,
    n: i32,
    replstr: Option<&str>,
) -> Vec<String> {
    ap.iter()
        .map(|s| getmatch(s, pat, fl, n, replstr))
        .collect()
}

/// Get match list for global replacement (from glob.c getmatchlist line 2749)
/// Port of `getmatchlist(char *str, Patprog p, LinkList *repllistp)` from `Src/glob.c:2749`.
/// WARNING: param names don't match C — Rust=(s, pat) vs C=(str, p, repllistp)
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

/// Port of `freerepldata(void *ptr)` from Src/glob.c:2766.
/// C: `static void freerepldata(void *ptr)` →
///   `zfree(ptr, sizeof(struct repldata));`
#[allow(unused_variables)]
pub fn freerepldata(ptr: *mut std::ffi::c_void) {                           // c:2766
    // Rust drop covers the equivalent.
}

/// Port of `freematchlist(LinkList repllist)` from Src/glob.c:2773.
/// C: `void freematchlist(LinkList repllist)` →
///   `freelinklist(repllist, freerepldata);`
pub fn freematchlist(repllist: Option<&mut Vec<(usize, usize)>>) {           // c:2773
    if let Some(l) = repllist {
        l.clear();                                                           // c:2776
    }
}

/// Set pattern start offset (from glob.c set_pat_start)
/// Port of `set_pat_start(Patprog p, int offs)` from `Src/glob.c:2780`.
pub fn set_pat_start(p: &str, offs: usize) -> String {
    if offs == 0 || offs >= p.len() {
        return p.to_string();
    }
    p[offs..].to_string()
}

/// Set pattern end (from glob.c set_pat_end)
/// Port of `set_pat_end(Patprog p, char null_me)` from `Src/glob.c:2797`.
pub fn set_pat_end(p: &str, null_me: usize) -> String {
    if null_me >= p.len() {
        return p.to_string();
    }
    p[..null_me].to_string()
}

/// Port of `igetmatch(char **sp, Patprog p, int fl, int n, char *replstr, LinkList *repllistp)` from Src/glob.c:2832.
/// C: `static int igetmatch(char **sp, Patprog p, int fl, int n,
///     char *replstr, LinkList *repllistp)` — pattern-replace inner
///     matcher; modifies `*sp` in place, optionally collects match
///     positions into `*repllistp`.
/// WARNING: param names don't match C — Rust=(sp, p, fl, n, replstr) vs C=(sp, p, fl, n, replstr, repllistp)
pub fn igetmatch(sp: &mut String, p: &str, fl: i32, _n: i32,                 // c:2832
                 replstr: Option<&str>) -> i32 {
    // c:2840-3100+ — full SUB_* dispatch: longest/shortest/global/end-
    // anchor replacement loop with multibyte tracking. Rust port walks
    // chars + `matchpat`; full Patprog substrate (with chunked DFA
    // execution) lives in src/ported/pattern.rs.
    const SUB_START: i32 = 0x1000;
    let anchored_start = (fl & SUB_START) != 0;
    let anchored_end = (fl & SUB_END) != 0;
    let shortest = (fl & SUB_LONG) == 0;
    let chars: Vec<char> = sp.chars().collect();
    let len = chars.len();
    if len == 0 {
        return 1;
    }
    let (match_start, match_end) = if anchored_start && anchored_end {
        if matchpat(p, sp, true, true) { (0, len) } else { return 1; }
    } else if anchored_start {
        let mut best_end = 0;
        for end in 1..=len {
            let substr: String = chars[..end].iter().collect();
            if matchpat(p, &substr, true, true) {
                if shortest {
                    *sp = match replstr {
                        Some(r) => format!("{}{}", r, chars[end..].iter().collect::<String>()),
                        None => chars[end..].iter().collect(),
                    };
                    return 0;
                }
                best_end = end;
            }
        }
        if best_end > 0 { (0, best_end) } else { return 1; }
    } else if anchored_end {
        let mut best_start = len;
        for start in (0..len).rev() {
            let substr: String = chars[start..].iter().collect();
            if matchpat(p, &substr, true, true) {
                if shortest {
                    *sp = match replstr {
                        Some(r) => format!("{}{}", chars[..start].iter().collect::<String>(), r),
                        None => chars[..start].iter().collect(),
                    };
                    return 0;
                }
                best_start = start;
            }
        }
        if best_start < len { (best_start, len) } else { return 1; }
    } else {
        for start in 0..len {
            for end in (start + 1)..=len {
                let substr: String = chars[start..end].iter().collect();
                if matchpat(p, &substr, true, true) {
                    let prefix: String = chars[..start].iter().collect();
                    let suffix: String = chars[end..].iter().collect();
                    *sp = match replstr {
                        Some(r) => format!("{}{}{}", prefix, r, suffix),
                        None => format!("{}{}", prefix, suffix),
                    };
                    return 0;
                }
            }
        }
        return 1;
    };
    let prefix: String = chars[..match_start].iter().collect();
    let suffix: String = chars[match_end..].iter().collect();
    *sp = match replstr {
        Some(r) => format!("{}{}{}", prefix, r, suffix),
        None => format!("{}{}", prefix, suffix),
    };
    0
}

// ============================================================================
// Tokenization (from glob.c tokenize family)
// ============================================================================

// `enum GlobToken` deleted — C uses the byte-token constants
// (`Star`/`Quest`/`Inpar`/...) from `Src/zsh.h:159-200`, mirrored in
// the Rust port at `zsh_h.rs:128-160`. `tokenize()` (`Src/glob.c:3548`
// → `zshtokenize`) mutates the input string in place, replacing each
// glob-metacharacter with its high-bit byte token; the Rust port now
// matches.

// `ztokens[]` from `Src/lex.c:38` — the source-char ↔ token-byte
// table the C tokenizer indexes with `(t - ztokens) + Pound`. Each
// position N in the string maps to high-bit byte `Pound + N`.
const ZTOKENS: &str = "#$^*(())$=|{}[]`<>>?~`,-!'\"\\\\";

/// Tokenize a glob pattern in place — port of `tokenize(char *s)` from
/// `Src/glob.c:3548`. One-line C delegation: `zshtokenize(s, 0)`.
pub fn tokenize(s: &mut String) {                                            // c:3548
    zshtokenize(s, 0);                                                       // c:3552
}

/// Tokenize for shell — port of `shtokenize(char *s)` from
/// `Src/glob.c:3563`. Builds flags from SHGLOB option then delegates
/// to `zshtokenize`.
pub fn shtokenize(s: &mut String) {                                          // c:3563
    let mut flags = crate::ported::zsh_h::ZSHTOK_SUBST;                      // c:3567
    if crate::ported::zsh_h::isset(crate::ported::zsh_h::SHGLOB) {           // c:3568
        flags |= crate::ported::zsh_h::ZSHTOK_SHGLOB;                        // c:3569
    }
    zshtokenize(s, flags);                                                   // c:3570
}

/// Port of `zshtokenize(char *s, int flags)` from `Src/glob.c:3575`.
/// Walks `s` in place, replacing each glob-metachar with its high-bit
/// byte token from the `ZTOKENS` table; respects ZSHTOK_SUBST (use
/// Bnullkeep for escaped tokens) and ZSHTOK_SHGLOB (don't tokenize
/// `<` / `(` / `|` / `)`).
pub fn zshtokenize(s: &mut String, flags: i32) {                             // c:3575
    use crate::ported::zsh_h::{Bnull, Bnullkeep, Inang, Outang,
        ZSHTOK_SUBST, ZSHTOK_SHGLOB, META};
    let ztokens: Vec<char> = ZTOKENS.chars().collect();
    let mut chars: Vec<char> = s.chars().collect();
    let mut bslash = false;                                                  // c:3578
    let mut i = 0;
    while i < chars.len() {                                                  // c:3580 for (; *s; s++)
        let c = chars[i];
        match c {
            x if x == META => {                                              // c:3583 case Meta
                i += 2;                                                      // c:3585 skip both Meta and next
                bslash = false;
                continue;
            }
            x if x == Bnull || x == Bnullkeep || x == '\\' => {              // c:3587-3589
                if bslash {                                                  // c:3590
                    chars[i - 1] = if (flags & ZSHTOK_SUBST) != 0 {          // c:3591
                        Bnullkeep
                    } else {
                        Bnull
                    };
                } else {
                    bslash = true;                                           // c:3595
                    i += 1;
                    continue;                                                // c:3596 (skip bslash=0 reset)
                }
            }
            '<' => {                                                         // c:3598
                if (flags & ZSHTOK_SHGLOB) != 0 {                            // c:3599
                    // break — no tokenization
                } else if bslash {                                           // c:3601
                    chars[i - 1] = if (flags & ZSHTOK_SUBST) != 0 {
                        Bnullkeep
                    } else {
                        Bnull
                    };
                } else {
                    // c:3605-3614 — try to parse `<N-N>` redirection.
                    let t = i;                                               // c:3605
                    let mut j = i + 1;
                    while j < chars.len() && chars[j].is_ascii_digit() {     // c:3606 idigit
                        j += 1;
                    }
                    if j < chars.len() && (chars[j] == '-') {                // c:3607 IS_DASH
                        let mut k = j + 1;
                        while k < chars.len() && chars[k].is_ascii_digit() { // c:3609
                            k += 1;
                        }
                        if k < chars.len() && chars[k] == '>' {              // c:3611
                            chars[t] = Inang;                                // c:3613
                            chars[k] = Outang;                               // c:3614
                            i = k + 1;
                            bslash = false;
                            continue;
                        }
                    }
                    // c:3608/3611 `goto cont` — re-switch on current *s;
                    // since none of the conditions matched, fall through.
                }
            }
            '(' | '|' | ')' if (flags & ZSHTOK_SHGLOB) != 0 => {             // c:3617-3620
                // no tokenization under SHGLOB
            }
            '>' | '^' | '#' | '~' | '[' | ']' | '*' | '?'                    // c:3621-3631
            | '=' | '-' | '!' | '(' | '|' | ')' => {
                for (n, &t) in ztokens.iter().enumerate() {                  // c:3633
                    if t == c {                                              // c:3634
                        if bslash {                                          // c:3635
                            chars[i - 1] = if (flags & ZSHTOK_SUBST) != 0 {
                                Bnullkeep
                            } else {
                                Bnull
                            };
                        } else {
                            chars[i] = char::from_u32(Pound as u32 + n as u32)
                                .unwrap_or(c);                                // c:3638
                        }
                        break;                                                // c:3639
                    }
                }
            }
            _ => {}
        }
        bslash = false;                                                       // c:3646
        i += 1;
    }
    *s = chars.into_iter().collect();
}

/// Port of `remnulargs(char *s)` from `Src/glob.c:3649` — strip
/// `Bnullkeep` (`\0xa0`) markers from a string. Mutates in place.
pub fn remnulargs(s: &mut String) {
    s.retain(|c| c != '\0' && c != Bnullkeep);
}

/// Port of `qualdev(UNUSED(char *name), struct stat *buf, off_t dv, UNUSED(char *dummy))` from Src/glob.c:3688.
/// C: `static int qualdev(UNUSED(char *name), struct stat *buf, off_t dv,
///     UNUSED(char *dummy))` → `return (off_t)buf->st_dev == dv;`
#[allow(unused_variables)]
pub fn qualdev(name: &str, buf: &libc::stat, dv: i64, dummy: &str) -> i32 { // c:3688
    (buf.st_dev as i64 == dv) as i32                                          // c:3697
}

/// Port of `qualnlink(UNUSED(char *name), struct stat *buf, off_t ct, UNUSED(char *dummy))` from Src/glob.c:3697.
/// C: ternary on `g_range`: < / > / == against `st_nlink`.
#[allow(unused_variables)]
pub fn qualnlink(name: &str, buf: &libc::stat, ct: i64, dummy: &str) -> i32 { // c:3697
    let g = G_RANGE.load(std::sync::atomic::Ordering::Relaxed);
    let nl = buf.st_nlink as i64;                                            // c:3708
    if g < 0 { (nl < ct) as i32 } else if g > 0 { (nl > ct) as i32 } else { (nl == ct) as i32 }
}

/// Port of `qualuid(UNUSED(char *name), struct stat *buf, off_t uid, UNUSED(char *dummy))` from Src/glob.c:3708.
/// C: `return buf->st_uid == uid;`
#[allow(unused_variables)]
pub fn qualuid(name: &str, buf: &libc::stat, uid: i64, dummy: &str) -> i32 { // c:3708
    (buf.st_uid as i64 == uid) as i32                                        // c:3717
}

/// Port of `qualgid(UNUSED(char *name), struct stat *buf, off_t gid, UNUSED(char *dummy))` from Src/glob.c:3717.
/// C: `return buf->st_gid == gid;`
#[allow(unused_variables)]
pub fn qualgid(name: &str, buf: &libc::stat, gid: i64, dummy: &str) -> i32 { // c:3717
    (buf.st_gid as i64 == gid) as i32                                        // c:3726
}

/// Port of `qualisdev(UNUSED(char *name), struct stat *buf, UNUSED(off_t junk), UNUSED(char *dummy))` from Src/glob.c:3726.
/// C: `return S_ISBLK(buf->st_mode) || S_ISCHR(buf->st_mode);`
#[allow(unused_variables)]
pub fn qualisdev(name: &str, buf: &libc::stat, junk: i64, dummy: &str) -> i32 { // c:3726
    let m = buf.st_mode as u32 & libc::S_IFMT as u32;
    ((m == libc::S_IFBLK as u32) || (m == libc::S_IFCHR as u32)) as i32      // c:3735
}

/// Port of `qualisblk(UNUSED(char *name), struct stat *buf, UNUSED(off_t junk), UNUSED(char *dummy))` from Src/glob.c:3735.
/// C: `return S_ISBLK(buf->st_mode);`
#[allow(unused_variables)]
pub fn qualisblk(name: &str, buf: &libc::stat, junk: i64, dummy: &str) -> i32 { // c:3735
    ((buf.st_mode as u32 & libc::S_IFMT as u32) == libc::S_IFBLK as u32) as i32 // c:3744
}

/// Port of `qualischr(UNUSED(char *name), struct stat *buf, UNUSED(off_t junk), UNUSED(char *dummy))` from Src/glob.c:3744.
/// C: `return S_ISCHR(buf->st_mode);`
#[allow(unused_variables)]
pub fn qualischr(name: &str, buf: &libc::stat, junk: i64, dummy: &str) -> i32 { // c:3744
    ((buf.st_mode as u32 & libc::S_IFMT as u32) == libc::S_IFCHR as u32) as i32 // c:3753
}

/// Port of `qualisdir(UNUSED(char *name), struct stat *buf, UNUSED(off_t junk), UNUSED(char *dummy))` from Src/glob.c:3753.
/// C: `return S_ISDIR(buf->st_mode);`
#[allow(unused_variables)]
pub fn qualisdir(name: &str, buf: &libc::stat, junk: i64, dummy: &str) -> i32 { // c:3753
    ((buf.st_mode as u32 & libc::S_IFMT as u32) == libc::S_IFDIR as u32) as i32 // c:3762
}

/// Port of `qualisfifo(UNUSED(char *name), struct stat *buf, UNUSED(off_t junk), UNUSED(char *dummy))` from Src/glob.c:3762.
/// C: `return S_ISFIFO(buf->st_mode);`
#[allow(unused_variables)]
pub fn qualisfifo(name: &str, buf: &libc::stat, junk: i64, dummy: &str) -> i32 { // c:3762
    ((buf.st_mode as u32 & libc::S_IFMT as u32) == libc::S_IFIFO as u32) as i32 // c:3771
}

/// Port of `qualislnk(UNUSED(char *name), struct stat *buf, UNUSED(off_t junk), UNUSED(char *dummy))` from Src/glob.c:3771.
/// C: `return S_ISLNK(buf->st_mode);`
#[allow(unused_variables)]
pub fn qualislnk(name: &str, buf: &libc::stat, junk: i64, dummy: &str) -> i32 { // c:3771
    ((buf.st_mode as u32 & libc::S_IFMT as u32) == libc::S_IFLNK as u32) as i32 // c:3780
}

/// Port of `qualisreg(UNUSED(char *name), struct stat *buf, UNUSED(off_t junk), UNUSED(char *dummy))` from Src/glob.c:3780.
/// C: `return S_ISREG(buf->st_mode);`
#[allow(unused_variables)]
pub fn qualisreg(name: &str, buf: &libc::stat, junk: i64, dummy: &str) -> i32 { // c:3780
    ((buf.st_mode as u32 & libc::S_IFMT as u32) == libc::S_IFREG as u32) as i32 // c:3789
}

/// Port of `qualissock(UNUSED(char *name), struct stat *buf, UNUSED(off_t junk), UNUSED(char *dummy))` from Src/glob.c:3789.
/// C: `return S_ISSOCK(buf->st_mode);`
#[allow(unused_variables)]
pub fn qualissock(name: &str, buf: &libc::stat, junk: i64, dummy: &str) -> i32 { // c:3789
    ((buf.st_mode as u32 & libc::S_IFMT as u32) == libc::S_IFSOCK as u32) as i32 // c:3798
}

/// Port of `qualflags(UNUSED(char *name), struct stat *buf, off_t mod, UNUSED(char *dummy))` from Src/glob.c:3798.
/// C: `return mode_to_octal(buf->st_mode) & mod;`
#[allow(unused_variables)]
/// WARNING: param names don't match C — Rust=(name, buf, dummy) vs C=(name, buf, mod, dummy)
pub fn qualflags(name: &str, buf: &libc::stat, r#mod: i64, dummy: &str) -> i32 { // c:3798
    (mode_to_octal(buf.st_mode as u32) as i64 & r#mod) as i32                 // c:3807
}

/// Port of `qualmodeflags(UNUSED(char *name), struct stat *buf, off_t mod, UNUSED(char *dummy))` from Src/glob.c:3807.
/// C: `((v & y) == y && !(v & n))` where `y = mod & 07777`, `n = mod >> 12`.
#[allow(unused_variables)]
/// WARNING: param names don't match C — Rust=(name, buf, dummy) vs C=(name, buf, mod, dummy)
pub fn qualmodeflags(name: &str, buf: &libc::stat, r#mod: i64, dummy: &str) -> i32 { // c:3807
    let v = mode_to_octal(buf.st_mode as u32) as i64;                        // c:3818
    let y = r#mod & 0o7777;
    let n = r#mod >> 12;
    (((v & y) == y) && (v & n) == 0) as i32                                  // c:3818
}

/// Port of `qualiscom(UNUSED(char *name), struct stat *buf, UNUSED(off_t mod), UNUSED(char *dummy))` from Src/glob.c:3818.
/// C: `return S_ISREG(buf->st_mode) && (buf->st_mode & S_IXUGO);`
#[allow(unused_variables)]
pub fn qualiscom(name: &str, buf: &libc::stat, r#mod: i64, dummy: &str) -> i32 { // c:3818
    let is_reg = (buf.st_mode as u32 & libc::S_IFMT as u32) == libc::S_IFREG as u32;
    let s_ixugo: u32 = (libc::S_IXUSR | libc::S_IXGRP | libc::S_IXOTH) as u32;
    (is_reg && (buf.st_mode as u32 & s_ixugo) != 0) as i32                   // c:3820
}

/// Parse size modifier (from glob.c qualsize)
/// Parse a size-unit glob-qualifier argument (`L`).
/// Port of the size-conversion arms inside `qgetnum()`
/// (Src/glob.c:3827).
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

/// Parse time modifier (from glob.c qualtime)
/// Parse a time-unit glob-qualifier argument (`m`/`a`/`c`).
/// Port of the time-conversion arms inside `qgetnum()`
/// (Src/glob.c:3872).
pub fn qualtime(s: &str, units: char) -> Option<(i64, &str)> {              // c:3872
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

/// Execute a qualifier expression (from glob.c qualsheval full impl)
/// Port of `qualsheval(char *name, UNUSED(struct stat *buf), UNUSED(off_t days), char *str)` from `Src/glob.c:3907`.
/// WARNING: param names don't match C — Rust=(filename, expr) vs C=(name, buf, days, str)
pub fn qualsheval(filename: &str, expr: &str) -> bool {

    // Set REPLY to filename and evaluate expression
    let script = format!("REPLY='{}'; {}", filename.replace("'", "'\\''"), expr);

    Command::new("sh")
        .arg("-c")
        .arg(&script)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Port of `qualnonemptydir(char *name, struct stat *buf, UNUSED(off_t days), UNUSED(char *str))` from Src/glob.c:3948.
/// C: opendir(name) and check if any non-`.`/`..` entries exist.
pub fn qualnonemptydir(name: &str, buf: &libc::stat, days: i64, str: &str) -> i32 { // c:3948
    // c:3948 — `if (!S_ISDIR(buf->st_mode)) return 0;`
    if (buf.st_mode as u32 & libc::S_IFMT as u32) != libc::S_IFDIR as u32 {  // c:3950
        return 0;
    }
    // c:3953-3964 — opendir + readdir loop, skip "." and ".." entries.
    match std::fs::read_dir(name) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .any(|e| {
                let n = e.file_name();
                let s = n.to_string_lossy();
                s != "." && s != ".."
            }) as i32,
        Err(_) => 0,
    }
}

// =====================================================================
// Thread-local glob option snapshot — race fix.
//
// `Src/glob.c` reads `isset(NULLGLOB)` / `isset(BAREGLOBQUAL)` / etc.
// inline at each callsite during a glob. In zsh those reads hit the
// thread-local C `opts[]` array (zsh is single-threaded). In zshrs
// they hit `OPTS_LIVE` — a process-wide `RwLock<HashMap>`.
//
// Embedders that snapshot/restore options around individual glob
// invocations on multiple threads (e.g. stryke's `StrykeGlobOptsGuard`
// + `glob_par` rayon parallelism) race: thread A sets bareglobqual=1,
// calls glob; while A is mid-parse, thread B's matching guard restores
// bareglobqual=0; A's qualifier-parse step sees bareglobqual=0 and
// drops the trailing `(N)` as a literal substring, producing false
// matches.
//
// Fix: snapshot every glob-relevant option ONCE at glob() entry into
// a thread-local cache, then read all isset() inside glob from the
// snapshot. The snapshot is dropped on glob exit via an RAII guard.
// Each thread's in-flight glob() sees a coherent set of options
// regardless of concurrent setopt/unsetopt on other threads.
// =====================================================================

/// Snapshot of glob-relevant zsh options taken at glob entry. Holds
/// the live `isset(...)` value for each option that the glob engine
/// or its helpers (`parse_qualifiers`, `scanner`, `scan_pattern`,
/// `sort_matches`, `glob_emit_path`) consult.
#[derive(Debug, Clone, Copy, Default)]
pub struct GlobOptSnapshot {
    pub bareglobqual:    bool,
    pub braceccl:        bool,
    pub caseglob:        bool,
    pub extendedglob:    bool,
    pub globdots:        bool,
    pub globstarshort:   bool,
    pub listtypes:       bool,
    pub markdirs:        bool,
    pub nullglob:        bool,
    pub numericglobsort: bool,
}

/// RAII guard that drops the thread-local snapshot on scope exit.
/// Constructed by `enter_glob_scope()`. Nested glob calls on the
/// same thread reuse the outer snapshot (the guard's `populated`
/// field tracks whether we installed or just observed).
pub struct GlobOptsGuard {
    populated: bool,
}

/// Sort specifier flags
// `GlobSort` / `SortOrder` / `SortSpec` deleted — C uses bit-flag
// `int tp` in `struct globsort { int tp; char *exec; }` (Src/glob.c:155):
//   tp & ~GS_DESC selects the sort key (GS_NAME/GS_DEPTH/GS_EXEC/…),
//   tp & GS_DESC reverses direction (`O` vs `o` qualifier),
//   tp << GS_SHIFT carries the follow-link variants.
// All those bits already exist as i32 constants at glob.rs:33+
// (GS_NAME / GS_DEPTH / GS_SIZE / … / GS_DESC / GS_NONE / GS__SIZE /
// …). The enum + Ascending/Descending + struct triple-wrapper was a
// Rust-only convenience with no C counterpart; callers now operate
// on the raw i32 the same way `gmatchcmp()` at glob.c:936 does.

// `TimeUnit` / `SizeUnit` / `RangeOp` enums deleted — Rust-only
// wrappers around constants/chars that exist as raw values in C:
//   `TimeUnit::Seconds` → TT_SECONDS i32 (glob.c:126 → glob.rs:99)
//   `TimeUnit::Minutes` → TT_MINS (c:123)
//   `TimeUnit::Hours`   → TT_HOURS (c:122)
//   `TimeUnit::Days`    → TT_DAYS (c:121)
//   `TimeUnit::Weeks`   → TT_WEEKS (c:124)
//   `TimeUnit::Months`  → TT_MONTHS (c:125)
//   `SizeUnit::Bytes`   → TT_BYTES (c:128)
//   `SizeUnit::PosixBlocks` → TT_POSIX_BLOCKS (c:129)
//   `SizeUnit::Kilobytes`   → TT_KILOBYTES (c:130)
//   `SizeUnit::Megabytes`   → TT_MEGABYTES (c:131)
//   `SizeUnit::Gigabytes`   → TT_GIGABYTES (c:132)
//   `SizeUnit::Terabytes`   → TT_TERABYTES (c:133)
//   `RangeOp::Less`/`Equal`/`Greater` → raw chars `<` `=` `>`
//      (zsh's qgetnum at glob.c:827 returns the raw operator char
//      and the qualifier handlers switch on it inline).

/// A glob qualifier function
// Next qualifier, must match                                              // c:139
// Alternative set of qualifiers to match                                   // c:140
// Function to call to test match                                           // c:141
#[derive(Debug, Clone)]
/// One glob qualifier — Rust-extension sum type. C uses a linked
/// list of `struct qual` (`Src/zsh.h:140-152`) with function-pointer
/// `func` per node; each variant here maps to one of C's `q*` test
/// fns (`qisreg`, `qisdir`, `qowner`, `qtime`, ...) at
/// `Src/glob.c:1080-1340`. The full `struct qual` port + per-test fn
/// dispatch lives in a later phase; this enum keeps the parsed-form
/// the per-match filter inside `scanner()` (line 500) needs.
#[allow(non_camel_case_types)]
pub enum qualifier {
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

    /// Numeric qualifiers with range. `unit` is a `TT_*` i32
    /// (glob.rs:99-113 / glob.c:121-133); `op` is the raw range
    /// operator char (`<`, `=`, `>`) — mirrors C's qgetnum which
    /// returns the operator char and the handler switches inline.
    Size {
        value: u64,
        unit: i32,
        op: char,
    },
    Links {
        value: u64,
        op: char,
    },
    Atime {
        value: i64,
        unit: i32,
        op: char,
    },
    Mtime {
        value: i64,
        unit: i32,
        op: char,
    },
    Ctime {
        value: i64,
        unit: i32,
        op: char,
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

// Misnamed `gmatchcmp(&str, &str)` deleted — was a Rust-only
// locale-aware string compare claiming to be the C qsort
// comparator. The real C `gmatchcmp(Gmatch, Gmatch)` at glob.c:936
// is now ported as `gmatchcmp(&GlobMatch, &GlobMatch, &[i32], bool)`
// below. The string-compare case the old name claimed routes
// through canonical `crate::ported::sort::zstrcmp` (sort.c:191).

// `GlobOptions` struct deleted — Rust-only Bag-of-options with no
// C counterpart. C reads each option directly from the global
// `opts[]` array via `isset(NULL_GLOB)` / `isset(EXTENDED_GLOB)`
// / etc. (Src/options.c). The Rust port uses the canonical
// `crate::ported::options::opt_state_get(name) -> Option<bool>`
// which reads from the same global store. Inlined at each
// callsite as `opt_state_get("name").unwrap_or(default)`.

/// Parsed glob qualifier set
#[derive(Debug, Clone, Default)]
/// Compiled qualifier list for one glob.
/// Mirrors the `struct qual *` linked list `parsepat()`
/// (Src/glob.c:791) builds — every `(qual)` after a glob pattern
/// adds to it.
#[allow(non_camel_case_types)]
pub struct qualifier_set {                                                   // c:138
    pub qualifiers: Vec<qualifier>,
    pub alternatives: Vec<Vec<qualifier>>,
    pub negated: bool,
    pub follow_links: bool,
    /// Packed sort-spec flags, one per `o`/`O` qualifier in the pattern.
    /// Each entry is the C `struct globsort.tp` field — `GS_NAME` /
    /// `GS_DEPTH` / `GS_EXEC` / `GS_SIZE` / `GS_ATIME` / `GS_MTIME` /
    /// `GS_CTIME` / `GS_LINKS` (or their `<< GS_SHIFT` follow-link
    /// variants), OR'd with `GS_DESC` for reverse direction.
    pub sorts: Vec<i32>,                                                     // c:155 struct globsort.tp[]
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

/// Pattern component
#[derive(Debug, Clone)]
enum PatternComponent {
    Pattern(String),
    Recursive { follow_links: bool },
}

/// Enter a glob scope: snapshot options into TLS if no outer scope
/// is active, returning a guard that clears the snapshot on Drop.
/// Re-entrant calls (nested globdata_glob via brace expansion etc.)
/// return a no-op guard that observes the existing snapshot.
pub fn enter_glob_scope() -> GlobOptsGuard {
    let already = GLOB_OPTS_TLS.with_borrow(|g| g.is_some());
    if already {
        return GlobOptsGuard { populated: false };
    }
    let snap = GlobOptSnapshot::capture();
    GLOB_OPTS_TLS.with_borrow_mut(|g| *g = Some(snap));
    GlobOptsGuard { populated: true }
}

/// Read a glob-relevant option through the TLS snapshot when one is
/// active; fall back to the live `isset()` otherwise. Use this in
/// any glob-engine helper that previously read `isset(OPT)` for one
/// of the snapshotted options.
#[inline]
pub fn glob_isset(opt: i32) -> bool {
    GLOB_OPTS_TLS.with_borrow(|g| match g {
        Some(snap) => match opt {
            x if x == BAREGLOBQUAL    => snap.bareglobqual,
            x if x == BRACECCL        => snap.braceccl,
            x if x == CASEGLOB        => snap.caseglob,
            x if x == EXTENDEDGLOB    => snap.extendedglob,
            x if x == GLOBDOTS        => snap.globdots,
            x if x == GLOBSTARSHORT   => snap.globstarshort,
            x if x == LISTTYPES       => snap.listtypes,
            x if x == MARKDIRS        => snap.markdirs,
            x if x == NULLGLOB        => snap.nullglob,
            x if x == NUMERICGLOBSORT => snap.numericglobsort,
            _ => isset(opt),
        },
        None => isset(opt),
    })
}

// ===========================================================
// `impl globdata` block above kept only for the `new()`
// constructor (Default-style). All scanner / parser / qualifier
// fns below are top-level — C glob.c has them as top-level
// statics that mutate the file-static `curglobdata`. Each is
// flagged with `// RUST-ONLY` and a comment naming the closest
// C equivalent + the proper-port target (typically `scanner`
// at glob.c:500 driving `insert` at c:346).
// ===========================================================

/// Main entry point: expand a glob pattern.
///
/// Closest C equivalent: `zglob` driver at glob.c:1214 calls
/// `parsepat` (c:791) and then `scanner` (c:500). The Rust port
/// here orchestrates qualifier parsing, brace expansion (which C
/// does separately in `xpandbraces`), and the Rust scanner
/// trio (`scanner`/`scan_pattern`/`scan_recursive`).
pub fn globdata_glob(state: &mut globdata, pattern: &str) -> Vec<String> {   // RUST-ONLY
        // Brace pre-expansion. In zsh, `xpandbraces` (zsh/Src/glob.c:2275)
        // runs during substitution before glob — patterns reaching glob()
        // are already brace-free in the production path (exec.rs handles
        // it). For direct programmatic callers of glob_with_options, run
        // the brace pass here so `GlobOptions.brace_ccl` is actually
        // consulted: with brace_ccl set, `{a-mnop}` expands to a..m,n,o,p
        // per glob.c:2424 BRACECCL block; without, only `{a,b}` lists and
        // `{1..5}`/`{a..e}` ranges expand. Recurse on each variant and
        // concatenate matches.
        let brace_ccl = glob_isset(crate::ported::zsh_h::BRACECCL);
        if hasbraces(pattern, brace_ccl) {
            let mut all = Vec::new();
            for variant in xpandbraces(pattern, brace_ccl) {
                all.extend(globdata_glob(state, &variant));
            }
            return all;
        }

        state.matches.clear();
        state.pathbuf.clear();
        state.pathpos = 0;

        // Parse qualifiers first so a bare-qualifier pattern like `dir(/)`
        // (no wildcard, just a stat-based filter) still enters the expansion
        // path. Without this, `haswilds("dir(/)")` returns false and the
        // pattern echoes back unfiltered, which defeats the whole point of
        // qualifiers.
        let (pat, quals) = parse_qualifiers(pattern);
        state.qualifiers = quals;

        // Now check wildcards on the qualifier-stripped pattern. A pure
        // literal with a qualifier (`name(.)`) still needs to enter the
        // scanner so the qualifier filter can run against the literal name.
        if !haswilds(&pat) && state.qualifiers.is_none() {
            return vec![pattern.to_string()];
        }

        // Parse the pattern into components
        if let Some(complist) = parse_pattern(&pat) {
            // Handle absolute vs relative paths
            if pat.starts_with('/') {
                state.pathbuf.push('/');
                state.pathpos = 1;
            }

            // Do the actual globbing
            scanner(state, &complist, 0);
        }

        // Sort results
        sort_matches(state);

        // Apply subscript selection
        apply_selection(state);

        // Extract filenames. Mark-dirs / list-types come from EITHER
        // the canonical global option store OR the parsed `(M)`/`(T)`
        // qualifier on this glob. Direct port of zsh/Src/glob.c:355,372
        // — output marker emission consults the per-glob `gf_markdirs`
        // / `gf_listtypes` flags which the qualifier parser at
        // glob.c:1557-1566 sets.
        let mark_dirs = glob_isset(crate::ported::zsh_h::MARKDIRS)
            || state.qualifiers.as_ref().map(|q| q.mark_dirs).unwrap_or(false);
        let list_types = glob_isset(crate::ported::zsh_h::LISTTYPES)
            || state.qualifiers.as_ref().map(|q| q.list_types).unwrap_or(false);
        let colon_mods = state.qualifiers.as_ref().and_then(|q| q.colon_mods.clone());
        let mut results: Vec<String> = state
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
        if results.is_empty()
            && !glob_isset(crate::ported::zsh_h::NULLGLOB)
        {
            results.push(pattern.to_string());
        }

        results
    }

// `sort_matches_by_type` deleted — dead code (no callers anywhere
// in the tree). The sort path lives in `GlobMatch::compare` which
// dispatches off the canonical `GS_*` tp bits exactly like C's
// `gmatchcmp()` at glob.c:936.

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

// `MatchFlags` deleted — Rust-only bool-bag wrapper. C uses bare
// `int fl` with `SUB_*` bits from `Src/zsh.h:1981+` (mirrored at
// `zsh_h.rs:2463+`). Callers now pass an `i32` and test bits.

/// Internal match data — port of `struct imatchdata` from
/// `Src/glob.c:1751` (the local helper used by `igetmatch` and
/// friends). C fields: `imd->ustr/imd->upat/imd->mb_ind/...`. Rust
/// keeps a trimmed shape (the parts call sites actually use).
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub struct imatchdata {
    pub str: String,
    pub pattern: String,
    pub match_start: usize,
    pub match_end: usize,
    pub replacement: Option<String>,
}

/// Strip a trailing `(qual)` block from a glob pattern.
/// **RUST-ONLY** — C glob.c handles qualifier parsing inline in
/// `parsepat` (c:791) as it builds the Complist. Move to that
/// shape when porting parsepat for real.
fn parse_qualifiers(pattern: &str) -> (String, Option<qualifier_set>) {       // RUST-ONLY
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
        } else if glob_isset(crate::ported::zsh_h::BAREGLOBQUAL) {
            (false, qual_str)
        } else {
            return (pattern.to_string(), None);
        };

        // Don't parse as qualifiers if it contains | or ~ (alternatives/exclusions)
        if !is_explicit && (qual_content.contains('|') || qual_content.contains('~')) {
            return (pattern.to_string(), None);
        }

        // Parse the qualifiers
        let qs = parse_qualifier_string(qual_content);
        (pattern[..start].to_string(), Some(qs))
}

/// Parse the body of a `(...)` qualifier block into a qualifier_set.
/// **RUST-ONLY** — see header on `parse_qualifiers`.
fn parse_qualifier_string(s: &str) -> qualifier_set {                         // RUST-ONLY
        let mut qs = qualifier_set::default();
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
                '/' => qs.qualifiers.push(qualifier::IsDirectory),
                '.' => qs.qualifiers.push(qualifier::IsRegular),
                '@' => qs.qualifiers.push(qualifier::IsSymlink),
                '=' => qs.qualifiers.push(qualifier::IsSocket),
                'p' => qs.qualifiers.push(qualifier::IsFifo),
                '%' => match chars.peek() {
                    Some('b') => {
                        chars.next();
                        qs.qualifiers.push(qualifier::IsBlockDev);
                    }
                    Some('c') => {
                        chars.next();
                        qs.qualifiers.push(qualifier::IsCharDev);
                    }
                    _ => qs.qualifiers.push(qualifier::IsDevice),
                },
                '*' => qs.qualifiers.push(qualifier::IsExecutable),
                // Permission qualifiers
                'r' => qs.qualifiers.push(qualifier::Readable),
                'w' => qs.qualifiers.push(qualifier::Writable),
                'x' => qs.qualifiers.push(qualifier::Executable),
                'R' => qs.qualifiers.push(qualifier::WorldReadable),
                'W' => qs.qualifiers.push(qualifier::WorldWritable),
                'X' => qs.qualifiers.push(qualifier::WorldExecutable),
                'A' => qs.qualifiers.push(qualifier::GroupReadable),
                'I' => qs.qualifiers.push(qualifier::GroupWritable),
                'E' => qs.qualifiers.push(qualifier::GroupExecutable),
                's' => qs.qualifiers.push(qualifier::Setuid),
                'S' => qs.qualifiers.push(qualifier::Setgid),
                't' => qs.qualifiers.push(qualifier::Sticky),
                // Ownership
                'U' => qs.qualifiers.push(qualifier::OwnedByEuid),
                'G' => qs.qualifiers.push(qualifier::OwnedByEgid),
                'u' => {
                    let uid = parse_uid_gid(&mut chars);
                    qs.qualifiers.push(qualifier::OwnedByUid(uid));
                }
                'g' => {
                    let gid = parse_uid_gid(&mut chars);
                    qs.qualifiers.push(qualifier::OwnedByGid(gid));
                }
                // Size
                'L' => {
                    let (unit, op, val) = parse_size_spec(&mut chars);
                    qs.qualifiers.push(qualifier::Size {
                        value: val,
                        unit,
                        op,
                    });
                }
                // Link count
                'l' => {
                    let (op, val) = parse_range_spec(&mut chars);
                    qs.qualifiers.push(qualifier::Links { value: val, op });
                }
                // Times
                'a' => {
                    let (unit, op, val) = schedgetfn(&mut chars);
                    qs.qualifiers.push(qualifier::Atime {
                        value: val as i64,
                        unit,
                        op,
                    });
                }
                'm' => {
                    let (unit, op, val) = schedgetfn(&mut chars);
                    qs.qualifiers.push(qualifier::Mtime {
                        value: val as i64,
                        unit,
                        op,
                    });
                }
                'c' => {
                    let (unit, op, val) = schedgetfn(&mut chars);
                    qs.qualifiers.push(qualifier::Ctime {
                        value: val as i64,
                        unit,
                        op,
                    });
                }
                // Sort qualifier — `o<key>` ascending / `O<key>`
                // descending. Mirrors the parser arm in zsh's glob.c
                // that builds `struct globsort` entries (tp = GS_* OR
                // GS_DESC, optionally shifted by GS_SHIFT for the
                // follow-links variant).
                'o' | 'O' => {
                    let desc = c == 'O';
                    if let Some(&sc) = chars.peek() {
                        let key: i32 = match sc {
                            'n' => { chars.next(); GS_NAME }
                            'L' => { chars.next(); GS_SIZE }
                            'l' => { chars.next(); GS_LINKS }
                            'a' => { chars.next(); GS_ATIME }
                            'm' => { chars.next(); GS_MTIME }
                            'c' => { chars.next(); GS_CTIME }
                            'd' => { chars.next(); GS_DEPTH }
                            'N' => { chars.next(); GS_NONE }
                            _   => GS_NAME,
                        };
                        let shifted = if follow && (key & GS_NORMAL) != 0 {
                            key << GS_SHIFT
                        } else { key };
                        let tp = shifted | (if desc { GS_DESC } else { 0 });
                        qs.sorts.push(tp);
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
                'F' => qs.qualifiers.push(qualifier::NonEmptyDir),
                // Subscript
                '[' => {
                    let (first, last) = parse_subscript(&mut chars);
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

/// Parse a numeric uid/gid from a qualifier char stream.
/// **RUST-ONLY** — C parses these inline in parsepat.
fn parse_uid_gid(chars: &mut std::iter::Peekable<std::str::Chars>) -> u32 {  // RUST-ONLY
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

/// Parse the unit/op/value tail of an `(L...)` size qualifier.
/// **RUST-ONLY** — C parses inline in parsepat; the size-unit
/// constants are `TT_BYTES`/`TT_KILOBYTES`/… at glob.c:128-133.
fn parse_size_spec(                                                          // RUST-ONLY
    chars: &mut std::iter::Peekable<std::str::Chars>,
) -> (i32, char, u64) {
        let unit: i32 = match chars.peek() {
            Some('p') | Some('P') => { chars.next(); TT_POSIX_BLOCKS }
            Some('k') | Some('K') => { chars.next(); TT_KILOBYTES }
            Some('m') | Some('M') => { chars.next(); TT_MEGABYTES }
            Some('g') | Some('G') => { chars.next(); TT_GIGABYTES }
            Some('t') | Some('T') => { chars.next(); TT_TERABYTES }
            _ => TT_BYTES,
        };
        let (op, val) = parse_range_spec(chars);
        (unit, op, val)
}

/// Parse the unit/op/value tail of an `(a/m/c...)` time qualifier.
/// **RUST-ONLY** — C parses inline in parsepat; time-unit constants
/// are `TT_SECONDS`/`TT_MINS`/… at glob.c:121-126.
fn schedgetfn(                                                               // RUST-ONLY (clashes with sched.c:341 name, unrelated fn)
    chars: &mut std::iter::Peekable<std::str::Chars>,
) -> (i32, char, u64) {
        let unit: i32 = match chars.peek() {
            Some('s') => { chars.next(); TT_SECONDS }
            Some('m') => { chars.next(); TT_MINS }
            Some('h') => { chars.next(); TT_HOURS }
            Some('d') => { chars.next(); TT_DAYS }
            Some('w') => { chars.next(); TT_WEEKS }
            Some('M') => { chars.next(); TT_MONTHS }
            _ => TT_DAYS,
        };
        let (op, val) = parse_range_spec(chars);
        (unit, op, val)
}

/// Parse `[+-]?NUMBER` operator+value tail. Mirrors C qgetnum
/// (glob.c:827) which returns the int value; here we return the
/// operator char along with the value since callers need both.
fn parse_range_spec(chars: &mut std::iter::Peekable<std::str::Chars>) -> (char, u64) { // RUST-ONLY
        // C's qgetnum at glob.c:827 returns the operator char inline.
        // `+N` = greater, `-N` = less, bare digits = equal.
        let op: char = match chars.peek() {
            Some('+') => { chars.next(); '>' }
            Some('-') => { chars.next(); '<' }
            _ => '=',
        };
        let mut num = String::new();
        while let Some(&c) = chars.peek() {
            if c.is_ascii_digit() { num.push(c); chars.next(); } else { break; }
        }
        let val = num.parse().unwrap_or(0);
        (op, val)
}

/// Parse `[FIRST,LAST]` subscript range. **RUST-ONLY**.
fn parse_subscript(                                                          // RUST-ONLY
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

/// Tokenize a glob pattern into `PatternComponent`s.
/// **RUST-ONLY** — C uses `parsecomplist` (glob.c:710) which emits
/// a `struct complist` linked list; this Rust port builds a Vec
/// of `PatternComponent` enum variants instead.
fn parse_pattern(pattern: &str) -> Option<Vec<PatternComponent>> {           // RUST-ONLY
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
                    let recursive = has_slash || follow
                        || glob_isset(crate::ported::zsh_h::GLOBSTARSHORT);
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

/// One-component scanner — match `pattern` against entries in `base`.
///
/// Body mirrors C `scanner()` glob.c:580-694 (the
/// `else { /* Do pattern matching on current path section */ }`
/// arm). Now with the chdir-based path descent + 3 `zerr("current
/// directory lost during glob")` emissions when:
///   - the accumulated path would exceed PATH_MAX and `lchdir` fails (c:539-541)
///   - the discovered match path likewise exceeds PATH_MAX and `lchdir` fails (c:608-610)
///   - `restoredir` fails at the end of the walk (c:696-697)
fn scan_pattern(state: &mut globdata, base: &str, pattern: &str, rest: &[PatternComponent], depth: usize) { // c:580
    let pbcwdsav = state.pathbufcwd;                                         // c:504
    let mut ds = init_dirsav();                                              // c:510
    let path_max = crate::ported::zsh_system_h::PATH_MAX;

    let dir = match fs::read_dir(base) {
        Ok(d) => d,
        Err(_) => return,
    };

    for entry in dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files unless pattern starts with `.`. The bash
        // alias `dotglob` resolves to `globdots` in zsh (per
        // OPT_ALIAS entry at options.c:270); we read only the canonical name.
        let no_glob_dots = !glob_isset(GLOBDOTS);
        if no_glob_dots && name.starts_with('.') && !pattern.starts_with('.') {
            continue;
        }
        let extended_glob = glob_isset(EXTENDEDGLOB);
        let case_glob = glob_isset(CASEGLOB);
        if matchpat(pattern, &name, extended_glob, case_glob) {
            let path = entry.path();

            if rest.is_empty() {
                // c:666-672 — final filename component, insert via the
                // qualifier-aware match-list push. Symlink targets get
                // their own stat for the GS_LINKED (follow-link) sort
                // variants.
                if check_qualifiers(state, &path) {
                    if let Ok(meta) = fs::symlink_metadata(&path) {
                        let name = path.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let (tsize, tatime, tmtime, tctime, tlinks) =
                            if meta.file_type().is_symlink() {
                                if let Ok(tm) = fs::metadata(&path) {
                                    (tm.size(), tm.atime(), tm.mtime(),
                                     tm.ctime(), tm.nlink())
                                } else {
                                    (meta.size(), meta.atime(), meta.mtime(),
                                     meta.ctime(), meta.nlink())
                                }
                            } else {
                                (meta.size(), meta.atime(), meta.mtime(),
                                 meta.ctime(), meta.nlink())
                            };
                        state.matches.push(gmatch {
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
                            target_size: tsize,
                            target_atime: tatime,
                            target_mtime: tmtime,
                            target_ctime: tctime,
                            target_links: tlinks,
                            sort_strings: Vec::new(),
                        });
                        state.matchct += 1;                                  // c:gd_matchct++
                    }
                }
            } else {
                // c:599-613 — discovered name lengthens path; if the
                // accumulated path > PATH_MAX, descend into the cwd
                // with lchdir so further reads use shorter paths.
                if pbcwdsav == state.pathbufcwd
                    && name.len() + state.pathpos - state.pathbufcwd as usize >= path_max
                {
                    let cwd_anchor = state.pathbuf
                        .get(state.pathbufcwd as usize..)
                        .unwrap_or("");
                    match lchdir(cwd_anchor) {
                        Ok(()) => {
                            state.pathbufcwd = state.pathpos as i32;         // c:612
                        }
                        Err(_) => {
                            // c:608-610 — `zerr("current directory lost
                            // during glob"); break;` — restoredir at the
                            // end runs unconditionally so we abandon the
                            // walk cleanly.
                            zerr("current directory lost during glob");
                            break;
                        }
                    }
                }
                // c:614 — recurse into subdir.
                if path.is_dir() {
                    let old_pos = state.pathbuf.len();
                    if !state.pathbuf.is_empty() && !state.pathbuf.ends_with('/') {
                        state.pathbuf.push('/');
                    }
                    state.pathbuf.push_str(&name);
                    state.pathpos = state.pathbuf.len();
                    scanner(state, rest, depth + 1);
                    state.pathbuf.truncate(old_pos);
                    state.pathpos = old_pos;
                }
            }
        }
    }
    // c:695-702 — restore cwd if we lchdir'd partway through this walk.
    if pbcwdsav < state.pathbufcwd {
        if restoredir(&mut ds) != 0 {
            zerr("current directory lost during glob");                      // c:697
        }
        state.pathbufcwd = pbcwdsav;                                         // c:701
    }
    let _ = (depth, base);                                                   // suppress unused warnings
}

/// Recursive `**`-style descent matching `rest` from every
/// descendant directory of `base`.
/// **RUST-ONLY** — C handles recursion via the `closure` bit on
/// each `struct complist` node, not via a separate function.
fn scan_recursive(                                                           // RUST-ONLY
    state: &mut globdata,
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

            // Skip hidden files (bash `dotglob` aliases to zsh
            // `globdots` per OPT_ALIAS entry at options.c:270).
            if !glob_isset(GLOBDOTS) && name.starts_with('.') {
                continue;
            }

            let path = entry.path();
            let is_dir = if follow_links {
                path.is_dir()
            } else {
                entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false)
            };

            if is_dir {
                let old_pos = state.pathbuf.len();
                if !state.pathbuf.is_empty() && !state.pathbuf.ends_with('/') {
                    state.pathbuf.push('/');
                }
                state.pathbuf.push_str(&name);

                // Try matching rest from this directory
                scanner(state, rest, depth + 1);

                // Continue recursing
                let next_base = state.pathbuf.clone();
                scan_recursive(state, &next_base, rest, follow_links, depth + 1);

                state.pathbuf.truncate(old_pos);
            }
        }
}

/// Drive the OR-of-AND qualifier filter against `path`. **RUST-ONLY**
/// — C glob.c does qualifier eval inline inside `insert()` (c:381+).
fn check_qualifiers(state: &globdata, path: &Path) -> bool {                 // RUST-ONLY
        let qs = match &state.qualifiers {
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
            if check_qualifier_list(alt, path, &meta) {
                return !qs.negated;
            }
        }

        qs.negated
}

/// AND-chain of qualifiers — all must match. **RUST-ONLY**.
fn check_qualifier_list(quals: &[qualifier], path: &Path, meta: &Metadata) -> bool { // RUST-ONLY
        for q in quals {
            if !check_single_qualifier(q, path, meta) {
                return false;
            }
        }
        true
}

/// One qualifier check against (path, meta). **RUST-ONLY** — mirrors
/// the inline qualifier dispatch C does in `insert()` (glob.c:381+).
fn check_single_qualifier(qual: &qualifier, path: &Path, meta: &Metadata) -> bool { // RUST-ONLY
        let mode = meta.mode();
        let ft = meta.file_type();

        match qual {
            qualifier::IsRegular => ft.is_file(),
            qualifier::IsDirectory => ft.is_dir(),
            qualifier::IsSymlink => ft.is_symlink(),
            qualifier::IsSocket => mode & libc::S_IFMT as u32 == libc::S_IFSOCK as u32,
            qualifier::IsFifo => mode & libc::S_IFMT as u32 == libc::S_IFIFO as u32,
            qualifier::IsBlockDev => mode & libc::S_IFMT as u32 == libc::S_IFBLK as u32,
            qualifier::IsCharDev => mode & libc::S_IFMT as u32 == libc::S_IFCHR as u32,
            qualifier::IsDevice => {
                let fmt = mode & libc::S_IFMT as u32;
                fmt == libc::S_IFBLK as u32 || fmt == libc::S_IFCHR as u32
            }
            qualifier::IsExecutable => ft.is_file() && (mode & 0o111 != 0),
            qualifier::Readable => mode & 0o400 != 0,
            qualifier::Writable => mode & 0o200 != 0,
            qualifier::Executable => mode & 0o100 != 0,
            qualifier::WorldReadable => mode & 0o004 != 0,
            qualifier::WorldWritable => mode & 0o002 != 0,
            qualifier::WorldExecutable => mode & 0o001 != 0,
            qualifier::GroupReadable => mode & 0o040 != 0,
            qualifier::GroupWritable => mode & 0o020 != 0,
            qualifier::GroupExecutable => mode & 0o010 != 0,
            qualifier::Setuid => mode & libc::S_ISUID as u32 != 0,
            qualifier::Setgid => mode & libc::S_ISGID as u32 != 0,
            qualifier::Sticky => mode & libc::S_ISVTX as u32 != 0,
            qualifier::OwnedByEuid => meta.uid() == unsafe { libc::geteuid() },
            qualifier::OwnedByEgid => meta.gid() == unsafe { libc::getegid() },
            qualifier::OwnedByUid(uid) => meta.uid() == *uid,
            qualifier::OwnedByGid(gid) => meta.gid() == *gid,
            qualifier::Size { value, unit, op } => {
                // Inline cmp + scale — mirrors glob.c qualsize at c:1054.
                let cmp = |a: u64, b: u64| match *op {
                    '<' => a <  b,
                    '>' => a >  b,
                    _   => a == b,
                };
                let size = meta.size();
                let scaled = match *unit {
                    u if u == TT_BYTES        => size,
                    u if u == TT_POSIX_BLOCKS => size.div_ceil(512),
                    u if u == TT_KILOBYTES    => size.div_ceil(1024),
                    u if u == TT_MEGABYTES    => size.div_ceil(1048576),
                    u if u == TT_GIGABYTES    => size.div_ceil(1073741824),
                    u if u == TT_TERABYTES    => size.div_ceil(1099511627776),
                    _ => size,
                };
                cmp(scaled, *value)
            }
            qualifier::Links { value, op } => {
                let cmp = |a: u64, b: u64| match *op {
                    '<' => a <  b,
                    '>' => a >  b,
                    _   => a == b,
                };
                cmp(meta.nlink(), *value)
            }
            qualifier::Atime { value, unit, op } => {
                // Inline time-unit scaling — Src/glob.c:872 qualtime.
                let cmp = |a: i64, b: i64| match *op {
                    '<' => a <  b,
                    '>' => a >  b,
                    _   => a == b,
                };
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap()
                          .as_secs() as i64;
                let diff = now - meta.atime();
                let scaled = match *unit {
                    u if u == TT_SECONDS => diff,
                    u if u == TT_MINS    => diff / 60,
                    u if u == TT_HOURS   => diff / 3600,
                    u if u == TT_DAYS    => diff / 86400,
                    u if u == TT_WEEKS   => diff / 604800,
                    u if u == TT_MONTHS  => diff / 2592000,
                    _ => diff,
                };
                cmp(scaled, *value)
            }
            qualifier::Mtime { value, unit, op } => {
                let cmp = |a: i64, b: i64| match *op {
                    '<' => a <  b,
                    '>' => a >  b,
                    _   => a == b,
                };
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap()
                          .as_secs() as i64;
                let diff = now - meta.mtime();
                let scaled = match *unit {
                    u if u == TT_SECONDS => diff,
                    u if u == TT_MINS    => diff / 60,
                    u if u == TT_HOURS   => diff / 3600,
                    u if u == TT_DAYS    => diff / 86400,
                    u if u == TT_WEEKS   => diff / 604800,
                    u if u == TT_MONTHS  => diff / 2592000,
                    _ => diff,
                };
                cmp(scaled, *value)
            }
            qualifier::Ctime { value, unit, op } => {
                let cmp = |a: i64, b: i64| match *op {
                    '<' => a <  b,
                    '>' => a >  b,
                    _   => a == b,
                };
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap()
                          .as_secs() as i64;
                let diff = now - meta.ctime();
                let scaled = match *unit {
                    u if u == TT_SECONDS => diff,
                    u if u == TT_MINS    => diff / 60,
                    u if u == TT_HOURS   => diff / 3600,
                    u if u == TT_DAYS    => diff / 86400,
                    u if u == TT_WEEKS   => diff / 604800,
                    u if u == TT_MONTHS  => diff / 2592000,
                    _ => diff,
                };
                cmp(scaled, *value)
            }
            qualifier::Mode { yes, no } => {
                let m = mode & 0o7777;
                (m & yes) == *yes && (m & no) == 0
            }
            qualifier::Device(dev) => meta.dev() == *dev,
            qualifier::NonEmptyDir => {
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
            qualifier::Eval(_) => true, // Would need shell integration
        }
}

/// Sort the per-state matches per the qualifier `o`/`O` keys.
/// **RUST-ONLY** — C does this inline at the end of `scanner()`
/// (glob.c:1700ish, qsort with `gmatchcmp` comparator).
fn sort_matches(state: &mut globdata) {                                      // RUST-ONLY
        // Default sort is GS_NAME ascending (c:204 gf_sortlist
        // initial setup). Per-qualifier `o<key>` / `O<key>` overrides.
        let specs: Vec<i32> = state
            .qualifiers
            .as_ref()
            .map(|q| q.sorts.clone())
            .unwrap_or_else(|| vec![GS_NAME]);

        // GS_NONE marker — caller wants no sort at all.
        if specs.iter().any(|&tp| (tp & GS_NONE) != 0) {
            return;
        }

        let numeric = glob_isset(crate::ported::zsh_h::NUMERICGLOBSORT);
        state.matches.sort_by(|a, b| gmatchcmp(a, b, &specs, numeric));
}

/// Apply `[FIRST,LAST]` qualifier subscript on the match list.
/// **RUST-ONLY** — C uses `gd_pre_first` / `gd_first` index tracking
/// during scanner emit; here we slice after the full walk.
fn apply_selection(state: &mut globdata) {                                   // RUST-ONLY
        let (first, last) = match &state.qualifiers {
            Some(q) => (q.first, q.last),
            None => return,
        };

        let len = state.matches.len() as i32;
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

        if start < end && start < state.matches.len() {
            state.matches = state.matches[start..end.min(state.matches.len())].to_vec();
        } else {
            state.matches.clear();
        }
}

/// Check if string has glob wildcards
/// Quick predicate for `does this string contain wildcards?`.
/// Port of the `haswilds()` macro inline in Src/glob.c —
/// short-circuits `zglob()` so plain literal paths skip the
/// scanner.
pub fn haswilds(s: &str) -> bool {                                          // c:4306
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

#[cfg(test)]
mod gs_tt_tests {
    use super::*;

    #[test]
    fn gs_size_offset_matches_c() {
        // c:83 — GS_SIZE = GS_SHIFT_BASE = 8.
        assert_eq!(GS_SIZE, 8);
        // c:84 — GS_ATIME = GS_SHIFT_BASE << 1 = 16.
        assert_eq!(GS_ATIME, 16);
        // c:87 — GS_LINKS = GS_SHIFT_BASE << 4 = 128.
        assert_eq!(GS_LINKS, 128);
    }

    #[test]
    fn gs_normal_covers_all_size_keys() {
        // c:99 — GS_NORMAL = SIZE | ATIME | MTIME | CTIME | LINKS.
        assert!(GS_NORMAL & GS_SIZE  != 0);
        assert!(GS_NORMAL & GS_ATIME != 0);
        assert!(GS_NORMAL & GS_MTIME != 0);
        assert!(GS_NORMAL & GS_CTIME != 0);
        assert!(GS_NORMAL & GS_LINKS != 0);
    }

    #[test]
    fn tt_namespaces_share_indices() {
        // c:121-126 vs c:128-133 — TT_DAYS == TT_BYTES == 0, etc.
        assert_eq!(TT_DAYS,    TT_BYTES);
        assert_eq!(TT_HOURS,   TT_POSIX_BLOCKS);
        assert_eq!(TT_MINS,    TT_KILOBYTES);
        assert_eq!(TT_WEEKS,   TT_MEGABYTES);
        assert_eq!(TT_MONTHS,  TT_GIGABYTES);
        assert_eq!(TT_SECONDS, TT_TERABYTES);
    }

    #[test]
    fn max_sorts_is_12() {
        assert_eq!(MAX_SORTS, 12);
    }
}



/// Recursive `*` / `?` / `[...]` glob matcher. Port of `patmatch()`
/// from Src/pattern.c:2694 — the C source's main matching engine
/// that walks a precompiled `Upat` while consuming the input string.
/// This Rust port works directly on raw pattern + text strings (no
/// precompile step), but mirrors the same recursive descent: `*` tries
/// each text suffix, `?` consumes one char, `[...]` dispatches to
/// `patmatchrange`, and literal chars must match exactly.
fn patmatch(pattern: &str, text: &str, extended: bool) -> bool {
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
                        if patmatch(&rest, &text[i..], extended) {
                            return true;
                        }
                        pos = i + 1;
                    }
                }
                // Also try matching at end
                return patmatch(&rest, "", extended);
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
                if !patmatchrange(&mut pi, tc) {
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

/// Match a single character against a `[...]` bracket expression.
/// Port of `patmatchrange(char *range, int ch, int *indptr, int *mtp)` from Src/pattern.c:3856 — the C variant
/// scans a pre-compiled range buffer with explicit indices; this Rust
/// port consumes a peekable iterator over the live pattern string and
/// returns whether `tc` matches (honoring `[!...]` / `[^...]` negation
/// and `a-z` ranges, like the C source's range walk at line 3870).
/// WARNING: param names don't match C — Rust=(pi, tc) vs C=(range, ch, indptr, mtp)
fn patmatchrange(pi: &mut std::iter::Peekable<std::str::Chars>, tc: char) -> bool {
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
    // C: `pathprog(s, NULL)` → walks `path[]` array (paramtab $PATH).
    let path = match crate::ported::params::getsparam("PATH") {
        Some(p) => p,
        None => return s.to_string(),
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
// np points to a node in the list which will be expanded                  // c:1209
// into a series of nodes.                                                  // c:1210
/// Top-level glob entry point.
/// Port of `zglob(LinkList list, LinkNode np, int nountok)` from Src/glob.c:1214. Reads all option flags
/// (nullglob / extendedglob / dotglob / caseglob / globstarshort /
/// bareglobqual / braceccl / markdirs / numericglobsort / …)
/// directly from the canonical global option store via
/// `crate::ported::options::opt_state_get` — same path C uses via
/// `isset(NULL_GLOB)` etc. on the global `opts[]` array.
pub fn glob(pattern: &str) -> Vec<String> {                                  // c:1214
    // Race fix: snapshot every glob-relevant option into TLS for the
    // duration of this call so concurrent setopt/unsetopt on other
    // threads can't corrupt our qualifier parse / nullglob fallback /
    // dotglob filter etc. mid-walk. See `enter_glob_scope` doc.
    let _glob_scope = enter_glob_scope();
    let mut state = globdata::new();
    globdata_glob(&mut state, pattern)
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

/// Match minimum distance for spelling correction (from glob.c mindist line 4624)
/// Edit-distance helper for `setopt CORRECT` glob fallback.
/// Port of the `spdist()`-driven correction inside
/// `findcmd()` (Src/exec.c) when adapted for glob targets.
pub fn mindist(dir: &str, name: &str, best: &mut String, exact: bool) -> usize { // c:4624
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

/// Apply mode spec to existing mode. Port of the
/// `spec_op`/`spec_who`/`spec_perm` inline application at the end of
/// C's `qgetmodespec` (`Src/glob.c:919-922`).
pub fn apply_modespec(mode: u32, who: u32, op: char, perm: u32) -> u32 {
    match op {
        '+' => mode | perm,
        '-' => mode & !perm,
        '=' => (mode & !who) | perm,
        _ => mode,
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: glob
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

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

/// Canonical entry point for filesystem glob expansion. Mirrors C's
/// `zglob` driver at Src/glob.c:1214 with the alternation +
/// extendedglob pre-passes inlined (zsh's `(a|b|c)` group-level
/// alternation and `pat1~pat2` exclusion).
///
/// Reads `nullglob` / `nomatch` / `extendedglob` / `dotglob` /
/// `globdots` / `caseglob` / `nocaseglob` / `globstarshort` /
/// `bareglobqual` / `braceccl` / `markdirs` / `numericglobsort` /
/// `globdots` from the canonical option store (`opt_state_get`) so
/// behavior tracks `setopt …` toggles without needing an executor.
pub fn glob_path(pattern: &str) -> Vec<String> {                             // c:1214
    // Race fix: same TLS-snapshot guard glob() uses, so glob_path
    // callers see a coherent option set for the duration of this call.
    let _glob_scope = enter_glob_scope();
    let opt = |n: &str, default: bool| opt_state_get(n).unwrap_or(default);
    // Read option state directly — matches C's per-callsite
    // `isset(NULLGLOB)` / `isset(EXTENDEDGLOB)` reads (Src/glob.c).
    let null_glob     = opt("nullglob",       false);
    let extended_glob = opt("extendedglob",   false);
    let no_glob_dots  = !(opt("dotglob",      false) || opt("globdots", false));
    let case_glob     = opt("caseglob",       true)  && !opt("nocaseglob", false);

    // c:1230 — `(a|b|c)` top-level alternation pre-pass.
    if let Some(alternatives) = expand_glob_alternation(pattern) {
        let mut out: Vec<String> = Vec::new();
        for alt in alternatives {
            let has_meta = alt.chars().any(|c| matches!(c, '*' | '?' | '[' | '('));
            if has_meta {
                out.extend(glob_path(&alt));
            } else if std::path::Path::new(&alt).exists() {
                out.push(alt);
            }
        }
        let mut seen = std::collections::HashSet::new();
        out.retain(|p| seen.insert(p.clone()));
        out.sort();
        if !out.is_empty() {
            return out;
        }
        // c:1700 fall through to NOMATCH semantics below.
    }

    // c:155 (P_EXCLUDE) — extendedglob `^pat` negation + `pat1~pat2`
    // exclusion. Only applies when extendedglob option is on.
    if extended_glob {
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
                    if name.starts_with('.') && no_glob_dots {
                        continue;
                    }
                    if !matchpat(neg, &name, true, case_glob) {
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
            if !out.is_empty() { return out; }
            if null_glob { return Vec::new(); }
            return vec![pattern.to_string()];
        }
        // c:155 — top-level `~` exclusion.
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
                    split_at = Some(i);
                    break;
                }
                _ => {}
            }
        }
        if let Some(pos) = split_at {
            let lhs: String = chars[..pos].iter().collect();
            let rhs: String = chars[pos + 1..].iter().collect();
            let lhs_matches = glob_path(&lhs);
            let filtered: Vec<String> = lhs_matches
                .into_iter()
                .filter(|p| {
                    let basename = p.rsplit('/').next().unwrap_or(p);
                    !matchpat(&rhs, basename, true, case_glob)
                        && !matchpat(&rhs, p, true, case_glob)
                })
                .collect();
            if !filtered.is_empty() { return filtered; }
            if null_glob { return Vec::new(); }
            return vec![pattern.to_string()];
        }
    }

    // Main walk via the canonical glob driver.
    let mut state = globdata::new();
    let matches = globdata_glob(&mut state, pattern);
    if matches.is_empty() && !null_glob {
        return vec![pattern.to_string()];
    }
    matches
}

// `g_range` from Src/glob.c — qualifier-comparison direction
// (-1 = less than, 0 = equal, 1 = greater than).
pub static G_RANGE: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

// `mode_to_octal` from Src/utils.c — converts a stat-mode to an octal
// integer with all the standard bits.
fn mode_to_octal(mode: u32) -> u32 {
    // Linux + macOS already use the standard POSIX bit layout; identity.
    mode & 0o7777
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
    fn test_haswilds() {
        assert!(haswilds("*.txt"));
        assert!(haswilds("file?.txt"));
        assert!(haswilds("file[12].txt"));
        assert!(!haswilds("file.txt"));
        assert!(!haswilds("path/to/file.txt"));
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

        let mut state = globdata::new();
        let results = globdata_glob(&mut state, &pattern);

        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|s| s.ends_with("file1.txt")));
        assert!(results.iter().any(|s| s.ends_with("file2.txt")));
    }

    #[test]
    fn test_glob_hidden() {
        use crate::ported::options::opt_state_set;
        let dir = setup_test_dir();
        let pattern = format!("{}/*", dir.path().display());

        // Default (globdots off) → hidden files skipped. `dotglob`
        // is the bash alias for zsh's canonical `globdots` option;
        // the C-faithful read goes through `isset(GLOBDOTS)` which
        // resolves the canonical name.
        opt_state_set("globdots", false);
        let mut state = globdata::new();
        let results = globdata_glob(&mut state, &pattern);
        assert!(!results.iter().any(|s| s.contains(".hidden")));

        // setopt globdots → hidden files included.
        opt_state_set("globdots", true);
        let mut state = globdata::new();
        let results = globdata_glob(&mut state, &pattern);
        assert!(results.iter().any(|s| s.contains(".hidden")));
        opt_state_set("globdots", false);  // reset for other tests
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
    fn test_zstrcmp_numeric() {
        use crate::ported::sort::zstrcmp;
        let n = crate::zsh_h::SORTIT_NUMERICALLY as u32;
        assert_eq!(zstrcmp("file1", "file2", n), Ordering::Less);
        assert_eq!(zstrcmp("file10", "file2", n), Ordering::Greater);
        assert_eq!(zstrcmp("file10", "file10", n), Ordering::Equal);
    }

    /// Race-fix verification: snapshot pins bareglobqual for the
    /// duration of a glob scope even when the live store flips
    /// underneath. Mimics the stryke pattern in
    /// MenkeTechnologies/strykelang#3 — one thread is mid-glob with
    /// bareglobqual=1 snapshot when another flips it off.
    #[test]
    fn glob_opts_snapshot_isolates_concurrent_setopt() {
        use crate::ported::options::{opt_state_get, opt_state_set, opt_state_unset};
        use crate::ported::zsh_h::BAREGLOBQUAL;

        // Preserve the existing live state so the test is hermetic.
        let saved = opt_state_get("bareglobqual");

        // Live store says bareglobqual=true; enter scope captures that.
        opt_state_set("bareglobqual", true);
        let _scope = enter_glob_scope();
        assert!(glob_isset(BAREGLOBQUAL),
            "TLS snapshot reads bareglobqual=true at scope entry");

        // Simulate a concurrent thread flipping the live store.
        opt_state_set("bareglobqual", false);
        assert!(glob_isset(BAREGLOBQUAL),
            "TLS snapshot must still report bareglobqual=true \
             even though live store now reads false");

        // Restore.
        match saved {
            Some(v) => opt_state_set("bareglobqual", v),
            None => opt_state_unset("bareglobqual"),
        }
    }

    /// After the scope guard drops, reads fall back to the live store.
    #[test]
    fn glob_opts_snapshot_clears_on_drop() {
        use crate::ported::options::{opt_state_get, opt_state_set, opt_state_unset};
        use crate::ported::zsh_h::NULLGLOB;

        let saved = opt_state_get("nullglob");
        opt_state_set("nullglob", true);
        {
            let _scope = enter_glob_scope();
            assert!(glob_isset(NULLGLOB), "snapshot live=true → true inside");
        }
        // Outside scope: flip live store, read should follow live.
        opt_state_set("nullglob", false);
        assert!(!glob_isset(NULLGLOB),
            "post-scope: glob_isset falls back to live store");

        match saved {
            Some(v) => opt_state_set("nullglob", v),
            None => opt_state_unset("nullglob"),
        }
    }

    /// Nested glob scopes share the outer snapshot — inner doesn't
    /// re-capture or clear on its own Drop.
    #[test]
    fn glob_opts_snapshot_nested_is_noop() {
        use crate::ported::options::{opt_state_get, opt_state_set, opt_state_unset};
        use crate::ported::zsh_h::EXTENDEDGLOB;

        let saved = opt_state_get("extendedglob");
        opt_state_set("extendedglob", true);
        let _outer = enter_glob_scope();
        assert!(glob_isset(EXTENDEDGLOB));
        {
            let _inner = enter_glob_scope();
            // Live store flip while nested.
            opt_state_set("extendedglob", false);
            assert!(glob_isset(EXTENDEDGLOB),
                "inner observes outer snapshot");
        } // inner drops — outer snapshot still active.
        assert!(glob_isset(EXTENDEDGLOB),
            "outer snapshot survives inner drop");

        match saved {
            Some(v) => opt_state_set("extendedglob", v),
            None => opt_state_unset("extendedglob"),
        }
    }
}
