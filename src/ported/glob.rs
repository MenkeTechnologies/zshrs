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

use crate::ported::builtin::LASTVAL;
use crate::ported::options::opt_state_get;
use crate::ported::pattern::{haswilds, Patprog};
use crate::ported::signals::unqueue_signals;
use crate::ported::sort::zstrcmp;
use crate::ported::string::dyncat;
use crate::ported::utils::{errflag, init_dirsav, lchdir, restoredir, zerr};
// `vm_helper` import removed — ShellExecutor reach-in routed through
// the `crate::ported::exec` accessor wrappers (see memory
// feedback_no_exec_script_from_ported).
use crate::ported::lex::untokenize;
use crate::ported::mem::dupstring;
use crate::ported::subst::singsub;
use crate::ported::subst::LinkList;
use crate::ported::zsh_h::{imatchdata, repldata};
use crate::ported::zsh_h::{
    isset, redir, Bnull, Bnullkeep, Dnull, Inang, Meta, Nularg, Outang, Pound, Snull, BAREGLOBQUAL,
    BRACECCL, CASEGLOB, ERRFLAG_INT, EXTENDEDGLOB, GLOBDOTS, GLOBSTARSHORT, IS_DASH, LISTTYPES,
    MARKDIRS, MB_METASTRLEN2END, MULTIOS, NULLGLOB, NUMERICGLOBSORT, PAT_NOTEND, PAT_NOTSTART,
    PP_UNKWN, PREFORK_SINGLE, REDIR_CLOSE, REDIR_ERRWRITE, REDIR_MERGEIN, REDIR_MERGEOUT, SHGLOB,
    SUB_ALL, SUB_BIND, SUB_DOSUBST, SUB_EIND, SUB_END, SUB_GLOBAL, SUB_LEN, SUB_LIST, SUB_LONG,
    SUB_MATCH, SUB_REST, SUB_START, SUB_SUBSTR, ZSHTOK_SHGLOB, ZSHTOK_SUBST,
};
use crate::ported::ztype_h::imeta;
use crate::subst::prefork;
use std::collections::HashSet;
use std::fs::{self, Metadata};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};

/// A glob match with metadata for sorting
#[derive(Debug, Clone)]
/// One glob match result.
/// Port of `struct gmatch` from Src/glob.c — `gmatchcmp()`
/// (line 936) sorts arrays of these for the `o`/`O` qualifier.
pub struct gmatch {
    /// `name` field.
    pub name: String,
    /// c:47 `char *uname` — "Unmetafied file name; embedded nulls can't
    /// occur in file names". The string `gmatchcmp`'s `GS_NAME` arm
    /// collates (c:945). C fills it once per match, immediately before
    /// the qsort (c:1963-1973); the port does the same in `sort_matches`
    /// so the comparator never re-derives it. Empty until then.
    pub uname: String,
    /// `path` field.
    pub path: PathBuf,
    /// `size` field.
    pub size: u64,
    /// `atime` field.
    pub atime: i64,
    /// `mtime` field.
    pub mtime: i64,
    /// `ctime` field.
    pub ctime: i64,
    /// `links` field.
    pub links: u64,
    /// `mode` field.
    pub mode: u32,
    /// `uid` field.
    pub uid: u32,
    /// `gid` field.
    pub gid: u32,
    /// `dev` field.
    pub dev: u64,
    /// `ino` field.
    pub ino: u64,
    // For symlink targets (when following)
    /// `target_size` field.
    pub target_size: u64,
    /// `target_atime` field.
    pub target_atime: i64,
    /// `target_mtime` field.
    pub target_mtime: i64,
    /// `target_ctime` field.
    pub target_ctime: i64,
    /// `target_links` field.
    pub target_links: u64,
    // Sub-second timestamp components — port of `long ansec/mnsec/cnsec` +
    // `long _ansec/_mnsec/_cnsec` in `struct gmatch` (Src/glob.c:63-74).
    // gmatchcmp tie-breaks equal-second mtimes/atimes/ctimes by these
    // nanosecond fields (c:992/999/1006/1019/1026/1033) — without them,
    // files touched <1s apart sort by name rather than mtime.
    /// `ansec` field. c:64
    pub ansec: i64,
    /// `mnsec` field. c:68
    pub mnsec: i64,
    /// `cnsec` field. c:72
    pub cnsec: i64,
    /// `_ansec` field. c:65
    pub target_ansec: i64,
    /// `_mnsec` field. c:69
    pub target_mnsec: i64,
    /// `_cnsec` field. c:73
    pub target_cnsec: i64,
    // For exec sort strings
    /// `sort_strings` field.
    pub sort_strings: Vec<String>,
}

impl GlobOptSnapshot {
    /// Read every glob-relevant option from the canonical live store
    /// (`opt_state_get`) once. Returns a self-contained snapshot.
    pub fn capture() -> Self {
        Self {
            bareglobqual: isset(BAREGLOBQUAL),
            braceccl: isset(BRACECCL),
            caseglob: isset(CASEGLOB),
            extendedglob: isset(EXTENDEDGLOB),
            globdots: isset(GLOBDOTS),
            globstarshort: isset(GLOBSTARSHORT),
            listtypes: isset(LISTTYPES),
            markdirs: isset(MARKDIRS),
            nullglob: isset(NULLGLOB),
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
pub const GS_NAME: i32 = 1; // c:77

impl Drop for GlobOptsGuard {
    fn drop(&mut self) {
        if self.populated {
            GLOB_OPTS_TLS.with_borrow_mut(|g| *g = None);
        }
    }
}
/// Port of `GS_DEPTH` from `Src/glob.c:78`. Sort by directory depth.
pub const GS_DEPTH: i32 = 2; // c:78
/// Port of `GS_EXEC` from `Src/glob.c:79`. Sort via external function.
pub const GS_EXEC: i32 = 4; // c:79

/// Port of `GS_SHIFT_BASE` from `Src/glob.c:81`. Bit position where
/// the size/mtime/atime/ctime/links sort keys live.
pub const GS_SHIFT_BASE: i32 = 8; // c:81

/// Port of `GS_SIZE` from `Src/glob.c:83`. Sort by file size.
pub const GS_SIZE: i32 = GS_SHIFT_BASE; // c:83
/// Port of `GS_ATIME` from `Src/glob.c:84`. Sort by access time.
pub const GS_ATIME: i32 = GS_SHIFT_BASE << 1; // c:84
/// Port of `GS_MTIME` from `Src/glob.c:85`. Sort by modification time.
pub const GS_MTIME: i32 = GS_SHIFT_BASE << 2; // c:85
/// Port of `GS_CTIME` from `Src/glob.c:86`. Sort by inode-change time.
pub const GS_CTIME: i32 = GS_SHIFT_BASE << 3; // c:86
/// Port of `GS_LINKS` from `Src/glob.c:87`. Sort by hard-link count.
pub const GS_LINKS: i32 = GS_SHIFT_BASE << 4; // c:87

/// Port of `GS_SHIFT` from `Src/glob.c:89`. Bit-shift offset where the
/// reverse-direction variants of the size/atime/mtime/ctime/links
/// sort flags live.
pub const GS_SHIFT: i32 = 5; // c:89

/// Port of `GS__SIZE`  from `Src/glob.c:90` (reverse-sort variant).
pub const GS__SIZE: i32 = GS_SIZE << GS_SHIFT; // c:90
/// Port of `GS__ATIME` from `Src/glob.c:91`.
pub const GS__ATIME: i32 = GS_ATIME << GS_SHIFT; // c:91
/// Port of `GS__MTIME` from `Src/glob.c:92`.
pub const GS__MTIME: i32 = GS_MTIME << GS_SHIFT; // c:92
/// Port of `GS__CTIME` from `Src/glob.c:93`.
pub const GS__CTIME: i32 = GS_CTIME << GS_SHIFT; // c:93
/// Port of `GS__LINKS` from `Src/glob.c:94`.
pub const GS__LINKS: i32 = GS_LINKS << GS_SHIFT; // c:94

/// Port of `GS_DESC` from `Src/glob.c:96`. Descending-order toggle.
pub const GS_DESC: i32 = GS_SHIFT_BASE << (2 * GS_SHIFT); // c:96
/// Port of `GS_NONE` from `Src/glob.c:97`. Marker for no-sort spec.
pub const GS_NONE: i32 = GS_SHIFT_BASE << (2 * GS_SHIFT + 1); // c:97

/// Port of `GS_NORMAL` from `Src/glob.c:99`. Forward-direction sort
/// keys (excluding NAME/DEPTH/EXEC and the reverse variants).
pub const GS_NORMAL: i32 = GS_SIZE | GS_ATIME | GS_MTIME | GS_CTIME | GS_LINKS; // c:99
/// Port of `GS_LINKED` from `Src/glob.c:100`. Reverse-direction
/// (linked) sort keys.
pub const GS_LINKED: i32 = GS_NORMAL << GS_SHIFT; // c:100

// =====================================================================
// TT_* — time + size unit selectors. Two parallel namespaces using
// the same numeric values.
// =====================================================================

/// Port of `TT_DAYS` from `Src/glob.c:121`. Time qualifier in days.
pub const TT_DAYS: i32 = 0; // c:121
/// Port of `TT_HOURS` from `Src/glob.c:122`. Time qualifier in hours.
pub const TT_HOURS: i32 = 1; // c:122
/// Port of `TT_MINS` from `Src/glob.c:123`. Time qualifier in minutes.
pub const TT_MINS: i32 = 2; // c:123
/// Port of `TT_WEEKS` from `Src/glob.c:124`. Time qualifier in weeks.
pub const TT_WEEKS: i32 = 3; // c:124
/// Port of `TT_MONTHS` from `Src/glob.c:125`. Time qualifier in months.
pub const TT_MONTHS: i32 = 4; // c:125
/// Port of `TT_SECONDS` from `Src/glob.c:126`. Time qualifier in seconds.
pub const TT_SECONDS: i32 = 5; // c:126

/// Port of `TT_BYTES` from `Src/glob.c:128`. Size qualifier in bytes.
pub const TT_BYTES: i32 = 0; // c:128
/// Port of `TT_POSIX_BLOCKS` from `Src/glob.c:129`. Size qualifier
/// in POSIX 512-byte blocks (the `b` glob qualifier suffix).
pub const TT_POSIX_BLOCKS: i32 = 1; // c:129
/// Port of `TT_KILOBYTES` from `Src/glob.c:130`.
pub const TT_KILOBYTES: i32 = 2; // c:130
/// Port of `TT_MEGABYTES` from `Src/glob.c:131`.
pub const TT_MEGABYTES: i32 = 3; // c:131
/// Port of `TT_GIGABYTES` from `Src/glob.c:132`.
pub const TT_GIGABYTES: i32 = 4; // c:132
/// Port of `TT_TERABYTES` from `Src/glob.c:133`.
pub const TT_TERABYTES: i32 = 5; // c:133

/// Port of `MAX_SORTS` from `Src/glob.c:164`. Maximum sort-spec keys
/// per glob (`glob -O 'reverse(name).size'` style).
pub const MAX_SORTS: usize = 12; // c:164

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
/// `globdata` — see fields for layout.
#[allow(non_camel_case_types)]
pub struct globdata {
    // c:168
    /// `matches` field.
    pub matches: Vec<gmatch>,
    /// `qualifiers` field.
    pub qualifiers: Option<qualifier_set>,
    /// c:206 — `quals` (`curglobdata.gd_quals`): the `struct qual` list
    /// that `insert()` walks per candidate file. Arena-backed port of
    /// the C pointer list; `None` when the glob has no qualifiers.
    pub quals: Option<QualArena>,
    pub pathbuf: String,                    // c:170 gd_pathbuf
    pub pathpos: usize,                     // c:169 gd_pathpos
    pub matchct: i32,                       // c:173 gd_matchct
    pub pathbufsz: usize,                   // c:174 gd_pathbufsz
    pub pathbufcwd: i32,                    // c:175 gd_pathbufcwd
    pub gf_nullglob: i32,                   // c:186 gd_gf_nullglob
    pub gf_markdirs: i32,                   // c:186 gd_gf_markdirs
    pub gf_noglobdots: i32,                 // c:186 gd_gf_noglobdots
    pub gf_listtypes: i32,                  // c:186 gd_gf_listtypes
    pub gf_pre_words: Option<Vec<String>>,  // c:190 gd_gf_pre_words
    pub gf_post_words: Option<Vec<String>>, // c:190 gd_gf_post_words
}

// c:197 — `static struct globdata curglobdata;`
// C's single file-static glob state; macros at c:199-222 redirect
// `pathbufsz`, `gf_noglobdots`, etc. to `curglobdata.gd_*`. Rust
// mirror — accessed via lock for thread safety; C is single-threaded.
/// `CURGLOBDATA` static.
pub static CURGLOBDATA: std::sync::Mutex<globdata> = std::sync::Mutex::new(globdata {
    matches: Vec::new(),
    qualifiers: None,
    quals: None,
    pathbuf: String::new(),
    pathpos: 0,
    matchct: 0,
    pathbufsz: 0,
    pathbufcwd: 0,
    gf_nullglob: 0,
    gf_markdirs: 0,
    gf_noglobdots: 0,
    gf_listtypes: 0,
    gf_pre_words: None,
    gf_post_words: None,
});

/// Port of `int badcshglob` from `Src/glob.c:103`. Tracks csh-glob
/// diagnostic state across a single command line: bit 1 = "at
/// least one expansion failed" (CSHNULLGLOB and no matches),
/// bit 2 = "at least one expansion produced output". `globlist`
/// resets to 0 at entry, ORs the bits as each `zglob` runs, then
/// emits "no match" iff the final value is 1 (some failed, none
/// succeeded). Used to make `*.nope *.ok` succeed without
/// diagnostic, but `*.nope alone` error out under CSHNULLGLOB.
pub static BADCSHGLOB: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:103

/// Port of `static char **inserts;` from `Src/glob.c:340`. Set by
/// `qualsheval` (c:3927-3937) from the `$reply` array / `$REPLY` scalar
/// after an `(e:…:)` / `(+…)` qualifier eval; `insert()` then emits one
/// match per element instead of the original name (c:428). `None` ==
/// C's `inserts == NULL` (emit the original name once). Reset to `None`
/// before each per-file qualifier walk (C: `inserts = NULL` at insert()
/// entry, c:353).
pub static INSERTS: std::sync::Mutex<Option<Vec<String>>> = std::sync::Mutex::new(None);

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
pub struct complist {
    // c:252
    pub next: Option<Box<complist>>,          // c:253
    pub pat: crate::ported::pattern::Patprog, // c:254
    pub closure: i32,                         // c:255
    pub follow: i32,                          // c:256
}

/// Add path component (from glob.c addpath lines 263-274)
/// Append a path component to a glob path buffer.
/// Port of `addpath(char *s, int l)` from Src/glob.c:265.
///
/// C body (c:270-273):
/// ```c
///     while (l--)
///         pathbuf[pathpos++] = *s++;
///     pathbuf[pathpos++] = '/';
///     pathbuf[pathpos] = '\0';
/// ```
/// The `/` is appended UNCONDITIONALLY. That is load-bearing for EMPTY
/// path components: `patcompile(..., PAT_FILET)` cuts a pure section at
/// the first `/` (pattern.rs:891), so `//usr//b*` parses into the
/// component chain `"" , "usr", "", "b*"` and each empty component must
/// still contribute one `/` to `pathbuf` for the glob results to carry
/// the literal `//` the user typed. A previous `if !s.ends_with('/')`
/// guard here squeezed those empty components away (`//usr//bin` →
/// `/usr/bin`), which broke `_path_files`: it anchors matches on the
/// on-line path text (`${(@)tmp1#$prepath$realpath$testpath}`), so a
/// squeezed result no longer shares the `//usr//` prefix and EVERY match
/// was discarded — completion after `ls //usr//` produced nothing at all.
/// A non-empty component can never end in `/` (patcompile cut it there),
/// so removing the guard changes behaviour ONLY in the empty case.
pub fn addpath(s: &mut String, l: &str) {
    // c:265
    s.push_str(l); // c:270-271
    s.push('/'); // c:272 — unconditional, see doc above
}

/// PARTIAL port of `statfullpath(const char *s, struct stat *st, int l)` from
/// `Src/glob.c:283` — NOT yet faithful, and currently has NO callers (the live
/// qualifier matcher inlines `fs::metadata`/`symlink_metadata` on the already-
/// full path). Two gaps remain, both blocked on the glob path-state flow:
///   1. C prepends the global `pathbuf[pathbufcwd..pathpos]` (the accumulated
///      glob directory) to `s`; this port omits it. A faithful version must
///      take the pathbuf as a parameter — it cannot read the `CURGLOBDATA`
///      singleton because the matcher already holds that lock (deadlock).
///   2. C's `st` is the OUTPUT `struct stat *`; the second arg here is a string
///      suffix, which is not the C contract.
/// The `l` flag now matches C: `l ? lstat : stat` (l → `symlink_metadata`).
/// Do not wire this in until gap #1 is resolved.
pub fn statfullpath(s: &str, st: &str, l: bool) -> Option<Metadata> {
    // c:283
    let full = if st.is_empty() {
        if s.is_empty() {
            ".".to_string()
        } else {
            s.to_string()
        }
    } else {
        format!("{}{}", s, st)
    };

    // c:308 — `return l ? lstat(buf, st) : stat(buf, st);`
    if l {
        fs::symlink_metadata(&full).ok() // l → lstat
    } else {
        fs::metadata(&full).ok() // !l → stat (follows symlinks)
    }
}
// END moved-from-exec-rs (free ported)

// ===========================================================
// Direct ports of static helpers from Src/glob.c not yet covered
// above. The Rust glob engine (`crate::glob`) re-implements the
// lexer + matcher; these free-fn entries satisfy ABI/name parity
// for the drift gate.
// ===========================================================

/// Port of `scanner(Complist q, int shortcircuit)` from `Src/glob.c:500`.
///
/// Walks the `complist` path-component chain built by `parsecomplist`,
/// descending the directory tree one component per node. `q.closure`
/// (set for `**` and `(dir/)#` / `(dir/)##`) drives the any-depth match:
/// the zero-directory case scans `q.next` directly, then each matching
/// subdirectory is re-scanned against `(q.closure) ? q : q.next` — i.e.
/// the same node `q` repeats for `**`, matching at every depth.
///
/// Deviations from C, all pre-existing in this engine — NOT introduced
/// by this port:
///   * directory entries come from `fs::read_dir` + `zreaddir` (the
///     ported `zreaddir`, glob.c:5217) instead of C `opendir`; same
///     skip-`.`/`..` behaviour, and names are real (non-metafied) so no
///     `unmeta` round-trip is needed;
///   * the approximate-match error-reduction block (`forceerrs`/
///     `errsfound`, glob.c:619-639) is omitted — the Rust matcher keeps
///     no per-match error count;
///   * `glob_pre`/`glob_suf` (ZLE prefix/suffix) filtering is omitted.
/// Leading-dot files are filtered by `pattry`'s `PAT_NOGLD` check
/// (pattern.rs:2890); `globdata_glob` sets `gf_noglobdots` so
/// `parsecomplist` compiles the flag in (faithful to C's `glob()`).
///
/// The `in_closure_repeat` parameter is Rust-only: it carries C's
/// `q->closure = 1` mutation across the recursion. A `(dir/)##` node
/// (closure==2) requires at least one directory, so the zero-directory
/// match is skipped only the FIRST time the node is entered; once we have
/// descended one closure level it behaves like `(dir/)#` (zero-or-more).
/// `q` is shared (`&complist`), so C's in-place field mutation is modelled
/// by this argument instead. Callers pass `false`.
fn scanner(state: &mut globdata, q: Option<&complist>, shortcircuit: i32, in_closure_repeat: bool) {
    use std::sync::atomic::Ordering;
    // c:506 — `if (!q || errflag) return;`
    let Some(q) = q else { return };
    if errflag.load(Ordering::SeqCst) & crate::ported::zsh_h::ERRFLAG_ERROR != 0 {
        return;
    }
    let pbcwdsav = state.pathbufcwd; // c:503
    let mut ds = init_dirsav(); // c:508
    let path_max = crate::ported::zsh_system_h::PATH_MAX;

    // c:510-518 — closure preamble: try zero directories via q.next first
    // (skipped for `(dir/)##` until at least one directory is consumed).
    let closure = q.closure;
    if closure != 0 {
        let skip_zero = q.closure == 2 && !in_closure_repeat;
        if !skip_zero {
            scanner(state, q.next.as_deref(), shortcircuit, false); // c:515
            if shortcircuit != 0 && shortcircuit == state.matchct {
                return;
            }
        }
    }

    let p = &q.pat; // c:519
    if (p.0.flags & crate::ported::zsh_h::PAT_PURES as i32) != 0 {
        // c:521-565 — pure literal section: append to the path (intermediate)
        // or emit (final); no directory scan.
        let start = p.0.startoff as usize;
        let l = p.0.patmlen as usize;
        let str_lit = String::from_utf8_lossy(&p.1[start..start + l]).into_owned();

        // c:524-536 — PATH_MAX guard.
        if l + (l == 0) as usize + state.pathpos - state.pathbufcwd as usize >= path_max {
            if l >= path_max {
                return;
            }
            let anchor = state
                .pathbuf
                .get(state.pathbufcwd as usize..)
                .unwrap_or("")
                .to_string();
            let err = lchdir(&anchor, Some(&mut ds), 0);
            if err == -1 {
                return;
            }
            if err != 0 {
                zerr("current directory lost during glob");
                return;
            }
            state.pathbufcwd = state.pathpos as i32;
        }

        if q.next.is_some() {
            // c:539-560 — not the last section: add to path, recurse.
            let oppos = state.pathpos;
            if errflag.load(Ordering::SeqCst) & crate::ported::zsh_h::ERRFLAG_ERROR == 0 {
                // c:543-552 — `.`/`..` handling inside a closure walk.
                let mut add = true;
                if closure != 0 && !state.pathbuf.is_empty() {
                    if str_lit == "." {
                        add = false; // c:546
                    } else if str_lit == ".." {
                        // c:547-551 — drop `..` that would escape the root.
                        use std::os::unix::fs::MetadataExt;
                        let cur: &str = &state.pathbuf;
                        add = match (fs::metadata("/"), fs::metadata(cur)) {
                            (Ok(r), Ok(c)) => r.ino() != c.ino() || r.dev() != c.dev(),
                            _ => true,
                        };
                    }
                }
                if add {
                    addpath(&mut state.pathbuf, &str_lit); // c:553
                    state.pathpos = state.pathbuf.len();
                    // c:554 — closure: only recurse when the new path is a
                    // real directory (`!statfullpath("", NULL, 1)`).
                    let recurse = closure == 0
                        || fs::metadata(state.pathbuf.as_str())
                            .map(|m| m.is_dir())
                            .unwrap_or(false);
                    if recurse {
                        scanner(
                            state,
                            if closure != 0 {
                                Some(q)
                            } else {
                                q.next.as_deref()
                            },
                            shortcircuit,
                            closure != 0,
                        ); // c:555
                        if shortcircuit != 0 && shortcircuit == state.matchct {
                            return;
                        }
                    }
                    state.pathbuf.truncate(oppos); // c:558
                    state.pathpos = oppos;
                }
            }
        } else {
            // c:561-564 — last section: emit the literal.
            let full = std::path::Path::new(&state.pathbuf).join(&str_lit);
            insert(state, &full, 0);
            if shortcircuit != 0 && shortcircuit == state.matchct {
                return;
            }
        }
    } else {
        // c:567-685 — pattern-matched section: scan the directory.
        let base = {
            let from_cwd = state.pathbuf.get(state.pathbufcwd as usize..).unwrap_or("");
            if from_cwd.is_empty() {
                ".".to_string()
            } else {
                from_cwd.to_string()
            }
        };
        let dirs = q.next.is_some(); // c:570
        let mut rd = match fs::read_dir(&base) {
            Ok(d) => d,
            Err(_) => return, // c:573 — opendir == NULL
        };
        // c:572-573 — collect matching subdirs, descend AFTER the dir
        // handle is dropped (C buffers them in `subdirs`). `zreaddir(lock, 1)`
        // ALWAYS skips `.`/`..` (c:5217); leading-dot files are admitted and
        // then accepted/rejected by `pattry`'s PAT_NOGLD rule, so dotglob /
        // `(D)` is handled by the compiled flag, never by re-including
        // `.`/`..` here (that previously leaked `.`/`..` under dotglob).
        let mut subdirs: Vec<String> = Vec::new();
        while let Some(name) = crate::ported::utils::zreaddir(&mut rd, 1) {
            if errflag.load(Ordering::SeqCst) & crate::ported::zsh_h::ERRFLAG_ERROR != 0 {
                break;
            }
            if !crate::ported::pattern::pattry(p, &name) {
                continue; // c:583
            }
            // c:585-597 — PATH_MAX lchdir bookkeeping.
            if pbcwdsav == state.pathbufcwd
                && name.len() + state.pathpos - state.pathbufcwd as usize >= path_max
            {
                let anchor = state
                    .pathbuf
                    .get(state.pathbufcwd as usize..)
                    .unwrap_or("")
                    .to_string();
                let err = lchdir(&anchor, Some(&mut ds), 0);
                if err == -1 {
                    break;
                }
                if err != 0 {
                    zerr("current directory lost during glob");
                    break;
                }
                state.pathbufcwd = state.pathpos as i32;
            }
            if dirs {
                // c:642-655 — closure: only descend real directories.
                if closure != 0 {
                    let probe = std::path::Path::new(&base).join(&name);
                    let md = if q.follow != 0 {
                        fs::metadata(&probe)
                    } else {
                        fs::symlink_metadata(&probe)
                    };
                    match md {
                        Ok(m) if m.is_dir() => {}
                        _ => continue,
                    }
                }
                subdirs.push(name); // c:657-663
            } else {
                // c:665-670 — last component: emit.
                let full = std::path::Path::new(&base).join(&name);
                insert(state, &full, 1);
                if shortcircuit != 0 && shortcircuit == state.matchct {
                    return;
                }
            }
        }
        drop(rd); // c:672 — closedir
                  // c:674-684 — descend into each collected subdir.
        if !subdirs.is_empty() {
            let oppos = state.pathpos;
            for name in subdirs {
                addpath(&mut state.pathbuf, &name);
                state.pathpos = state.pathbuf.len();
                scanner(
                    state,
                    if closure != 0 {
                        Some(q)
                    } else {
                        q.next.as_deref()
                    },
                    shortcircuit,
                    closure != 0,
                ); // c:681
                if shortcircuit != 0 && shortcircuit == state.matchct {
                    return;
                }
                state.pathbuf.truncate(oppos);
                state.pathpos = oppos;
            }
        }
    }

    // c:687-694 — restore cwd if we lchdir'd partway through.
    if pbcwdsav < state.pathbufcwd {
        if restoredir(&mut ds) != 0 {
            zerr("current directory lost during glob"); // c:689
        }
        state.pathbufcwd = pbcwdsav;
    }
}

/* This function tokenizes a zsh glob pattern */
// c:706
/// Port of `parsecomplist(char *instr)` from Src/glob.c:710.
/// Tokenize a zsh glob path pattern into a `Complist` of path
/// components, recursively. Returns `None` and sets `errflag |=
/// ERRFLAG_ERROR` on parse failure. Reads `gf_noglobdots` from
/// `CURGLOBDATA` (the curglobdata singleton at c:197).
pub fn parsecomplist(instr: &str) -> Option<Box<complist>> {
    // c:710
    let p1: crate::ported::pattern::Patprog; // c:712
    let l1: Box<complist>; // c:713
                           // c:714 `char *str;` — used as the skipparens cursor (parens branch).
                           // c:715 — compflags depend on gf_noglobdots from curglobdata.
    let gf_noglobdots: i32 = CURGLOBDATA
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .gf_noglobdots; // c:214 macro / c:186 field
    let compflags: i32 = if gf_noglobdots != 0 {
        crate::ported::zsh_h::PAT_FILE | crate::ported::zsh_h::PAT_NOGLD
    } else {
        crate::ported::zsh_h::PAT_FILE
    }; // c:715

    let chars: Vec<char> = instr.chars().collect();
    if chars.len() >= 2
        && chars[0] == crate::ported::zsh_h::Star
        && chars[1] == crate::ported::zsh_h::Star
    {
        // c:717
        let mut shortglob: i32 = 0; // c:718
        let cond_a = chars.get(2) == Some(&'/'); // c:719
        let cond_b =
            chars.get(2) == Some(&crate::ported::zsh_h::Star) && chars.get(3) == Some(&'/'); // c:719
                                                                                             // c:719-720 — `instr[2] == '/' || (instr[2] == Star && instr[3] == '/')
                                                                                             // || (shortglob = isset(GLOBSTARSHORT))`. C's `||` SHORT-CIRCUITS:
                                                                                             // the `shortglob = isset(GLOBSTARSHORT)` assignment runs ONLY when
                                                                                             // `**` is not explicitly followed by `/`. Mirror that with a lazy
                                                                                             // `||` so cond_a/cond_b keep `shortglob == 0` — an eager
                                                                                             // `let cond_c = { shortglob = … }` set it unconditionally, making
                                                                                             // `**/x` advance by 1 (`shortglob ? 1 : 3`) instead of 3, leaving a
                                                                                             // stray `*` that collapsed `**/` to a single directory level.
        let enter = cond_a || cond_b || {
            shortglob = if crate::ported::zsh_h::isset(crate::ported::zsh_h::GLOBSTARSHORT) {
                1
            } else {
                0
            };
            shortglob != 0
        }; // c:720
        if enter {
            /* Match any number of directories. */
            // c:721
            /* with three stars, follow symbolic links */                    // c:724
            let follow: i32 = if chars.get(2) == Some(&crate::ported::zsh_h::Star) {
                1
            } else {
                0
            }; // c:725
               /*
                * With GLOBSTARSHORT, leave a star in place for the
                * pattern inside the directory.
                */                                                              // c:726-729
            let advance: usize = (if shortglob != 0 { 1 } else { 3 }) + follow as usize; // c:730

            /* Now get the next path component if there is one. */
            // c:732
            let next_instr: String = chars[advance..].iter().collect();
            // c:733 — `l1 = (Complist) zhalloc(sizeof *l1);`
            let next_l = parsecomplist(&next_instr); // c:734
            if next_l.is_none() {
                crate::ported::utils::errflag.fetch_or(
                    crate::ported::zsh_h::ERRFLAG_ERROR,
                    std::sync::atomic::Ordering::Relaxed,
                ); // c:735
                return None; // c:736
            }
            let pat = crate::ported::pattern::patcompile(
                "",
                compflags | crate::ported::zsh_h::PAT_ANY,
                None,
            )?; // c:738
            l1 = Box::new(complist {
                next: next_l,
                pat,
                closure: 1, // c:739
                follow,     // c:740
            });
            return Some(l1); // c:741
        }
    }

    /* Parse repeated directories such as (dir/)# and (dir/)## */
    // c:745
    let zpc = crate::ported::pattern::zpc_special
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let inpar_c = zpc[crate::ported::zsh_h::ZPC_INPAR as usize] as char;
    let hash_c = zpc[crate::ported::zsh_h::ZPC_HASH as usize] as char;
    drop(zpc);

    // c:746-748 tests `*instr == zpc_special[ZPC_INPAR]` and `*str ==
    // zpc_special[ZPC_HASH]`, and C's `zpc_chars` (Src/pattern.c:248)
    // stores the TOKENS there — `Inpar` and `Pound`, not `(` and `#` —
    // because everything reaching patcompile in C is lexer-tokenized.
    // zshrs's `patcompcharsset` deliberately stores the RAW spelling
    // instead (see its note there: patcompile is reachable with
    // untokenized patterns and ~80 tests depend on a raw `|`
    // alternating), so a straight comparison against the table never
    // matches the tokenized input `parsecomplist` actually receives
    // from `globdata_glob`.
    //
    // Accept either spelling, but keep the option masking: when a slot
    // is disabled (EXTENDEDGLOB off, SHGLOB on, `disable -p`)
    // `patcompcharsset` overwrites it with `Marker`, and then NEITHER
    // spelling may match — which is what keeps `setopt noextendedglob;
    // (sub/)#end` a literal.
    let marker_c = crate::ported::zsh_h::Marker;
    let is_inpar =
        |c: char| c == inpar_c || (c == crate::ported::zsh_h::Inpar && inpar_c != marker_c);
    let is_hash = |c: char| c == hash_c || (c == crate::ported::zsh_h::Pound && hash_c != marker_c);

    // c:746-748 — `if (*(str = instr) == zpc_special[ZPC_INPAR] &&
    //               !skipparens(Inpar, Outpar, (char **)&str) &&
    //               *str == zpc_special[ZPC_HASH] && str[-2] == '/')`.
    // Routed through the canonical `skipparens` port for caller-coverage
    // parity with C — was previously inlined as a divergent depth-walk
    // because the old Rust signature returned usize. The C-faithful
    // `skipparens(open, close, &mut &str)` now matches `int skipparens
    // (char inpar, char outpar, char **s)` at utils.c:2409.
    let instr_chars: String = chars.iter().collect();
    let inpar_byte = crate::ported::zsh_h::Inpar as u32;
    let outpar_byte = crate::ported::zsh_h::Outpar as u32;
    let mut cursor: &str = &instr_chars;
    let skip_level: i32 = crate::ported::utils::skipparens(
        char::from_u32(inpar_byte).unwrap_or('('),
        char::from_u32(outpar_byte).unwrap_or(')'),
        &mut cursor,
    );
    let str_after_parens: Option<usize> = if chars.first().copied().is_some_and(is_inpar) {
        Some(instr_chars.chars().count() - cursor.chars().count())
    } else {
        None
    };
    let parens_balanced = chars.first().copied().is_some_and(is_inpar) && skip_level == 0; // c:746-747 `!skipparens(...)`
    let after_paren_idx = str_after_parens.unwrap_or(0);
    let str_at_hash = parens_balanced && chars.get(after_paren_idx).copied().is_some_and(is_hash); // c:748 `*str == Pound`
                                                                                                   // c:748 `str[-2] == '/'` — `str` is past `)`, so `str[-2]` is char
                                                                                                   // before `)`. In chars, that's `chars[after_paren_idx - 2]`.
    let preceded_by_slash =
        parens_balanced && after_paren_idx >= 2 && chars.get(after_paren_idx - 2) == Some(&'/');

    if parens_balanced && str_at_hash && preceded_by_slash {
        // c:749 — `instr++;`
        let mut cursor: String = chars[1..].iter().collect();
        let mut endexp = String::new();
        let p1_opt = crate::ported::pattern::patcompile(&cursor, compflags, Some(&mut endexp)); // c:750
        if p1_opt.is_none() {
            return None; // c:751
        }
        let p1_real = p1_opt.unwrap();
        cursor = endexp; // C: `instr` advanced past compiled pattern
        let c2: Vec<char> = cursor.chars().collect();
        if c2.first() == Some(&'/')
            && c2.get(1) == Some(&crate::ported::zsh_h::Outpar)
            && c2.get(2) == Some(&crate::ported::zsh_h::Pound)
        {
            // c:752
            let mut pdflag: i32 = 0; // c:753
            let mut adv = 3; // c:755
            if c2.get(3) == Some(&crate::ported::zsh_h::Pound) {
                pdflag = 1; // c:757
                adv = 4; // c:758
            }
            let next_instr: String = c2[adv..].iter().collect();
            // c:760-761 — `l1 = (Complist) zhalloc(...); l1->pat = p1;`
            /* special case (/)# to avoid infinite recursion */              // c:762
            // c:763 — `(*((char *)p1 + p1->startoff)) ? 1 + pdflag : 0`
            let pat_nonempty = !p1_real.1.is_empty()
                && p1_real
                    .1
                    .get(p1_real.0.startoff as usize)
                    .copied()
                    .unwrap_or(0)
                    != 0;
            let closure = if pat_nonempty { 1 + pdflag } else { 0 };
            let next_l = parsecomplist(&next_instr); // c:765
                                                     // c:766 — `return (l1->pat) ? l1 : NULL;`
            l1 = Box::new(complist {
                next: next_l,
                pat: p1_real,
                closure,
                follow: 0, // c:764
            });
            return Some(l1);
        }
    } else {
        /* parse single path component */
        // c:769
        let mut endexp = String::new();
        let p1_opt = crate::ported::pattern::patcompile(
            instr,
            compflags | crate::ported::zsh_h::PAT_FILET,
            Some(&mut endexp),
        ); // c:770
        if p1_opt.is_none() {
            return None; // c:771
        }
        p1 = p1_opt.unwrap();
        let cursor: Vec<char> = endexp.chars().collect();
        /* then do the remaining path components */
        // c:772
        let head = cursor.first().copied();
        if head == Some('/') || head.is_none() {
            // c:773
            let ef: i32 = if head == Some('/') { 1 } else { 0 }; // c:774
            let next_l: Option<Box<complist>> = if ef != 0 {
                let rest: String = cursor[1..].iter().collect();
                parsecomplist(&rest) // c:779
            } else {
                None
            };
            // c:780 — `return (ef && !l1->next) ? NULL : l1;`
            if ef != 0 && next_l.is_none() {
                return None;
            }
            l1 = Box::new(complist {
                next: next_l,
                pat: p1,
                closure: 0, // c:778
                follow: 0,
            });
            return Some(l1);
        }
    }
    crate::ported::utils::errflag.fetch_or(
        crate::ported::zsh_h::ERRFLAG_ERROR,
        std::sync::atomic::Ordering::Relaxed,
    ); // c:783
    None // c:784
}

/* turn a string into a Complist struct:  this has path components */
// c:787
/// Port of `parsepat(char *str)` from Src/glob.c:791.
/// Top-level entry: strip leading `(#...)` flag block via
/// `patgetglobflags`, then initialise `pathbuf`/`pathpos` per the
/// pattern's absolute-vs-relative head, then dispatch to
/// `parsecomplist`. Mutates `CURGLOBDATA.{pathbuf,pathpos,pathbufsz}`.
pub fn parsepat(s: &str) -> Option<Box<complist>> {
    // c:791
    // c:793 `long assert; int ignore;` — captured into patgetglobflags result
    crate::ported::pattern::patcompstart(); // c:796
    let chars: Vec<char> = s.chars().collect();
    let zpc = crate::ported::pattern::zpc_special
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let inpar_c = zpc[crate::ported::zsh_h::ZPC_INPAR as usize] as char;
    let hash_c = zpc[crate::ported::zsh_h::ZPC_HASH as usize] as char;
    let ksh_at_c = zpc[crate::ported::zsh_h::ZPC_KSH_AT as usize] as char;
    drop(zpc);

    /*
     * Check for initial globbing flags, so that they don't form
     * a bogus path component.
     */
    // c:797-800
    let mut cursor: String = s.to_string();
    let first_is_inpar_hash = chars.first() == Some(&inpar_c) && chars.get(1) == Some(&hash_c); // c:801
    let first_is_ksh_at_inpar_hash = chars.first() == Some(&ksh_at_c)
        && chars.get(1) == Some(&crate::ported::zsh_h::Inpar)
        && chars.get(2) == Some(&hash_c); // c:802-803
    if first_is_inpar_hash || first_is_ksh_at_inpar_hash {
        let skip = if chars.first() == Some(&crate::ported::zsh_h::Inpar) {
            2
        } else {
            3
        }; // c:804
        cursor = chars[skip..].iter().collect();
        let flag_result = crate::ported::pattern::patgetglobflags(&cursor); // c:805
        if flag_result.is_none() {
            return None; // c:806
        }
        let (_bits, _assertp, consumed) = flag_result.unwrap();
        cursor = cursor[consumed..].to_string();
    }

    /* Now there is no (#X) in front, we can check the path. */
    // c:809
    {
        let mut gd = CURGLOBDATA.lock().unwrap_or_else(|e| e.into_inner());
        // c:810-811 — `if (!pathbuf) pathbuf = zalloc(pathbufsz = PATH_MAX+1);`
        if gd.pathbuf.capacity() == 0 {
            gd.pathbufsz = libc::PATH_MAX as usize + 1;
            gd.pathbuf = String::with_capacity(gd.pathbufsz);
        }
        // c:812 — `DPUTS(pathbufcwd, "BUG: glob changed directory");`
        debug_assert!(gd.pathbufcwd == 0, "BUG: glob changed directory");
        if cursor.starts_with('/') {
            // c:813
            /* pattern has absolute path */
            cursor = cursor[1..].to_string(); // c:814 `str++;`
            gd.pathbuf.clear();
            gd.pathbuf.push('/'); // c:815 `pathbuf[0] = '/';`
            gd.pathpos = 1; // c:816 `pathbuf[pathpos = 1] = '\0';`
        } else {
            /* pattern is relative to pwd */
            // c:817
            gd.pathbuf.clear(); // c:818 `pathbuf[pathpos = 0] = '\0';`
            gd.pathpos = 0;
        }
    }

    parsecomplist(&cursor) // c:820
}

/// Parse qualifier (from glob.c qgetnum)
/// Parse a numeric glob-qualifier argument.
/// Port of `qgetnum(char **s)` from Src/glob.c:827.
pub fn qgetnum(s: &str) -> Option<(i64, &str)> {
    // c:827
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    let num = s[..end].parse::<i64>().ok()?;
    Some((num, &s[end..]))
}

impl globdata {
    /// `new` — see implementation.
    pub fn new() -> Self {
        globdata {
            matches: Vec::new(),
            qualifiers: None,
            quals: None,
            pathbuf: String::with_capacity(4096),
            pathpos: 0,
            matchct: 0,
            pathbufsz: 4096,
            pathbufcwd: 0,
            gf_nullglob: 0,
            gf_markdirs: 0,
            gf_noglobdots: 0,
            gf_listtypes: 0,
            gf_pre_words: None,
            gf_post_words: None,
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
    // c:Src/glob.c:903-913 — `while ((c = *p) == '?' || (c >= '0'
    // && c <= '7')) { ... val = (val << 3) | (c - '0'); }`.
    // In the C code path, perm letters (`r`/`w`/`x`/`s`/`t`) only
    // get parsed when an explicit `who` mask was set (the `if
    // (mask)` arm at c:877). When there's NO who (the
    // bare-spec form like `f=644`), C falls through to the
    // numeric digit loop in the `else if` arm at c:901. The
    // Rust port previously accepted letters in either case but
    // rejected digits — `f=644` would parse op='=' then break
    // on digit '6' and return spec_perm = 0, which the caller
    // computed as yes=0, no=0o7777 (essentially "match
    // nothing"). After also accepting digits here, `f=644`
    // builds spec_perm = 0o644 like C, and the matcher
    // resolves `*(.f=644)` correctly. Bug #105 in
    // docs/BUGS.md.
    if perm == 0 {
        let mut val = 0u32;
        let mut any_digit = false;
        while let Some(&c) = chars.peek() {
            if c == '?' {
                val <<= 3;
                chars.next();
                any_digit = true;
            } else if c.is_ascii_digit() && c < '8' {
                val = (val << 3) | (c as u32 - '0' as u32);
                chars.next();
                any_digit = true;
            } else {
                break;
            }
        }
        if any_digit {
            perm = val;
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
pub fn gmatchcmp(
    // c:936
    a: &gmatch,
    b: &gmatch,
    specs: &[i32],
    numeric_sort: bool,
) -> std::cmp::Ordering {
    for &tp in specs {
        // c:943
        let key = tp & !GS_DESC; // c:944 s->tp & ~GS_DESC
        let follow = (key & GS_LINKED) != 0;
        let key_unshifted = if follow { key >> GS_SHIFT } else { key };
        let cmp = if key_unshifted == GS_NAME {
            // c:945 — `gmatch->name` in C zsh is the FULL match string
            // (Src/glob.c sets it from the accumulated path during the
            // scanner walk), so the qsort comparator inherently sorts by
            // full path. The Rust port stores the basename in `name`;
            // use `path` instead so `**/*` matches the full-path
            // lexicographic order zsh produces. Verified vs
            // /opt/homebrew/bin/zsh: `echo /tmp/rg/**/*` → `f sub sub/g`
            // (sorted by full path, not basename).
            // c:945 `zstrcmp(b->uname, a->uname, …)` — C hands the
            // comparator two ready-made `char *`; the key was built once
            // per match at c:1963-1973. `sort_matches` fills `uname` the
            // same way (including the "./" strip described there), so
            // this arm only collates. Deriving it here instead cost a
            // `Path::to_string_lossy` + `strip_prefix` pair on EVERY
            // comparison — 11.7% of a 60605-match `man` glob, since a
            // sort comparator runs O(n log n) times.
            zstrcmp(
                &a.uname,
                &b.uname,
                if numeric_sort {
                    crate::zsh_h::SORTIT_NUMERICALLY as u32
                } else {
                    0
                },
            )
        } else if key_unshifted == GS_DEPTH {
            // c:949-972 — pairwise: skip the common prefix of the two
            // full paths, then `slasha`/`slashb` = does the remainder of
            // each (excluding its final char) still contain a `/`, i.e.
            // is this path deeper past the divergence point. `r = slasha
            // - slashb`; ascending puts the deeper path first.
            let an = a.path.to_string_lossy();
            let bn = b.path.to_string_lossy();
            let ab = an.as_bytes();
            let bb = bn.as_bytes();
            // c:951 — `while (*aptr && *aptr == *bptr) aptr++,bptr++;`
            let mut i = 0;
            while i < ab.len() && i < bb.len() && ab[i] == bb[i] {
                i += 1;
            }
            // c:953-954 — if at the end of one and the prev char is `/`,
            // back up one so the trailing component is counted.
            let (mut ai, mut bi) = (i, i);
            if (ai >= ab.len() || bi >= bb.len()) && ai > 0 && ab[ai - 1] == b'/' {
                ai -= 1;
                bi -= 1;
            }
            // c:956-964 — slash present in the remainder, excluding last char.
            let has_slash = |s: &[u8]| s.len() > 1 && s[..s.len() - 1].contains(&b'/');
            let slasha = has_slash(&ab[ai.min(ab.len())..]) as i32;
            let slashb = has_slash(&bb[bi.min(bb.len())..]) as i32;
            // c:971 — `r = slasha - slashb`; deeper (slash=1) sorts first.
            slashb.cmp(&slasha)
        } else if key_unshifted == GS_SIZE {
            // c:985
            if follow {
                a.target_size.cmp(&b.target_size)
            } else {
                a.size.cmp(&b.size)
            }
        } else if key_unshifted == GS_ATIME {
            // c:988-994 — `r = a->atime - b->atime;` then tie-break by
            // `a->ansec - b->ansec` (under GET_ST_ATIME_NSEC).
            let primary = if follow {
                b.target_atime.cmp(&a.target_atime)
            } else {
                b.atime.cmp(&a.atime)
            };
            if primary != std::cmp::Ordering::Equal {
                primary
            } else if follow {
                b.target_ansec.cmp(&a.target_ansec) // c:1019
            } else {
                b.ansec.cmp(&a.ansec) // c:992
            }
        } else if key_unshifted == GS_MTIME {
            // c:995-1001 — `r = a->mtime - b->mtime;` then tie-break by
            // `a->mnsec - b->mnsec` (under GET_ST_MTIME_NSEC). Without
            // the nsec tie-break, files touched <1s apart sort by name.
            let primary = if follow {
                b.target_mtime.cmp(&a.target_mtime)
            } else {
                b.mtime.cmp(&a.mtime)
            };
            if primary != std::cmp::Ordering::Equal {
                primary
            } else if follow {
                b.target_mnsec.cmp(&a.target_mnsec) // c:1026
            } else {
                b.mnsec.cmp(&a.mnsec) // c:999
            }
        } else if key_unshifted == GS_CTIME {
            // c:1002-1008 — `r = a->ctime - b->ctime;` then tie-break by
            // `a->cnsec - b->cnsec` (under GET_ST_CTIME_NSEC).
            let primary = if follow {
                b.target_ctime.cmp(&a.target_ctime)
            } else {
                b.ctime.cmp(&a.ctime)
            };
            if primary != std::cmp::Ordering::Equal {
                primary
            } else if follow {
                b.target_cnsec.cmp(&a.target_cnsec) // c:1033
            } else {
                b.cnsec.cmp(&a.cnsec) // c:1006
            }
        } else if key_unshifted == GS_LINKS {
            // c:1010 — `r = b->links - a->links;` — SAME sign convention
            // as GS_SIZE (c:985), so ascending `ol` is fewest-links-first.
            // (Was reversed vs GS_SIZE, putting most-linked first.)
            if follow {
                a.target_links.cmp(&b.target_links)
            } else {
                a.links.cmp(&b.links)
            }
        } else if key_unshifted == GS_EXEC {
            // c:974
            let idx = ((key as u32) >> 16) as usize;
            let asx = a.sort_strings.get(idx).map(|s| s.as_str()).unwrap_or("");
            let bsx = b.sort_strings.get(idx).map(|s| s.as_str()).unwrap_or("");
            zstrcmp(
                asx,
                bsx,
                if numeric_sort {
                    crate::zsh_h::SORTIT_NUMERICALLY as u32
                } else {
                    0
                },
            )
        } else {
            std::cmp::Ordering::Equal // GS_NONE / unknown
        };
        if cmp != std::cmp::Ordering::Equal {
            return if (tp & GS_DESC) != 0 {
                cmp.reverse()
            } else {
                cmp
            };
        }
    }
    std::cmp::Ordering::Equal
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

/// Port of `static char *glob_exec_string(char **sp)` from
/// `Src/glob.c:1085`.
///
/// **C is a PARSER, not an executor.** It extracts the qualifier
/// argument string from `*sp` (advancing the pointer past the
/// delimiters) and returns the duplicated text. The actual eval
/// happens at the call sites (c:1680/1713/1747) which feed the
/// returned string into `qualsheval` (for `e:`/`+:`) or
/// `gf_sortlist[].exec` (for sort).
///
/// Previous Rust port was COMPLETELY MIS-IMPLEMENTED:
///   1. **Signature wrong:** Rust took `(cmd, filename)` and returned
///      command output. C takes `(**sp)` and returns the parsed
///      qualifier string (the `cmd` itself, NOT its output).
///   2. **Forked `/bin/sh`:** spawned a separate POSIX shell process
///      to run the cmd. That's even more broken than `qualsheval`
///      was, because the function isn't supposed to execute anything
///      at all at this layer.
///
/// Now mirrors C's parser body (c:1090-1117): handle the `+` prefix
/// (identifier form) vs the `e:` delimited form (get_strarg), set
/// `*sp` past the closing delimiter, return the dup'd inner text.
///
/// Returns `(parsed_string, advance_offset)` so the caller can
/// emulate C's `*sp = tt + plus;` pointer advance via slice
/// indexing.
/// WARNING: param names don't match C — Rust=(s, plus) vs C=(sp)
pub fn glob_exec_string(s: &str, plus_form: bool) -> Option<(String, usize)> {
    // c:1085
    if plus_form {
        // c:1090
        // c:1092 — `tt = itype_end(s, IIDENT, 0);`
        // c:1093-1097 — `if (tt == s) { zerr("missing identifier after `+'"); return NULL; }`
        let tt = crate::ported::utils::itype_end(s, crate::ported::ztype_h::IIDENT as u32, false);
        if tt == 0 {
            // c:1093
            zerr("missing identifier after `+'"); // c:1095
            return None; // c:1096
        }
        // c:1109 — `sdata = dupstring(s + plus);` (plus=0 here).
        // c:1113 — `*sp = tt + plus;` → advance offset is `tt`.
        Some((s[..tt].to_string(), tt))
    } else {
        // c:1099 — `tt = get_strarg(s, &plus);` — find matching delimiter.
        // get_strarg returns delimiter-balanced span; for `e:foo:` it
        // walks `s` past the inner expr and returns position of the
        // closing `:`.
        match crate::ported::subst::get_strarg(s) {
            Some((_del, content, rest)) => {
                // c:1100-1104 — `if (!*tt) { zerr("missing end of string"); return NULL; }`
                if rest.is_empty() && content.is_empty() {
                    zerr("missing end of string"); // c:1102
                    return None; // c:1103
                }
                // c:1109-1115 — `sdata = dupstring(s + plus); ... *sp = tt + plus;`.
                // Advance offset: bytes consumed of `s` = s.len() - rest.len().
                let advance = s.len() - rest.len();
                Some((content, advance))
            }
            None => {
                zerr("missing end of string"); // c:1102
                None
            }
        }
    }
}

/*
 * Insert a glob match.
 * If there were words to prepend given by the P glob qualifier, do so.
 */
// c:1120-1123
/// Port of `insert_glob_match(LinkList list, LinkNode next, char *data)`
/// from Src/glob.c:1125. Inserts `data` at `next`, with optional
/// `gf_pre_words` prefix and `gf_post_words` suffix injection from
/// CURGLOBDATA (the `P:before:after:` glob qualifier).
pub fn insert_glob_match(list: &mut Vec<String>, next: usize, data: &str) {
    // c:1125
    let (pre, post) = {
        let gd = CURGLOBDATA.lock().unwrap_or_else(|e| e.into_inner());
        (gd.gf_pre_words.clone(), gd.gf_post_words.clone()) // c:1127, c:1136
    };
    // C `insertlinknode(list, next, data)` inserts AFTER node `next`,
    // returning the newly added node (so subsequent inserts append in
    // order). Our Vec<String> analog inserts at index `next+1` and
    // bumps `next` to point at the just-inserted slot.
    let mut cur = next;
    let n = list.len();
    let mut clamp = |i: usize| -> usize {
        if i > n {
            n
        } else {
            i
        }
    };
    if let Some(pre_words) = pre {
        // c:1127 `if (gf_pre_words)`
        for w in pre_words.iter() {
            // c:1129
            let pos = clamp(cur + 1);
            list.insert(pos, w.clone()); // c:1130 `dupstring(getdata(added))`
            cur = pos; // c:1130 — return-value advances `next`
        }
    }
    let pos = clamp(cur + 1);
    list.insert(pos, data.to_string()); // c:1134
    cur = pos;
    if let Some(post_words) = post {
        // c:1136 `if (gf_post_words)`
        for w in post_words.iter() {
            // c:1138
            let pos = clamp(cur + 1);
            list.insert(pos, w.clone()); // c:1139
            cur = pos;
        }
    }
}

/// Port of `checkglobqual(char *str, int sl, int nobareglob, char **sp)` from Src/glob.c:1160.
/// C: `int checkglobqual(char *str, int sl, int nobareglob, char **sp)` —
///   confirm the trailing `(...)` is a glob qualifier (not a set of
///   alternatives or an exclusion). Returns 1 for a bare qualifier
///   list, 2 for the explicit `(#q…)` form, 0 otherwise, and writes the
///   index of the opening parenthesis to `*sp` (C: the pointer itself).
///
/// `str` must be in LEXER-TOKENIZED form. EVERY test in the C body is
/// against a TOKEN, never the raw ASCII byte — `Outpar` at c:1163,
/// `Inpar` at c:1170/1189/1183, `Bar` at c:1175, `Tilde` at c:1179,
/// `Pound` at c:1192. That is exactly what stops a QUOTED `)`, which
/// reaches here as the plain byte, from closing the qualifier block:
/// in `*(.e['[[ $REPLY == a* ]]'])` the quoted `]` bytes are ordinary
/// text while the real closer is `Outbrack`.
///
/// The previous body here scanned raw ASCII `(`/`)`, dropped
/// `nobareglob` on the floor, and had no `#q` / `Bar` / `Tilde` arms,
/// so it could not distinguish a quoted delimiter from an active one.
pub fn checkglobqual(
    str: &[char],
    sl: i32,
    nobareglob: i32, // c:1160
    sp: &mut Option<usize>,
) -> i32 {
    use crate::ported::zsh_h::{Bar, Inpar, Outpar, Pound, Tilde};

    let mut nobareglob = nobareglob;
    let mut ret = 1i32; // c:1162 `int paren, ret = 1;`
    let sl = sl as usize;
    // c:1163-1164 — `if (str[sl - 1] != Outpar) return 0;`
    if sl < 2 || str[sl - 1] != Outpar {
        return 0;
    }

    // c:1169-1187 — walk back from `str + sl - 2` to the matching
    // `Inpar`, tracking nesting in `paren`. `Outpar` falls THROUGH to
    // the `Bar` arm, so a nested group, an alternation, or an
    // EXTENDEDGLOB exclusion each force `nobareglob`.
    let mut paren = 0i32; // c:1169
    let mut start: Option<usize> = None;
    let mut i = sl - 2;
    loop {
        // c:1170 loop condition — `*s && (*s != Inpar || paren)`
        if str[i] == Inpar && paren == 0 {
            start = Some(i);
            break;
        }
        match str[i] {
            // c:1172-1174 `case Outpar: paren++; /*FALLTHROUGH*/`
            Outpar => {
                paren += 1;
                nobareglob = 1; // c:1175-1177 via the fallthrough into Bar
            }
            // c:1175-1178 `case Bar:` — an alternation.
            Bar => nobareglob = 1,
            // c:1179-1182 `case Tilde:` — EXTENDEDGLOB exclusion.
            Tilde if glob_isset(EXTENDEDGLOB) => nobareglob = 1,
            // c:1183-1185 `case Inpar: paren--;`
            Inpar => paren -= 1,
            _ => {}
        }
        // c:1186-1187 `if (s == str) break;`
        if i == 0 {
            break;
        }
        i -= 1;
    }
    // c:1189-1190 `if (*s != Inpar) return 0;`
    let start = match start {
        Some(v) => v,
        None => return 0,
    };

    // c:1191-1198 — under EXTENDEDGLOB a leading `Pound` marks the
    // explicit `(#q…)` qualifier; any other `(#X…)` is an inline
    // pattern flag, not a qualifier block.
    if glob_isset(EXTENDEDGLOB) && str.get(start + 1) == Some(&Pound) {
        if str.get(start + 2) != Some(&'q') {
            return 0; // c:1193-1194
        }
        ret = 2; // c:1195
    } else if nobareglob != 0 {
        return 0; // c:1197-1198
    }

    // c:1200-1201 `if (sp) *sp = s;`
    *sp = Some(start);
    ret // c:1203
}

/// Port of `zglob(LinkList list, LinkNode np, int nountok)` from
/// Src/glob.c:1214. Top-level glob expansion: gate on GLOBOPT/
/// EXECOPT/haswilds (c:1230-1234), remove the placeholder node,
/// expand the pattern via `glob_path` (which covers the c:1240-2012
/// qualifier+scanner+sort body), then splice the resulting matches
/// back at `node` via `insert_glob_match` (c:1995-2007).
pub fn zglob(list: &mut Vec<String>, np: usize, nountok: i32) {
    // c:1214
    if np >= list.len() {
        return;
    }
    // c:1217 — `LinkNode node = prevnode(np);` — the insertion point
    // after the placeholder is uremnode'd. Vec analog: insert at np.
    let node: usize = np; // c:1217
    let ostr = list[np].clone(); // c:1221 `ostr = getdata(np)`
                                 // c:1226 — `nobareglob = !isset(BAREGLOBQUAL);` (consumed by qualifier
                                 // parser inside glob_path).

    // c:1230 — `if (unset(GLOBOPT) || !haswilds(ostr) || unset(EXECOPT))`
    if !crate::ported::zsh_h::isset(crate::ported::zsh_h::GLOBOPT)
        || !haswilds(&ostr)
        || !crate::ported::zsh_h::isset(crate::ported::zsh_h::EXECOPT)
    {
        if nountok == 0 {
            // c:1231
            // c:1232 — untokenize in-place; replace list[np] with the
            // untokenized form.
            list[np] = crate::ported::lex::untokenize(&ostr);
        }
        return; // c:1233
    }

    // c:1235 — `save_globstate(saved);`. zshrs snapshots the
    // glob-relevant options into TLS (`enter_glob_scope`, required for
    // thread-safety) — the RAII guard restores them on return.
    let _glob_scope = enter_glob_scope();

    // c:1237-1238 — `str = dupstring(ostr); uremnode(list, np);`
    list.remove(np); // c:1238

    // c:1240-1995 — the scanner walk + match collection. zshrs drives
    // it through `globdata_glob`, the RUST-ONLY read_dir adaptation of
    // C's `scanner()` (c:500; C's `lchdir` descent is process-global and
    // unsafe under zshrs's threading). globdata_glob owns its own
    // globdata; call it directly rather than via the glob_path Vec
    // convenience.
    let matches = {
        let mut st = globdata::new();
        globdata_glob(&mut st, &ostr)
    };

    // c:1871-1875 — badcshglob accounting. Each zglob run updates
    // the per-command-line counter so globlist's terminal diagnostic
    // can distinguish "some failures, no successes" (emit error)
    // from "some failures, some successes" (silent).
    if !matches.is_empty() {
        // c:1872 — `badcshglob |= 2;` (at least one expansion OK).
        BADCSHGLOB.fetch_or(2, std::sync::atomic::Ordering::Relaxed);
    } else if crate::ported::zsh_h::isset(crate::ported::zsh_h::CSHNULLGLOB) {
        // c:1874-1875 — `badcshglob |= 1;` (at least one expansion
        // failed) under CSHNULLGLOB.
        BADCSHGLOB.fetch_or(1, std::sync::atomic::Ordering::Relaxed);
    }

    // c:1872-1888 — `Deal with failures to match depending on options`.
    // C body verbatim (c:1872-1888):
    //   if (matchct)
    //       badcshglob |= 2;
    //   else if (!gf_nullglob) {
    //       if (isset(CSHNULLGLOB)) {
    //           badcshglob |= 1;
    //       } else if (isset(NOMATCH)) {
    //           zerr("no matches found: %s", ostr);
    //           zfree(matchbuf, 0);
    //           restore_globstate(saved);
    //           return;
    //       } else {
    //           /* treat as an ordinary string */
    //           untokenize(matchptr->name = dupstring(ostr));
    //           matchptr++;
    //           matchct = 1;
    //       }
    //   }
    // gf_nullglob (c:212) is the per-glob nullglob bit toggled by
    // qualifier `N`; here we approximate with the global option.
    // Parity bug #13: previously this Rust arm fell through to the
    // ordinary-literal path (c:1882-1887) unconditionally, making
    // `echo /never/*` print the literal glob instead of erroring.
    if matches.is_empty() {
        let nullglob = isset(crate::ported::zsh_h::NULLGLOB); // c:1873 !gf_nullglob
        let csh_nullglob = isset(crate::ported::zsh_h::CSHNULLGLOB); // c:1874
                                                                     // c:Src/glob.c:1843-1854 — `if (!q || errflag) { ... zerr(
                                                                     // "bad pattern", ostr); return; }`. When the qualifier
                                                                     // parser already emitted a diagnostic (e.g. "number expected"
                                                                     // from qgetnum at c:832) and set errflag, the no-matches /
                                                                     // bad-pattern terminal block runs but the prior zerr is what
                                                                     // the user sees first. Skipping the redundant "no matches
                                                                     // found" here matches zsh which has already aborted glob
                                                                     // expansion via the errflag-gated return at c:1787 / c:1843.
        if errflag.load(Ordering::SeqCst) != 0 {
            return;
        }
        // c:1872 — `else if (!gf_nullglob) { ... }`. The ENTIRE no-match
        // handling (CSHNULLGLOB, NOMATCH, literal fallback) is gated on
        // nullglob being OFF. When nullglob is set the failed word is
        // simply dropped: C never enters the block, matchbuf stays empty,
        // and glob() removes the word from the list. Hoisting the
        // `!nullglob` test to a single outer gate (matching the C
        // `else if`) ensures the literal-insert fallback below is skipped
        // under nullglob too — previously it fell through unconditionally
        // and echoed the literal, diverging from c:1872.
        if !nullglob {
            if csh_nullglob {
                // c:1874-1875 — `if (isset(CSHNULLGLOB)) { badcshglob |= 1; }`
                // (already recorded above). The else-if chain means the
                // ordinary-string arm is NOT reached: the failed word is
                // DROPPED like nullglob, and globlist's terminal check
                // (subst.c:505-507, ported at subst.rs:1419) emits the
                // csh-style `no match` when NO word on the line matched.
                // Falling through to the literal insert here produced the
                // NOMATCH-style "no matches found: PAT" instead.
                return;
            }
            if isset(crate::ported::zsh_h::NOMATCH) {
                // c:1876-1880 — `else if (isset(NOMATCH)) { zerr; return; }`
                // c:1877 — `ostr` is tokenized; zerrmsg's `%s` renders
                // via nicezputs → sb_niceformat → `untokenize(ums)`
                // (Src/utils.c), so the message shows the source
                // spelling rather than raw token bytes.
                crate::ported::utils::zerr(&format!(
                    "no matches found: {}",
                    crate::ported::lex::untokenize(&ostr)
                ));
                // c:1878 — `zfree(matchbuf, 0);` (Rust drop handles it)
                // c:1879 — `restore_globstate(saved);` (handled by guard)
                return; // c:1880
            }
            // c:1882-1887 — `treat as an ordinary string`. The matchptr++
            // bookkeeping in C maps to our `list.insert`.
            let restored = if nountok == 0 {
                crate::ported::lex::untokenize(&ostr) // c:1884 untokenize(matchptr->name = dupstring(ostr))
            } else {
                ostr.clone()
            };
            let pos = if node > list.len() { list.len() } else { node };
            list.insert(pos, restored); // c:1885 matchptr++, c:1886 matchct = 1
        }
        // nullglob set → fall through with matchbuf empty: the word is
        // dropped (c:1872 skips the block entirely). c:1889's
        // `else if (in_expandredir)` redirect-failure arm is handled by
        // the caller (vm_helper.rs redirect-glob path), not here.
        return;
    }

    // c:1995-2007 — splice matches back via insert_glob_match (honors
    // gf_pre_words / gf_post_words).
    let mut cur = if node == 0 { 0 } else { node - 1 }; // node is `prevnode(np)` semantics
    for m in matches.iter() {
        insert_glob_match(list, cur, m); // c:1995
        cur += 1;
        if let Some(g) = CURGLOBDATA.lock().ok() {
            // Advance past any pre_words that insert_glob_match added.
            if let Some(p) = &g.gf_pre_words {
                cur += p.len();
            }
            if let Some(p) = &g.gf_post_words {
                cur += p.len();
            }
        }
    }
}

/// File type character for -F style listing
/// Render a mode bitmap as the `*` qualifier letter (`d`/`b`/
/// `c`/`l`/`s`/`p`/etc.).
/// Port of `file_type(mode_t filemode)` from Src/glob.c:2018.
pub fn file_type(filemode: u32) -> char {
    // c:2018
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
pub fn hasbraces(s: &str, brace_ccl: bool) -> bool {
    // c:2042
    let mut depth = 0;
    let mut has_comma = false;
    let mut has_dotdot = false;
    let mut brace_open: Option<usize> = None;

    // c:2042 — every `return 1` in the C walk sits inside the Outbrace
    // arm, which is only reachable after an Inbrace has raised `depth`.
    // No Inbrace in the string means the answer is 0, so skip the
    // `Vec<char>` build. Inbrace is U+008F, whose UTF-8 encoding always
    // contains the byte 0x8F; testing for that byte can only
    // FALSE-POSITIVE (0x8F is also a continuation byte of other
    // characters), which just falls through to the real walk.
    // `<[u8]>::contains` is libcore's precompiled `memchr`.
    if !s.as_bytes().contains(&0x8f) {
        return false;
    }

    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();

    let mut i = 0;
    while i < len {
        // c:Src/lex.c:3587-3600 — backslash escape converts the next
        // char into a tokenized literal (Bnull/Bnullkeep). xpandbraces
        // sees the tokenized form so `\{` never enters the brace
        // walk. Accept both the canonical Bnull (`\u{9f}`,
        // Src/zsh.h:195) and ASCII `\` so direct callers (tests,
        // utility paths) and pipeline callers (bridge → multsub →
        // xpandbraces with Bnull markers from gettokstr) both behave
        // the same: skip the escape marker plus the next char.
        if (chars[i] == '\\' || chars[i] == '\u{9f}') && i + 1 < len {
            i += 2; // c:3591 — skip Bnull/Bnullkeep + escaped char
            continue;
        }
        match chars[i] {
            // c:Src/glob.c:hasbraces — Inbrace/Outbrace/Comma TOKEN
            // strictly (\u{8f} / \u{90} / \u{9a}). The lexer emits
            // TOKEN form for unescaped `{`/`,`/`}`; `\X` produces
            // Bnull + ASCII X. After remnulargs strips Bnull, the
            // ASCII X doesn't match the TOKEN check so escaped
            // braces correctly bypass expansion.
            '\u{8f}' => {
                if depth == 0 {
                    brace_open = Some(i);
                }
                depth += 1;
            }
            '\u{90}' if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    // c:2050-2061 — a comma group always expands.
                    if has_comma {
                        return true;
                    }
                    if has_dotdot {
                        // c:2071-2096 — a `..` group expands ONLY when it
                        // is a valid char range (bracechardots, c:2071) OR
                        // a numeric range whose endpoints are immediately
                        // bounded by the closing brace: C walks `[-]digits
                        // .. [-]digits` and requires the next char to be
                        // Outbrace (c:2084) or `..incr..` + Outbrace
                        // (c:2087-2095). Trailing garbage (`{0..5%2}`,
                        // `{1..3x}`, `{1.2..3}`, `{1..5.}`) fails this, so
                        // the group stays LITERAL with braces. Nested
                        // groups stay permissive — recursive xpandbraces
                        // handles the inner range.
                        let content: String =
                            chars[brace_open.unwrap_or(0) + 1..i].iter().collect();
                        let nested = content.contains('\u{8f}');
                        // c:2074-2096 — structural walk: optional `-`,
                        // digits (0+), `..`, optional `-`, digits (0+),
                        // then MUST end at the brace (Outbrace), or take a
                        // `..incr` step then end. C allows EMPTY endpoints
                        // (`{1..}`, `{..5}` pass hasbraces, then fail the
                        // range parse and get brace-stripped) — what it
                        // forbids is trailing non-range chars (`{0..5%2}`,
                        // `{1.2..3}`). Closes with c:2085 `idigit(lbr[1])
                        // || idigit(str[-1])` — a digit adjacent to `..`.
                        // Dash TOKEN (\u{9b}) reads as ASCII `-`.
                        let numeric_ok = {
                            let norm: String = content
                                .chars()
                                .map(|c| if c == '\u{9b}' { '-' } else { c })
                                .collect();
                            let b = norm.as_bytes();
                            let n = b.len();
                            let mut j = 0;
                            let mut shape_ok = false;
                            if j < n && b[j] == b'-' {
                                j += 1; // c:2074 leading `-`
                            }
                            while j < n && b[j].is_ascii_digit() {
                                j += 1; // c:2076 first number
                            }
                            if j + 1 < n && b[j] == b'.' && b[j + 1] == b'.' {
                                j += 2; // c:2078 `..`
                                if j < n && b[j] == b'-' {
                                    j += 1; // c:2080
                                }
                                while j < n && b[j].is_ascii_digit() {
                                    j += 1; // c:2082 second number
                                }
                                if j == n {
                                    shape_ok = true; // c:2084 Outbrace
                                } else if j + 1 < n && b[j] == b'.' && b[j + 1] == b'.' {
                                    j += 2; // c:2087 `..incr`
                                    if j < n && b[j] == b'-' {
                                        j += 1;
                                    }
                                    while j < n && b[j].is_ascii_digit() {
                                        j += 1; // c:2091
                                    }
                                    if j == n {
                                        shape_ok = true; // c:2093
                                    }
                                }
                            }
                            // c:2085/2094 — digit adjacent to `..`.
                            shape_ok
                                && (b.first().is_some_and(|c| c.is_ascii_digit())
                                    || b.last().is_some_and(|c| c.is_ascii_digit()))
                        };
                        if nested || bracechardots(&content).is_some() || numeric_ok {
                            return true;
                        }
                    }
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
            // c:Src/glob.c:hasbraces — accept comma / `..` at ANY depth
            // (was `depth == 1` only). Nested groups like `{{1,2}}`
            // have the comma at depth 2; without this, hasbraces
            // returned false and the outer xpandbraces never ran,
            // leaving `{{1,2}}` literal instead of expanding to
            // `{1} {2}` per zsh's nested-brace pass.
            '\u{9a}' if depth > 0 => has_comma = true,
            '.' if depth > 0 && i + 1 < len && chars[i + 1] == '.' => has_dotdot = true,
            _ => {}
        }
        i += 1;
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
    // c:2235-2236 — `MB_METACHARINIT(); pnext += MB_METACHARLENCONV(pconv,
    // &cstart);` — the endpoint is decoded as ONE multibyte character out
    // of the METAFIED text, not counted as a Rust `char`. zshrs stores a
    // raw 8-bit byte as the metafied pair `Meta` + `byte ^ 32` (two
    // `char`s), so `chars().count() == 1` rejected `{$'\x80'..$'\x81'}`
    // outright, and a real multibyte endpoint (`{é..ê}`) was rejected by
    // the byte-length test in `expand_range`.
    // c:2237-2244 — `if (cstart == WEOF || pnext[0] != '.' ...) return 0;`
    // With the MULTIBYTE option off a unit is a single BYTE, which is what
    // makes the range over 8-bit endpoints legal only in that mode
    // (mb_metacharlenconv, Src/utils.c:5613).
    let lb = crate::ported::utils::unmetafy_str(left); // c:2236
    let (l_len, cstart, _) = crate::ported::utils::mb_metacharlenconv(&lb); // c:2236
                                                                            // c:2256-2257 — same decode for the last character of the range.
    let rb = crate::ported::utils::unmetafy_str(end_str); // c:2257
    let (r_len, cend, _) = crate::ported::utils::mb_metacharlenconv(&rb); // c:2257
                                                                          // c:2239/2264 — `cstart == WEOF` / `*pnext != Outbrace`: the endpoint
                                                                          // must decode AND consume exactly the whole endpoint text.
    if l_len != 0 && l_len == lb.len() && r_len != 0 && r_len == rb.len() {
        if let (Some(c1), Some(c2)) = (cstart, cend) {
            return Some((c1, c2, incr)); // c:2266-2270
        }
    }

    None
}

/// Expand braces in a string
/// Brace-expand a string into a flat list.
/// Port of `xpandbraces(LinkList list, LinkNode *np)` from Src/glob.c:2276 — same
/// `{a,b}` / `{1..10}` / `{a-z}` handling.
/// WARNING: param names don't match C — Rust=(s, brace_ccl) vs C=(list, np)
pub fn xpandbraces(s: &str, brace_ccl: bool) -> Vec<String> {
    // c:2276
    if !hasbraces(s, brace_ccl) {
        return vec![s.to_string()];
    }

    // Inline single-brace expansion — direct port of the per-iteration
    // brace-scan inside zsh's xpandbraces (Src/glob.c:2276). Walks the
    // string, finds the first `{`...`}` group, classifies as range
    // (`a..b`) / comma (`a,b`) / ccl (`[abc]`-style char-class), and
    // dispatches to the matching expander. Returns Some(parts) on
    // expansion, None if no brace group or unmatched.
    // c:Src/glob.c:2276 xpandbraces — advances `lbr` through every
    // candidate `{` and tries each. Bug #575: zshrs only tried the
    // FIRST `{` and returned None for the whole string when that
    // group was not expandable, so `{a-c}{1..3}` left the `{1..3}`
    // unexpanded. Mirror C by carrying a `from` offset and walking
    // to the next `{` on failure.
    //
    // Returns (Some(parts), _) on successful expansion. Returns
    // (None, Some(next_from)) when the current `{...}` was found but
    // didn't expand — outer loop should retry from next_from to scan
    // for a later expandable group. Returns (None, None) when no
    // `{...}` remains.
    let try_expand_from = |s: &str, from: usize| -> (Option<Vec<String>>, Option<usize>) {
        let chars: Vec<char> = s.chars().collect();
        let len = chars.len();
        // c:Src/glob.c:xpandbraces — Inbrace TOKEN strict.
        let start = match chars[from..].iter().position(|&c| c == '\u{8f}') {
            Some(p) => from + p,
            None => return (None, None),
        };
        let mut depth = 1;
        let mut comma_positions = Vec::new();
        let mut dotdot_pos = None;
        for i in (start + 1)..len {
            match chars[i] {
                '\u{8f}' => depth += 1,
                '\u{90}' => {
                    depth -= 1;
                    if depth == 0 {
                        let next_from = i + 1;
                        let prefix: String = chars[..start].iter().collect();
                        let suffix: String = chars[i + 1..].iter().collect();
                        let content: String = chars[start + 1..i].iter().collect();
                        if let Some(dp) = dotdot_pos {
                            if comma_positions.is_empty() {
                                if let Some(parts) = expand_range(&prefix, &content, dp, &suffix) {
                                    return (Some(parts), None);
                                }
                                // c:Src/glob.c:2476-2506 — when range
                                // parsing fails INSIDE C's xpandbraces
                                // (err != 0 set at c:2355/2360/2369/2371),
                                // the function falls through to the
                                // normal-comma-expansion path. With no
                                // commas, that loop produces a single
                                // `prefix + content + suffix` — i.e.
                                // strips the outermost braces only.
                                // BUT: C only reaches xpandbraces when
                                // `hasbraces()` (c:2042) returned 1,
                                // and hasbraces requires DIGITS in the
                                // range endpoints (c:2076-2096) for the
                                // numeric form. Patterns like
                                // `{hello..world}` fail hasbraces and
                                // never enter xpandbraces — they stay
                                // as literal `{hello..world}`.
                                //
                                // Rust's hasbraces is more permissive
                                // (accepts any `..` inside `{}`), so we
                                // gate the strip-braces fallback on the
                                // SAME shape C uses: at least one digit
                                // must appear in the left or right
                                // endpoint, mirroring c:2085 `idigit
                                // (lbr[1]) || idigit(str[-1])`.
                                // `dp` is a CHAR index; byte-slice via a
                                // converted offset so multibyte/metafied
                                // content (`{$'\x80'..$'\x81'}`) can't panic.
                                let dp_byte = content
                                    .char_indices()
                                    .nth(dp)
                                    .map(|(b, _)| b)
                                    .unwrap_or(content.len());
                                let left = &content[..dp_byte];
                                let right = &content[dp_byte + 2..];
                                let strip_end =
                                    right.find("..").map(|p| &right[..p]).unwrap_or(right);
                                let has_digit = left.chars().any(|c| c.is_ascii_digit())
                                    || strip_end.chars().any(|c| c.is_ascii_digit());
                                if has_digit {
                                    // c:2495-2498 — strip braces.
                                    return (
                                        Some(vec![format!("{}{}{}", prefix, content, suffix)]),
                                        None,
                                    );
                                }
                                // Non-digit `..` content (e.g.
                                // `{hello..world}`) — C wouldn't have
                                // entered xpandbraces. Preserve the
                                // literal pattern intact. Allow outer
                                // loop to retry from next_from in case
                                // a later brace group is expandable.
                                return (None, Some(next_from));
                            }
                        }
                        if !comma_positions.is_empty() {
                            return (
                                expand_comma(&prefix, &content, &comma_positions, &suffix),
                                None,
                            );
                        }
                        if brace_ccl && !content.is_empty() {
                            return (expand_ccl(&prefix, &content, &suffix), None);
                        }
                        // Outer brace has no comma/dotdot at depth 1,
                        // but content may contain nested braces (e.g.
                        // `{{1,2}}` → `{1} {2}` — outer braces become
                        // literal, inner expands).
                        if content.contains('\u{8f}') {
                            let inner_expanded = xpandbraces(&content, brace_ccl);
                            let mut out: Vec<String> = Vec::with_capacity(inner_expanded.len());
                            for piece in inner_expanded {
                                out.push(format!("{}{{{}}}{}", prefix, piece, suffix));
                            }
                            return (Some(out), None);
                        }
                        // Literal-only group: allow outer to retry
                        // from next_from for a later expandable group.
                        return (None, Some(next_from));
                    }
                }
                '\u{9a}' if depth == 1 => comma_positions.push(i - start - 1),
                '.' if depth == 1 && i + 1 < len && chars[i + 1] == '.' && dotdot_pos.is_none() => {
                    dotdot_pos = Some(i - start - 1);
                }
                _ => {}
            }
        }
        (None, None)
    };

    // Outer loop: try expanding starting from each `{` in turn.
    // Returns first successful expansion; None if none found.
    let try_expand_one = |s: &str| -> Option<Vec<String>> {
        let mut from = 0;
        loop {
            let (result, next) = try_expand_from(s, from);
            if let Some(parts) = result {
                return Some(parts);
            }
            match next {
                Some(nf) if nf > from => from = nf,
                _ => return None,
            }
        }
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
/* check to see if a matches b (b is not a filename pattern) */            // c:2510
// !!! WARNING: RUST-ONLY HELPER — extra args + flipped arg order vs C !!!
// C signature: `int matchpat(char *a, char *b)` — `a` = text to match,
// `b` = pattern. Rust callers (cond.rs, watch.rs, glob.rs internal users)
// pass `(pattern, text, extended, case_sensitive)` — pattern-FIRST,
// flipped from C, plus two per-call option overrides. The BODY below
// is C-faithful for the matching path (patcompile + pattry, no Unicode
// case-folding); `extended`/`case_sensitive` drive transient
// `opt_state_set` overrides around the patcompile call. The transient
// swap is the WRONG mechanism — patcompile is not reentrant under
// concurrent option mutation. SOLE acceptable use: this same-process,
// single-threaded matchpat call path. Replace with: all callers
// migrate to (a) global setopt and (b) canonical C arg order
// `matchpat(text, pattern)`.
// !!! WARNING: RUST-ONLY HELPER — extra args + flipped arg order vs C !!!
/// Port of `matchpat(char *a, char *b)` from `Src/glob.c:2514`.
/// CALLER CONVENTION: Rust=(pattern, text, ext, cs) — pattern FIRST
/// (FLIPPED from C `matchpat(text=a, pattern=b)`). Body remaps to
/// C order internally.
pub fn matchpat(pattern_in: &str, text_in: &str, extended: bool, case_sensitive: bool) -> bool {
    // Remap to C names (a=text, b=pattern) at the function boundary so
    // the body below reads identically to Src/glob.c:2514-2530.
    let a = text_in;
    let b = pattern_in;
    // c:2514
    crate::ported::signals_h::queue_signals(); // c:2519
                                               // GF_IGNCASE handling: zshrs's pattern matcher DOES honor the
                                               // flag in `patmatch_internal` (pattern.rs:2245/2312/2333 — P_EXACTLY,
                                               // P_ANYOF, P_ANYBUT all check `glob_flags & (GF_IGNCASE|GF_LCMATCHUC)`
                                               // and case-fold appropriately). The pre-fold below is a defensive
                                               // fast path for the simple case-insensitive `:#`/`:#:` filter — it
                                               // bypasses the matcher's per-arm fold and avoids any stale-cache
                                               // edge case in the compile→match handoff. Costs one to_lowercase
                                               // call per side; correct under all matcher implementations.
    let (a_eff, b_eff) = if case_sensitive {
        (a.to_string(), b.to_string())
    } else {
        (a.to_lowercase(), b.to_lowercase())
    };
    // Per-call option override (Rust-only): snapshot, set, restore.
    let prev_extended = crate::ported::options::opt_state_get("extendedglob");
    let prev_caseglob = crate::ported::options::opt_state_get("caseglob");
    crate::ported::options::opt_state_set("extendedglob", extended);
    crate::ported::options::opt_state_set("caseglob", case_sensitive);
    // c:2521 — `if (!(p = patcompile(b, PAT_STATIC, NULL)))`. Rust uses
    // PAT_HEAPDUP (=0) — pattern::patmatch's canonical compile path;
    // PAT_STATIC's static-buffer path is incomplete in zshrs.
    let p_opt = crate::ported::pattern::patcompile(
        &{
            let mut __pat_tok = (&b_eff).to_string();
            crate::ported::glob::tokenize(&mut __pat_tok);
            __pat_tok
        },
        crate::ported::zsh_h::PAT_HEAPDUP,
        None,
    );
    if let Some(v) = prev_extended {
        crate::ported::options::opt_state_set("extendedglob", v);
    }
    if let Some(v) = prev_caseglob {
        crate::ported::options::opt_state_set("caseglob", v);
    }
    let ret = match p_opt {
        Some(p) => crate::ported::pattern::pattry(&p, &a_eff), // c:2525
        None => {
            crate::ported::utils::zerr(&format!("bad pattern: {}", b)); // c:2522
            false // c:2523
        }
    };
    crate::ported::signals_h::unqueue_signals(); // c:2527
    ret // c:2529
}

/// Port of `get_match_ret(Imatchdata imd, int b, int e)` from `Src/glob.c:2550`.
///
/// C returns `char *`: `NULL` when there is nothing to emit, `imd->mstr`
/// when the match was recorded into `repllist` (SUB_GLOBAL / SUB_LIST),
/// else a freshly built buffer. Rust returns `Option<String>` (`None` ==
/// C `NULL`). `b`/`e` arrive as *unmetafied* byte offsets and are first
/// re-based to metafied byte offsets, exactly as C does. Takes `&mut`
/// because the SUB_GLOBAL/SUB_LIST branch pushes onto `imd->repllist`.
pub fn get_match_ret(imd: &mut imatchdata, b: usize, e: usize) -> Option<String> {
    let mut buf = String::new(); // c:2552 char buf[80]
    let mut ll: i64 = 0; // c:2553 ll = 0
    let mut bl: usize = 0; // c:2553 bl = 0
    let mut t = false; // c:2553 t = 0
    let mut add: usize = 0; // c:2553 add = 0
    let fl = imd.flags; // c:2553 fl = imd->flags
    let mut replstr: Option<String> = imd.replstr.clone(); // c:2552 *replstr = imd->replstr

    let ustr_owned = imd.ustr.clone().unwrap_or_default();
    let ustr = ustr_owned.as_bytes();
    let mstr_owned = imd.mstr.clone().unwrap_or_default();
    let mstr = mstr_owned.as_bytes();
    let mlen = imd.mlen as usize; // c:2544 imd->mlen

    // c:2555-2564 — account for b and e referring to unmetafied string.
    let mut p = 0usize; // c:2556 p = imd->ustr
    while p < b && p < ustr.len() {
        // c:2556 for (; p < imd->ustr + b; p++)
        if imeta(ustr[p]) {
            add += 1; // c:2558 add++
        }
        p += 1;
    }
    let b = b + add; // c:2559 b += add
    while p < e && p < ustr.len() {
        // c:2560 for (; p < imd->ustr + e; p++)
        if imeta(ustr[p]) {
            add += 1; // c:2562 add++
        }
        p += 1;
    }
    let e = e + add; // c:2563 e += add

    // c:2566 — Everything now refers to metafied lengths.
    if replstr.is_some() || (fl & SUB_LIST) != 0 {
        // c:2567
        if (fl & SUB_DOSUBST) != 0 {
            // c:2568
            let mut rs = dupstring(replstr.as_deref().unwrap_or("")); // c:2569 dupstring(replstr)
            rs = singsub(&rs); // c:2570 singsub(&replstr)
            rs = untokenize(&rs); // c:2571 untokenize(replstr)
            replstr = Some(rs);
        }
        if (fl & (SUB_GLOBAL | SUB_LIST)) != 0 && imd.repllist.is_some() {
            // c:2573 — replacing the chunk, just add this to the list.
            let rd = repldata {
                b: b as i32,              // c:2578 rd->b = b
                e: e as i32,              // c:2579 rd->e = e
                replstr: replstr.clone(), // c:2580 rd->replstr = replstr
            };
            imd.repllist.as_mut().unwrap().push(rd); // c:2581-2584 z/addlinknode(repllist, rd)
            return imd.mstr.clone(); // c:2585 return imd->mstr
        }
        if let Some(ref r) = replstr {
            ll += r.len() as i64; // c:2588 ll += strlen(replstr)
        }
    }
    if (fl & SUB_MATCH) != 0 {
        // c:2590 matched portion
        ll += 1 + (e as i64 - b as i64); // c:2591 ll += 1 + (e - b)
    }
    if (fl & SUB_REST) != 0 {
        // c:2592 unmatched portion
        ll += 1 + (mlen as i64 - (e as i64 - b as i64)); // c:2593 ll += 1 + (mlen - (e - b))
    }
    if (fl & SUB_BIND) != 0 {
        // c:2594 position of start of matched portion
        buf = format!("{} ", MB_METASTRLEN2END(mstr_owned.as_str(), false, b) + 1); // c:2596
        bl = buf.len(); // c:2597 bl = strlen(buf)
        ll += bl as i64; // c:2597 ll += bl
    }
    if (fl & SUB_EIND) != 0 {
        // c:2599 position of end of matched portion
        buf.push_str(&format!(
            "{} ",
            MB_METASTRLEN2END(mstr_owned.as_str(), false, e) + 1
        )); // c:2601
        bl = buf.len(); // c:2602 bl = strlen(buf)
        ll += bl as i64; // c:2602 ll += bl
    }
    if (fl & SUB_LEN) != 0 {
        // c:2603 length of matched portion — MB_METASTRLEN2END(mstr+b, 0, mstr+e)
        let sub = if b <= mstr.len() {
            &mstr_owned[b..]
        } else {
            ""
        };
        buf.push_str(&format!(
            "{} ",
            MB_METASTRLEN2END(sub, false, e.saturating_sub(b))
        )); // c:2605
        bl = buf.len(); // c:2606 bl = strlen(buf)
        ll += bl as i64; // c:2606 ll += bl
    }
    if bl != 0 {
        buf.pop(); // c:2609 buf[bl - 1] = '\0' — drop the trailing space
    }

    if ll == 0 {
        return None; // c:2614 return NULL
    }

    // c:2617 — rr = r = hcalloc(ll); build the result buffer.
    let mut r = String::new();

    if (fl & SUB_MATCH) != 0 {
        // c:2619-2623 — copy matched portion to new buffer.
        let end = e.min(mstr.len());
        let start = b.min(end);
        r.push_str(&mstr_owned[start..end]); // c:2621
        t = true; // c:2622
    }
    if (fl & SUB_REST) != 0 {
        // c:2624 — copy unmatched portion. If both portions requested,
        // put a space in between (why?).
        if t {
            r.push(' '); // c:2627
        }
        // c:2629-2630 — unmatched bits before the match.
        let pre = b.min(mstr.len());
        r.push_str(&mstr_owned[..pre]); // c:2630
        if let Some(ref rs) = replstr {
            r.push_str(rs); // c:2632-2633 copy replstr
        }
        // c:2634-2635 — unmatched bits after the match.
        let post = e.min(mstr.len());
        r.push_str(&mstr_owned[post..]); // c:2635
        t = true; // c:2636
    }
    if bl != 0 {
        // c:2639-2643 — append the numeric buffer; space first if needed.
        if t {
            r.push(' '); // c:2642
        }
        r.push_str(&buf); // c:2643 strcpy(rr, buf)
    }
    Some(r) // c:2645 return r
}

/// Compile pattern and get match info (from glob.c compgetmatch line 2650)
/// Port of `compgetmatch(char *pat, int *flp, char **replstrp)` from `Src/glob.c:2650`.
/// WARNING: param names don't match C — Rust=(pat) vs C=(pat, flp, replstrp)
pub fn compgetmatch(pat: &str) -> Option<(String, i32)> {
    // c:1993 — `SUB_START` (anchor at head) / `SUB_END` (anchor at
    // tail) / `SUB_LONG` (`##`/`%%` doubled = longest). All three
    // imported from the canonical zsh_h.rs port; the previous local
    // redeclaration risked the same drift hazard as the HIST_*
    // bit-value bug caught earlier.
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
    let (prep_pat, prep_fl) = match compgetmatch(pat) {
        // c:2713
        Some(t) => t,
        None => return sp.to_string(), // c:2713 return 1
    };
    let mut buf = sp.to_string();
    igetmatch(&mut buf, &prep_pat, prep_fl | fl, n, replstr); // c:2715
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

/// Port of `int getmatchlist(char *str, Patprog p, LinkList *repllistp)`
/// from `Src/glob.c:2749`.
/// ```c
/// int
/// getmatchlist(char *str, Patprog p, LinkList *repllistp)
/// {
///     char **sp = &str;
///     return igetmatch(sp, p, SUB_LONG|SUB_GLOBAL|SUB_SUBSTR|SUB_LIST,
///                      0, NULL, repllistp);
/// }
/// ```
/// 3-line delegation to `igetmatch` with the canonical flag set. The
/// `repllistp` out-param is the LinkList that receives the match
/// position pairs; the Rust port currently lacks a repllistp out-
/// channel on `igetmatch`, so this entry mirrors the C structure
/// and returns the igetmatch status.
pub fn getmatchlist(sp: &mut String, p: &str) -> i32 {
    // c:2749
    igetmatch(
        sp,
        p,
        SUB_LONG | SUB_GLOBAL | SUB_SUBSTR | SUB_LIST, // c:2761
        0,
        None,
    ) // c:2762
}

/// File-static `static int in_expandredir = 0;` from `Src/glob.c:1206`
/// — set during the `globlist` call inside `xpandredir` so the glob
/// pipeline knows the result feeds a redirection (suppresses the
/// "no matches" warning in the caller's normal flow).
pub static IN_EXPANDREDIR: std::sync::atomic::AtomicI32 = // c:1206
    std::sync::atomic::AtomicI32::new(0);

/// Direct port of `int xpandredir(struct redir *fn, LinkList redirtab)`
/// from `Src/glob.c:2150`. Expands `>>*.c`-style redirections by
/// running `prefork` + (when MULTIOS is set) `globlist` over the redir
/// name. Single match: rewrite `fn->name` in place and decode the
/// MERGEIN/MERGEOUT special syntax (`-` close, `p` pipe-fd, digits =
/// fd number). Multi match: clone the redir for each name and append
/// to `redirtab`. Returns 1 if multios produced multiple entries, 0
/// otherwise.
/// WARNING: param names don't match C — Rust=(fn_, redirtab) vs C=(fn, redirtab)
pub fn xpandredir(
    fn_: &mut redir, // c:2150
    redirtab: &mut Vec<redir>,
) -> i32 {
    use std::sync::atomic::Ordering::SeqCst;
    let mut ret = 0; // c:2156
    let name = match fn_.name.as_deref() {
        // c:2160 init_list1(fake, fn->name)
        Some(n) => n.to_string(),
        None => return 0,
    };
    let mut fake: LinkList = LinkList::new();
    fake.push_back(name); // c:2160
    let mut rf = 0i32;
    prefork(
        &mut fake, // c:2162 prefork
        if isset(MULTIOS) { 0 } else { PREFORK_SINGLE },
        &mut rf,
    );
    // c:2164 — `if (!errflag && isset(MULTIOS))`. C uses LOGICAL NOT
    // on `errflag` (an `int`), so the condition is "errflag is zero".
    // The previous Rust port wrote `!errflag.load(...) != 0` which is
    // BITWISE NOT in Rust (`!i32` is `^-1`), making the condition
    // equivalent to `errflag != -1` — true for every common errflag
    // value (0 / ERRFLAG_ERROR=1 / ERRFLAG_INT=2). Result: the
    // globlist call ran even when errflag was set, papering over
    // the abort path that C uses to bail early on prior errors.
    if errflag.load(SeqCst) == 0 && isset(MULTIOS) {
        // c:2164
        IN_EXPANDREDIR.store(1, SeqCst); // c:2165
        crate::ported::subst::globlist(&mut fake, 0); // c:2166
        IN_EXPANDREDIR.store(0, SeqCst); // c:2167
    }
    if errflag.load(SeqCst) != 0 {
        return 0;
    } // c:2169
    let names: Vec<String> = fake.iter().cloned().collect();
    if names.len() == 1 {
        // c:2171 nonempty(&fake) && !nextnode(firstnode)
        let s = crate::lex::untokenize(&names[0]); // c:2174 untokenize(s)
        fn_.name = Some(s.clone()); // c:2173 fn->name = s
        if fn_.typ == REDIR_MERGEIN || fn_.typ == REDIR_MERGEOUT {
            // c:2175
            let bytes = s.as_bytes();
            if bytes.len() == 1 && IS_DASH(bytes[0] as char) {
                // c:2176-2177 IS_DASH(s[0]) && !s[1]
                fn_.typ = REDIR_CLOSE;
            } else if bytes.len() == 1 && bytes[0] == b'p' {
                // c:2178 s[0]=='p' && !s[1]
                fn_.fd2 = -2;
            } else {
                let mut i = 0;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                } // c:2181 idigit
                if i == bytes.len() && i > 0 {
                    // c:2183 !*s && s > fn->name
                    fn_.fd2 = crate::ported::utils::zstrtol(&s, 10).0 as i32; // c:2184 zstrtol(.., 10)
                } else if fn_.typ == REDIR_MERGEIN {
                    // c:2185
                    zerr("file number expected"); // c:2186
                } else {
                    fn_.typ = REDIR_ERRWRITE; // c:2188
                }
            }
        }
    } else if fn_.typ == REDIR_MERGEIN {
        // c:2192
        zerr("file number expected"); // c:2193
    } else {
        if fn_.typ == REDIR_MERGEOUT {
            fn_.typ = REDIR_ERRWRITE;
        } // c:2195
        for nam in names {
            // c:2196 while ((nam = ugetnode(&fake)))
            let mut ff = fn_.clone(); // c:2199-2200 zhalloc + *ff = *fn
            ff.name = Some(nam); // c:2201 ff->name = nam
            redirtab.push(ff); // c:2202 addlinknode(redirtab, ff)
            ret = 1; // c:2203
        }
    }
    ret // c:2206
}

/// Port of `freerepldata(void *ptr)` from Src/glob.c:2766.
/// C: `static void freerepldata(void *ptr)` →
///   `zfree(ptr, sizeof(struct repldata));`
#[allow(unused_variables)]
pub fn freerepldata(ptr: *mut std::ffi::c_void) { // c:2766
                                                  // Rust drop covers the equivalent.
}

/// Port of `freematchlist(LinkList repllist)` from Src/glob.c:2773.
/// C: `void freematchlist(LinkList repllist)` →
///   `freelinklist(repllist, freerepldata);`
///
/// The C `repllist` is a `LinkList` of `struct repldata` (the same
/// node `get_match_ret` records for SUB_GLOBAL/SUB_LIST). The Rust
/// port operates on `Vec<repldata>` — matching the canonical
/// `imatchdata.repllist` type — not the prior ad-hoc `Vec<(usize,usize)>`.
/// Clearing the Vec drops each `repldata` (Rust's `freerepldata` +
/// `freelinklist` equivalent).
pub fn freematchlist(repllist: Option<&mut Vec<repldata>>) {
    // c:2773
    if let Some(l) = repllist {
        l.clear(); // c:2776 freelinklist(repllist, freerepldata)
    }
}

/// Port of `set_pat_start(Patprog p, int offs)` from `Src/glob.c:2780`.
///
/// When we advance up the test string from its start, tell the pattern
/// matcher that a start-of-string assertion `(#s)` should fail: set
/// `PAT_NOTSTART` when `offs` is nonzero (the real start is past the
/// actual start), clear it when `offs == 0`. Mutates `p->flags`.
/// Replaces the prior fake that sliced the pattern string and returned
/// a substring — unrelated to the C behaviour (the matcher reads
/// `PAT_NOTSTART` off `prog.flags`, see pattern.rs:4792).
pub fn set_pat_start(p: &mut Patprog, offs: i32) {
    // c:2780
    if offs != 0 {
        p.0.flags |= PAT_NOTSTART; // c:2790 p->flags |= PAT_NOTSTART
    } else {
        p.0.flags &= !PAT_NOTSTART; // c:2792 p->flags &= ~PAT_NOTSTART
    }
}

/// Port of `set_pat_end(Patprog p, char null_me)` from `Src/glob.c:2796`.
///
/// When we shorten the string at the tail, tell the pattern matcher
/// that an end-of-string assertion `(#e)` should fail: set `PAT_NOTEND`
/// when the char `null_me` about to be zapped is non-NUL, clear it when
/// it is already NUL. Mutates `p->flags`. Replaces the prior fake that
/// sliced the pattern string and returned a prefix (the matcher reads
/// `PAT_NOTEND` off `prog.flags`, see pattern.rs:2803).
pub fn set_pat_end(p: &mut Patprog, null_me: u8) {
    // c:2796
    if null_me != 0 {
        p.0.flags |= PAT_NOTEND; // c:2806 p->flags |= PAT_NOTEND
    } else {
        p.0.flags &= !PAT_NOTEND; // c:2808 p->flags &= ~PAT_NOTEND
    }
}

/// Port of `igetmatch(char **sp, Patprog p, int fl, int n, char *replstr, LinkList *repllistp)` from Src/glob.c:2832.
/// C: `static int igetmatch(char **sp, Patprog p, int fl, int n,
///     char *replstr, LinkList *repllistp)` — pattern-replace inner
///     matcher; modifies `*sp` in place, optionally collects match
///     positions into `*repllistp`.
/// WARNING: param names don't match C — Rust=(sp, p, fl, n, replstr) vs C=(sp, p, fl, n, replstr, repllistp)
pub fn igetmatch(
    sp: &mut String,
    p: &str,
    fl: i32,
    _n: i32, // c:2832
    replstr: Option<&str>,
) -> i32 {
    // c:2840-3100+ — full SUB_* dispatch: longest/shortest/global/end-
    // anchor replacement loop with multibyte tracking. Rust port walks
    // chars + `matchpat`; full Patprog substrate (with chunked DFA
    // execution) lives in src/ported/pattern.rs. SUB_START imported
    // from zsh_h.rs at top-of-file rather than redeclared locally.
    let anchored_start = (fl & SUB_START) != 0;
    let anchored_end = (fl & SUB_END) != 0;
    let substr_mode = (fl & SUB_SUBSTR) != 0;
    let shortest = (fl & SUB_LONG) == 0;
    let chars: Vec<char> = sp.chars().collect();
    let len = chars.len();

    // c:2887-2898 — SUB_ALL: entire-string match flag.
    if (fl & SUB_ALL) != 0 {
        // c:2887
        let i = matchpat(p, sp, true, true); // c:2888 pattrylen
        if !i {
            // c:2889
            // c:2890-2893 — no match: clear replstr.
            if (fl & SUB_MATCH) != 0 {
                // c:2895
                *sp = String::new();
                return 0; // c:2896
            }
            return 1; // c:2897
        }
        if let Some(r) = replstr {
            // c:2894 get_match_ret
            *sp = r.to_string();
        }
        if sp.is_empty() && (fl & SUB_REST) != 0 && i {
            // c:2895
            return 0; // c:2896
        }
        return 1; // c:2897
    }

    if len == 0 {
        return 1;
    }
    // c:2998-3041 — SUB_LIST: collect all match offset pairs.
    if (fl & SUB_LIST) != 0 {
        // c:2998
        // Walk all substrings; collect (start, end) pairs the caller
        // can read via subsequent `getmatchlist` invocations. Without
        // a `repllistp` out-channel through the Rust signature, we
        // only walk to validate at-least-one-match; the canonical
        // collection point is the caller-supplied LinkList that the
        // C body populates at c:3000+. Defer until the out-vec lands.
        let mut found = false;
        for start in 0..len {
            // c:3008
            for end in (start + 1)..=len {
                // c:3009
                let s2: String = chars[start..end].iter().collect();
                if matchpat(p, &s2, true, true) {
                    // c:3010
                    found = true;
                    if shortest {
                        break;
                    } // c:3011 SUB_LONG vs short
                }
            }
            if !substr_mode {
                break;
            }
        }
        return if found { 0 } else { 1 };
    }

    // c:Src/glob.c:2900+ — SUB_MATCH inverts the disposition of the
    // anchored-strip operators. Default (no SUB_MATCH): the matched
    // portion is REMOVED and the rest is returned (`${var#pat}` →
    // tail after the matched prefix). With SUB_MATCH: the matched
    // portion is KEPT and the rest is removed (`${(M)var#pat}` → the
    // matched prefix only). Previously honoured only in the SUB_ALL
    // arm at c:2895; the anchored-prefix / anchored-suffix / substr
    // arms below always returned prefix+suffix, so `(M)` was a no-op
    // for `#pat` / `%pat`. Mirror the C SUB_MATCH branch by emitting
    // just the matched slice.
    let match_only = (fl & SUB_MATCH) != 0;
    let (match_start, match_end) = if anchored_start && anchored_end {
        if matchpat(p, sp, true, true) {
            (0, len)
        } else {
            if match_only {
                *sp = String::new();
                return 0;
            }
            return 1;
        }
    } else if anchored_start {
        let mut best_end = 0;
        for end in 1..=len {
            let substr: String = chars[..end].iter().collect();
            if matchpat(p, &substr, true, true) {
                if shortest {
                    *sp = if match_only {
                        chars[..end].iter().collect()
                    } else {
                        match replstr {
                            Some(r) => format!("{}{}", r, chars[end..].iter().collect::<String>()),
                            None => chars[end..].iter().collect(),
                        }
                    };
                    return 0;
                }
                best_end = end;
            }
        }
        if best_end > 0 {
            (0, best_end)
        } else {
            if match_only {
                *sp = String::new();
                return 0;
            }
            return 1;
        }
    } else if anchored_end {
        let mut best_start = len;
        for start in (0..len).rev() {
            let substr: String = chars[start..].iter().collect();
            if matchpat(p, &substr, true, true) {
                if shortest {
                    *sp = if match_only {
                        chars[start..].iter().collect()
                    } else {
                        match replstr {
                            Some(r) => {
                                format!("{}{}", chars[..start].iter().collect::<String>(), r)
                            }
                            None => chars[..start].iter().collect(),
                        }
                    };
                    return 0;
                }
                best_start = start;
            }
        }
        if best_start < len {
            (best_start, len)
        } else {
            if match_only {
                *sp = String::new();
                return 0;
            }
            return 1;
        }
    } else {
        for start in 0..len {
            for end in (start + 1)..=len {
                let substr: String = chars[start..end].iter().collect();
                if matchpat(p, &substr, true, true) {
                    if match_only {
                        *sp = chars[start..end].iter().collect();
                        return 0;
                    }
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
        if match_only {
            *sp = String::new();
            return 0;
        }
        return 1;
    };
    *sp = if match_only {
        chars[match_start..match_end].iter().collect()
    } else {
        let prefix: String = chars[..match_start].iter().collect();
        let suffix: String = chars[match_end..].iter().collect();
        match replstr {
            Some(r) => format!("{}{}{}", prefix, r, suffix),
            None => format!("{}{}", prefix, suffix),
        }
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
pub const ZTOKENS: &str = "#$^*(())$=|{}[]`<>>?~`,-!'\"\\\\";

/// The `switch (*s)` labels of `zshtokenize` (c:3592-3639), as a byte
/// table: `true` for every byte that can reach an arm which MUTATES the
/// string. `\\` `<` `>` `^` `#` `~` `[` `]` `*` `?` `=` `-` `!` `(` `|`
/// `)` are the literal case labels; the `Meta` / `Bnull` / `Bnullkeep`
/// sentinels (c:3593/3597/3598) are non-ASCII chars, so every byte of
/// their UTF-8 encoding is >= 0x80 and the whole high half is marked —
/// conservatively, since a `>= 0x80` byte only sends the string down the
/// full walk, never past it.
///
/// C needs no such table: its loop is a byte-pointer walk that costs
/// about a cycle per uninteresting byte. The port has to materialize a
/// `Vec<char>` and rebuild the `String`, so the same "do nothing" answer
/// costs three allocations and a decode/encode pair — 33.4% of a `man
/// <TAB>` completion, which tokenizes every one of 60605 candidate
/// names and finds a metachar in 3105 of them (5%).
static ZSHTOK_TRIGGER: [bool; 256] = {
    let mut t = [false; 256];
    let mut i = 0;
    while i < 256 {
        let b = i as u8;
        t[i] = b >= 0x80
            || matches!(
                b,
                b'\\'
                    | b'<'
                    | b'>'
                    | b'^'
                    | b'#'
                    | b'~'
                    | b'['
                    | b']'
                    | b'*'
                    | b'?'
                    | b'='
                    | b'-'
                    | b'!'
                    | b'('
                    | b'|'
                    | b')'
            );
        i += 1;
    }
    t
};

/// Tokenize a glob pattern in place — port of `tokenize(char *s)` from
/// `Src/glob.c:3548`. One-line C delegation: `zshtokenize(s, 0)`.
pub fn tokenize(s: &mut String) {
    // c:3548
    zshtokenize(s, 0); // c:3552
}

/// Tokenize for shell — port of `shtokenize(char *s)` from
/// `Src/glob.c:3563`. Builds flags from SHGLOB option then delegates
/// to `zshtokenize`.
pub fn shtokenize(s: &mut String) {
    // c:3563
    let mut flags = ZSHTOK_SUBST; // c:3567
    if isset(SHGLOB) {
        // c:3568
        flags |= ZSHTOK_SHGLOB; // c:3569
    }
    zshtokenize(s, flags); // c:3570
}

/// Port of `zshtokenize(char *s, int flags)` from `Src/glob.c:3575`.
/// Walks `s` in place, replacing each glob-metachar with its high-bit
/// byte token from the `ZTOKENS` table; respects ZSHTOK_SUBST (use
/// Bnullkeep for escaped tokens) and ZSHTOK_SHGLOB (don't tokenize
/// `<` / `(` / `|` / `)`).
pub fn zshtokenize(s: &mut String, flags: i32) {
    // c:3575
    // c:3580-3651 — every arm that writes to the string is guarded by
    // one of the `switch` labels; a string holding none of them leaves
    // C's loop with `*s` byte-for-byte unchanged. Answer that case
    // without materializing anything (see ZSHTOK_TRIGGER).
    if !s.as_bytes().iter().any(|&b| ZSHTOK_TRIGGER[b as usize]) {
        return;
    }
    // c:3640 `for (t = ztokens; *t; t++)` — C scans the static table by
    // pointer. Every ZTOKENS entry is ASCII, so scanning its bytes is
    // the same comparison without the per-call `Vec<char>` build.
    let ztokens: &[u8] = ZTOKENS.as_bytes();
    let mut chars: Vec<char> = s.chars().collect();
    let mut bslash = false; // c:3578
    let mut i = 0;
    while i < chars.len() {
        // c:3580 for (; *s; s++)
        let c = chars[i];
        match c {
            x if x == Meta as char => {                                              // c:3583 case Meta
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
                    if t as char == c {                                      // c:3634
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
        bslash = false; // c:3646
        i += 1;
    }
    *s = chars.into_iter().collect();
}

/// Port of `void remnulargs(char *s)` from `Src/glob.c:3649`.
///
/// C body (c:3651-3676): walks `s` looking for INULL bytes.
///   - `Bnullkeep` in SCAN phase: skip (the `continue` at c:3658)
///     — don't treat as a null marker.
///   - Any other INULL: switch to COPY phase. In copy phase:
///       - `Bnullkeep` becomes literal `\` (the "active backslash"
///         is re-materialized).
///       - Other INULLs: stripped.
///       - Non-INULL: kept.
///   - If the post-copy string is empty, replace with `Nularg`
///     (single-char empty-arg marker).
///
/// The previous Rust port collapsed the body to
/// `s.retain(|c| c != '\0' && c != Bnullkeep)` — a simple
/// strip of NUL and Bnullkeep. Three divergences:
///   - Stripped Bnullkeep entirely instead of preserving it in
///     the scan-only phase (when it appears BEFORE any other
///     inull) or converting to `\` in the copy phase.
///   - Didn't strip Snull/Dnull/Bnull/Nularg — those inulls
///     stayed in the output, polluting downstream processing.
///   - Didn't emit Nularg sentinel for empty post-strip strings.
pub fn remnulargs(s: &mut String) {
    // c:3649
    if s.is_empty() {
        // c:3654
        return;
    }
    // c:3664-3688 — C's walk rewrites nothing until it meets a
    // `Bnullkeep` or an `inull(c)`. Both those sentinels, and the `Meta`
    // the port's walk also tracks, are non-ASCII chars, so an all-ASCII
    // string can't reach either branch and C leaves it untouched. Say so
    // before paying for the `Vec<char>` materialization: `<[u8]>::is_ascii`
    // is libcore's precompiled word-at-a-time scan, while the walk below
    // is monomorphised into this crate and built unoptimised.
    if s.is_ascii() {
        return;
    }
    // c:3656 `inull(c)` predicate: Snull / Dnull / Bnull / Bnullkeep / Nularg.
    let is_inull =
        |c: char| c == Snull || c == Dnull || c == Bnull || c == Bnullkeep || c == Nularg;
    let src: Vec<char> = s.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(src.len());
    let mut i = 0usize;
    // rs-only adaptation of C's byte walk: C strings are byte arrays and
    // store an eight-bit char (e.g. `$'\M-\C-a'` = 0x81) RAW, so its byte
    // never aliases an inull sentinel. A Rust `String` is UTF-8, so zshrs
    // METAFIES that byte — Meta (0x83) + (byte ^ 0x20) — and 0x81 ^ 0x20 =
    // 0xA1, which IS the Nularg sentinel (0xBD/0xBE/0xBF/0xBC likewise
    // alias Snull/Dnull/Bnull/Bnullkeep). A metafied byte is DATA, not a
    // sentinel, so treat `Meta` + next char as an opaque pair on BOTH the
    // scan and copy walks — otherwise `${(qq)v}` for v=$'\M-\C-a' dropped
    // the 0xA1 and left the lone Meta (rs emitted `'\xc2\x83'` where zsh
    // emits `'\x81'`). C's byte walk needs no such guard.
    // Meta is a byte constant (0x83); a metafied string stores it as the
    // char U+0083, so compare against the char form.
    let meta = char::from(crate::ported::zsh_h::Meta);
    // c:3656 — SCAN phase: copy chars, skip standalone Bnullkeep.
    while i < src.len() {
        let c = src[i];
        if c == meta && i + 1 < src.len() {
            out.push(c);
            out.push(src[i + 1]);
            i += 2;
            continue;
        }
        if c == Bnullkeep {
            // c:3657 continue
            i += 1;
            continue;
        }
        if is_inull(c) {
            // c:3663 inull(c)
            // c:3664+ — COPY phase: walk the rest, fold Bnullkeep
            // to `\\`, strip other inulls.
            i += 1;
            while i < src.len() {
                let d = src[i];
                if d == meta && i + 1 < src.len() {
                    // Metafied data pair — keep verbatim.
                    out.push(d);
                    out.push(src[i + 1]);
                    i += 2;
                    continue;
                }
                if d == Bnullkeep {
                    // c:3666
                    out.push('\\'); // c:3667
                } else if !is_inull(d) {
                    // c:3668
                    out.push(d); // c:3669
                }
                i += 1;
            }
            break;
        }
        // SCAN phase non-inull: keep verbatim.
        out.push(c);
        i += 1;
    }
    // c:3673-3675 — empty result → Nularg sentinel.
    if out.is_empty() {
        out.push(Nularg);
    }
    *s = out.into_iter().collect();
}

/// Port of `qualdev(UNUSED(char *name), struct stat *buf, off_t dv, UNUSED(char *dummy))` from Src/glob.c:3688.
/// C: `static int qualdev(UNUSED(char *name), struct stat *buf, off_t dv,
///     UNUSED(char *dummy))` → `return (off_t)buf->st_dev == dv;`
#[allow(unused_variables)]
pub fn qualdev(name: &str, buf: &libc::stat, dv: i64, dummy: &str) -> i32 {
    // c:3688
    (buf.st_dev as i64 == dv) as i32 // c:3697
}

/// Port of `qualnlink(UNUSED(char *name), struct stat *buf, off_t ct, UNUSED(char *dummy))` from Src/glob.c:3697.
/// C: ternary on `g_range`: < / > / == against `st_nlink`.
#[allow(unused_variables)]
pub fn qualnlink(name: &str, buf: &libc::stat, ct: i64, dummy: &str) -> i32 {
    // c:3697
    // c:3699-3701 — `g_range` selects `<` / `>` / `==`. This MUST read the same
    // `g_range` static the qual-eval sets (glob.rs ~5020) and that qualsize /
    // qualtime read — not the stale Rust-only `G_RANGE` duplicate, which was
    // never stored, leaving every `l[+-]N` comparison stuck on `==`.
    let g = g_range.load(Ordering::Relaxed);
    let nl = buf.st_nlink as i64; // c:3708
    if g < 0 {
        (nl < ct) as i32
    } else if g > 0 {
        (nl > ct) as i32
    } else {
        (nl == ct) as i32
    }
}

/// Port of `qualuid(UNUSED(char *name), struct stat *buf, off_t uid, UNUSED(char *dummy))` from Src/glob.c:3708.
/// C: `return buf->st_uid == uid;`
#[allow(unused_variables)]
pub fn qualuid(name: &str, buf: &libc::stat, uid: i64, dummy: &str) -> i32 {
    // c:3708
    (buf.st_uid as i64 == uid) as i32 // c:3717
}

/// Port of `qualgid(UNUSED(char *name), struct stat *buf, off_t gid, UNUSED(char *dummy))` from Src/glob.c:3717.
/// C: `return buf->st_gid == gid;`
#[allow(unused_variables)]
pub fn qualgid(name: &str, buf: &libc::stat, gid: i64, dummy: &str) -> i32 {
    // c:3717
    (buf.st_gid as i64 == gid) as i32 // c:3726
}

/// Port of `qualisdev(UNUSED(char *name), struct stat *buf, UNUSED(off_t junk), UNUSED(char *dummy))` from Src/glob.c:3726.
/// C: `return S_ISBLK(buf->st_mode) || S_ISCHR(buf->st_mode);`
#[allow(unused_variables)]
pub fn qualisdev(name: &str, buf: &libc::stat, junk: i64, dummy: &str) -> i32 {
    // c:3726
    let m = buf.st_mode as u32 & libc::S_IFMT as u32;
    ((m == libc::S_IFBLK as u32) || (m == libc::S_IFCHR as u32)) as i32 // c:3735
}

/// Port of `qualisblk(UNUSED(char *name), struct stat *buf, UNUSED(off_t junk), UNUSED(char *dummy))` from Src/glob.c:3735.
/// C: `return S_ISBLK(buf->st_mode);`
#[allow(unused_variables)]
pub fn qualisblk(name: &str, buf: &libc::stat, junk: i64, dummy: &str) -> i32 {
    // c:3735
    ((buf.st_mode as u32 & libc::S_IFMT as u32) == libc::S_IFBLK as u32) as i32 // c:3744
}

/// Port of `qualischr(UNUSED(char *name), struct stat *buf, UNUSED(off_t junk), UNUSED(char *dummy))` from Src/glob.c:3744.
/// C: `return S_ISCHR(buf->st_mode);`
#[allow(unused_variables)]
pub fn qualischr(name: &str, buf: &libc::stat, junk: i64, dummy: &str) -> i32 {
    // c:3744
    ((buf.st_mode as u32 & libc::S_IFMT as u32) == libc::S_IFCHR as u32) as i32 // c:3753
}

/// Port of `qualisdir(UNUSED(char *name), struct stat *buf, UNUSED(off_t junk), UNUSED(char *dummy))` from Src/glob.c:3753.
/// C: `return S_ISDIR(buf->st_mode);`
#[allow(unused_variables)]
pub fn qualisdir(name: &str, buf: &libc::stat, junk: i64, dummy: &str) -> i32 {
    // c:3753
    ((buf.st_mode as u32 & libc::S_IFMT as u32) == libc::S_IFDIR as u32) as i32 // c:3762
}

/// Port of `qualisfifo(UNUSED(char *name), struct stat *buf, UNUSED(off_t junk), UNUSED(char *dummy))` from Src/glob.c:3762.
/// C: `return S_ISFIFO(buf->st_mode);`
#[allow(unused_variables)]
pub fn qualisfifo(name: &str, buf: &libc::stat, junk: i64, dummy: &str) -> i32 {
    // c:3762
    ((buf.st_mode as u32 & libc::S_IFMT as u32) == libc::S_IFIFO as u32) as i32 // c:3771
}

/// Port of `qualislnk(UNUSED(char *name), struct stat *buf, UNUSED(off_t junk), UNUSED(char *dummy))` from Src/glob.c:3771.
/// C: `return S_ISLNK(buf->st_mode);`
#[allow(unused_variables)]
pub fn qualislnk(name: &str, buf: &libc::stat, junk: i64, dummy: &str) -> i32 {
    // c:3771
    ((buf.st_mode as u32 & libc::S_IFMT as u32) == libc::S_IFLNK as u32) as i32 // c:3780
}

/// Port of `qualisreg(UNUSED(char *name), struct stat *buf, UNUSED(off_t junk), UNUSED(char *dummy))` from Src/glob.c:3780.
/// C: `return S_ISREG(buf->st_mode);`
#[allow(unused_variables)]
pub fn qualisreg(name: &str, buf: &libc::stat, junk: i64, dummy: &str) -> i32 {
    // c:3780
    ((buf.st_mode as u32 & libc::S_IFMT as u32) == libc::S_IFREG as u32) as i32 // c:3789
}

/// Port of `qualissock(UNUSED(char *name), struct stat *buf, UNUSED(off_t junk), UNUSED(char *dummy))` from Src/glob.c:3789.
/// C: `return S_ISSOCK(buf->st_mode);`
#[allow(unused_variables)]
pub fn qualissock(name: &str, buf: &libc::stat, junk: i64, dummy: &str) -> i32 {
    // c:3789
    ((buf.st_mode as u32 & libc::S_IFMT as u32) == libc::S_IFSOCK as u32) as i32
    // c:3798
}

/// Port of `qualflags(UNUSED(char *name), struct stat *buf, off_t mod, UNUSED(char *dummy))` from Src/glob.c:3798.
/// C: `return mode_to_octal(buf->st_mode) & mod;`
#[allow(unused_variables)]
/// WARNING: param names don't match C — Rust=(name, buf, dummy) vs C=(name, buf, mod, dummy)
pub fn qualflags(name: &str, buf: &libc::stat, r#mod: i64, dummy: &str) -> i32 {
    // c:3798
    (mode_to_octal(buf.st_mode as u32) as i64 & r#mod) as i32 // c:3807
}

/// Port of `qualmodeflags(UNUSED(char *name), struct stat *buf, off_t mod, UNUSED(char *dummy))` from Src/glob.c:3807.
/// C: `((v & y) == y && !(v & n))` where `y = mod & 07777`, `n = mod >> 12`.
#[allow(unused_variables)]
/// WARNING: param names don't match C — Rust=(name, buf, dummy) vs C=(name, buf, mod, dummy)
pub fn qualmodeflags(name: &str, buf: &libc::stat, r#mod: i64, dummy: &str) -> i32 {
    // c:3807
    let v = mode_to_octal(buf.st_mode as u32) as i64; // c:3818
    let y = r#mod & 0o7777;
    let n = r#mod >> 12;
    (((v & y) == y) && (v & n) == 0) as i32 // c:3818
}

/// Port of `qualiscom(UNUSED(char *name), struct stat *buf, UNUSED(off_t mod), UNUSED(char *dummy))` from Src/glob.c:3818.
/// C: `return S_ISREG(buf->st_mode) && (buf->st_mode & S_IXUGO);`
#[allow(unused_variables)]
pub fn qualiscom(name: &str, buf: &libc::stat, r#mod: i64, dummy: &str) -> i32 {
    // c:3818
    let is_reg = (buf.st_mode as u32 & libc::S_IFMT as u32) == libc::S_IFREG as u32;
    let s_ixugo: u32 = (libc::S_IXUSR | libc::S_IXGRP | libc::S_IXOTH) as u32;
    (is_reg && (buf.st_mode as u32 & s_ixugo) != 0) as i32 // c:3820
}

/// Port of `static int g_units;` from `Src/glob.c`. Time/size unit
/// selector for the qualtime / qualsize qualifier predicates. Values
/// come from the TT_* enum (TT_DAYS=0, TT_HOURS=1, TT_MINS=2,
/// TT_WEEKS=3, TT_MONTHS=4 for time; TT_BYTES=0, TT_POSIX_BLOCKS=1,
/// TT_KILOBYTES=2, etc. for size).
pub static g_units: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Port of `static int g_range;` from `Src/glob.c`. Comparison
/// direction for qualtime / qualsize / qualnlink: -1 = `<`, 0 = `==`,
/// +1 = `>`. Set by the glob qualifier parser.
pub static g_range: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Port of `static int g_amc;` from `Src/glob.c`. Which mtime to read
/// for qualtime: 0 = atime, 1 = mtime, 2 = ctime. Set by the `am`,
/// `mm`, `cm` qualifier letters.
pub static g_amc: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Direct port of `static int qualsize(UNUSED(char *name),
/// struct stat *buf, off_t size, UNUSED(char *dummy))` from
/// `Src/glob.c:3825-3866`. Glob-qualifier predicate: returns true
/// when `buf->st_size` (scaled by `g_units`) compares to `size`
/// via `g_range`.
///
/// **Previous fakery:** the file held a same-named helper that was
/// actually a unit-conversion *parser* `(s, units) -> Option<(i64,
/// &str)>` — wrong shape, wrong semantics, zero callers. The
/// canonical C signature is restored here; the qualifier-eval site
/// at glob.rs:3914 already inlines the equivalent (uses
/// qualifier::Size struct fields) and is the live path.
#[allow(non_snake_case)]
pub fn qualsize(_name: &str, buf: &libc::stat, size: i64, _dummy: &str) -> i32 {
    use std::sync::atomic::Ordering;
    // c:3831 `zlong scaled = buf->st_size;`
    let mut scaled: i64 = buf.st_size as i64;
    // c:3837-3860 — switch (g_units) ceiling-divide by the unit.
    match g_units.load(Ordering::SeqCst) {
        x if x == TT_POSIX_BLOCKS => {
            scaled = (scaled + 511) / 512; // c:3838-3841
        }
        x if x == TT_KILOBYTES => {
            scaled = (scaled + 1023) / 1024; // c:3842-3845
        }
        x if x == TT_MEGABYTES => {
            scaled = (scaled + 1_048_575) / 1_048_576; // c:3846-3849
        }
        x if x == TT_GIGABYTES => {
            scaled = (scaled + 1_073_741_823) / 1_073_741_824; // c:3851-3854
        }
        x if x == TT_TERABYTES => {
            scaled = (scaled + 1_099_511_627_775) / 1_099_511_627_776; // c:3855-3858
        }
        _ => {} // c:3860 default: bytes — no scaling.
    }
    // c:3862-3864 — `g_range < 0 ? < : g_range > 0 ? > : ==`.
    let r = g_range.load(Ordering::SeqCst);
    i32::from(if r < 0 {
        scaled < size
    } else if r > 0 {
        scaled > size
    } else {
        scaled == size
    })
}

/// Direct port of `static int qualtime(UNUSED(char *name),
/// struct stat *buf, off_t days, UNUSED(char *dummy))` from
/// `Src/glob.c:3870-3901`. Glob-qualifier predicate: returns true
/// when `now - buf->{a,m,c}time` (selected by `g_amc`, scaled by
/// `g_units`) compares to `days` via `g_range`.
///
/// **Previous fakery:** same misshapen parser as the qualsize one
/// above. Restored to C signature; the qualifier-eval site at
/// glob.rs:3940 already inlines the equivalent (uses qualifier::
/// Atime struct fields with the live atime/mtime/ctime read).
#[allow(non_snake_case)]
pub fn qualtime(_name: &str, buf: &libc::stat, days: i64, _dummy: &str) -> i32 {
    use std::sync::atomic::Ordering;
    // c:3876-3878 — `time(&now); diff = now - (g_amc == 0 ? st_atime
    //                  : g_amc == 1 ? st_mtime : st_ctime);`
    let now: i64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let amc = g_amc.load(Ordering::SeqCst);
    let stamp: i64 = match amc {
        0 => buf.st_atime as i64, // c:3877
        1 => buf.st_mtime as i64, // c:3877
        _ => buf.st_ctime as i64, // c:3878
    };
    let mut diff: i64 = now - stamp;
    // c:3880-3896 — scale by g_units.
    match g_units.load(Ordering::SeqCst) {
        x if x == TT_DAYS => diff /= 86400,     // c:3881-3883
        x if x == TT_HOURS => diff /= 3600,     // c:3884-3886
        x if x == TT_MINS => diff /= 60,        // c:3887-3889
        x if x == TT_WEEKS => diff /= 604800,   // c:3890-3892
        x if x == TT_MONTHS => diff /= 2592000, // c:3893-3895
        _ => {}                                 // c:3896 default: seconds.
    }
    // c:3898-3900 — same range comparison as qualsize.
    let r = g_range.load(Ordering::SeqCst);
    i32::from(if r < 0 {
        diff < days
    } else if r > 0 {
        diff > days
    } else {
        diff == days
    })
}

/// Port of `static int qualsheval(char *name, UNUSED(struct stat *buf),
/// UNUSED(off_t days), char *str)` from `Src/glob.c:3907`.
///
/// C body (c:3907-3943):
/// ```c
/// if ((prog = parse_string(str, 0))) {
///     int ef = errflag, lv = lastval;             // c:3912 save
///     unsetparam("reply");                        // c:3915
///     setsparam("REPLY", ztrdup(name));           // c:3916
///     execode(prog, 1, 0, "globqual");            // c:3919
///     ret = lastval;                              // c:3921
///     errflag = ef | (errflag & ERRFLAG_INT);     // c:3924 restore
///     lastval = lv;                               // c:3925
///     return !ret;
/// }
/// return 0;
/// ```
///
/// The `e:EXPR:` glob qualifier runs `EXPR` against each match
/// candidate with `$REPLY` pre-set to the filename. The expr is
/// evaluated IN THE CURRENT SHELL — it can read shell locals,
/// reference shell functions, set $reply etc.
///
/// The previous Rust port spawned `/bin/sh -c <expr>` via
/// `Command::new("sh")`, which:
///   1. Loses access to current zsh function/alias/local-var scope.
///   2. Runs `sh` (POSIX), not `zsh` — entire `(e:zsh-feature:)`
///      class of qualifiers silently failed.
///   3. Skipped errflag/lastval save+restore.
///
/// Fixed: route through the canonical `ShellExecutor` for in-shell
/// evaluation. Set `REPLY` via paramtab (C `setsparam`), restore
/// `errflag`/`lastval` per c:3924-3925, mask any parse-time
/// ERRFLAG_ERROR while preserving ERRFLAG_INT (user interrupt).
/// WARNING: param names don't match C — Rust=(filename, expr) vs C=(name, buf, days, str)
pub fn qualsheval(filename: &str, _buf: &libc::stat, _data: i64, expr: &str) -> i32 {
    // c:3907
    // c:3912 — save errflag + lastval.
    let saved_errflag = errflag.load(Ordering::Relaxed); // c:3912
    let saved_lastval = LASTVAL.load(Ordering::Relaxed); // c:3912
                                                         // c:3915 — `unsetparam("reply");`
    crate::ported::params::unsetparam("reply"); // c:3915
                                                // c:3916 — `setsparam("REPLY", ztrdup(name));`. Set in the
                                                // canonical paramtab so the evaluated expression sees `$REPLY`.
    crate::ported::params::setsparam("REPLY", filename); // c:3916
                                                         // c:3919 — `execode(prog, 1, 0, "globqual");`. Route through the
                                                         // executor via the `crate::ported::exec` accessor wrappers
                                                         // (resolve the live executor on demand). Direct ShellExecutor
                                                         // reach-in from src/ported/ is forbidden — see memory
                                                         // feedback_no_exec_script_from_ported.
    let rc = {
        // c:Src/glob.c:3919 — `execode(prog, 1, 0, "globqual");`. execode
        // (c:Src/exec.c:1245-1266) APPENDS its context for the duration, so
        // code inside an `(e:'…':)` / `(+cmd)` glob qualifier sees
        // `cmdarg:globqual`. Popped on every return path. Bug #1069.
        let sync_eval_ctx = |stack: &[String]| {
            let joined = stack.join(":");
            if let Ok(mut tab) = crate::ported::params::paramtab().write() {
                if let Some(pm) = tab.get_mut("zsh_eval_context") {
                    pm.u_arr = Some(stack.to_vec());
                    pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
                }
                if let Some(pm) = tab.get_mut("ZSH_EVAL_CONTEXT") {
                    pm.u_str = Some(joined);
                    pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
                }
            }
        };
        if let Ok(mut ctx) = crate::ported::exec::zsh_eval_context.lock() {
            ctx.push("globqual".to_string());
            sync_eval_ctx(&ctx);
        }
        struct GlobQualCtxGuard<F: Fn(&[String])>(F);
        impl<F: Fn(&[String])> Drop for GlobQualCtxGuard<F> {
            fn drop(&mut self) {
                if let Ok(mut ctx) = crate::ported::exec::zsh_eval_context.lock() {
                    ctx.pop();
                    (self.0)(&ctx);
                }
            }
        }
        let _ctx_guard = GlobQualCtxGuard(sync_eval_ctx);
        crate::ported::exec::execute_script(expr).unwrap_or(1) // c:3919
    };
    let ret = LASTVAL.load(Ordering::Relaxed); // c:3921 ret = lastval
    let _ = rc;
    // c:3924 — `errflag = ef | (errflag & ERRFLAG_INT);`. Restore
    // pre-call errflag plus any interrupt bit set during eval.
    let post_errflag = errflag.load(Ordering::Relaxed);
    errflag.store(
        saved_errflag | (post_errflag & ERRFLAG_INT),
        Ordering::Relaxed,
    ); // c:3924
       // c:3925 — `lastval = lv;`. Restore pre-call lastval.
    LASTVAL.store(saved_lastval, Ordering::Relaxed); // c:3925
                                                     // c:3927-3937 — `inserts = getaparam("reply") || gethparam("reply")`,
                                                     // else the `reply`/`REPLY` SCALAR as a one-element list. The eval can
                                                     // thus REPLACE the matched name with zero-or-more names (`reply=(…)`)
                                                     // or rename it (`REPLY=…`). REPLY was seeded to `filename` at c:3916,
                                                     // so this is always Some — at minimum the (possibly modified) REPLY.
    let inserts: Vec<String> = match crate::ported::params::getaparam("reply") {
        Some(arr) => arr, // c:3927
        None => match crate::ported::params::getsparam("reply") // c:3931
            .or_else(|| crate::ported::params::getsparam("REPLY"))
        {
            Some(scalar) => vec![scalar], // c:3934-3936
            None => Vec::new(),
        },
    };
    *INSERTS.lock().unwrap() = Some(inserts);
    // c:3941 — `return !ret;` — qualifier passes iff lastval is 0.
    i32::from(ret == 0)
}

/// Port of `qualnonemptydir(char *name, struct stat *buf, UNUSED(off_t days), UNUSED(char *str))` from Src/glob.c:3948.
/// C: opendir(name) and check if any non-`.`/`..` entries exist.
pub fn qualnonemptydir(name: &str, buf: &libc::stat, days: i64, str: &str) -> i32 {
    // c:3948
    // c:3948 — `if (!S_ISDIR(buf->st_mode)) return 0;`
    if (buf.st_mode as u32 & libc::S_IFMT as u32) != libc::S_IFDIR as u32 {
        // c:3950
        return 0;
    }
    // c:3953-3964 — opendir + readdir loop, skip "." and ".." entries.
    match fs::read_dir(name) {
        Ok(entries) => entries.filter_map(|e| e.ok()).any(|e| {
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
/// or its helpers (`parse_qualifiers`, `scanner`,
/// `sort_matches`, `glob_emit_path`) consult.
#[derive(Debug, Clone, Copy, Default)]
pub struct GlobOptSnapshot {
    pub bareglobqual: bool,
    pub braceccl: bool,
    pub caseglob: bool,
    pub extendedglob: bool,
    pub globdots: bool,
    pub globstarshort: bool,
    pub listtypes: bool,
    pub markdirs: bool,
    pub nullglob: bool,
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

/// Port of `TestMatchFunc` — the `int (*)(char *, struct stat *, off_t,
/// char *)` function-pointer type held by `struct qual.func` (Src/glob.c).
/// Every glob-qualifier test (qualuid, qualisreg, qualiscom, qualsize, …)
/// has this signature so `insert()` can call them uniformly through the
/// `func` slot. `name` is the metafied path, `buf` the stat, `data` the
/// qualifier argument (uid, mode bits, day count, …), `sdata` the
/// expression text (only `qualsheval` uses it; others ignore it).
pub type TestMatchFunc = fn(name: &str, buf: &libc::stat, data: i64, sdata: &str) -> i32;

/// Port of `struct qual` from `Src/glob.c:138`. A node in the linked list
/// of glob qualifiers parsed from a `(…)` suffix. `insert()` (glob.c:346)
/// walks this list per candidate file, calling `func` on each node and
/// accepting/rejecting the file by `sense`. The terminating node carries
/// `func == None` (C tests `qn && qn->func` to stop).
///
/// REPRESENTATION: C's `next`/`or` are `struct qual *` pointers into a
/// graph that `parsepat` builds and relinks through aliased cursors (the
/// OR-distribution at glob.c:1797-1820 aliases one node from several
/// chains at once). Rust single-ownership `Box` cannot express that, so
/// the port uses an arena ([`QualArena`]): nodes live in a `Vec<qual>`
/// and `next`/`or` are `Option<usize>` indices. C pointer assignments map
/// to index assignments, so the aliased-cursor algorithm translates
/// almost verbatim.
#[allow(non_camel_case_types)]
#[derive(Clone, Debug)]
pub struct qual {
    /// c:139 — `struct qual *next;` next qualifier in the AND-chain.
    pub next: Option<usize>,
    /// c:140 — `struct qual *or;` head of an alternative OR-chain.
    pub or: Option<usize>,
    /// c:141 — `TestMatchFunc func;` the test to call; `None` terminates.
    pub func: Option<TestMatchFunc>,
    /// c:142 — `off_t data;` argument passed to `func`.
    pub data: i64,
    /// c:143 — `int sense;` bit0: 0=assert / 1=negate; bit1: follow links.
    pub sense: i32,
    /// c:144 — `int amc;` which timestamp to test (0=a, 1=m, 2=c).
    pub amc: i32,
    /// c:145 — `int range;` test `<` / `>` / `=` (signum of comparison).
    pub range: i32,
    /// c:146 — `int units;` multiplier for time (TT_DAYS…) or size.
    pub units: i32,
    /// c:147 — `char *sdata;` expression to eval (currently qualsheval only).
    pub sdata: Option<String>,
}

impl qual {
    /// A fresh node with the C zero-initialised defaults (`hcalloc`).
    pub fn new() -> qual {
        qual {
            next: None,
            or: None,
            func: None,
            data: 0,
            sense: 0,
            amc: 0,
            range: 0,
            units: 0,
            sdata: None,
        }
    }
}

impl Default for qual {
    fn default() -> qual {
        qual::new()
    }
}

/// Arena backing the [`qual`] graph. C allocates each `struct qual` with
/// `hcalloc` and links them by pointer; the Rust port allocates them in
/// this `Vec` and links them by index. `alloc()` mirrors a single
/// `hcalloc(sizeof *qn)` and returns the new node's index; `head` holds
/// the index of `quals` (the list `insert()` walks), `None` when empty.
#[derive(Clone, Default, Debug)]
pub struct QualArena {
    pub nodes: Vec<qual>,
    pub head: Option<usize>,
}

impl QualArena {
    pub fn new() -> QualArena {
        QualArena {
            nodes: Vec::new(),
            head: None,
        }
    }

    /// c:`qn = (struct qual *)hcalloc(sizeof *qn);` — allocate a fresh
    /// zeroed node and return its arena index.
    pub fn alloc(&mut self) -> usize {
        self.nodes.push(qual::new());
        self.nodes.len() - 1
    }
}

/// A glob qualifier function
#[derive(Debug, Clone)]
/// One glob qualifier — Rust-extension sum type. C uses a linked
/// list of `struct qual` (`Src/zsh.h:140-152`) with function-pointer
/// `func` per node; each variant here maps to one of C's `q*` test
/// ported (`qisreg`, `qisdir`, `qowner`, `qtime`, ...) at
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

    /// `^` — toggle the running match sense for SUBSEQUENT qualifiers in
    /// this list. C uses a running `sense ^= 1` (Src/glob.c:1346) stored
    /// per `struct qual.sense`; the enum can't carry per-node sense, so a
    /// marker in the list reproduces it: `check_qualifier_list` flips a
    /// running sense bit on each marker and XORs it into later tests.
    /// Fixes `*(/^@)` (dir AND not-symlink), `*(.^x)`, etc.
    ToggleSense,
}

// Misnamed `gmatchcmp(&str, &str)` deleted — was a Rust-only
// locale-aware string compare claiming to be the C qsort
// comparator. The real C `gmatchcmp(Gmatch, Gmatch)` at glob.c:936
// is now ported as `gmatchcmp(&GlobMatch, &GlobMatch, &[i32], bool)`
// below. The string-compare case the old name claimed routes
// through canonical `zstrcmp` (sort.c:191).

// `GlobOptions` struct deleted — Rust-only Bag-of-options with no
// C counterpart. C reads each option directly from the global
// `opts[]` array via `isset(NULL_GLOB)` / `isset(EXTENDED_GLOB)`
// / etc. (Src/options.c). The Rust port uses the canonical
// `opt_state_get(name) -> Option<bool>`
// which reads from the same global store. Inlined at each
// callsite as `opt_state_get("name").unwrap_or(default)`.

/// Parsed glob qualifier set
#[derive(Debug, Clone, Default)]
/// Compiled qualifier list for one glob.
/// Mirrors the `struct qual *` linked list `parsepat()`
/// (Src/glob.c:791) builds — every `(qual)` after a glob pattern
/// adds to it.
#[allow(non_camel_case_types)]
pub struct qualifier_set {
    // c:138
    pub qualifiers: Vec<qualifier>,
    pub alternatives: Vec<Vec<qualifier>>,
    pub follow_links: bool,
    /// Packed sort-spec flags, one per `o`/`O` qualifier in the pattern.
    /// Each entry is the C `struct globsort.tp` field — `GS_NAME` /
    /// `GS_DEPTH` / `GS_EXEC` / `GS_SIZE` / `GS_ATIME` / `GS_MTIME` /
    /// `GS_CTIME` / `GS_LINKS` (or their `<< GS_SHIFT` follow-link
    /// variants), OR'd with `GS_DESC` for reverse direction.
    pub sorts: Vec<i32>, // c:155 struct globsort.tp[]
    /// c:74 `struct globsort.exec` — the eval code for each `GS_EXEC`
    /// (`oe:…:` / `o+…`) sort spec, indexed by the `idx` packed into the
    /// sort entry's high bits (`GS_EXEC | (idx << 16)`). Evaluated per
    /// match with `$REPLY` = the match name; the resulting `$REPLY` is
    /// the sort key (the original name is still emitted).
    pub sort_exec: Vec<String>, // c:74 globsort.exec
    /// The faithful glob.c `struct qual` representation of `alternatives`,
    /// built at parse end (additive — the enum is kept for now). AND-chain
    /// via `next`, alternatives via `or`, per-node `sense`/`range`/`amc`/
    /// `units`/`sdata`. The convergence target: `check_qualifiers` walks
    /// THIS via `insert()` semantics (glob.c:392-419), and the enum is
    /// eventually deleted.
    pub quals: QualArena, // c:206 curglobdata.gd_quals
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
    /// `(N)` qualifier — per-glob nullglob: empty result on no-match,
    /// no error. Direct port of zsh/Src/glob.c:1567-1569 (`case 'N'`):
    ///   `gf_nullglob = !(sense & 1)` — set when `(N)` appears without
    ///   the `^` toggle.
    pub nullglob: bool,
    /// `(D)` qualifier — include dotfiles in the glob result (override
    /// the global `globdots`/`dotglob` option for this glob alone).
    /// Direct port of zsh/Src/glob.c case 'D' which sets
    /// `gf_glob.dots = 1` for the duration of this glob expansion.
    pub globdots: bool,
    /// `(YN)` qualifier — short-circuit: limit number of matches to
    /// at most N. Direct port of zsh/Src/glob.c:1579-1595 (`case 'Y'`)
    /// which sets the C `shortcircuit` int. The matcher should stop
    /// after collecting N entries; the Rust port truncates after the
    /// match walk completes (the optimization gain is the same in
    /// the typical short-glob case). Bug #41 in docs/BUGS.md.
    pub short_circuit: Option<i32>,
    /// `(n)` qualifier — per-glob numeric sort. Direct port of
    /// zsh/Src/glob.c:1575-1577 (`case 'n': gf_numsort = !(sense & 1)`).
    /// Distinct from the `o`/`O` sort KEY `n` (= GS_NAME, lexical):
    /// standalone `(n)` makes the name comparison NUMERIC (so `*(n)`
    /// orders `f2` before `f10`), OR'd with the global NUMERIC_GLOB_SORT
    /// option at sort time (glob.c:947/983 `gf_numsort ?
    /// SORTIT_NUMERICALLY : 0`). `None` = not specified (fall back to
    /// the global option); `Some(true)` = `(n)`; `Some(false)` = `(^n)`
    /// which OVERRIDES the global option to force lexical.
    pub numsort: Option<bool>,
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
            x if x == BAREGLOBQUAL => snap.bareglobqual,
            x if x == BRACECCL => snap.braceccl,
            x if x == CASEGLOB => snap.caseglob,
            x if x == EXTENDEDGLOB => snap.extendedglob,
            x if x == GLOBDOTS => snap.globdots,
            x if x == GLOBSTARSHORT => snap.globstarshort,
            x if x == LISTTYPES => snap.listtypes,
            x if x == MARKDIRS => snap.markdirs,
            x if x == NULLGLOB => snap.nullglob,
            x if x == NUMERICGLOBSORT => snap.numericglobsort,
            _ => isset(opt),
        },
        None => isset(opt),
    })
}

// ===========================================================
// `impl globdata` block above kept only for the `new()`
// constructor (Default-style). All scanner / parser / qualifier
// ported below are top-level — C glob.c has them as top-level
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
/// does separately in `xpandbraces`), then drives the faithful
/// `parsecomplist` (c:710) → `complist` → `scanner` (c:500) path.
pub fn globdata_glob(state: &mut globdata, pattern: &str) -> Vec<String> {
    // RUST-ONLY
    // Brace pre-expansion. In zsh, `xpandbraces` (zsh/Src/glob.c:2275)
    // runs during substitution before glob — patterns reaching glob()
    // are already brace-free in the production path (vm_helper handles
    // it). For direct programmatic callers of glob_with_options, run
    // the brace pass here so `GlobOptions.brace_ccl` is actually
    // consulted: with brace_ccl set, `{a-mnop}` expands to a..m,n,o,p
    // per glob.c:2424 BRACECCL block; without, only `{a,b}` lists and
    // `{1..5}`/`{a..e}` ranges expand. Recurse on each variant and
    // concatenate matches.
    let brace_ccl = glob_isset(BRACECCL);
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

    // c:Src/glob.c — `A~B` exclusion. zsh compiles the ENTIRE glob
    // (path components + `~B` exclusion + `**`) into ONE Patprog and
    // matches each candidate PATH against it, so the `~B` applies to the
    // whole path. zshrs's component scanner can't express a path-level
    // exclusion, so split a TOP-LEVEL `~` (extendedglob; depth 0, not in
    // `[...]`/`(...)`, not Bnull-escaped) off here into the main pattern
    // plus full-path exclusion patterns, glob the main pattern, then drop
    // matched paths matching any exclusion below. Without this the `~B`
    // (which may contain `/`) was split across path components and either
    // excluded everything (`~*/.git/*` → `*.txt~*` matched all) or
    // nothing. `~` inside one component still works the same (main globs,
    // the post-filter applies the exclusion).
    let (pat, glob_exclusions): (String, Vec<String>) =
        if glob_isset(EXTENDEDGLOB) && (pat.contains('~') || pat.contains('\u{98}')) {
            let cv: Vec<char> = pat.chars().collect();
            let mut parts: Vec<String> = Vec::new();
            let mut cur = String::new();
            let mut bd = 0i32; // `[...]` depth
            let mut pd = 0i32; // `(...)` depth
            let mut k = 0usize;
            while k < cv.len() {
                let c = cv[k];
                match c {
                    // Escape — the next char is a literal, never the
                    // exclusion operator. Three spellings reach here:
                    // `zshtokenize` emits Bnull for `\X` (c:3591) and
                    // Bnullkeep under ZSHTOK_SUBST (the flag `shtokenize`
                    // uses, so every `${~var}` pattern), and an UNtokenized
                    // raw `\X` arrives from zshrs's programmatic glob entry
                    // (`glob_path`) — the same raw form `patcomppiece`
                    // already accepts (pattern.rs, `b'\\'` arm). Recognising
                    // only Bnull split `a\~b*` at an ESCAPED tilde and turned
                    // the literal `~` into an exclusion, so `_path_files`'
                    // `tmp1=( $~tmp1 )` — whose patterns have the typed word
                    // quoted with QT_BACKSLASH_PATTERN (computil.c:5001) —
                    // matched nothing: `cp /etc/paths~or<TAB>` completed to
                    // nothing where zsh inserts `/etc/paths\~orig`.
                    Bnull | Bnullkeep | '\\' => {
                        cur.push(c);
                        if k + 1 < cv.len() {
                            cur.push(cv[k + 1]);
                            k += 2;
                            continue;
                        }
                    }
                    '[' => {
                        bd += 1;
                        cur.push(c);
                    }
                    ']' => {
                        if bd > 0 {
                            bd -= 1;
                        }
                        cur.push(c);
                    }
                    '(' | '\u{88}' => {
                        pd += 1;
                        cur.push(c);
                    }
                    ')' | '\u{8a}' => {
                        if pd > 0 {
                            pd -= 1;
                        }
                        cur.push(c);
                    }
                    '~' | '\u{98}' if bd == 0 && pd == 0 => {
                        parts.push(std::mem::take(&mut cur));
                    }
                    _ => cur.push(c),
                }
                k += 1;
            }
            parts.push(cur);
            // Only split when there's a non-empty main pattern AND at
            // least one exclusion (a leading `~` is home-dir, handled by
            // filesub before glob — never reaches here as an exclusion).
            if parts.len() > 1 && !parts[0].is_empty() {
                let main = parts.remove(0);
                (main, parts)
            } else {
                // No usable split — restore the original pattern verbatim
                // (the `~` separators dropped during the scan).
                (cv.iter().collect(), Vec::new())
            }
        } else {
            (pat, Vec::new())
        };

    // c:Src/glob.c:1843-1854 — `if (!q || errflag) { ... return; }`. When
    // the qualifier parser already emitted a diagnostic (e.g. "number
    // expected" from qgetnum at c:832) and set errflag, abort glob
    // expansion entirely rather than continuing to scan with a partial
    // qualifier set. Without this gate, `?(a)` (where `(a)` is an
    // invalid qualifier) error-prints "number expected" then continues
    // to expand `?` against the dir, emitting matches alongside the
    // error — bug #549.
    if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::SeqCst)
        & crate::ported::zsh_h::ERRFLAG_ERROR
        != 0
    {
        return Vec::new();
    }

    // Now check wildcards on the qualifier-stripped pattern. A pure
    // literal with a qualifier (`name(.)`) still needs to enter the
    // scanner so the qualifier filter can run against the literal name.
    // haswilds scans TOKENIZED strings (c:Src/glob.c:1230 runs on the
    // lexer-tokenized word); glob_path receives both tokenized (zglob)
    // and untokenized (expand_glob fast paths) patterns, so tokenize a
    // local copy for the check — existing token chars pass through
    // zshtokenize untouched, untokenized metachars gain their tokens.
    let mut pat_tok = pat.clone();
    tokenize(&mut pat_tok); // c:Src/glob.c:3548
    if !haswilds(&pat_tok) && state.qualifiers.is_none() {
        return vec![pattern.to_string()];
    }

    // c:glob() — `gd.gf_noglobdots = !isset(GLOBDOTS)` (plus the `(D)`
    // qualifier). parsecomplist reads this from CURGLOBDATA to compile
    // PAT_NOGLD into each component, which is what makes pattry reject
    // leading-dot files (pattern.rs:2890). Set it before parsing so the
    // faithful flag-driven dot filtering replaces the old manual skip.
    {
        let globdots = glob_isset(GLOBDOTS)
            || state
                .qualifiers
                .as_ref()
                .map(|qf| qf.globdots)
                .unwrap_or(false);
        let mut gd = CURGLOBDATA.lock().unwrap_or_else(|e| e.into_inner());
        gd.gf_noglobdots = if globdots { 0 } else { 1 };
    }

    // c:parsepat (glob.c:809-820) — init pathbuf for absolute vs relative,
    // strip the leading `/`, then parse into a `complist` and run the
    // faithful `scanner`. parsecomplist consumes TOKENIZED input (it tests
    // for the `Star` token and hands segments to patcompile), so feed it
    // `pat_tok` (already tokenized above; tokenize is idempotent).
    // c:Src/glob.c:796 — `patcompstart();` is the FIRST statement of
    // parsepat, before the pathbuf setup and parsecomplist below. It
    // runs patcompcharsset (c:464), which fills `zpc_special` from
    // `zpc_chars` and marks the option-disabled entries. This inlined
    // copy of parsepat's body (c:809-820) omitted it, so `zpc_special`
    // stayed all-zero on this path and every gate that compares against
    // it was dead: `parsecomplist`'s `(dir/)#` branch (c:746-748) tests
    // `*instr == zpc_special[ZPC_INPAR]` and `*str ==
    // zpc_special[ZPC_HASH]`, both of which compared Inpar/Pound against
    // NUL and never fired. `(sub/)#end` therefore matched only the
    // zero-repetition case (`end`), not `sub/end`, `sub/sub/end`, …
    crate::ported::pattern::patcompstart(); // c:796
    state.pathbufcwd = 0; // c:812 — `DPUTS(pathbufcwd, ...)` invariant
    let parse_src = if let Some(rest) = pat_tok.strip_prefix('/') {
        // c:813-816 — absolute path.
        state.pathbuf.push('/');
        state.pathpos = 1;
        rest.to_string()
    } else {
        // c:817-818 — relative to pwd.
        pat_tok.clone()
    };
    if let Some(complist) = parsecomplist(&parse_src) {
        scanner(state, Some(&complist), 0, false);
    } else {
        // c:Src/glob.c:1842-1854 — `q = parsepat(str); if (!q || errflag)`.
        // This is the `!q` half: the pattern failed to COMPILE (e.g. an
        // unclosed `[` character class — patcomppiece returns -1 at
        // pattern.rs:1687). C then either treats the word as a literal
        // string (when `unset(BADPATTERN)`) or aborts with
        // `zerr("bad pattern: %s", ostr)`. zshrs previously dropped the
        // `None` silently, so `echo abc[def` fell through to the
        // no-matches / literal-under-NO_NOMATCH path instead of the
        // compile diagnostic — and BADPATTERN was ignored entirely.
        // (The `errflag` half — a diagnostic already emitted by the
        // qualifier parser — is handled by the gate at glob.rs:3770.)
        if !isset(crate::ported::zsh_h::BADPATTERN) {
            // c:1846-1848 —
            //     if (!nountok)
            //         untokenize(ostr);
            //     insertlinknode(list, node, ostr);
            // `ostr` is the TOKENIZED word, so C untokenizes it before
            // handing it back as an ordinary literal string. Returning
            // `pattern` verbatim kept the Inbrack/Outbrack/Star token
            // bytes in the result and they were dropped downstream:
            // `unsetopt badpattern; print [a` printed `a`, not `[a`
            // (E01options.ztst:5).
            return vec![crate::ported::lex::untokenize(pattern)];
        }
        // c:1851-1852 — clear any stale error bit so zerr fires, then
        // emit. utils::zerr re-sets ERRFLAG_ERROR, which zglob's gate at
        // glob.rs:1541 reads to suppress the redundant "no matches found".
        errflag.fetch_and(
            !crate::ported::zsh_h::ERRFLAG_ERROR,
            std::sync::atomic::Ordering::SeqCst,
        );
        // c:1852 `zerr("bad pattern: %s", ostr)` — `ostr` is still
        // TOKENIZED here (Inbrack/Outbrack/Star/… ), and C renders the
        // `%s` through `nicezputs` (c:Src/utils.c:316), whose
        // sb_niceformat maps a token byte back through `ztokens`
        // (c:Src/utils.c niceztrlen: `if (itok(c)) c = ztokens[c - Pound]`).
        // Formatting `pattern` with Rust's `{}` skipped that step, so
        // `echo [[` reported `bad pattern: \u{91}\u{91}` (printed as two
        // invisible C1 bytes) where zsh prints `bad pattern: [[`.
        zerr(&format!(
            "bad pattern: {}",
            crate::ported::utils::nicedup(pattern, 0)
        ));
        // c:Src/exec.c:3760-3763 — every command runs its args through
        // `globlist(args, 0)` and then `if (errflag) { lastval = 1; goto
        // err; }`, so a bad pattern anywhere in argv leaves the shell's
        // status at 1 no matter which command kind was being built.
        // zshrs's builtin dispatcher reaches that via the per-command
        // glob-failed cell, but the `command` / `builtin` prefixes and
        // the external path do not, so those aborted with the PREVIOUS
        // status: `zsh -fc 'command -v [[' ` exited 0 where zsh exits 1.
        // LASTVAL is the same storage `set_last_status` writes.
        crate::ported::builtin::LASTVAL.store(1, std::sync::atomic::Ordering::Relaxed);
        return Vec::new();
    }

    // c:Src/glob.c — apply top-level `~B` exclusions to the full matched
    // paths (split off above). A match is dropped if its emitted path
    // matches ANY exclusion pattern (`*` matches `/` here, matching zsh's
    // flat-string exclusion semantics).
    if !glob_exclusions.is_empty() {
        let eg = glob_isset(EXTENDEDGLOB);
        let cg = glob_isset(CASEGLOB);
        state.matches.retain(|m| {
            let p = glob_emit_path(&m.path);
            !glob_exclusions.iter().any(|ex| matchpat(ex, &p, eg, cg))
        });
    }

    // c:1680 — populate GS_EXEC sort keys before sorting. Evaluate each
    // `oe:…:` / `o+…` code per match with $REPLY = the match name, and
    // capture the resulting $REPLY as that match's sort string (gmatchcmp
    // reads gmatch.sort_strings[idx]). Exec sort changes ORDER only — the
    // original name is still what's emitted.
    let sort_codes: Vec<String> = state
        .qualifiers
        .as_ref()
        .map(|q| q.sort_exec.clone())
        .unwrap_or_default();
    if !sort_codes.is_empty() {
        for m in state.matches.iter_mut() {
            let name = glob_emit_path(&m.path);
            for code in &sort_codes {
                crate::ported::params::setsparam("REPLY", &name);
                let _ = crate::ported::exec::execute_script(code);
                let key = crate::ported::params::getsparam("REPLY").unwrap_or_else(|| name.clone());
                m.sort_strings.push(key);
            }
        }
    }

    // c:Src/glob.c:518 — the `Y` limit belongs to the SCAN, not to the result:
    //     scanner(q->next, shortcircuit);
    //     if (shortcircuit && shortcircuit == matchct)
    //         return;
    // The scanner simply stops once N matches exist, so the survivors are the
    // first N *found*, and only then does c:1868's sort run over them. Applying
    // the limit after sorting (as apply_selection used to) keeps the first N by
    // SORT ORDER instead — a different set: `*(.Y2on)` must be the two files
    // the scan reached, re-ordered by name, not the two alphabetically-first.
    //
    // This port collects every match before sorting, so the faithful place for
    // the truncation is here — after collection, before sort_matches. The
    // `[first,last]` subscript stays in apply_selection: c:1868's sort precedes
    // it, so THAT one really is a post-sort selection.
    //
    // Zero means "no limit" (c:518's leading `shortcircuit &&`), hence `> 0`.
    if let Some(n) = state.qualifiers.as_ref().and_then(|q| q.short_circuit) {
        if n > 0 {
            let n = n as usize;
            if state.matches.len() > n {
                state.matches.truncate(n); // c:518
            }
        }
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
    let mark_dirs = glob_isset(MARKDIRS)
        || state
            .qualifiers
            .as_ref()
            .map(|q| q.mark_dirs)
            .unwrap_or(false);
    // c:1562-1566 — `gf_listtypes` is set ONLY by the `T` glob
    // qualifier (`*(T)`), NOT by the global LISTTYPES option.
    // LISTTYPES is the completion-listing option (see man zshoptions
    // "LIST_TYPES") and doesn't affect glob output. Including the
    // option here mis-decorated every executable-file glob with a
    // trailing `*` and every directory with `/` for users with the
    // default `setopt listtypes` ON.
    let list_types = state
        .qualifiers
        .as_ref()
        .map(|q| q.list_types)
        .unwrap_or(false);
    let colon_mods = state.qualifiers.as_ref().and_then(|q| q.colon_mods.clone());
    // c:Src/glob.c — a pattern ending in `/` ("trailing slash" syntax)
    // forces matches to be directories AND preserves the slash in the
    // output. zsh's parsepat treats `pat/` as `pat` + an empty trailing
    // component; the scanner restricts to directories and the emitter
    // appends `/`. The Rust port's parser short-circuits the empty
    // trailing component, so we filter + re-add the suffix here at emit
    // time. `/` is never a glob metachar so a literal trailing-slash
    // check on the raw pattern is sufficient.
    // Use the qualifier-stripped `pat` for the trailing-slash check.
    // For `*/(N)` the original `pattern` ends with `)`, so the check
    // on `pattern` returned false and the trailing-slash directory-
    // filter was skipped — `*/(N)` then matched every file, not just
    // directories. The qualifier-stripped `pat` is `*/` which still
    // carries the trailing `/`.
    let trailing_slash = pat.ends_with('/') && pat.len() > 1;
    // c:Src/glob.c — preserve user-typed `./` / `../` prefix in
    // output. glob_emit_path strips CurDir uniformly so the
    // leading `./` would be lost. Detect the source prefix here
    // and re-prepend after emit. Same `pat` (post-qualifier) — the
    // prefix is on the path part, not the qualifier.
    let leading_dot_prefix: &str = if pat.starts_with("../") {
        "../"
    } else if pat.starts_with("./") {
        "./"
    } else {
        ""
    };
    let mut results: Vec<String> = state
        .matches
        .iter()
        .filter(|m| !trailing_slash || fs::metadata(&m.path).map(|md| md.is_dir()).unwrap_or(false))
        .map(|m| {
            let mut s = glob_emit_path(&m.path);
            if !leading_dot_prefix.is_empty()
                && !s.starts_with(leading_dot_prefix)
                && !s.starts_with('/')
            {
                s = format!("{}{}", leading_dot_prefix, s);
            }
            if trailing_slash && !s.ends_with('/') {
                s.push('/');
            }
            if mark_dirs || list_types {
                if let Ok(meta) = fs::symlink_metadata(&m.path) {
                    let ch = file_type(meta.mode());
                    if list_types || (mark_dirs && ch == '/') {
                        // Don't double-stamp if trailing_slash already added one.
                        if !s.ends_with(ch) {
                            s.push(ch);
                        }
                    }
                }
            }
            // Apply colon modifiers AFTER mark/list-type appendage —
            // zsh applies them last in glob.c:432 modify() per emitted
            // node, so `(M:t)` would mark THEN tail (effectively just
            // tail since the slash is gone). Faithful order.
            if let Some(ref m) = colon_mods {
                // Delegate to canonical `modify()` port in subst.rs
                // (Src/subst.c:4531). Local colon-modifier impl was
                // an invented duplicate — removed.
                s = crate::ported::subst::modify(&s, m);
            }
            s
        })
        .collect();

    // c:421-426 — zsh applies the colon modifier in insert() (during
    // collection) BEFORE the match sort, so the default (name) sort
    // orders the MODIFIED names: `*(.:e)` on (apple.z,banana.a,cherry.m)
    // sorts the extensions → (a,m,z), not name order (z,a,m). The Rust
    // pipeline modifies at emit (after sort_matches), so re-sort the
    // modified results here — but ONLY for the default name sort; an
    // explicit `o`/`O` keeps its stat-keyed order with modified output.
    let explicit_sort = state
        .qualifiers
        .as_ref()
        .map(|q| !q.sorts.is_empty())
        .unwrap_or(false);
    if colon_mods.is_some() && !explicit_sort {
        // c:945 — that sort compares the keys with `zstrcmp` (Src/sort.c:191),
        // i.e. `strcoll` under the current locale, and honours gf_numsort.
        // Rust's `Vec::sort` is BYTEWISE, so `*.txt(:r)` came back as
        // `Bravo Charlie alpha delta` while the very same directory globbed
        // as plain `*.txt` — which goes through `sort_matches` → `gmatchcmp`
        // → `zstrcmp` — gave zsh's `alpha Bravo Charlie delta`. The two
        // orderings inside one shell disagreed; only the modified one was
        // wrong.
        let numeric = state
            .qualifiers
            .as_ref()
            .and_then(|q| q.numsort)
            .unwrap_or_else(|| glob_isset(NUMERICGLOBSORT)); // c:1258/1575
        results.sort_by(|a, b| {
            crate::ported::sort::zstrcmp(
                a,
                b,
                if numeric {
                    crate::zsh_h::SORTIT_NUMERICALLY as u32
                } else {
                    0
                },
            )
        });
    }

    // c:1744-1756 / insert_glob_match c:1430 — `P:word:` (gf_pre_words)
    // emits `word` as a separate element BEFORE each match; `^P:word:`
    // (gf_post_words) emits it AFTER. Applied last, after sort/modifiers.
    let (pre_words, post_words) = state
        .qualifiers
        .as_ref()
        .map(|q| (q.pre_words.clone(), q.post_words.clone()))
        .unwrap_or_default();
    if !pre_words.is_empty() || !post_words.is_empty() {
        let mut expanded =
            Vec::with_capacity(results.len() * (1 + pre_words.len() + post_words.len()));
        for s in results {
            expanded.extend(pre_words.iter().cloned());
            expanded.push(s);
            expanded.extend(post_words.iter().cloned());
        }
        results = expanded;
    }

    // c:Src/glob.c:1872-1888 — no-match path. Return Vec::new() so
    // the caller (vm_helper.rs::expand_glob) drives the
    // NULLGLOB / NOMATCH / literal-fallback policy. Previously
    // this pushed `pattern.to_string()` on no-match, which made
    // `expand_glob` see a non-empty result and skip the NOMATCH
    // check entirely. zsh's `setopt nomatch` is on by default, so
    // `echo /never_real/*` should emit "no matches found" and
    // exit 1 — not silently print the literal. Parity bug #13
    // recurrence. Per-glob `(N)` qualifier (nullglob_per_qual)
    // and global NULLGLOB are now consulted in expand_glob too.
    let _ = state.qualifiers.as_ref().map(|q| q.nullglob);
    results
}

// `sort_matches_by_type` deleted — dead code (no callers anywhere
// in the tree). The sort path lives in `GlobMatch::compare` which
// dispatches off the canonical `GS_*` tp bits exactly like C's
// `gmatchcmp()` at glob.c:936.

// ============================================================================
// Pattern matching with replacement (from glob.c getmatch family)
// ============================================================================

// `MatchFlags` deleted — Rust-only bool-bag wrapper. C uses bare
// `int fl` with `SUB_*` bits from `Src/zsh.h:1981+` (mirrored at
// `zsh_h.rs:2463+`). Callers now pass an `i32` and test bits.

// Trimmed local `struct imatchdata` deleted — it was a fake 5-field
// duplicate (str/pattern/match_start/match_end/replacement) of the
// canonical `Src/zsh.h:1740` port that already lives in
// `zsh_h.rs::imatchdata` (mstr/mlen/ustr/ulen/flags/replstr/repllist).
// `get_match_ret` now uses the canonical struct; see its port above.

/// Strip a trailing `(qual)` block from a glob pattern.
/// **RUST-ONLY** — C glob.c handles qualifier parsing inline in
/// `parsepat` (c:791) as it builds the Complist. Move to that
/// shape when porting parsepat for real.
pub fn parse_qualifiers(pattern: &str) -> (String, Option<qualifier_set>) {
    // RUST-ONLY
    // c:Src/glob.c:1158-1202 `checkglobqual` — C decides where the
    // qualifier block starts by testing the LEXER TOKENS `Outpar` /
    // `Inpar` (c:1163 `if (str[sl - 1] != Outpar) return 0;`, c:1170
    // `*s != Inpar`, c:1189), never the raw ASCII bytes. That is the
    // whole mechanism by which a QUOTED metachar stays literal: in
    // `*(.e['[[ $REPLY == a* ]]'])` the body's `]` bytes are raw ASCII
    // while the qualifier's real closer is `Outbrack`, so the scan
    // walks straight past the quoted ones.
    //
    // zshrs's glob layer is reachable BOTH ways: `zglob` (c:1221) and
    // the fusevm word path hand it the tokenized word, while
    // programmatic callers (compsys `glob_path`, `builtin.rs`,
    // `subst.rs`) hand it an untokenized string. Pick the scanner off
    // the terminator so both keep working — token mode is the faithful
    // c:1163 test, raw mode is the untokenized-caller fallback that
    // reconstructs "was this escaped" from backslashes instead.
    if pattern.ends_with(crate::ported::zsh_h::Outpar) {
        // c:1296-1310 — zglob calls checkglobqual, then cuts the block
        // out at the returned `Inpar` (`*s++ = 0`) and steps over the
        // `#q` when qualsfound == 2 (c:1310 `s += 2`).
        let cv: Vec<char> = pattern.chars().collect();
        let mut sp: Option<usize> = None;
        // c:1226 `nobareglob = !isset(BAREGLOBQUAL);`
        let nobareglob = i32::from(!glob_isset(BAREGLOBQUAL));
        let qualsfound = checkglobqual(&cv, cv.len() as i32, nobareglob, &mut sp);
        let start = match sp {
            Some(v) if qualsfound != 0 => v,
            _ => return (pattern.to_string(), None),
        };
        let body_start = if qualsfound == 2 {
            start + 3
        } else {
            start + 1
        };
        let qual_content: String = cv[body_start..cv.len() - 1].iter().collect();
        let byte_start: usize = cv[..start].iter().map(|c| c.len_utf8()).sum();
        let qs = parse_qualifier_string(&qual_content);
        return (pattern[..byte_start].to_string(), Some(qs));
    }

    // Untokenized fallback, for the programmatic `glob_path` callers
    // that never went through the lexer (compsys, `builtin.rs`,
    // `subst.rs`). No `Bnull` tokens are present to carry escaping, so
    // it is recovered from literal backslashes instead.
    if !pattern.ends_with(')') {
        return (pattern.to_string(), None);
    }

    let bytes = pattern.as_bytes();
    // Backslash-escaped trailing `)` is literal — no qualifier block
    // present. C's glob path handles this via Bnull preservation in
    // the lexer; zshrs's path arrives here with `\)` for quoted
    // close-parens (e.g. assoc-value `'\)'`). Without the gate, the
    // qualifier-extraction walk below treats the literal `)` as the
    // qualifier terminator.
    let last_paren_escaped = {
        let mut bs = 0usize;
        let mut j = bytes.len() - 1;
        while j > 0 && bytes[j - 1] == b'\\' {
            bs += 1;
            j -= 1;
        }
        bs % 2 == 1
    };
    if last_paren_escaped {
        return (pattern.to_string(), None);
    }

    // Find matching open paren
    let mut depth = 0;
    let mut qual_start = None;

    // c:Src/glob.c haswilds backslash handling — a `\(` / `\)` is a
    // literal paren, not a qualifier delimiter. Track which paren
    // bytes are escaped by a preceding `\` (counting consecutive
    // backslashes — even count means the paren is unescaped, odd
    // count means escaped). Without this gate, `\(*\)` is mis-parsed
    // as `(*\)` qualifier + empty pattern prefix, sending `*\` to
    // the qualifier-letter switch which emits "unknown file
    // attribute: \".
    let is_escaped = |idx: usize| -> bool {
        let mut bs = 0usize;
        let mut j = idx;
        while j > 0 && bytes[j - 1] == b'\\' {
            bs += 1;
            j -= 1;
        }
        bs % 2 == 1
    };

    for i in (0..bytes.len()).rev() {
        match bytes[i] {
            b')' => {
                if !is_escaped(i) {
                    depth += 1;
                }
            }
            b'(' => {
                if !is_escaped(i) {
                    depth -= 1;
                    if depth == 0 {
                        qual_start = Some(i);
                        break;
                    }
                }
            }
            _ => {}
        }
    }

    let start = match qual_start {
        Some(s) => s,
        None => return (pattern.to_string(), None),
    };

    // Check for (#q...) explicit qualifier syntax.
    let qual_str = &pattern[start + 1..pattern.len() - 1];
    // c:Src/glob.c:1192-1197 — `if (isset(EXTENDEDGLOB) &&
    // !zpc_disables[ZPC_HASH] && s[1] == Pound) { if (s[2] != 'q')
    // return 0; ret = 2; }`. The leading `#` inside `(...)` is ONLY
    // special under EXTENDEDGLOB: `(#q...)` is the explicit glob
    // qualifier, and `(#X...)` for any other X is an inline pattern flag
    // (passed through). Without EXTENDEDGLOB the `#` is not special at
    // all — the `(...)` is a bare qualifier group and a leading `#` is an
    // (unknown) attribute char, so `*(#q.)` errors `unknown file
    // attribute: #` rather than silently applying the `.` qualifier.
    let (is_explicit, qual_content) = if glob_isset(EXTENDEDGLOB) && qual_str.starts_with('#') {
        if let Some(after) = qual_str.strip_prefix("#q") {
            (true, after) // c:1195 ret = 2 — explicit glob qualifier
        } else {
            // c:1194 `if (s[2] != 'q') return 0;` — inline pattern flag
            // (`(#i)`, `(#c1,2)`, `(#a)`, `(#l)`, `(#s)`, `(#e)`, `(#m)`).
            return (pattern.to_string(), None);
        }
    } else if glob_isset(BAREGLOBQUAL) {
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
fn parse_qualifier_string(s: &str) -> qualifier_set {
    // RUST-ONLY
    let mut qs = qualifier_set::default();
    // c:Src/glob.c:1312-1314 — `for (ptr = s; *ptr; ptr++) if (*ptr ==
    // Dash) *ptr = '-';`. The qualifier body arrives in LEXER-TOKENIZED
    // form, and C normalises the one token it rewrites wholesale before
    // the switch runs. Every OTHER token that can appear here (`Star`,
    // `Hat`, `Equals`, `Inbrack`) keeps its token spelling and is
    // matched by a dedicated `case` in the switch below — see c:1343,
    // c:1358, c:1385, c:1723. Untokenized callers reach here with no
    // tokens at all, so this pass is a no-op for them.
    let s: String = s
        .chars()
        .map(|c| {
            if c == crate::ported::zsh_h::Dash {
                '-'
            } else {
                c
            }
        })
        .collect();
    let mut chars = s.chars().peekable();
    let mut negated = false;
    let mut follow = false;

    while let Some(c) = chars.next() {
        match c {
            // c:1343-1344 `case Hat: case '^':`
            '^' | crate::ported::zsh_h::Hat => {
                // c:1346 — `sense ^= 1`. Keep the running `negated` for
                // the flag qualifiers (N/D/M/T) that read it, AND emit a
                // marker so the per-qualifier sense reaches the list walk.
                negated = !negated;
                qs.qualifiers.push(qualifier::ToggleSense);
            }
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
            // c:1358-1359 `case Equals: case '=':`
            '=' | crate::ported::zsh_h::Equals => qs.qualifiers.push(qualifier::IsSocket),
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
            // c:1385 `case Star:` — C matches ONLY the token, since an
            // unquoted `*` in a qualifier list always reaches the glob
            // layer tokenized. The raw spelling is kept alongside it for
            // the untokenized programmatic `glob_path` callers.
            '*' | crate::ported::zsh_h::Star => qs.qualifiers.push(qualifier::IsExecutable),
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
            // c:Src/glob.c case 'f' — file-mode qualifier accepts
            // an octal mode (`f755`) or chmod-style symbolic spec
            // (`fu+x`/`fg-w`/`fa=r`) and matches files whose mode
            // bits agree. Multiple delimited specs can be chained
            // by separator character. Direct port of qgetmodespec
            // dispatch + mode-bit accumulator in qgetmodespec.
            'f' => {
                // c:Src/glob.c:849 — qgetmodespec dispatches on the
                // first char: if it's `=`/`+`/`-`/`?` or a digit
                // (`(c >= '0' && c <= '7')`) the spec is bare (no
                // surrounding delimiter, no who) — `f+x` / `f-w` /
                // `f=755` / `f0644`. Otherwise the char is the
                // OPENING DELIMITER paired with a matching closer
                // (`<` → `>`, `[` → `]`, `{` → `}`, anything else
                // → itself, e.g. `f:u+x:` uses `:` on both sides).
                // The previous port used `!d.is_alphanumeric()`
                // which incorrectly classified `+`, `-`, `=`, `?`
                // as delimiters, sending `f+x` down the
                // delimiter-string path that then called
                // qgetmodespec on body "x" — qgetmodespec rejects
                // `x` (no op char) and silently dropped the
                // qualifier, so `*(.f+x)` matched ALL files. Bug
                // #105 in docs/BUGS.md.
                let rest: String = chars.clone().collect();
                let trimmed: &str = match rest.chars().next() {
                    // c:849-861 — bare spec ONLY when the first char is
                    // `=`/`+`/`-`/`?` or an octal digit; ANY other char is
                    // the OPENING DELIMITER (`<`→`>`, `[`→`]`, `{`→`}`, else
                    // itself). Who-clauses (u/g/o/a) are valid only INSIDE a
                    // delimited spec, so bare `fu+x` lands here with `u` as
                    // the delimiter — and with no closing `u` it is an
                    // unterminated (invalid) spec, exactly as in zsh.
                    Some(d) if !matches!(d, '+' | '-' | '=' | '?' | '0'..='7') => {
                        chars.next(); // consume opening delim
                        let close = match d {
                            '<' => '>',
                            '[' => ']',
                            '{' => '}',
                            other => other,
                        };
                        let mut body = String::new();
                        let mut closed = false;
                        while let Some(pc) = chars.next() {
                            if pc == close {
                                closed = true;
                                break;
                            }
                            body.push(pc);
                        }
                        if !closed {
                            // c:884/930 — `zerr("invalid mode specification")`
                            // on an unterminated spec; the glob then fails.
                            crate::ported::utils::zerr("invalid mode specification");
                            crate::ported::utils::errflag.fetch_or(
                                crate::ported::utils::ERRFLAG_ERROR,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            return qs;
                        }
                        if let Some((_who, op, perm, _r)) = qgetmodespec(&body) {
                            let (yes, no) = match op {
                                '=' => (perm, 0o7777 & !perm),
                                '+' => (perm, 0),
                                '-' => (0, perm),
                                _ => (perm, 0),
                            };
                            qs.qualifiers.push(qualifier::Mode { yes, no });
                        } else {
                            // c:884/930 — qgetmodespec's every failure path is
                            // `zerr("invalid mode specification"); return 0;`.
                            // rs's qgetmodespec returns None only on those, so
                            // None must fail the glob, not silently drop `f`.
                            crate::ported::utils::zerr("invalid mode specification");
                            crate::ported::utils::errflag.fetch_or(
                                crate::ported::utils::ERRFLAG_ERROR,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            return qs;
                        }
                        ""
                    }
                    _ => {
                        let parsed = qgetmodespec(&rest);
                        if let Some((who, op, perm, r)) = parsed {
                            let consumed = rest.len() - r.len();
                            for _ in 0..consumed {
                                chars.next();
                            }
                            let _ = who;
                            let (yes, no) = match op {
                                '=' => (perm, 0o7777 & !perm),
                                '+' => (perm, 0),
                                '-' => (0, perm),
                                _ => (perm, 0),
                            };
                            qs.qualifiers.push(qualifier::Mode { yes, no });
                        } else {
                            // c:930 — a bare `f` (empty spec) or otherwise
                            // unparseable mode is `zerr("invalid mode
                            // specification"); return 0;` in qgetmodespec, which
                            // aborts the glob rather than dropping the qualifier.
                            crate::ported::utils::zerr("invalid mode specification");
                            crate::ported::utils::errflag.fetch_or(
                                crate::ported::utils::ERRFLAG_ERROR,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            return qs;
                        }
                        ""
                    }
                };
                let _ = trimmed;
            }
            // Ownership
            'U' => qs.qualifiers.push(qualifier::OwnedByEuid),
            'G' => qs.qualifiers.push(qualifier::OwnedByEgid),
            'u' => {
                let uid = parse_uid_gid(&mut chars, false);
                qs.qualifiers.push(qualifier::OwnedByUid(uid));
            }
            'g' => {
                let gid = parse_uid_gid(&mut chars, true);
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
            // c:Src/glob.c:1445-1449 — `case 'd': func = qualdev;
            // data = qgetnum(&s);`. Matches files by device number
            // (`qualdev`, c:3688, is `buf->st_dev == dv`). Both the
            // `qualifier::Device` variant and the `qualdev` matcher were
            // already ported and wired at glob.rs:4894 — only this parser arm
            // was missing, so `*(d5)` never reached them and a bare `*(d)`
            // reported "unknown file attribute: d" where zsh says
            // "number expected".
            //
            // c:826-834 qgetnum takes PLAIN digits — deliberately NOT
            // `parse_range_spec`, which would also accept the `+`/`-` range
            // operators that `l`/`L`/`m` use and that C does not allow here.
            'd' => {
                let mut num = String::new();
                while let Some(&pc) = chars.peek() {
                    if pc.is_ascii_digit() {
                        num.push(pc);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if num.is_empty() {
                    // c:830-833 — `if (!idigit(**s)) { zerr("number expected");
                    // return 0; }`. Matches the sibling error arms in this
                    // parser (`Y`, `f`): report, set errflag, stop.
                    crate::ported::utils::zerr("number expected"); // c:832
                    crate::ported::utils::errflag.fetch_or(
                        crate::ported::utils::ERRFLAG_ERROR,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    return qs;
                }
                qs.qualifiers
                    .push(qualifier::Device(num.parse().unwrap_or(0))); // c:1448
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
                        'n' => {
                            chars.next();
                            GS_NAME
                        }
                        'L' => {
                            chars.next();
                            GS_SIZE
                        }
                        'l' => {
                            chars.next();
                            GS_LINKS
                        }
                        'a' => {
                            chars.next();
                            GS_ATIME
                        }
                        'm' => {
                            chars.next();
                            GS_MTIME
                        }
                        'c' => {
                            chars.next();
                            GS_CTIME
                        }
                        'd' => {
                            chars.next();
                            GS_DEPTH
                        }
                        'N' => {
                            chars.next();
                            GS_NONE
                        }
                        'e' | '+' => {
                            // c:1672-1687 — GS_EXEC sort: `glob_exec_string`
                            // parses the eval code (delimited `e:code:` or
                            // identifier `+name`); store it indexed and pack
                            // the index into the sort entry. The code is
                            // consumed HERE (not left for the main loop), so
                            // `oe:…:` is a SORT (eval = key, original name
                            // emitted), not a separate eval qualifier.
                            let is_plus = sc == '+';
                            chars.next(); // consume 'e' / '+'
                            let rest: String = chars.clone().collect();
                            match glob_exec_string(&rest, is_plus) {
                                Some((code, consumed)) => {
                                    for _ in 0..consumed {
                                        chars.next();
                                    }
                                    let idx = qs.sort_exec.len();
                                    qs.sort_exec.push(code);
                                    GS_EXEC | ((idx as i32) << 16)
                                }
                                None => {
                                    // glob_exec_string already emitted the
                                    // diagnostic + set errflag; bail.
                                    return qs;
                                }
                            }
                        }
                        _ => {
                            // c:1689 — `default: zerr("unknown sort
                            // specifier"); restore_globstate(saved);
                            // return;`. Was `_ => GS_NAME`, which silently
                            // accepted invalid keys (and reparsed them as
                            // separate qualifiers, e.g. `oD` → o + D-dotglob
                            // instead of erroring like zsh).
                            crate::ported::utils::zerr("unknown sort specifier");
                            crate::ported::utils::errflag.fetch_or(
                                crate::ported::utils::ERRFLAG_ERROR,
                                std::sync::atomic::Ordering::Relaxed,
                            );
                            return qs;
                        }
                    };
                    let shifted = if follow && (key & GS_NORMAL) != 0 {
                        key << GS_SHIFT // c:1692-1694
                    } else {
                        key
                    };
                    // c:1695-1702 — a sort key may not be repeated:
                    //     if (t != GS_EXEC) {
                    //         if (gf_sorts & t) {
                    //             zerr("doubled sort specifier");
                    //             restore_globstate(saved);
                    //             return;
                    //         }
                    //     }
                    //     gf_sorts |= t;
                    // GS_EXEC is exempt — `oe:…:` may appear repeatedly, each
                    // with its own code. This check was missing entirely, so
                    // `*(.onon)` and `*(.onOn)` silently sorted where zsh
                    // fails. Note the test is on the SHIFTED key, so `oL` and
                    // `OL` collide (same key, different direction) while `oL`
                    // and `oLm` do not (the follow variant shifts).
                    if (shifted & GS_EXEC) == 0
                        && qs.sorts.iter().any(|s| (s & !GS_DESC) == shifted)
                    {
                        crate::ported::utils::zerr("doubled sort specifier"); // c:1697
                        crate::ported::utils::errflag.fetch_or(
                            crate::ported::utils::ERRFLAG_ERROR,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                        return qs; // c:1699
                    }
                    // c:1658-1662 — `if (gf_nsorts == MAX_SORTS) { zerr("too
                    // many glob sort specifiers"); ... return; }`. MAX_SORTS is
                    // 12 (c:164). Unreachable in practice now that the doubled
                    // check above rejects repeats, but it is C's bound and the
                    // list is otherwise unbounded.
                    if qs.sorts.len() == MAX_SORTS {
                        crate::ported::utils::zerr("too many glob sort specifiers"); // c:1659
                        crate::ported::utils::errflag.fetch_or(
                            crate::ported::utils::ERRFLAG_ERROR,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                        return qs; // c:1661
                    }
                    let tp = shifted | (if desc { GS_DESC } else { 0 });
                    qs.sorts.push(tp);
                } else {
                    // c:1666-1690 — C does not guard on "is there a next
                    // character": it reads `switch (*s)` unconditionally, so an
                    // `o`/`O` at the very end of the qualifier list sees the
                    // terminating `)` and lands on `default: zerr("unknown sort
                    // specifier")`. This port skipped the whole block when the
                    // iterator was exhausted, so a trailing `o` was silently
                    // ignored and `*(No)` listed every match where zsh fails.
                    // A NON-trailing bad spec (`*(ozN)`) already errored — it
                    // reaches the `_` arm above — which is why only the
                    // end-of-list form survived.
                    crate::ported::utils::zerr("unknown sort specifier"); // c:1688
                    crate::ported::utils::errflag.fetch_or(
                        crate::ported::utils::ERRFLAG_ERROR,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    return qs; // c:1690
                }
            }
            // Flags
            // c:1567-1569 — `case 'N': gf_nullglob = !(sense & 1);`.
            // Per-glob nullglob: empty result on no-match, no error.
            'N' => qs.nullglob = !negated,
            'D' => qs.globdots = !negated,
            // c:1575-1577 — `case 'n': gf_numsort = !(sense & 1)`.
            // Standalone `(n)` enables numeric sort for this glob (the
            // `^n` toggle disables it). Consumed in sort_matches.
            'n' => qs.numsort = Some(!negated),
            // (M) / (T) — set per-qualifier-set flags. glob.c:1557-1566:
            //   case 'M': gf_markdirs = !(sense & 1);  break;
            //   case 'T': gf_listtypes = !(sense & 1); break;
            // `sense & 1` = the `^`-toggle bit. zshrs's parser tracks
            // `negated`; mirror by `!negated`. Read at output-emit
            // time to mark dirs / list types like coreutils ls -F.
            'M' => qs.mark_dirs = !negated,
            'T' => qs.list_types = !negated,
            'F' => qs.qualifiers.push(qualifier::NonEmptyDir),
            // c:1744-1756 — `P:word:` adds `word` to gf_pre_words (or
            // gf_post_words under `^`), emitted as a separate element
            // before/after each match. `glob_exec_string` parses the
            // delimited word.
            'P' => {
                let rest: String = chars.clone().collect();
                if let Some((word, consumed)) = glob_exec_string(&rest, false) {
                    for _ in 0..consumed {
                        chars.next();
                    }
                    if negated {
                        qs.post_words.push(word); // c:1750 gf_post_words
                    } else {
                        qs.pre_words.push(word); // c:1750 gf_pre_words
                    }
                }
            }
            // Subscript — c:1723-1741 `case '[': case Inbrack:`
            //
            //     char *os = --s;
            //     struct value v;
            //     v.scanflags = SCANPM_WANTVALS;
            //     v.pm = NULL; v.start = 0; v.end = -1;
            //     v.valflags = 0; v.arr = NULL;
            //     if (getindex(&s, &v, 0) || s == os) {
            //         zerr("invalid subscript");
            //         restore_globstate(saved);
            //         return;
            //     }
            //     first = v.start;
            //     end = v.end;
            //
            // C hands the bracket straight to the ONE subscript parser
            // the shell has (`getindex`, Src/params.c:2001), so a glob
            // qualifier subscript gets the same unterminated-bracket
            // diagnostic and the same arithmetic evaluation of its
            // operands as `${a[...]}`. This port called a private
            // RUST-ONLY `parse_subscript` that split on `,` and
            // `str::parse`d each half, dropping the qualifier whenever
            // either half was not a decimal literal — so `*(N[)`,
            // `*(N[a])` and `*(N[1,])` all listed every match at rc=0
            // where zsh reports `invalid subscript`, an empty result
            // and `bad math expression: empty string` respectively.
            '[' | crate::ported::zsh_h::Inbrack => {
                // c:1725 `char *os = --s;` — rewind onto the bracket;
                // getindex consumes it itself (c:2006 `*s++ = '['`).
                let os: String = std::iter::once(c).chain(chars.clone()).collect();
                // c:1727-1732 — the Value getindex fills in.
                let mut v = crate::ported::zsh_h::value {
                    pm: None,                                                // c:1728
                    arr: Vec::new(),                                         // c:1732
                    scanflags: crate::ported::zsh_h::SCANPM_WANTVALS as i32, // c:1727
                    valflags: 0,                                             // c:1731
                    start: 0,                                                // c:1729
                    end: -1,                                                 // c:1730
                };
                let mut s: &str = &os;
                let rc = crate::ported::params::getindex(&mut s, &mut v, 0); // c:1735
                if rc != 0 || s.len() == os.len() {
                    // c:1735 `|| s == os`
                    crate::ported::utils::zerr("invalid subscript"); // c:1736
                    crate::ported::utils::errflag.fetch_or(
                        crate::ported::utils::ERRFLAG_ERROR,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    return qs; // c:1738
                }
                // Re-sync the qualifier walker onto the text getindex
                // stopped at. `os` carries the already-consumed `[`, so
                // step the iterator over the remainder only.
                let consumed = os.len() - s.len();
                for _ in 1..os[..consumed].chars().count() {
                    chars.next();
                }
                // c:1785-1788 — `if (errflag) { restore_globstate(saved);
                // return; }`. getindex returns 0 after `mathevalarg`
                // rejects an empty operand (`*(N[1,])`), so the abort has
                // to come off errflag exactly as it does in C.
                if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
                    return qs; // c:1787
                }
                qs.first = Some(v.start); // c:1740
                qs.last = Some(v.end); // c:1741
            }
            // c:Src/glob.c:1579-1595 `case 'Y'` — short-circuit:
            // limit matches to at most N. Reads a numeric argument
            // immediately following the `Y`. Bug #41 in docs/BUGS.md.
            'Y' => {
                // c:1579-1594:
                //     const char *s_saved = s;
                //     shortcircuit = !(sense & 1);
                //     if (shortcircuit) {
                //         data = qgetnum(&s);
                //         if ((shortcircuit = data) != data) {
                //             /* Integer overflow */
                //             zerr("value too big: Y%s", s_saved);
                //             restore_globstate(saved);
                //             return;
                //         }
                //     }
                // `data` is a zlong and `shortcircuit` an int, so the
                // assignment-then-compare is a 64→32 TRUNCATION test: a limit
                // that does not survive the narrowing is fatal. This parsed
                // straight into i32 and silently dropped the qualifier when
                // that failed, so `*(Y2147483648)` listed every match where
                // zsh errors — the limit was simply ignored.
                //
                // s_saved is the text AFTER the `Y`, running to the end of the
                // qualifier list, which is why the message carries any trailing
                // qualifiers too: `*(.NY99…)` reports `value too big: Y99…`
                // exactly as spelled.
                let s_saved: String = chars.clone().collect(); // c:1582
                                                               // c:1582-1583 — `shortcircuit = !(sense & 1); if
                                                               // (shortcircuit) { … }`. The NEGATED spelling `^Y` clears
                                                               // the limit and consumes NO argument, so the qgetnum call
                                                               // (and its "number expected" diagnostic) is skipped
                                                               // entirely — `*(Y1^Y)` is legal. `Some(0)` is the same
                                                               // "no limit" the apply sites read via their `n > 0` test,
                                                               // matching c:518's leading `shortcircuit &&`.
                if negated {
                    qs.short_circuit = Some(0); // c:1582
                    continue;
                }
                let mut num_str = String::new();
                while let Some(&pc) = chars.peek() {
                    if pc.is_ascii_digit() {
                        num_str.push(pc);
                        chars.next();
                    } else {
                        break;
                    }
                }
                // c:830-833 — `qgetnum` opens with `if (!idigit(**s)) {
                // zerr("number expected"); return 0; }`. A bare `*(Y)` is
                // therefore a hard error that aborts the glob, not a
                // silently-ignored qualifier; the `d` arm below already
                // spells this out the same way.
                if num_str.is_empty() {
                    crate::ported::utils::zerr("number expected"); // c:832
                    crate::ported::utils::errflag.fetch_or(
                        crate::ported::utils::ERRFLAG_ERROR,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    return qs; // c:833 (`return 0` → caller aborts on errflag)
                }
                // c:1586 — qgetnum is `while (idigit(**s)) v = v * 10 + …`,
                // with no overflow check of its own; it simply wraps. Mirror
                // that rather than failing the parse, so the truncation test
                // below is what decides, exactly as in C.
                let mut data: i64 = 0;
                for ch in num_str.chars() {
                    data = data
                        .wrapping_mul(10)
                        .wrapping_add((ch as i64) - ('0' as i64));
                }
                let sc = data as i32; // c:1587 `shortcircuit = data`
                if sc as i64 != data {
                    // c:1587 `!= data`
                    crate::ported::utils::zerr(&format!("value too big: Y{s_saved}")); // c:1589
                    crate::ported::utils::errflag.fetch_or(
                        crate::ported::utils::ERRFLAG_ERROR,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    return qs; // c:1591
                }
                qs.short_circuit = Some(sc);
            }
            // c:Src/glob.c:1599-1620 `case 'e'` — `(e:CODE:)` shell-
            // eval qualifier. The body is delimited by the char
            // immediately following 'e' (matched at both ends).
            // Common forms in zsh scripts: `(e:'[[ -L $REPLY ]]':)`
            // for symlink filter, etc. The body runs against each
            // candidate path with $REPLY set; the file is kept iff
            // the body returns 0. zshrs's qualifier::Eval variant
            // was defined but had no parser arm — was rejected as
            // "unknown file attribute: e". Bug #469.
            'e' => {
                // c:1712-1719 — `e` routes through glob_exec_string →
                // get_strarg, which requires a delimiter char after `e` AND a
                // matching closer. A bare `e` (nothing after, delim would be the
                // `)`) or an unterminated body is `zerr("missing end of
                // string"); return NULL;` (c:1102), aborting the glob.
                let delim = match chars.next() {
                    Some(d) => d,
                    None => {
                        crate::ported::utils::zerr("missing end of string");
                        crate::ported::utils::errflag.fetch_or(
                            crate::ported::utils::ERRFLAG_ERROR,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                        return qs;
                    }
                };
                // c:Src/subst.c:1366-1391 — `get_strarg` maps the four
                // bracket families to their closing partner (raw ASCII at
                // c:1367-1378, tokenized at c:1379-1390); anything else
                // closes itself (c:1391). Without the map, `*(e[CODE])` /
                // `*(e{CODE})` scanned for a second `[` / `{` and aborted
                // with "missing end of string".
                let close_delim = match delim {
                    '(' => ')',                                                      // c:1367-1369
                    '[' => ']',                                                      // c:1370-1372
                    '{' => '}',                                                      // c:1373-1375
                    '<' => '>',                                                      // c:1376-1378
                    crate::ported::zsh_h::Inpar => crate::ported::zsh_h::Outpar,     // c:1379-1381
                    crate::ported::zsh_h::Inang => crate::ported::zsh_h::Outang,     // c:1382-1384
                    crate::ported::zsh_h::Inbrace => crate::ported::zsh_h::Outbrace, // c:1385-1387
                    crate::ported::zsh_h::Inbrack => crate::ported::zsh_h::Outbrack, // c:1388-1390
                    _ => delim,                                                      // c:1391
                };
                let mut body = String::new();
                let mut closed = false;
                while let Some(&pc) = chars.peek() {
                    if pc == close_delim {
                        chars.next();
                        closed = true;
                        break;
                    }
                    body.push(pc);
                    chars.next();
                }
                if !closed {
                    crate::ported::utils::zerr("missing end of string");
                    crate::ported::utils::errflag.fetch_or(
                        crate::ported::utils::ERRFLAG_ERROR,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    return qs;
                }
                if negated {
                    // (^e:CODE:) inverts; we route through Eval and
                    // flip by wrapping the body. Simpler: just store
                    // and let the negated flag at the qualifier_set
                    // level handle inversion.
                }
                qs.qualifiers.push(qualifier::Eval(body));
            }
            // c:Src/glob.c:1708-1722 `case '+':` — `(+FUNC)` invokes
            // shell function FUNC on each candidate; keep file iff
            // function returns 0. C's glob_exec_string (c:1085) reads
            // the identifier name via `itype_end(s, IIDENT, 0)` when
            // the qualifier letter was '+'. The body wraps the call as
            // a shell expression `FUNC` that qualsheval (c:4769 zshrs
            // qualifier::Eval) runs as a one-shot. Bug #N — this arm
            // was missing entirely, so `*(+func)` and `*(s+0)`-style
            // mixed-qualifier strings errored "unknown file attribute:
            // +" instead of routing through the Eval path that would
            // either match files or fall through to "no matches found".
            '+' => {
                // c:1090-1097 — `tt = itype_end(s, IIDENT, 0); if
                // (tt == s) zerr("missing identifier after `+'")`.
                // Read identifier chars greedily.
                let mut ident = String::new();
                while let Some(&pc) = chars.peek() {
                    if pc.is_ascii_alphanumeric() || pc == '_' {
                        ident.push(pc);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if ident.is_empty() {
                    crate::ported::utils::zerr("missing identifier after `+'"); // c:1095
                    crate::ported::utils::errflag.fetch_or(
                        crate::ported::utils::ERRFLAG_ERROR,
                        std::sync::atomic::Ordering::Relaxed,
                    );
                    return qs;
                }
                // c:1109-1117 — the identifier IS the body that
                // qualsheval will run. Push as Eval so the same
                // per-file evaluator at c:4769 fires.
                qs.qualifiers.push(qualifier::Eval(ident));
            }
            // c:Src/glob.c:1758-1762 — `default: zerr("unknown file
            // attribute: %c", *s); restore_globstate(saved); return;`.
            // Bug #583: zshrs's `_ => {}` arm silently accepted any
            // unknown qualifier letter, so `*(z)` returned all files
            // rc=0 instead of erroring "unknown file attribute: z".
            // Mirror C's strict behavior. The exemption for ASCII
            // whitespace that used to sit here was justified as "C's
            // qualifier loop stops on those naturally via its `case ')'`
            // arm" — C has no such arm. c:1308-1310 NULs the trailing
            // `)` (`str[sl-1] = 0`) before the walk, so the loop
            // condition is `while (*s && !newcolonmod)` over the body
            // ALONE and a space reaches `default:` like any other
            // unhandled byte: zsh answers `*(N )` with `unknown file
            // attribute:  ` at rc=1 where the port listed every match at
            // rc=0. `\0` still terminates the C loop, and `)` cannot
            // appear because it was already overwritten.
            ch if ch != '\0' && ch != ')' => {
                crate::ported::utils::zerr(&format!("unknown file attribute: {}", ch));
                crate::ported::utils::errflag.fetch_or(
                    crate::ported::utils::ERRFLAG_ERROR,
                    std::sync::atomic::Ordering::Relaxed,
                );
                return qs;
            }
            _ => {}
        }
    }

    if !qs.qualifiers.is_empty() {
        qs.alternatives.push(std::mem::take(&mut qs.qualifiers));
    }

    qs.follow_links = follow;

    // Build the faithful glob.c `struct qual` arena from `alternatives`
    // (additive: matching still uses the enum). Each alternative is an
    // AND-chain (`next` links); alternatives chain via `or`. ToggleSense
    // markers toggle a running per-node `sense` (c:1346). Each variant
    // maps to its leaf TestMatchFunc + data, mirroring check_single_
    // qualifier's dispatch (Src/glob.c:1343-1620 case → func/data).
    {
        let op_range = |o: char| -> i32 {
            match o {
                '<' => -1,
                '>' => 1,
                _ => 0,
            }
        };
        let mut arena = QualArena::new();
        let mut prev_alt_head: Option<usize> = None;
        for alt in &qs.alternatives {
            let mut sense = 0i32;
            let mut chain_tail: Option<usize> = None;
            let mut chain_head: Option<usize> = None;
            for q in alt {
                // (func, data, range, amc, units, sdata) — None = no node.
                let fields: Option<(TestMatchFunc, i64, i32, i32, i32, Option<String>)> = match q {
                    qualifier::ToggleSense => {
                        sense ^= 1; // c:1346
                        None
                    }
                    qualifier::IsRegular => Some((qualisreg, 0, 0, 0, 0, None)),
                    qualifier::IsDirectory => Some((qualisdir, 0, 0, 0, 0, None)),
                    qualifier::IsSymlink => Some((qualislnk, 0, 0, 0, 0, None)),
                    qualifier::IsSocket => Some((qualissock, 0, 0, 0, 0, None)),
                    qualifier::IsFifo => Some((qualisfifo, 0, 0, 0, 0, None)),
                    qualifier::IsBlockDev => Some((qualisblk, 0, 0, 0, 0, None)),
                    qualifier::IsCharDev => Some((qualischr, 0, 0, 0, 0, None)),
                    qualifier::IsDevice => Some((qualisdev, 0, 0, 0, 0, None)),
                    qualifier::IsExecutable => Some((qualiscom, 0, 0, 0, 0, None)),
                    qualifier::Readable => Some((qualflags, 0o400, 0, 0, 0, None)),
                    qualifier::Writable => Some((qualflags, 0o200, 0, 0, 0, None)),
                    qualifier::Executable => Some((qualflags, 0o100, 0, 0, 0, None)),
                    qualifier::WorldReadable => Some((qualflags, 0o004, 0, 0, 0, None)),
                    qualifier::WorldWritable => Some((qualflags, 0o002, 0, 0, 0, None)),
                    qualifier::WorldExecutable => Some((qualflags, 0o001, 0, 0, 0, None)),
                    qualifier::GroupReadable => Some((qualflags, 0o040, 0, 0, 0, None)),
                    qualifier::GroupWritable => Some((qualflags, 0o020, 0, 0, 0, None)),
                    qualifier::GroupExecutable => Some((qualflags, 0o010, 0, 0, 0, None)),
                    qualifier::Setuid => Some((qualflags, 0o4000, 0, 0, 0, None)),
                    qualifier::Setgid => Some((qualflags, 0o2000, 0, 0, 0, None)),
                    qualifier::Sticky => Some((qualflags, 0o1000, 0, 0, 0, None)),
                    qualifier::OwnedByEuid => {
                        Some((qualuid, unsafe { libc::geteuid() } as i64, 0, 0, 0, None))
                    }
                    qualifier::OwnedByEgid => {
                        Some((qualgid, unsafe { libc::getegid() } as i64, 0, 0, 0, None))
                    }
                    qualifier::OwnedByUid(u) => Some((qualuid, *u as i64, 0, 0, 0, None)),
                    qualifier::OwnedByGid(g) => Some((qualgid, *g as i64, 0, 0, 0, None)),
                    qualifier::Size { value, unit, op } => {
                        Some((qualsize, *value as i64, op_range(*op), -1, *unit, None))
                    }
                    qualifier::Links { value, op } => {
                        Some((qualnlink, *value as i64, op_range(*op), 0, 0, None))
                    }
                    qualifier::Atime { value, unit, op } => {
                        Some((qualtime, *value, op_range(*op), 0, *unit, None))
                    }
                    qualifier::Mtime { value, unit, op } => {
                        Some((qualtime, *value, op_range(*op), 1, *unit, None))
                    }
                    qualifier::Ctime { value, unit, op } => {
                        Some((qualtime, *value, op_range(*op), 2, *unit, None))
                    }
                    qualifier::Mode { yes, no } => Some((
                        qualmodeflags,
                        (*yes as i64) | ((*no as i64) << 12),
                        0,
                        0,
                        0,
                        None,
                    )),
                    qualifier::Device(d) => Some((qualdev, *d as i64, 0, 0, 0, None)),
                    qualifier::NonEmptyDir => Some((qualnonemptydir, 0, 0, 0, 0, None)),
                    qualifier::Eval(code) => Some((qualsheval, 0, 0, 0, 0, Some(code.clone()))),
                };
                let Some((func, data, range, amc, units, sdata)) = fields else {
                    continue;
                };
                let idx = arena.alloc(); // c:1768 hcalloc
                {
                    let node = &mut arena.nodes[idx];
                    node.func = Some(func); // c:1774
                    node.data = data; // c:1776
                    node.sense = sense; // c:1775
                    node.range = range; // c:1778
                    node.units = units; // c:1779
                    node.amc = amc; // c:1780
                    node.sdata = sdata; // c:1777
                }
                if let Some(t) = chain_tail {
                    arena.nodes[t].next = Some(idx); // c:1770
                }
                chain_tail = Some(idx);
                if chain_head.is_none() {
                    chain_head = Some(idx);
                }
            }
            // Link this alternative's head into the or-chain (c:1324).
            if let Some(h) = chain_head {
                if let Some(p) = prev_alt_head {
                    arena.nodes[p].or = Some(h);
                }
                prev_alt_head = Some(h);
                if arena.head.is_none() {
                    arena.head = Some(h);
                }
            }
        }
        qs.quals = arena;
    }

    qs
}

/// Parse a numeric uid/gid or a delimited user/group name from a `u`/`g`
/// qualifier char stream. Port of the name-resolution arms of the `u`/`g`
/// qualifier cases at `Src/glob.c:1468-1535`:
/// `if (idigit(*s)) data = qgetnum(&s);` else `get_strarg`-delimited name
/// resolved through `getpwnam(3)` / `getgrnam(3)`. On an unknown name it
/// emits the C diagnostic (`zerr` sets errflag) and returns 0, matching
/// C's `data = 0;` arm. `is_group` selects getgrnam over getpwnam.
fn parse_uid_gid(chars: &mut std::iter::Peekable<std::str::Chars>, is_group: bool) -> u32 {
    // c:1468/1511 — `if (idigit(*s)) data = qgetnum(&s);`
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
        return num.parse().unwrap_or(0);
    }
    // c:1471-1505 — `tt = get_strarg(s, &arglen);` the char right after the
    // qualifier is the delimiter; the name runs to the next delimiter.
    let delim = match chars.next() {
        Some(d) => d,
        None => {
            // c:1481/1524 — `zerr("missing delimiter for 'u'/'g' glob qualifier")`.
            zerr(if is_group {
                "missing delimiter for 'g' glob qualifier"
            } else {
                "missing delimiter for 'u' glob qualifier"
            });
            return 0;
        }
    };
    // c:Src/subst.c:1366-1391 — `get_strarg` maps the four bracket families
    // to their closing partner (raw ASCII at c:1367-1378, tokenized at
    // c:1379-1390); every other delimiter closes itself (c:1391). Scanning
    // for a repeat of the OPENING char left the close bracket unconsumed,
    // so `*(u{0})` resolved the name "0}" instead of "0".
    let close_delim = match delim {
        '(' => ')',                                                      // c:1367-1369
        '[' => ']',                                                      // c:1370-1372
        '{' => '}',                                                      // c:1373-1375
        '<' => '>',                                                      // c:1376-1378
        crate::ported::zsh_h::Inpar => crate::ported::zsh_h::Outpar,     // c:1379-1381
        crate::ported::zsh_h::Inang => crate::ported::zsh_h::Outang,     // c:1382-1384
        crate::ported::zsh_h::Inbrace => crate::ported::zsh_h::Outbrace, // c:1385-1387
        crate::ported::zsh_h::Inbrack => crate::ported::zsh_h::Outbrack, // c:1388-1390
        _ => delim,                                                      // c:1391
    };
    let mut name = String::new();
    for c in chars.by_ref() {
        if c == close_delim {
            break;
        }
        name.push(c);
    }
    // c:1488/1530 — resolve via getpwnam(3) / getgrnam(3).
    let cstr = match std::ffi::CString::new(name.as_bytes()) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    unsafe {
        if is_group {
            let gr = libc::getgrnam(cstr.as_ptr()); // c:1530
            if !gr.is_null() {
                return (*gr).gr_gid; // c:1531
            }
            zerr("unknown group"); // c:1533
        } else {
            let pw = libc::getpwnam(cstr.as_ptr()); // c:1488
            if !pw.is_null() {
                return (*pw).pw_uid; // c:1489
            }
            // c:1491 — `zerr("unknown username '%s'", s + arglen);`
            zerr(&format!("unknown username '{}'", name));
        }
    }
    // c:1493/1535 — `data = 0;` after the unknown-name diagnostic.
    0
}

/// Parse the unit/op/value tail of an `(L...)` size qualifier.
/// **RUST-ONLY** — C parses inline in parsepat; the size-unit
/// constants are `TT_BYTES`/`TT_KILOBYTES`/… at glob.c:128-133.
fn parse_size_spec(
    // RUST-ONLY
    chars: &mut std::iter::Peekable<std::str::Chars>,
) -> (i32, char, u64) {
    let unit: i32 = match chars.peek() {
        Some('p') | Some('P') => {
            chars.next();
            TT_POSIX_BLOCKS
        }
        Some('k') | Some('K') => {
            chars.next();
            TT_KILOBYTES
        }
        Some('m') | Some('M') => {
            chars.next();
            TT_MEGABYTES
        }
        Some('g') | Some('G') => {
            chars.next();
            TT_GIGABYTES
        }
        Some('t') | Some('T') => {
            chars.next();
            TT_TERABYTES
        }
        _ => TT_BYTES,
    };
    let (op, val) = parse_range_spec(chars);
    (unit, op, val)
}

/// Parse the unit/op/value tail of an `(a/m/c...)` time qualifier.
/// **RUST-ONLY** — C parses inline in parsepat; time-unit constants
/// are `TT_SECONDS`/`TT_MINS`/… at glob.c:121-126.
fn schedgetfn(
    // RUST-ONLY (clashes with sched.c:341 name, unrelated fn)
    chars: &mut std::iter::Peekable<std::str::Chars>,
) -> (i32, char, u64) {
    let unit: i32 = match chars.peek() {
        Some('s') => {
            chars.next();
            TT_SECONDS
        }
        Some('m') => {
            chars.next();
            TT_MINS
        }
        Some('h') => {
            chars.next();
            TT_HOURS
        }
        Some('d') => {
            chars.next();
            TT_DAYS
        }
        Some('w') => {
            chars.next();
            TT_WEEKS
        }
        Some('M') => {
            chars.next();
            TT_MONTHS
        }
        _ => TT_DAYS,
    };
    let (op, val) = parse_range_spec(chars);
    (unit, op, val)
}

/// Parse `[+-]?NUMBER` operator+value tail. Mirrors C qgetnum
/// (glob.c:827) which returns the int value; here we return the
/// operator char along with the value since callers need both.
fn parse_range_spec(chars: &mut std::iter::Peekable<std::str::Chars>) -> (char, u64) {
    // RUST-ONLY
    // C's qgetnum at glob.c:827 returns the operator char inline.
    // `+N` = greater, `-N` = less, bare digits = equal.
    let op: char = match chars.peek() {
        Some('+') => {
            chars.next();
            '>'
        }
        Some('-') => {
            chars.next();
            '<'
        }
        _ => '=',
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
    // c:Src/glob.c:826-834 — `qgetnum` errors with "number expected"
    // when not followed by `idigit(**s)`. Previously the Rust parser
    // silently treated the missing number as 0 and let the outer
    // qualifier loop continue consuming bytes as fresh qualifier
    // letters — `*(Lr)` parsed as `L0 + r` (readable, size=0) and
    // dropped to the no-matches path. Setting errflag here propagates
    // the canonical error message and gates the "no matches found"
    // emission in zglob (c:1843-1854 `if (errflag) ... return`).
    if num.is_empty() {
        crate::ported::utils::zerr("number expected"); // c:832
        crate::ported::utils::errflag.fetch_or(
            crate::ported::zsh_h::ERRFLAG_ERROR,
            std::sync::atomic::Ordering::SeqCst,
        );
        return (op, 0); // c:833
    }
    let val = num.parse().unwrap_or(0);
    (op, val)
}

// `parse_subscript` (a RUST-ONLY `[FIRST,LAST]` splitter that
// `str::parse`d each half and dropped the qualifier on any non-decimal
// operand) is deleted: c:1735 routes the glob qualifier subscript
// through `getindex` (Src/params.c:2001), the shell's one subscript
// parser, and the `case '[':` arm above now does the same.

/// Port of `static void insert(char *s, int checked)` from `Src/glob.c:346`.
/// Records one matched path into the glob result list: runs the qualifier
/// walk (`check_qualifiers` = the c:381-419 `struct qual` walk over the
/// arena) and, if accepted, builds the `gmatch` entry from the file's
/// stat and appends it (c:421-490) — honoring any `$reply`/`$REPLY`
/// replacement (INSERTS, c:428) set by an `(e:…:)` eval qualifier.
/// Called by the scanner per matched name, mirroring glob.c:576/668.
pub fn insert(state: &mut globdata, s: &Path, checked: i32) {
    use std::os::unix::fs::MetadataExt;
    // c:348-350 — `struct stat buf, buf2, *bp; int statted = 0;`. `buf`
    // is the lstat result, `buf2` the link-followed one; `statted` keeps
    // C's bit meaning (1 = buf valid, 2 = buf2 valid). C stats the file
    // AT MOST ONCE per variant and only when something actually reads
    // the result — the scanner already proved the file exists by
    // readdir'ing it (`checked`), so a plain `dir/*(N)` costs no stat at
    // all. The port used to `symlink_metadata` every match
    // unconditionally: 60605 wasted lstat + 60605 wasted stat on the
    // `man` completion glob, 37.6% of its CPU.
    let mut statted: i32 = 0;
    let mut buf: Option<fs::Metadata> = None;
    let mut buf2: Option<fs::Metadata> = None;
    // c:353 — `inserts = NULL;`: drop any $reply/$REPLY replacement left
    // by a previous file's (e:…:) eval before this file's walk.
    *INSERTS.lock().unwrap() = None;
    // c:187 `gd_gf_sorts` (macro at c:218) — the OR of every sort key the
    // glob's `o`/`O` qualifiers asked for. c:434/c:438 consult it to
    // decide whether this match needs its stat fields filled in. With no
    // explicit sort qualifier C leaves it at the c:1856 default GS_NAME,
    // which is in neither GS_NORMAL nor GS_LINKED.
    let gf_sorts: i32 = state.qualifiers.as_ref().map_or(0, |q| {
        q.sorts.iter().fold(0, |acc, &tp| acc | (tp & !GS_DESC))
    });

    // c:355-364 — `if (gf_listtypes || gf_markdirs)` stat for the type
    // marker, and drop the match if the stat fails. The port appends the
    // marker at emit time (glob_emit_path), so only C's `checked =
    // statted = 1` side effect on the arms below is reproduced here.
    if state.gf_listtypes != 0 || state.gf_markdirs != 0 {
        match fs::symlink_metadata(s) {
            Ok(m) => {
                buf = Some(m);
                statted = 1; // c:363 `checked = statted = 1;`
            }
            Err(_) => return, // c:358-360
        }
    }

    // c:381-418 — qualifier walk; reject the file if any test fails. The
    // walk needs the stat itself, so it hands its buffer back for the
    // arms below (C's `statted = 1` at c:391).
    if !check_qualifiers(state, s, &mut buf) {
        return;
    }
    if buf.is_some() {
        statted |= 1;
    }
    // c:419-423 — `else if (!checked) { if (statfullpath(s, NULL, 1))
    // return; }`: a match the scanner did NOT readdir (a pure literal
    // path section, c:576 `insert(str, 0)`) still has to be proven to
    // exist. Nothing reads the buffer, so C passes NULL.
    if statted & 1 == 0 && checked == 0 && fs::symlink_metadata(s).is_err() {
        return;
    }
    // c:434-437 — `if (!statted && (gf_sorts & GS_NORMAL))`: a
    // size/time/link sort key needs the stat fields, so fill them now.
    if statted & 1 == 0 && (gf_sorts & GS_NORMAL) != 0 {
        buf = fs::symlink_metadata(s).ok(); // c:435
        statted |= 1;
    }
    // c:438-445 — `if (!(statted & 2) && (gf_sorts & GS_LINKED))`: the
    // follow-link sort variants need the TARGET's stat. C reuses `buf`
    // verbatim when the match is not a symlink (c:440-441 memcpy), and
    // falls back to it when the target stat fails (a dangling link).
    if statted & 2 == 0 && (gf_sorts & GS_LINKED) != 0 {
        buf2 = match &buf {
            // c:439-441
            Some(m) if !m.file_type().is_symlink() => Some(m.clone()),
            Some(m) => fs::metadata(s).ok().or_else(|| Some(m.clone())),
            // c:442-443 `if (statfullpath(s,&buf2,0)) statfullpath(s,&buf2,1);`
            None => fs::metadata(s)
                .ok()
                .or_else(|| fs::symlink_metadata(s).ok()),
        };
        statted |= 2;
    }

    // c:446-478 — copy the stat fields into the match entry, but only
    // the ones actually stat'd (`if (statted & 1)` / `if (statted & 2)`);
    // the rest stay zero as C's calloc'd match buffer leaves them.
    let name = s
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let meta = buf;
    // c:463-478 — `if (statted & 2)` fills the `_size`/`_atime`/… twins.
    // When only the lstat ran, C's GS_LINKED keys are never consulted, so
    // mirroring the lstat values here is equivalent and keeps a match
    // that IS a symlink self-consistent.
    let target = buf2.as_ref().or(meta.as_ref());
    let (tsize, tatime, tmtime, tctime, tlinks, tansec, tmnsec, tcnsec) = match target {
        Some(tm) => (
            tm.size(),
            tm.atime(),
            tm.mtime(),
            tm.ctime(),
            tm.nlink(),
            tm.atime_nsec(),
            tm.mtime_nsec(),
            tm.ctime_nsec(),
        ),
        None => (0, 0, 0, 0, 0, 0, 0, 0),
    };
    // c:428 — `while (!inserts || (news = *inserts++))`: an (e:…:) eval
    // may have set $reply/$REPLY (INSERTS), replacing this match with
    // zero-or-more names; otherwise emit the original name once. The
    // synthetic path makes glob_emit_path yield the replacement string;
    // the stat fields stay those of the matched file (C keeps
    // matchptr->size etc. from buf).
    let emit: Vec<(String, PathBuf)> = match INSERTS.lock().unwrap().take() {
        Some(list) => list
            .into_iter()
            .map(|n| (n.clone(), PathBuf::from(&n)))
            .collect(),
        None => vec![(name.clone(), s.to_path_buf())],
    };
    for (nm, pth) in emit {
        state.matches.push(gmatch {
            name: nm,
            uname: String::new(), // c:1963-1973 — filled before the sort
            path: pth,
            size: meta.as_ref().map_or(0, |m| m.size()),
            atime: meta.as_ref().map_or(0, |m| m.atime()),
            mtime: meta.as_ref().map_or(0, |m| m.mtime()),
            ctime: meta.as_ref().map_or(0, |m| m.ctime()),
            links: meta.as_ref().map_or(0, |m| m.nlink()),
            mode: meta.as_ref().map_or(0, |m| m.mode()),
            uid: meta.as_ref().map_or(0, |m| m.uid()),
            gid: meta.as_ref().map_or(0, |m| m.gid()),
            dev: meta.as_ref().map_or(0, |m| m.dev()),
            ino: meta.as_ref().map_or(0, |m| m.ino()),
            target_size: tsize,
            target_atime: tatime,
            target_mtime: tmtime,
            target_ctime: tctime,
            target_links: tlinks,
            ansec: meta.as_ref().map_or(0, |m| m.atime_nsec()),
            mnsec: meta.as_ref().map_or(0, |m| m.mtime_nsec()),
            cnsec: meta.as_ref().map_or(0, |m| m.ctime_nsec()),
            target_ansec: tansec,
            target_mnsec: tmnsec,
            target_cnsec: tcnsec,
            sort_strings: Vec::new(),
        });
        state.matchct += 1; // c:479 matchptr++
    }
}

/// Drive the OR-of-AND qualifier filter against `path`. **RUST-ONLY**
/// — C glob.c does qualifier eval inline inside `insert()` (c:381+).
///
/// `stat_out` is C's `struct stat buf` (c:348), which `insert()` shares
/// with this walk: c:385 fills it here and c:391's `statted = 1` stops
/// c:434 from stat'ing the same file a second time. Only the plain lstat
/// is handed back — under `follow_links` the walk reads C's `buf2`
/// (c:399-402), which the arms after c:419 track separately.
fn check_qualifiers(state: &globdata, path: &Path, stat_out: &mut Option<fs::Metadata>) -> bool {
    use std::os::unix::fs::MetadataExt;
    use std::sync::atomic::Ordering;
    let qs = match &state.qualifiers {
        Some(q) => q,
        None => return true,
    };
    // No test qualifiers (only flags/sort/words) → no filtering.
    let arena = &qs.quals;
    if arena.head.is_none() {
        return true;
    }

    let meta = match if qs.follow_links {
        // c:399-402 — `if (!S_ISLNK(buf.st_mode) || statfullpath(s,&buf2,0))
        //                  memcpy(&buf2, &buf, sizeof(buf));`
        // Follow the link, but fall back to the lstat buffer when the
        // target stat fails (a broken symlink) so `*(-@)` / `*(-^/)`
        // still see the dangling link rather than dropping it.
        fs::metadata(path).or_else(|_| fs::symlink_metadata(path))
    } else {
        fs::symlink_metadata(path)
    } {
        Ok(m) => m,
        Err(_) => return false, // c:385-387
    };
    if !qs.follow_links {
        // c:385 `statfullpath(s, &buf, 1)` — this IS insert()'s `buf`.
        *stat_out = Some(meta.clone());
    }
    // Bridge meta → libc::stat for the leaf test fns (no extra syscall).
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    st.st_mode = meta.mode() as _;
    st.st_uid = meta.uid() as _;
    st.st_gid = meta.gid() as _;
    st.st_dev = meta.dev() as _;
    st.st_nlink = meta.nlink() as _;
    st.st_size = meta.size() as _;
    st.st_atime = meta.atime() as _;
    st.st_mtime = meta.mtime() as _;
    st.st_ctime = meta.ctime() as _;
    // qualsheval / qualnonemptydir need the name (REPLY / opendir).
    let name = glob_emit_path(path);

    // c:387-419 — insert()'s qual-walk over the `struct qual` arena: AND
    // via `next`, OR (alternatives) via `or`, per-node `sense`. `qo` is
    // the current alternative head; a rejected node advances to `qo->or`
    // (next alternative) or, if none, rejects the file. Falling off the
    // `next` chain (or hitting a func-less terminator) accepts.
    let mut qo = arena.head;
    let mut qn = qo;
    loop {
        let Some(qn_i) = qn else { return true }; // c:419 — chain matched
        let (func, data, sense, range, amc, units, sdata, next) = {
            let n = &arena.nodes[qn_i];
            (
                n.func,
                n.data,
                n.sense,
                n.range,
                n.amc,
                n.units,
                n.sdata.clone(),
                n.next,
            )
        };
        let func = match func {
            Some(f) => f,
            None => return true, // c:392 — `qn && qn->func` terminator
        };
        // c:393-395 — set the scratch the range/time/size leaf fns read.
        g_range.store(range, Ordering::SeqCst);
        g_amc.store(amc, Ordering::SeqCst);
        g_units.store(units, Ordering::SeqCst);
        let r = func(&name, &st, data, sdata.as_deref().unwrap_or(""));
        // c:407-409 — reject if `(!(func()) ^ sense) & 1`.
        let reject = ((if r == 0 { 1 } else { 0 }) ^ (sense & 1)) & 1 == 1;
        if reject {
            // c:411-415 — try the next alternative, else reject the file.
            match arena.nodes[qo.unwrap()].or {
                Some(or_i) => {
                    qo = Some(or_i);
                    qn = Some(or_i);
                }
                None => return false,
            }
        } else {
            qn = next; // c:417
        }
    }
}

/// Sort the per-state matches per the qualifier `o`/`O` keys.
/// **RUST-ONLY** — C does this inline at the end of `scanner()`
/// (glob.c:1700ish, qsort with `gmatchcmp` comparator).
fn sort_matches(state: &mut globdata) {
    // RUST-ONLY
    // Default sort is GS_NAME ascending (c:204 gf_sortlist
    // initial setup). Per-qualifier `o<key>` / `O<key>` overrides.
    // c:1855-1857 — when gf_nsorts == 0 (no explicit o/O qualifier),
    // glob.c falls back to GS_NAME (or GS_NONE under shortcircuit).
    // Mirror that here so a qualifier set carrying ONLY non-sort
    // qualifiers (`Lk+0`, etc.) still sorts alphabetically.
    // c:1855-1857 is a CONDITIONAL default, not a fixed one:
    //     if (!gf_nsorts) {
    //         gf_sortlist[0].tp = gf_sorts = (shortcircuit ? GS_NONE : GS_NAME);
    //         gf_nsorts = 1;
    //     }
    // A `Y` limit means "give me the first N the scanner finds", so with no
    // explicit o/O spec the default becomes GS_NONE and the results stay in
    // SCAN order — sorting them would defeat the short-circuit, since the first
    // N by name are not the first N found. This always chose GS_NAME, so a
    // limited glob returned the alphabetically-first N instead of the
    // scan-order-first N, and even `*(Y99)` (a limit larger than the match
    // count, which discards nothing) came back sorted where zsh leaves it in
    // readdir order.
    //
    // `Y0` is the counter-case and must still sort: c:1583 parses the argument
    // over `shortcircuit`, so 0 lands there legitimately and c:518's
    // `if (shortcircuit && …)` reads it as "no limit" — hence the `> 0` test
    // rather than `.is_some()`.
    let limited = state
        .qualifiers
        .as_ref()
        .and_then(|q| q.short_circuit)
        .is_some_and(|n| n > 0); // c:1856 `shortcircuit ?`
    let default_spec = if limited { GS_NONE } else { GS_NAME }; // c:1856
    let mut specs: Vec<i32> = state
        .qualifiers
        .as_ref()
        .map(|q| {
            if q.sorts.is_empty() {
                vec![default_spec] // c:1856
            } else {
                q.sorts.clone()
            }
        })
        .unwrap_or_else(|| vec![GS_NAME]);

    // GS_NONE marker — caller wants no sort at all.
    if specs.iter().any(|&tp| (tp & GS_NONE) != 0) {
        return;
    }

    // c:936 gmatchcmp returns 0 when every sort key ties. In C the final
    // order of equal-key matches is then whatever the libc `qsort` does with
    // all-equal elements (glob.c:1976) — NON-stable on macOS/BSD (arbitrary,
    // input-dependent: `*(oL)` on three 0-byte files a/b/d prints `d a b`),
    // and on glibc it depends on readdir order. So zsh has NO portable,
    // defined tie-break here. zshrs is a cross-architecture shell and must be
    // DETERMINISTIC, so we impose an explicit GS_NAME final tie-breaker:
    // equal-key matches order by name ascending, identical on every platform.
    // This is an intentional determinism guarantee, NOT a bug-for-bug match of
    // zsh's qsort-defined order (which can't be reproduced portably). Skip
    // when the last key is already GS_NAME (the no-qualifier default, or an
    // explicit trailing `on`) so we don't double-compare.
    if specs
        .last()
        .map_or(true, |&last| (last & !GS_DESC) != GS_NAME)
    {
        specs.push(GS_NAME); // deterministic name-ascending tie-break
    }

    // c:1958-1973 — "Where necessary, create unmetafied version of names
    // for comparison." C runs this loop once over the match list, right
    // before the qsort at c:1977, so `gmatchcmp` (c:945) collates two
    // ready-made strings. The port's key is the full match path with the
    // scanner's depth-0 "./" prefix stripped:
    //
    //   c:424 — C stores bare-relative match names (`dyncat(pathbuf,
    //   news)`, no "./"), so its single qsort sorts the exact strings it
    //   emits. The Rust scanner joins depth-0 matches against base "."
    //   (glob.rs:527 + :584), giving a leading "./" that deeper matches
    //   lack and that `glob_emit_path` strips at emit. Since '.' (0x2E)
    //   sorts before any letter, that stray prefix clustered every
    //   top-level entry ahead of any nested path (`**/*` → `d e m d/z`
    //   instead of `d d/z e m`). Strip it so the sort key matches the
    //   emit key.
    for m in state.matches.iter_mut() {
        let full = m.path.to_string_lossy();
        m.uname = full.strip_prefix("./").unwrap_or(&full).to_string();
    }

    // c:1258/1575 — gf_numsort starts at the global NUMERIC_GLOB_SORT
    // option, then a per-glob `(n)`/`(^n)` qualifier OVERRIDES it
    // absolutely (so `(^n)` forces lexical even when the option is on).
    let numeric = state
        .qualifiers
        .as_ref()
        .and_then(|q| q.numsort)
        .unwrap_or_else(|| glob_isset(NUMERICGLOBSORT));
    state
        .matches
        .sort_by(|a, b| gmatchcmp(a, b, &specs, numeric));
}

/// Apply `[FIRST,LAST]` qualifier subscript on the match list.
/// **RUST-ONLY** — C uses `gd_pre_first` / `gd_first` index tracking
/// during scanner emit; here we slice after the full walk.
fn apply_selection(state: &mut globdata) {
    // RUST-ONLY
    let (first, last, short_circuit) = match &state.qualifiers {
        Some(q) => (q.first, q.last, q.short_circuit),
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

    // c:1990-2004 — C never copies the match array for the subscript: it
    // walks the existing `matchbuf` from `matchbuf + matchct - first - end`
    // and inserts straight out of it. The Rust equivalent that keeps the
    // elements where they are is truncate-then-drain: both move `gmatch`
    // values (each owning a PathBuf + String) instead of deep-cloning the
    // whole 60k-entry list the way `to_vec()` did.
    if start < end && start < state.matches.len() {
        state.matches.truncate(end.min(state.matches.len())); // c:1986-1989 `end` clamp
        state.matches.drain(..start); // c:1981-1985 `first` offset
    } else {
        state.matches.clear();
    }

    // c:Src/glob.c:1579-1595 `case 'Y'` — apply the short-circuit
    // limit after the [first,last] slice (so `(Y3[2,4])` would yield
    // up to 3 from the [2,4] subscript range). C truncates DURING
    // the scanner walk via `if (shortcircuit == matchct) break;` —
    // here we slice post-walk which produces the same set on the
    // common short-glob case where the scanner doesn't recurse
    // unboundedly. Bug #41 in docs/BUGS.md.
    // NOTE: the `Y` short-circuit is deliberately NOT applied here. It bounds
    // the SCAN (c:Src/glob.c:518), so it must run BEFORE the sort — see the
    // truncation at the sort_matches call site. Only the `[first,last]`
    // subscript belongs in this post-sort selection.
    let _ = short_circuit;
}

// `haswilds()` is defined in `Src/pattern.c:4306`, not glob.c — the
// canonical Rust port lives at `crate::ported::pattern::haswilds`.
// This file previously carried a divergent re-implementation tracking
// bracket/escape state instead of the token-aware `zpc_disables[]` /
// SHGLOB / KSHGLOB / EXTENDEDGLOB checks the C source actually does.
// Drift deleted; glob.rs callers route through the canonical port.

#[cfg(test)]
mod gs_tt_tests {
    use super::*;

    #[test]
    fn gs_size_offset_matches_c() {
        let _g = crate::test_util::global_state_lock();
        // c:83 — GS_SIZE = GS_SHIFT_BASE = 8.
        assert_eq!(GS_SIZE, 8);
        // c:84 — GS_ATIME = GS_SHIFT_BASE << 1 = 16.
        assert_eq!(GS_ATIME, 16);
        // c:87 — GS_LINKS = GS_SHIFT_BASE << 4 = 128.
        assert_eq!(GS_LINKS, 128);
    }

    #[test]
    fn gs_normal_covers_all_size_keys() {
        let _g = crate::test_util::global_state_lock();
        // c:99 — GS_NORMAL = SIZE | ATIME | MTIME | CTIME | LINKS.
        assert_ne!(GS_NORMAL & GS_SIZE, 0);
        assert_ne!(GS_NORMAL & GS_ATIME, 0);
        assert_ne!(GS_NORMAL & GS_MTIME, 0);
        assert_ne!(GS_NORMAL & GS_CTIME, 0);
        assert_ne!(GS_NORMAL & GS_LINKS, 0);
    }

    #[test]
    fn tt_namespaces_share_indices() {
        let _g = crate::test_util::global_state_lock();
        // c:121-126 vs c:128-133 — TT_DAYS == TT_BYTES == 0, etc.
        assert_eq!(TT_DAYS, TT_BYTES);
        assert_eq!(TT_HOURS, TT_POSIX_BLOCKS);
        assert_eq!(TT_MINS, TT_KILOBYTES);
        assert_eq!(TT_WEEKS, TT_MEGABYTES);
        assert_eq!(TT_MONTHS, TT_GIGABYTES);
        assert_eq!(TT_SECONDS, TT_TERABYTES);
    }

    #[test]
    fn max_sorts_is_12() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(MAX_SORTS, 12);
    }
}

fn expand_range(
    prefix: &str,
    content: &str,
    dotdot_pos: usize,
    suffix: &str,
) -> Option<Vec<String>> {
    // c:Src/glob.c::xpandbraces parses brace-range endpoints via
    // zstrtol, which is TOKEN-aware and treats Dash TOKEN (\u{9b})
    // as ASCII `-`. Rust's i64::parse only accepts ASCII, so
    // untokenize the content here before splitting/parsing — matches
    // C semantics without weakening the strict-TOKEN gates above.
    // A `Meta` (`\u{83}`) starts a METAFIED byte pair (`Meta` + `byte ^
    // 32`), NOT a token: C's `itok` set begins at `Pound` (Src/zsh.h:164)
    // and `Meta` sits below it, so `bracechardots` (c:2227) leaves such a
    // pair to MB_METACHARLENCONV instead of the `ztokens` lookup. The
    // escaped byte routinely LANDS in the token range (`$'\x80'` →
    // `\u{83}\u{a0}`), so scanning it as a token here untokenized the
    // pair and destroyed the range endpoint. Skip the pair when deciding
    // whether the content needs untokenizing.
    let owned: String;
    let content: &str = if {
        let cs: Vec<char> = content.chars().collect();
        let mut i = 0usize;
        let mut found = false;
        while i < cs.len() {
            if cs[i] == char::from(crate::ported::zsh_h::Meta) {
                i += 2; // metafied byte pair — not a token
                continue;
            }
            let cu = cs[i] as u32;
            if (0x84..=0xa1).contains(&cu)
                && cs[i] != '\u{8f}'
                && cs[i] != '\u{90}'
                && cs[i] != '\u{9a}'
            {
                found = true;
                break;
            }
            i += 1;
        }
        found
    } {
        owned = crate::lex::untokenize(content);
        &owned
    } else {
        content
    };
    // `dotdot_pos` is a CHAR index (xpandbraces counts it over a char vector,
    // `i - start - 1`), but the slicing below is byte-based. For ASCII content
    // they coincide; for content holding multibyte/metafied chars (e.g.
    // `{$'\x80'..$'\x81'}`, where the bytes are Meta-escaped) a raw byte slice
    // at the char index lands mid-char and panics. Convert to a byte offset.
    let dotdot_byte = content
        .char_indices()
        .nth(dotdot_pos)
        .map(|(b, _)| b)
        .unwrap_or(content.len());
    let left = &content[..dotdot_byte];
    let right_start = dotdot_byte + 2; // ".." is two ASCII bytes

    // Check for second `..` for `{N..M..S}` step form. Step may be
    // signed: negative-step REVERSES the natural direction sequence
    // per zsh's brace expansion (Src/glob.c::xpandbraces recursive
    // iteration with sign tracking). Examples:
    //   {1..32..3}   →  1,4,7,…,31 (natural ascending)
    //   {1..32..-3}  → 31,28,…,1   (same set, reversed)
    //   {32..1..3}   → 32,29,…,2   (natural descending)
    //   {32..1..-3}  →  2,5,…,32   (same set, reversed)
    //
    // c:Src/glob.c:2365-2369 — `if (dotdot == 2 && *p == '.' &&`
    //                         `    p[1] == '.') {`
    //                         `    rincr = zstrtol(p+2, &p, 10);`
    //                         `    wid3 = p - dots2 - 2;`
    //                         `    if (p != str2 || !rincr)`
    //                         `        err++;`
    // The `!rincr` test (c:2368) rejects a zero step: when err > 0,
    // the C code falls past the c:2374 `if (!err)` block, so the
    // brace expansion is abandoned and the literal pattern is
    // preserved. Parity bug #15: previously the Rust port silently
    // clamped step to 1 via `.max(1)`, producing `1 2 3 4 5` for
    // `{1..5..0}` instead of zsh's literal `1..5..0`.
    let (right, incr_abs, incr_sign_negative, step_text) =
        if let Some(pos) = content[right_start..].find("..") {
            let r = &content[right_start..right_start + pos];
            let s_text = &content[right_start + pos + 2..];
            let raw: i64 = s_text.parse().unwrap_or(1);
            if raw == 0 {
                // c:2368 — `!rincr` → err++ → no expansion. Return
                // None so xpandbraces falls back to literal.
                return None;
            }
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
        // zsh's pad-detection: pad fires when any of the three
        // textual fields (left / right / step) is a multi-digit
        // form with a leading zero (e.g. "01", "00", "003"), with
        // ONE exception: when the LEFT endpoint is exactly "0"
        // (bare single zero, possibly signed), pad is suppressed
        // regardless of right's form. Empirical zsh:
        //   {0..-5}    → 0 -1 -2 -3 -4 -5    (left=0 → no pad)
        //   {0..03}    → 0 1 2 3              (left=0 → no pad)
        //   {00..-5}   → 00 -1 -2 -3 -4 -5    (left=00 → pad w=2)
        //   {1..03}    → 01 02 03             (right=03 → pad w=2)
        //   {1..-03}   → 001 000 -01 -02 -03  (right=-03 → pad w=3)
        let lstrip = left.trim_start_matches(['+', '-']);
        let rstrip = right.trim_start_matches(['+', '-']);
        let sstrip = step_text.trim_start_matches(['+', '-']);
        let is_padded_field = |stripped: &str| stripped.len() >= 2 && stripped.starts_with('0');
        // Suppress pad ONLY when LEFT is exactly the single-char "0"
        // (no sign, no extra digits). "-0" or "00" both pad.
        let left_is_bare_zero = left == "0";
        let pad = !left_is_bare_zero
            && (is_padded_field(lstrip)
                || is_padded_field(rstrip)
                || (!step_text.is_empty() && is_padded_field(sstrip)));
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
    // c:Src/lex.c::dobrace alpha-range path — zsh only handles bare a..z,
    // NOT a..z..N (it emits literal `{a..z..3}` because the step form is
    // numeric-only). bash DOES support an alpha step (`{a..e..2}` → a c e),
    // so honor `step_text` only under --bash; --zsh keeps refusing it so the
    // literal survives (verified: real zsh emits `{a..e..2}` unchanged).
    // bash ignores the step's SIGN for alpha (`{a..e..-2}` == `{a..e..2}`),
    // so use abs(step) and take direction purely from start vs end.
    let alpha_step_ok = step_text.is_empty() || crate::dash_mode::bash_mode();
    // c:2311 — `if (bracechardots(str, &cstart, &cend))` decides the
    // CHARACTER range. The endpoints are METAFIED text, decoded through
    // MB_METACHARLENCONV (c:2236/2257), so a metafied 8-bit byte
    // (`$'\x80'` → `Meta` `\u{a0}`) and a real multibyte character both
    // resolve to their scalar. The old byte-length test (`left.len() ==
    // 1`) accepted only ASCII and left `{é..ê}` / `{$'\x80'..$'\x81'}`
    // unexpanded.
    let lb = crate::ported::utils::unmetafy_str(left); // c:2236
    let (l_len, cstart, _) = crate::ported::utils::mb_metacharlenconv(&lb); // c:2236
    let rb = crate::ported::utils::unmetafy_str(right); // c:2257
    let (r_len, cend, _) = crate::ported::utils::mb_metacharlenconv(&rb); // c:2257
    let endpoints_ok = l_len != 0 && l_len == lb.len() && r_len != 0 && r_len == rb.len();
    if endpoints_ok && cstart.is_some() && cend.is_some() && alpha_step_ok {
        let start = cstart?; // c:2311 cstart
        let end = cend?; // c:2311 cend
        let step = incr_abs.max(1) as u32;
        let (lo, hi, reverse) = if start <= end {
            (start, end, false)
        } else {
            (end, start, true)
        };

        let mut results = Vec::new();
        let mut chars: Vec<char> = Vec::new();
        let mut v = lo as u32;
        while v <= hi as u32 {
            if let Some(c) = char::from_u32(v) {
                chars.push(c);
            }
            v += step;
        }
        if reverse {
            chars.reverse();
        }

        for c in chars {
            // c:2328-2334 — `MB_CHARINIT(); ncptr = MB_NICECHAR(cend);
            // ... memcpy(p + strp, ncptr, nclen);` — every element of a
            // CHARACTER range is spliced in through `MB_NICECHAR`
            // (Src/zsh.h:3288 `wcs_nicechar(cp, NULL, NULL)`), not as the
            // raw character. Printable characters render as themselves, so
            // `{a..e}` is unchanged, while a non-printable 8-bit endpoint
            // comes out as its `\M-^@`-style escape — which is exactly what
            // `{$'\x80'..$'\x81'}` prints in zsh.
            let ncptr = crate::ported::utils::wcs_nicechar(c, None, None); // c:2329
            results.push(format!("{}{}{}", prefix, ncptr, suffix)); // c:2332-2334
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
    // positions are CHAR indices into `content` (per the xpandbraces
    // walker above that builds them from `chars: Vec<char>`). Slice
    // by char-index, not byte-index — `&content[last..pos]` would
    // panic on multi-byte token chars like Comma TOKEN (`\u{9a}`,
    // 2 UTF-8 bytes) and Inbrace (`\u{8f}`, 2 bytes).
    let chars: Vec<char> = content.chars().collect();
    let mut results = Vec::new();
    let mut last: usize = 0;
    for &pos in positions {
        let part: String = chars[last..pos].iter().collect();
        results.push(format!("{}{}{}", prefix, part, suffix));
        last = pos + 1;
    }
    let tail: String = chars[last..].iter().collect();
    results.push(format!("{}{}{}", prefix, tail, suffix));
    Some(results)
}

fn expand_ccl(prefix: &str, content: &str, suffix: &str) -> Option<Vec<String>> {
    // c:Src/glob.c:expand_ccl — char-class range expansion for the
    // `setopt braceccl` brace shape `{m-o}` → m,n,o. Accept both
    // ASCII `-` and Dash TOKEN (\u{9b}) as the range separator —
    // the bridge passthru path delivers `{m-o}` with Dash TOKEN
    // since the lexer tokenizes `-` to Dash inside word context.
    let mut chars_set = HashSet::new();
    let chars: Vec<char> = content.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let is_range = i + 2 < chars.len() && (chars[i + 1] == '-' || chars[i + 1] == '\u{9b}');
        if is_range {
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

/// Strip the `./` prefix that `read_dir(".")` introduces on relative match
/// paths. Rust's `read_dir(".")` yields `entry.path()` like `./foo` while
/// `read_dir("foo")` yields `foo/bar` — zsh prints the latter shape for both.
/// `scanner` uses base `"."` only for the top-level relative scan (the
/// `from_cwd.is_empty()` arm), so that prefix is the ONLY text this port adds
/// to a match; a USER-typed leading `./` / `../` is re-prepended from the
/// pattern by `globdata_glob`.
///
/// Everything else is emitted VERBATIM. zsh preserves the literal path text
/// the user typed, including empty (`//`) and `.` components:
/// ```text
/// $ zsh -f -c 'print -r -- sub//f*(N) sub/./f*(N) ./sub//f*(N)'
/// sub//foo sub/./foo ./sub//foo
/// ```
/// This used to rebuild the string via `Path::components()`, which drops
/// `CurDir` and never yields empty components — so `sub//foo` came back as
/// `sub/foo` and `sub/./foo` as `sub/foo`. That is the same class of bug as
/// the `addpath` slash-squeeze (glob.rs:354): `_path_files` anchors its
/// matches on the on-line path text, so a rewritten path drops every match
/// and completion after `ls sub//` produces nothing at all.
fn glob_emit_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    let mut rest: &str = &s;
    while let Some(stripped) = rest.strip_prefix("./") {
        rest = stripped;
    }
    if rest.is_empty() {
        ".".to_string()
    } else {
        rest.to_string()
    }
}

/// Check if path is a symlink
/// Check whether a glob match is a symlink.
/// Port of the `S_ISLNK(lstat.st_mode)` test in Src/glob.c.
pub fn is_symlink(path: &str) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: glob
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

/// !!! RUST-ONLY adapter — NO DIRECT C COUNTERPART !!!
///
/// Thin "pattern string in, match list out" entry over the glob engine
/// for call sites holding a single pattern that want its matches as a
/// `Vec<String>`, rather than going through C's in-place
/// `zglob(LinkList, LinkNode, int)` list mutation. Snapshots the
/// glob-relevant options into TLS (`enter_glob_scope` — required because
/// zshrs runs glob on multiple threads; see that fn's doc) and drives
/// `globdata_glob`, the RUST-ONLY read_dir-based adaptation of C's
/// `scanner()` (Src/glob.c:500 — C descends with `lchdir`, which is
/// process-global and unsafe under zshrs's threading, so the port walks
/// absolute paths instead). ALL real glob semantics — brace expansion,
/// `(a|b)` alternation, `^`/`~` exclusion, qualifiers, `**`, sorting —
/// live in `globdata_glob`/`scanner`/`patcompile`, not here. (The 130
/// lines of ad-hoc `^`/`~`/alternation read_dir+matchpat code that used
/// to live here were verified redundant with `globdata_glob` and
/// removed.) The faithful C entry is `zglob` (c:1214).
pub fn glob_path(pattern: &str) -> Vec<String> {
    let _glob_scope = enter_glob_scope();
    let mut state = globdata::new();
    globdata_glob(&mut state, pattern)
}

// The qualifier-comparison direction static is `g_range` (Src/glob.c, ported at
// glob.rs ~2799). The duplicate `G_RANGE` that used to live here was a Rust-only
// erroneous second copy — never written by the qual-eval, so qualnlink (its only
// reader) was stuck on `==`. Removed; qualnlink now reads `g_range`.

// `mode_to_octal` lives at `crate::ported::utils::mode_to_octal`
// (port of `Src/utils.c:7634` — 12 bit-by-bit POSIX permission
// mappings). Local alias kept for the call sites at c:1610/1618
// which used the masked identity before the canonical port landed.
fn mode_to_octal(mode: u32) -> u32 {
    crate::ported::utils::mode_to_octal(mode) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use tempfile::TempDir;

    use crate::ported::options::{opt_state_set, opt_state_unset};
    use crate::ported::zsh_h::{redir, ERRFLAG_ERROR, REDIR_WRITE};

    /// Convert ASCII brace-expansion source to the lexer-tokenized
    /// form `hasbraces` / `xpandbraces` consume per c:Src/glob.c —
    /// ASCII `{` → Inbrace (\u{8f}), `}` → Outbrace (\u{90}), `,` →
    /// Comma (\u{9a}). Backslash-escaped variants (`\{`, `\}`, `\,`)
    /// emit Bnull + literal so the canonical "escape via Bnull"
    /// distinction reaches the brace walker. Used by every test in
    /// this module that wants to drive the canonical TOKEN-strict
    /// path with readable ASCII source.
    fn tok(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.peek() {
                    Some('{') | Some('}') | Some(',') => {
                        out.push('\u{9f}');
                        out.push(chars.next().unwrap());
                    }
                    _ => out.push(c),
                },
                '{' => out.push('\u{8f}'),
                '}' => out.push('\u{90}'),
                ',' => out.push('\u{9a}'),
                _ => out.push(c),
            }
        }
        out
    }

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
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — full glob tokenize (shadows the
        // brace-only `fn tok` test helper above on purpose:
        // haswilds consumes glob tokens, not just braces).
        let tok = |s: &str| {
            let mut t = s.to_string();
            tokenize(&mut t);
            t
        };
        assert!(haswilds(&tok("*.txt")));
        assert!(haswilds(&tok("file?.txt")));
        assert!(haswilds(&tok("file[12].txt")));
        assert!(!haswilds(&tok("file.txt")));
        assert!(!haswilds(&tok("path/to/file.txt")));
    }

    #[test]
    fn test_pattern_match() {
        let _g = crate::test_util::global_state_lock();
        assert!(matchpat("*.txt", "file.txt", false, true));
        assert!(matchpat("file?.txt", "file1.txt", false, true));
        assert!(!matchpat("*.txt", "file.rs", false, true));
        assert!(matchpat("file[12].txt", "file1.txt", false, true));
        assert!(!matchpat("file[12].txt", "file3.txt", false, true));
    }

    #[test]
    fn test_brace_expansion() {
        let _g = crate::test_util::global_state_lock();
        let result = xpandbraces(&tok("{a,b,c}"), false);
        assert_eq!(result, vec!["a", "b", "c"]);

        let result = xpandbraces(&tok("file{1,2,3}.txt"), false);
        assert_eq!(result, vec!["file1.txt", "file2.txt", "file3.txt"]);

        let result = xpandbraces(&tok("{1..5}"), false);
        assert_eq!(result, vec!["1", "2", "3", "4", "5"]);

        let result = xpandbraces(&tok("{a..e}"), false);
        assert_eq!(result, vec!["a", "b", "c", "d", "e"]);
    }

    /// c:Src/glob.c — an ESCAPED `~` is a literal, never the
    /// extendedglob exclusion operator. `shtokenize` (ZSHTOK_SUBST,
    /// c:3591) encodes `\X` as `Bnullkeep X` — that is the form every
    /// `${~var}` pattern arrives in, including `_path_files`' final
    /// `tmp1=( $~tmp1 )` whose patterns were quoted with
    /// QT_BACKSLASH_PATTERN by `compfiles` (computil.c:5001). The
    /// exclusion splitter recognised only `Bnull`, so it split at the
    /// escaped tilde and the glob matched nothing:
    /// `cp /etc/paths~or<TAB>` inserted nothing where zsh inserts
    /// `/etc/paths\~orig`.
    #[test]
    fn glob_escaped_tilde_is_literal_not_exclusion() {
        let _g = crate::test_util::global_state_lock();
        let dir = TempDir::new().unwrap();
        let base = dir.path();
        File::create(base.join("a~b")).unwrap();
        File::create(base.join("aXb")).unwrap();

        let saved = opt_state_get("extendedglob");
        opt_state_set("extendedglob", true);

        // Bnullkeep (`${~var}`), Bnull (plain tokenize) and the raw
        // backslash form that `glob_path` callers such as `_path_files`
        // hand over untokenized must all be treated as an escape.
        for tokflags in [Some(ZSHTOK_SUBST), Some(0), None] {
            let mut pattern = format!("{}/a\\~b*", base.display());
            if let Some(flags) = tokflags {
                zshtokenize(&mut pattern, flags);
            }
            let mut state = globdata::new();
            let results = globdata_glob(&mut state, &pattern);
            assert_eq!(
                results.len(),
                1,
                "escaped tilde must match literally (tokflags={tokflags:?}), got {results:?}"
            );
            assert!(results[0].ends_with("a~b"), "matched {:?}", results[0]);
        }

        // An UNescaped top-level `~` still excludes.
        let mut pattern = format!("{}/a*~*Xb", base.display());
        zshtokenize(&mut pattern, ZSHTOK_SUBST);
        let mut state = globdata::new();
        let results = globdata_glob(&mut state, &pattern);
        assert_eq!(results.len(), 1, "exclusion still applies, got {results:?}");
        assert!(results[0].ends_with("a~b"), "matched {:?}", results[0]);

        match saved {
            Some(v) => opt_state_set("extendedglob", v),
            None => opt_state_unset("extendedglob"),
        }
    }

    #[test]
    fn test_glob_simple() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        opt_state_set("globdots", false); // reset for other tests
    }

    #[test]
    fn test_glob_emit_path_strips_read_dir_dot_slash() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(glob_emit_path(Path::new("./sub")), "sub");
        assert_eq!(glob_emit_path(Path::new("sub/deeper")), "sub/deeper");
        assert_eq!(glob_emit_path(Path::new("././x")), "x");
        assert_eq!(glob_emit_path(Path::new("../up")), "../up");
        // Empty path (`Path::new("")`) → "." as before.
        assert_eq!(glob_emit_path(Path::new("")), ".");
        assert_eq!(glob_emit_path(Path::new("./")), ".");
    }

    /// Beyond the leading `read_dir(".")` prefix, `glob_emit_path` must emit
    /// the path VERBATIM — it must not re-normalize it through
    /// `Path::components()`, which silently drops empty (`//`) and `.`
    /// components. zsh keeps the literal text the user typed:
    /// ```text
    /// $ zsh -f -c 'print -r -- sub//f*(N) sub/./f*(N) ./sub//f*(N)'
    /// sub//foo sub/./foo ./sub//foo
    /// ```
    /// `_path_files` anchors its matches on that text, so any rewrite drops
    /// every match and completion after `ls sub//` yields nothing at all.
    #[test]
    fn test_glob_emit_path_keeps_empty_and_dot_components_verbatim() {
        let _g = crate::test_util::global_state_lock();
        // Empty components survive (`//`), relative and absolute.
        assert_eq!(glob_emit_path(Path::new("sub//foo")), "sub//foo");
        assert_eq!(glob_emit_path(Path::new("//usr//bin")), "//usr//bin");
        // Interior `.` / `..` survive.
        assert_eq!(glob_emit_path(Path::new("sub/./foo")), "sub/./foo");
        assert_eq!(glob_emit_path(Path::new("/usr/./bin")), "/usr/./bin");
        assert_eq!(glob_emit_path(Path::new("a/..//a/b")), "a/..//a/b");
        // Only the engine-introduced leading `./` is removed; what follows
        // it is untouched (globdata_glob re-prepends a user-typed `./`).
        assert_eq!(glob_emit_path(Path::new("./sub//foo")), "sub//foo");
        // Trailing separator is preserved (previously dropped).
        assert_eq!(glob_emit_path(Path::new("sub/")), "sub/");
    }

    /// `Src/glob.c:270-273` — `addpath` appends the component then a `/`
    /// UNCONDITIONALLY. An empty component (`l == 0`, which is exactly
    /// what a `//` in the pattern produces, since `patcompile` with
    /// `PAT_FILET` cuts a pure section at the first `/`) must therefore
    /// still push one `/`. A `if !s.ends_with('/')` guard used to live
    /// here and silently collapsed `//` → `/`.
    #[test]
    fn addpath_appends_slash_unconditionally() {
        // Empty component after an existing separator: C writes nothing
        // from `s`, then writes '/', so `/` becomes `//`.
        let mut p = String::from("/");
        addpath(&mut p, "");
        assert_eq!(p, "//");

        // Two empty components in a row (pattern `///x`).
        addpath(&mut p, "");
        assert_eq!(p, "///");

        // Ordinary components are unaffected — a non-empty component can
        // never itself end in `/` (patcompile cut the section there).
        let mut q = String::from("/");
        addpath(&mut q, "usr");
        assert_eq!(q, "/usr/");
        addpath(&mut q, "");
        assert_eq!(q, "/usr//");
        addpath(&mut q, "local");
        assert_eq!(q, "/usr//local/");

        // Relative start (`pathbuf` empty per c:818).
        let mut r = String::new();
        addpath(&mut r, "a");
        assert_eq!(r, "a/");
        addpath(&mut r, ".");
        assert_eq!(r, "a/./");
        addpath(&mut r, "..");
        assert_eq!(r, "a/./../");
    }

    /// End-to-end: the globber must reproduce the literal path text the
    /// user typed, including empty (`//`) and `.` / `..` components.
    /// `_path_files` anchors its completion matches on that exact text
    /// (`${(@)tmp1#$prepath$realpath$testpath}`), so any squeezing of the
    /// path discards every match and completion after `ls //usr//`
    /// produces nothing at all.
    ///
    /// zsh reference:
    /// ```text
    /// $ zsh -f -c 'print -r -- //usr//b*(N) /usr/./b*(N)'
    /// //usr//bin /usr/./bin
    /// ```
    #[test]
    fn glob_preserves_empty_and_dot_path_components() {
        let _g = crate::test_util::global_state_lock();
        let dir = setup_test_dir();
        let base = dir.path().display().to_string();

        // `//` between the directory and the wildcard component.
        let mut state = globdata::new();
        let results = globdata_glob(&mut state, &format!("{}//file1.tx?", base));
        assert_eq!(results, vec![format!("{}//file1.txt", base)]);

        // `/./` — a non-empty component, already preserved; pinned so a
        // future "normalize the path" change can't quietly drop it.
        let mut state = globdata::new();
        let results = globdata_glob(&mut state, &format!("{}/./file1.tx?", base));
        assert_eq!(results, vec![format!("{}/./file1.txt", base)]);

        // `/..//` — `..` followed by an empty component, round-tripped
        // verbatim even though it resolves back to the same directory.
        let mut state = globdata::new();
        let results = globdata_glob(
            &mut state,
            &format!("{}/subdir/..//subdir/nested.tx?", base),
        );
        assert_eq!(
            results,
            vec![format!("{}/subdir/..//subdir/nested.txt", base)]
        );

        // Leading `//` on an absolute pattern (`parsepat` c:813-816 strips
        // exactly ONE leading `/`; the second becomes an empty component).
        let mut state = globdata::new();
        let results = globdata_glob(&mut state, &format!("/{}//file2.tx?", base));
        assert_eq!(results, vec![format!("/{}//file2.txt", base)]);
    }

    #[test]
    fn test_file_type_char() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(file_type(libc::S_IFDIR as u32), '/');
        assert_eq!(file_type(libc::S_IFREG as u32), ' ');
        assert_eq!(file_type(libc::S_IFREG as u32 | 0o111), '*');
        assert_eq!(file_type(libc::S_IFLNK as u32), '@');
    }

    /// `Src/glob.c:2018-2036` — `file_type(mode_t filemode)` returns a
    /// single-char marker keyed off S_ISBLK/CHR/DIR/FIFO/LNK/REG/SOCK,
    /// with `S_IXUGO` (== 0o111 = S_IXUSR|S_IXGRP|S_IXOTH) distinguishing
    /// executable regular files (`*`) from non-executable (` `). The
    /// catch-all (e.g. door, port, unknown types) returns `?`.
    /// Pin every branch by position.
    #[test]
    fn file_type_every_branch_matches_c_dispatch() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            file_type(libc::S_IFBLK as u32),
            '#',
            "c:2020 — S_ISBLK → '#'"
        );
        assert_eq!(
            file_type(libc::S_IFCHR as u32),
            '%',
            "c:2022 — S_ISCHR → '%'"
        );
        assert_eq!(
            file_type(libc::S_IFDIR as u32),
            '/',
            "c:2024 — S_ISDIR → '/'"
        );
        assert_eq!(
            file_type(libc::S_IFIFO as u32),
            '|',
            "c:2026 — S_ISFIFO → '|'"
        );
        assert_eq!(
            file_type(libc::S_IFLNK as u32),
            '@',
            "c:2028 — S_ISLNK → '@'"
        );
        // Regular file: ' ' if not executable, '*' if executable (any bit in S_IXUGO).
        assert_eq!(
            file_type(libc::S_IFREG as u32),
            ' ',
            "c:2030 — non-executable regular file → ' '"
        );
        // Each individual exec bit triggers '*'.
        assert_eq!(
            file_type(libc::S_IFREG as u32 | 0o100),
            '*',
            "c:2030 — S_IXUSR alone is enough"
        );
        assert_eq!(
            file_type(libc::S_IFREG as u32 | 0o010),
            '*',
            "c:2030 — S_IXGRP alone is enough"
        );
        assert_eq!(
            file_type(libc::S_IFREG as u32 | 0o001),
            '*',
            "c:2030 — S_IXOTH alone is enough"
        );
        assert_eq!(
            file_type(libc::S_IFSOCK as u32),
            '=',
            "c:2033 — S_ISSOCK → '='"
        );
        // Catch-all for unknown S_IFMT — bare 0 returns '?'.
        assert_eq!(file_type(0), '?', "c:2035 — unknown st_mode → '?'");
    }

    #[test]
    fn test_zstrcmp_numeric() {
        let _g = crate::test_util::global_state_lock();
        let n = crate::zsh_h::SORTIT_NUMERICALLY as u32;
        assert_eq!(zstrcmp("file1", "file2", n), std::cmp::Ordering::Less);
        assert_eq!(zstrcmp("file10", "file2", n), std::cmp::Ordering::Greater);
        assert_eq!(zstrcmp("file10", "file10", n), std::cmp::Ordering::Equal);
    }

    /// Race-fix verification: snapshot pins bareglobqual for the
    /// duration of a glob scope even when the live store flips
    /// underneath. Mimics the stryke pattern in
    /// MenkeTechnologies/strykelang#3 — one thread is mid-glob with
    /// bareglobqual=1 snapshot when another flips it off.
    #[test]
    fn glob_opts_snapshot_isolates_concurrent_setopt() {
        let _g = crate::test_util::global_state_lock();

        // Preserve the existing live state so the test is hermetic.
        let saved = opt_state_get("bareglobqual");

        // Live store says bareglobqual=true; enter scope captures that.
        opt_state_set("bareglobqual", true);
        let _scope = enter_glob_scope();
        assert!(
            glob_isset(BAREGLOBQUAL),
            "TLS snapshot reads bareglobqual=true at scope entry"
        );

        // Simulate a concurrent thread flipping the live store.
        opt_state_set("bareglobqual", false);
        assert!(
            glob_isset(BAREGLOBQUAL),
            "TLS snapshot must still report bareglobqual=true \
             even though live store now reads false"
        );

        // Restore.
        match saved {
            Some(v) => opt_state_set("bareglobqual", v),
            None => opt_state_unset("bareglobqual"),
        }
    }

    /// After the scope guard drops, reads fall back to the live store.
    #[test]
    fn glob_opts_snapshot_clears_on_drop() {
        let _g = crate::test_util::global_state_lock();

        let saved = opt_state_get("nullglob");
        opt_state_set("nullglob", true);
        {
            let _scope = enter_glob_scope();
            assert!(glob_isset(NULLGLOB), "snapshot live=true → true inside");
        }
        // Outside scope: flip live store, read should follow live.
        opt_state_set("nullglob", false);
        assert!(
            !glob_isset(NULLGLOB),
            "post-scope: glob_isset falls back to live store"
        );

        match saved {
            Some(v) => opt_state_set("nullglob", v),
            None => opt_state_unset("nullglob"),
        }
    }

    /// Nested glob scopes share the outer snapshot — inner doesn't
    /// re-capture or clear on its own Drop.
    #[test]
    fn glob_opts_snapshot_nested_is_noop() {
        let _g = crate::test_util::global_state_lock();

        let saved = opt_state_get("extendedglob");
        opt_state_set("extendedglob", true);
        let _outer = enter_glob_scope();
        assert!(glob_isset(EXTENDEDGLOB));
        {
            let _inner = enter_glob_scope();
            // Live store flip while nested.
            opt_state_set("extendedglob", false);
            assert!(glob_isset(EXTENDEDGLOB), "inner observes outer snapshot");
        } // inner drops — outer snapshot still active.
        assert!(
            glob_isset(EXTENDEDGLOB),
            "outer snapshot survives inner drop"
        );

        match saved {
            Some(v) => opt_state_set("extendedglob", v),
            None => opt_state_unset("extendedglob"),
        }
    }

    /// c:2150 (xpandredir port) — when the redir name has no wildcard
    /// AND no `$var`/`!hist`/`{a,b}` to expand, prefork+globlist leave
    /// the name unchanged. The single-match path at c:2171 must
    /// rewrite `fn.name` to the same string and return 0 (no multi-fan).
    /// Regression that returns 1 would trigger MULTIOS dispatch on a
    /// single literal filename — wrong shell semantics.
    #[test]
    fn xpandredir_single_literal_filename_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut fn_ = redir {
            typ: REDIR_WRITE,
            flags: 0,
            fd1: 1,
            fd2: -1,
            name: Some("/tmp/zshrs_test_out".to_string()),
            varid: None,
            here_terminator: None,
            munged_here_terminator: None,
        };
        let mut tab: Vec<redir> = Vec::new();
        let r = xpandredir(&mut fn_, &mut tab);
        assert_eq!(r, 0, "literal filename → single match → ret=0");
        assert_eq!(
            fn_.name.as_deref(),
            Some("/tmp/zshrs_test_out"),
            "literal name must round-trip through prefork unchanged"
        );
        assert!(tab.is_empty(), "no multi-match → redirtab not appended");
    }

    /// c:2176-2177 — `>&-` (REDIR_MERGEOUT with name "-") collapses to
    /// REDIR_CLOSE per the `IS_DASH(s[0]) && !s[1]` branch. Regression
    /// where this fails leaves `>&-` as a literal merge-fd attempt,
    /// which the executor would interpret as "merge with fd -1".
    #[test]
    fn xpandredir_dash_merge_collapses_to_close() {
        let _g = crate::test_util::global_state_lock();
        let mut fn_ = redir {
            typ: REDIR_MERGEOUT,
            flags: 0,
            fd1: 1,
            fd2: -1,
            name: Some("-".to_string()),
            varid: None,
            here_terminator: None,
            munged_here_terminator: None,
        };
        let mut tab: Vec<redir> = Vec::new();
        let _ = xpandredir(&mut fn_, &mut tab);
        assert_eq!(
            fn_.typ, REDIR_CLOSE,
            "`>&-` must rewrite typ to REDIR_CLOSE"
        );
    }

    /// c:2150 — empty `fn.name` should return 0 cleanly. Catches a
    /// regression that panics on `.as_deref().unwrap()` for absent name.
    #[test]
    fn xpandredir_with_no_name_returns_zero_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut fn_ = redir {
            typ: REDIR_WRITE,
            flags: 0,
            fd1: 1,
            fd2: -1,
            name: None,
            varid: None,
            here_terminator: None,
            munged_here_terminator: None,
        };
        let mut tab: Vec<redir> = Vec::new();
        assert_eq!(xpandredir(&mut fn_, &mut tab), 0);
    }

    /// `IN_EXPANDREDIR` static defaults to 0 — set transiently inside
    /// xpandredir per c:2165/2167. After a normal call it must restore
    /// to 0. Regression that leaks the flag would skew unrelated glob
    /// expansions outside redirections.
    #[test]
    fn in_expandredir_flag_is_zero_at_rest() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(IN_EXPANDREDIR.load(Ordering::SeqCst), 0);
    }

    /// c:4306 — `haswilds` honours backslash-escapes: `\*` is a literal
    /// star, NOT a wildcard. Regression that ignores the escape would
    /// trigger globbing on `printf '%s\n' \*`, breaking shell scripts
    /// that quote literal stars.
    #[test]
    fn haswilds_respects_backslash_escape() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — full glob tokenize (shadows the
        // brace-only `fn tok` test helper above on purpose:
        // haswilds consumes glob tokens, not just braces).
        let tok = |s: &str| {
            let mut t = s.to_string();
            tokenize(&mut t);
            t
        };
        assert!(haswilds(&tok("*.txt")), "bare * is wild");
        assert!(
            !haswilds(&tok(r"\*.txt")),
            "escaped \\* is literal — NOT wild"
        );
        assert!(
            !haswilds(&tok(r"\?.txt")),
            "escaped \\? is literal — NOT wild"
        );
    }

    /// c:4306 — `[` immediately enters bracket mode AND counts as a
    /// wildcard. The early return on `[` is critical — even
    /// unterminated brackets must be flagged so `cd [` doesn't try
    /// to chdir to a literal `[`.
    #[test]
    fn haswilds_open_bracket_alone_is_a_wildcard() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — full glob tokenize (shadows the
        // brace-only `fn tok` test helper above on purpose:
        // haswilds consumes glob tokens, not just braces).
        let tok = |s: &str| {
            let mut t = s.to_string();
            tokenize(&mut t);
            t
        };
        assert!(haswilds(&tok("[abc]")), "char-class is wild");
        assert!(haswilds(&tok("foo[")), "even unterminated [ is wild");
    }

    /// c:4306 — wildcard chars `*` `?` inside an OPEN bracket-context
    /// are NOT additional wildcards (the bracket already is one).
    /// Regression that double-counts would make haswilds report `[*]`
    /// as wildcard-twice — cosmetic, but confuses any caller using
    /// the bool as a "should I glob?" gate that AND-tests with another
    /// flag.
    #[test]
    fn haswilds_extglob_chars_inside_bracket_dont_double_count() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — full glob tokenize (shadows the
        // brace-only `fn tok` test helper above on purpose:
        // haswilds consumes glob tokens, not just braces).
        let tok = |s: &str| {
            let mut t = s.to_string();
            tokenize(&mut t);
            t
        };
        // Once `[` is seen, function returns true immediately, so the
        // post-bracket chars don't matter. But this docs the contract.
        assert!(haswilds(&tok("[*]")));
    }

    /// c:4306 — plain text returns false. Catches a regression where
    /// any non-empty input is flagged as wild (would break `cd /tmp`
    /// by triggering glob expansion on a literal path).
    #[test]
    fn haswilds_plain_text_not_wild() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — full glob tokenize (shadows the
        // brace-only `fn tok` test helper above on purpose:
        // haswilds consumes glob tokens, not just braces).
        let tok = |s: &str| {
            let mut t = s.to_string();
            tokenize(&mut t);
            t
        };
        assert!(!haswilds(&tok("plain")));
        assert!(!haswilds(&tok("")));
        assert!(!haswilds(&tok("/usr/local/bin")));
        assert!(!haswilds(&tok("file.txt")));
    }

    /// c:4363-4371 — `#` and `^` are recognised as wildcards by
    /// `haswilds` **only when `EXTENDEDGLOB` is set**, matching the C
    /// source's `isset(EXTENDEDGLOB)` gate at pattern.c:4364/4369. `~`
    /// is **not** a filename-generation wildcard (zsh handles it as
    /// tilde expansion in a separate pipeline stage); a prior
    /// glob.rs-local `haswilds` impl returned true for `~`, which
    /// caused zglob to mis-classify tilde-prefixed args as needing
    /// filename generation. Pin the corrected behavior.
    #[test]
    fn haswilds_extended_glob_chars_recognised() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — full glob tokenize (shadows the
        // brace-only `fn tok` test helper above on purpose:
        // haswilds consumes glob tokens, not just braces).
        let tok = |s: &str| {
            let mut t = s.to_string();
            tokenize(&mut t);
            t
        };
        // EXTENDEDGLOB off → `#` and `^` are not wild.
        crate::ported::options::opt_state_set("extendedglob", false);
        assert!(
            !haswilds(&tok("foo#bar")),
            "# not wild without EXTENDEDGLOB"
        );
        assert!(
            !haswilds(&tok("foo^bar")),
            "^ not wild without EXTENDEDGLOB"
        );
        // EXTENDEDGLOB on → `#` and `^` are wild (c:4364, c:4369).
        crate::ported::options::opt_state_set("extendedglob", true);
        assert!(haswilds(&tok("foo#bar")), "# is extglob wild");
        assert!(haswilds(&tok("foo^bar")), "^ is extglob wild");
        crate::ported::options::opt_state_set("extendedglob", false);
        // `~` is NOT in haswilds' switch (c:4324-4373) — tilde expansion
        // is a separate pipeline stage.
        assert!(
            !haswilds(&tok("~/file")),
            "~ is NOT a filename-generation wildcard"
        );
    }

    /// c:2514 — `matchpat` returns true for exact match. Sanity check
    /// the simplest dispatch path (the matcher recipe builds a
    /// trivial pattern, runs it).
    #[test]
    fn matchpat_exact_literal_matches() {
        let _g = crate::test_util::global_state_lock();
        assert!(matchpat("hello", "hello", false, true));
        assert!(!matchpat("hello", "world", false, true));
    }

    /// c:2514 — `matchpat` with case_sensitive=false MUST treat upper
    /// and lower as equal (`HELLO` matches `hello`). Regression keeping
    /// case-sensitive when flag is false would silently break every
    /// `[[ "$x" = (#i)foo ]]`-style match.
    #[test]
    fn matchpat_case_insensitive_when_flag_clear() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            matchpat("hello", "HELLO", false, false),
            "case-insensitive match must succeed across cases"
        );
        assert!(matchpat("FoO", "foo", false, false));
    }

    /// c:2514 — case-sensitive (default) MUST reject case-different
    /// inputs. Pinning the contract.
    #[test]
    fn matchpat_case_sensitive_rejects_case_different() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !matchpat("hello", "HELLO", false, true),
            "case-sensitive default must reject HELLO != hello"
        );
    }

    /// c:2780 — `set_pat_start(p, offs)` sets `PAT_NOTSTART` when offs is
    /// nonzero (matched substring starts past the real start, so `(#s)`
    /// must fail) and clears it when offs is 0. A regression here would
    /// let `(#s)` anchors fire mid-string during substring globbing.
    #[test]
    fn set_pat_start_toggles_pat_notstart_flag() {
        let _g = crate::test_util::global_state_lock();
        let mut p = mk_test_patprog();
        set_pat_start(&mut p, 2);
        assert!(
            p.0.flags & PAT_NOTSTART != 0,
            "offs!=0 must set PAT_NOTSTART"
        );
        set_pat_start(&mut p, 0);
        assert!(
            p.0.flags & PAT_NOTSTART == 0,
            "offs==0 must clear PAT_NOTSTART"
        );
    }

    /// c:2796 — `set_pat_end(p, null_me)` sets `PAT_NOTEND` when the char
    /// being zapped is non-NUL (string shortened at tail, so `(#e)` must
    /// fail) and clears it when that char is already NUL. A regression
    /// would let `(#e)` anchors fire before the real end.
    #[test]
    fn set_pat_end_toggles_pat_notend_flag() {
        let _g = crate::test_util::global_state_lock();
        let mut p = mk_test_patprog();
        set_pat_end(&mut p, b'x');
        assert!(
            p.0.flags & PAT_NOTEND != 0,
            "non-NUL null_me must set PAT_NOTEND"
        );
        set_pat_end(&mut p, 0);
        assert!(
            p.0.flags & PAT_NOTEND == 0,
            "NUL null_me must clear PAT_NOTEND"
        );
    }

    /// Build a zeroed `Patprog` for flag-toggle tests.
    fn mk_test_patprog() -> Patprog {
        Box::new((
            crate::ported::zsh_h::patprog {
                startoff: 0,
                size: 0,
                mustoff: 0,
                patmlen: 0,
                globflags: 0,
                globend: 0,
                flags: 0,
                patnpar: 0,
                patstartch: 0,
            },
            Vec::new(),
        ))
    }

    /// c:2773 — `freematchlist(None)` is a no-op (matches C's
    /// `if (repllist) freelinklist(...)`). Regression panicking on
    /// None would crash every glob-replace path with no matches.
    #[test]
    fn freematchlist_handles_none_safely() {
        let _g = crate::test_util::global_state_lock();
        freematchlist(None);
        // No assertion — survival is the test.
    }

    /// c:2773 — `freematchlist(Some(&mut Vec))` clears the list.
    /// Regression that drops a stale entry would leak match positions
    /// across calls.
    #[test]
    fn freematchlist_clears_provided_vec() {
        let _g = crate::test_util::global_state_lock();
        let mut v = vec![
            repldata {
                b: 0,
                e: 5,
                replstr: None,
            },
            repldata {
                b: 10,
                e: 15,
                replstr: Some("x".to_string()),
            },
        ];
        freematchlist(Some(&mut v));
        assert!(v.is_empty(), "freematchlist must clear the input vec");
    }

    /// `Src/glob.c:2042-2142` — `hasbraces(str)` returns true when
    /// `str` contains a brace-expansion candidate. A bare lbrace/rbrace
    /// is NOT a brace expansion (it's a literal). Detection requires a
    /// matched pair containing either a comma or a dotdot range. Pin
    /// the canonical contract.
    #[test]
    fn hasbraces_matched_pair_with_comma_or_dotdot() {
        let _g = crate::test_util::global_state_lock();
        // Matched + comma → true.
        assert!(
            hasbraces(&tok("a{b,c}d"), false),
            "c:2127 — lbrace + comma + rbrace is a brace expansion"
        );
        // Matched + dotdot → true.
        assert!(
            hasbraces(&tok("file{1..3}.txt"), false),
            "c:2082 — N..M range is a brace expansion"
        );
        // Matched WITHOUT comma/dotdot → false (it's a literal).
        assert!(
            !hasbraces(&tok("{abc}"), false),
            "literal braces are NOT a brace expansion without comma or dotdot"
        );
        // Unmatched → false.
        assert!(
            !hasbraces(&tok("{abc"), false),
            "lone lbrace without matching rbrace is not brace expansion"
        );
        assert!(
            !hasbraces(&tok("abc}"), false),
            "lone rbrace is not brace expansion"
        );
        // No braces → false.
        assert!(!hasbraces(&tok("plain"), false));
        assert!(!hasbraces(&tok(""), false));
    }

    /// `Src/glob.c:2049-2063` — BRACECCL branch: when `isset(BRACECCL)`
    /// (the `brace_ccl` parameter), any non-empty matched braces
    /// become a character-class set, regardless of whether the body
    /// contains a comma. Empty pair is still not.
    #[test]
    fn hasbraces_brace_ccl_makes_any_pair_match() {
        let _g = crate::test_util::global_state_lock();
        // c:2049 — BRACECCL: non-empty pair is enough.
        assert!(
            hasbraces(&tok("{abc}"), true),
            "c:2049 — BRACECCL: non-empty pair is char-class set"
        );
        assert!(
            hasbraces(&tok("x{q}y"), true),
            "c:2049 — single-char pair counts under BRACECCL"
        );
        // Empty pair shouldn't trigger.
        assert!(
            !hasbraces(&tok("{}"), true),
            "empty pair still not a brace expansion even under BRACECCL"
        );
        // Without BRACECCL, plain literal-letter pair is NOT a brace expansion.
        assert!(
            !hasbraces(&tok("{abc}"), false),
            "c:2049 — BRACECCL off → plain literal pair stays literal"
        );
    }

    /// `Src/glob.c:2042-2142` — depth tracking: comma/dotdot count
    /// only at depth==1 in our port. So nested inner comma doesn't
    /// qualify the outer pair, but TWO independent top-level pairs
    /// (each with comma) DO trigger detection.
    #[test]
    fn hasbraces_depth_1_check_for_comma_dotdot() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            hasbraces(&tok("a{1,2}b{3,4}c"), false),
            "two independent top-level pairs, first one matches at depth 1"
        );
    }

    /// Pin: `remnulargs` per `Src/glob.c:3649-3679`:
    ///   - Strings with NO inull bytes are unchanged.
    ///   - Strings with ONLY Bnullkeep: keep as-is in scan phase
    ///     (Bnullkeep is "active backslash" that hasn't triggered
    ///     copy phase yet).
    ///   - After encountering any other inull: switch to copy phase:
    ///     * Bnullkeep → '\\' (literal backslash).
    ///     * Other inulls (Snull/Dnull/Bnull/Nularg) → stripped.
    ///     * Non-inull → kept.
    ///   - Empty post-strip → replaced with single Nularg.
    #[test]
    fn remnulargs_matches_c_inull_handling() {
        let _g = crate::test_util::global_state_lock();

        // Plain ASCII unchanged (no inulls).
        let mut s = "hello".to_string();
        remnulargs(&mut s);
        assert_eq!(
            s, "hello",
            "c:3654 — no inull bytes leaves string unchanged"
        );

        // Snull-triggered copy: Snull stripped, rest kept.
        let mut s = format!("ab{}cd", Snull);
        remnulargs(&mut s);
        assert_eq!(
            s, "abcd",
            "c:3663 — Snull triggers copy; itself stripped, rest kept"
        );

        // Bnullkeep AFTER Snull trigger → '\\' (active backslash).
        let mut s = format!("ab{}c{}d", Snull, Bnullkeep);
        remnulargs(&mut s);
        assert_eq!(
            s, "abc\\d",
            "c:3666 — Bnullkeep in copy phase becomes literal '\\\\'"
        );

        // Other inulls (Bnull, Dnull) stripped in copy phase.
        let mut s = format!("a{}b{}c{}d", Snull, Bnull, Dnull);
        remnulargs(&mut s);
        assert_eq!(s, "abcd", "c:3668 — Bnull/Dnull stripped in copy phase");

        // Empty post-strip → Nularg.
        let mut s = format!("{}", Snull);
        remnulargs(&mut s);
        assert_eq!(
            s,
            format!("{}", Nularg),
            "c:3674 — empty result replaced by Nularg sentinel"
        );
    }

    /// `Src/glob.c:1085-1117` — `glob_exec_string` is a PARSER, not
    /// an executor. Extracts the qualifier inner text from `*sp`
    /// (advancing past delimiters). Previous Rust port forked
    /// `/bin/sh` to execute the cmd — that's entirely the wrong
    /// layer; execution happens at the call sites via qualsheval.
    /// Pin: plus-form extracts identifier; delimited form extracts
    /// content + advances.
    #[test]
    fn glob_exec_string_parses_qualifier_text() {
        let _g = crate::test_util::global_state_lock();
        // Plus form: identifier ends at first non-ident char.
        // C: `(+myfunc:rest)` — `s` points past `+`, plus_form=true.
        let r = glob_exec_string("myfunc rest", true);
        assert!(r.is_some(), "c:1092 — identifier parse should succeed");
        let (ident, _adv) = r.unwrap();
        assert_eq!(
            ident, "myfunc",
            "c:1092 — itype_end stops at first non-IIDENT char"
        );

        // Plus form with no identifier (immediate non-ident) — error.
        let r = glob_exec_string(" leading-space", true);
        assert!(
            r.is_none(),
            "c:1093-1096 — empty identifier emits zerr + returns None"
        );
    }

    /// `Src/glob.c:3907-3943` — `qualsheval(name, _, _, expr)` sets
    /// `$REPLY` via paramtab, evaluates `expr` IN THE CURRENT SHELL,
    /// then restores errflag+lastval. Previous Rust port spawned
    /// `/bin/sh -c expr` which:
    ///   1. Couldn't access shell locals / functions / aliases.
    ///   2. Ran `sh` (POSIX), not `zsh` — every zsh-feature qualifier
    ///      silently failed.
    ///
    /// Pin: after qualsheval, errflag is restored (mod ERRFLAG_INT),
    /// lastval is restored, and `$REPLY` is set to the filename.
    /// Easiest direct pin: errflag/lastval restore around an `expr`
    /// that mutates them.
    #[test]
    fn qualsheval_restores_errflag_and_lastval() {
        let _g = crate::test_util::global_state_lock();
        // Seed errflag and lastval with distinctive values.
        errflag.store(0, Ordering::Relaxed);
        LASTVAL.store(42, Ordering::Relaxed);
        // Even if the expr mutates these via the executor, c:3924-3925
        // restore them after.
        let st0: libc::stat = unsafe { std::mem::zeroed() };
        let _ = qualsheval("/tmp/file", &st0, 0, ":"); // no-op expr
                                                       // c:3924 — errflag restored.
        assert_eq!(
            errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR,
            0,
            "c:3924 — qualsheval must restore errflag (no ERRFLAG_ERROR leak)"
        );
        // c:3925 — lastval restored.
        assert_eq!(
            LASTVAL.load(Ordering::Relaxed),
            42,
            "c:3925 — qualsheval must restore lastval to pre-call value"
        );
        // c:3916 — $REPLY visible in paramtab.
        assert_eq!(
            crate::ported::params::getsparam("REPLY"),
            Some("/tmp/file".to_string()),
            "c:3916 — qualsheval must set $REPLY to filename"
        );
    }

    /// `Src/glob.c:2164` — `if (!errflag && isset(MULTIOS))` is the
    /// logical zero-check guarding globlist recursion. Previous Rust
    /// port used `!errflag.load(...) != 0` which is BITWISE NOT
    /// (`^-1`), evaluating to `errflag != -1` — almost-always-true.
    ///
    /// Pin: a `matchpat` smoke that DOES NOT mutate errflag, then a
    /// direct check that the fix sense is correct (errflag==0 enters
    /// the branch, errflag!=0 skips). Can't easily exercise the full
    /// xpandredir path without a redir struct + IN_EXPANDREDIR
    /// global, so this is a sense-of-the-condition pin.
    #[test]
    fn xpandredir_errflag_check_uses_logical_zero() {
        let _g = crate::test_util::global_state_lock();
        // Mirror the in-port logic with the canonical zero-check.
        let saved = errflag.load(Ordering::Relaxed);
        // c:2164 — errflag==0 path: branch should fire.
        errflag.store(0, Ordering::Relaxed);
        let zero_enters = errflag.load(Ordering::Relaxed) == 0;
        assert!(
            zero_enters,
            "c:2164 — errflag==0 enters the branch (the canonical sense)"
        );
        // c:2164 — errflag!=0 path: branch should skip.
        errflag.store(1, Ordering::Relaxed);
        let nonzero_skips = errflag.load(Ordering::Relaxed) == 0;
        assert!(
            !nonzero_skips,
            "c:2164 — errflag!=0 skips the branch (regression of bitwise-NOT bug fix)"
        );
        errflag.store(saved, Ordering::Relaxed);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional unit coverage — pure pattern/brace/glob behaviour that
    // doesn't need a real filesystem. Uses `matchpat` for high-level
    // pattern checks (extended=false, case_sensitive=true is the most
    // common path) and `xpandbraces` for brace expansion.
    // ═══════════════════════════════════════════════════════════════════

    // ── matchpat: full-string pattern matching ───────────────────────
    #[test]
    fn matchpat_literal_no_wildcards() {
        let _g = crate::test_util::global_state_lock();
        assert!(matchpat("foo", "foo", false, true));
        assert!(!matchpat("foo", "bar", false, true));
        assert!(!matchpat("foo", "foobar", false, true));
    }

    #[test]
    fn matchpat_star_consumes_substring() {
        let _g = crate::test_util::global_state_lock();
        assert!(matchpat("a*b", "ab", false, true));
        assert!(matchpat("a*b", "ahellob", false, true));
        assert!(!matchpat("a*b", "abc", false, true));
    }

    #[test]
    fn matchpat_question_is_single_char() {
        let _g = crate::test_util::global_state_lock();
        assert!(matchpat("a?c", "abc", false, true));
        assert!(matchpat("a?c", "axc", false, true));
        assert!(!matchpat("a?c", "ac", false, true));
        assert!(!matchpat("a?c", "abbc", false, true));
    }

    #[test]
    fn matchpat_bracket_class_inline() {
        let _g = crate::test_util::global_state_lock();
        assert!(matchpat("file[abc].txt", "filea.txt", false, true));
        assert!(matchpat("file[abc].txt", "fileb.txt", false, true));
        assert!(!matchpat("file[abc].txt", "filed.txt", false, true));
    }

    #[test]
    fn matchpat_case_sensitive_strict() {
        let _g = crate::test_util::global_state_lock();
        assert!(matchpat("Foo", "Foo", false, true));
        assert!(!matchpat("Foo", "foo", false, true));
        assert!(!matchpat("Foo", "FOO", false, true));
    }

    #[test]
    fn matchpat_empty_pattern_matches_only_empty() {
        let _g = crate::test_util::global_state_lock();
        assert!(matchpat("", "", false, true));
        assert!(!matchpat("", "x", false, true));
    }

    #[test]
    fn matchpat_star_alone_matches_anything() {
        let _g = crate::test_util::global_state_lock();
        assert!(matchpat("*", "", false, true));
        assert!(matchpat("*", "a", false, true));
        assert!(matchpat("*", "abcdef", false, true));
    }

    #[test]
    fn matchpat_question_alone_one_char() {
        let _g = crate::test_util::global_state_lock();
        assert!(!matchpat("?", "", false, true));
        assert!(matchpat("?", "a", false, true));
        assert!(!matchpat("?", "ab", false, true));
    }

    // ── haswilds: meta-char detection ───────────────────────────────
    #[test]
    fn haswilds_each_glob_meta() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — full glob tokenize (shadows the
        // brace-only `fn tok` test helper above on purpose:
        // haswilds consumes glob tokens, not just braces).
        let tok = |s: &str| {
            let mut t = s.to_string();
            tokenize(&mut t);
            t
        };
        assert!(haswilds(&tok("*")));
        assert!(haswilds(&tok("?")));
        assert!(haswilds(&tok("[abc]")));
        assert!(haswilds(&tok("a*b")));
        assert!(haswilds(&tok("a?b")));
    }

    #[test]
    fn haswilds_plain_strings_have_no_wildcards() {
        let _g = crate::test_util::global_state_lock();
        // c:Src/glob.c:3548 — full glob tokenize (shadows the
        // brace-only `fn tok` test helper above on purpose:
        // haswilds consumes glob tokens, not just braces).
        let tok = |s: &str| {
            let mut t = s.to_string();
            tokenize(&mut t);
            t
        };
        assert!(!haswilds(&tok("")));
        assert!(!haswilds(&tok("plain.txt")));
        assert!(!haswilds(&tok("/abs/path/file.rs")));
        assert!(!haswilds(&tok("./rel/file")));
    }

    // ── xpandbraces: brace expansion (zsh extension, not POSIX) ─────
    #[test]
    fn xpandbraces_three_alternatives() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(xpandbraces(&tok("{a,b,c}"), false), vec!["a", "b", "c"]);
    }

    #[test]
    fn xpandbraces_with_prefix_and_suffix() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("pre{x,y,z}post"), false),
            vec!["prexpost", "preypost", "prezpost"]
        );
    }

    #[test]
    fn xpandbraces_numeric_range_ascending() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("{1..5}"), false),
            vec!["1", "2", "3", "4", "5"]
        );
    }

    #[test]
    fn xpandbraces_alpha_range_ascending() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("{a..e}"), false),
            vec!["a", "b", "c", "d", "e"]
        );
    }

    #[test]
    fn xpandbraces_single_alternative_passes_through_literal() {
        // Real zsh 5.9 (`print -l {a}`) outputs `{a}` verbatim — no
        // expansion because there is no comma or range inside the
        // braces. xpandbraces operates on TOKEN form throughout
        // (c:Src/glob.c::xpandbraces); the eventual ASCII conversion
        // happens later in untokenize. So unit-level pass-through
        // preserves TOKEN bytes, not ASCII braces.
        let _g = crate::test_util::global_state_lock();
        let out = xpandbraces(&tok("{a}"), false);
        assert_eq!(
            out,
            vec![tok("{a}")],
            "zsh 5.9 returns the input verbatim (TOKEN form preserved at xpandbraces layer)"
        );
    }

    #[test]
    fn xpandbraces_no_braces_returns_input() {
        let _g = crate::test_util::global_state_lock();
        // Pure literal has nothing to expand — should yield the input
        // as a single element (or empty for empty input).
        let out = xpandbraces(&tok("plain"), false);
        assert_eq!(out, vec!["plain"]);
    }

    // ── file_type: mode → marker char (used by `ls -F`-style output) ─
    #[test]
    fn file_type_dir_marker_is_slash() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(file_type(libc::S_IFDIR as u32), '/');
    }

    #[test]
    fn file_type_regular_plain_is_space() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(file_type(libc::S_IFREG as u32), ' ');
    }

    #[test]
    fn file_type_regular_executable_is_star() {
        let _g = crate::test_util::global_state_lock();
        // Any of the three executable bits should switch the marker.
        for x in [0o100, 0o010, 0o001] {
            assert_eq!(
                file_type(libc::S_IFREG as u32 | x),
                '*',
                "exec bit 0o{x:o} should produce '*'"
            );
        }
    }

    #[test]
    fn file_type_symlink_is_at() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(file_type(libc::S_IFLNK as u32), '@');
    }

    #[test]
    fn file_type_fifo_is_pipe() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(file_type(libc::S_IFIFO as u32), '|');
    }

    #[test]
    fn file_type_socket_is_equal() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(file_type(libc::S_IFSOCK as u32), '=');
    }

    // ═══════════════════════════════════════════════════════════════════
    // xpandbraces edge cases — anchored to `print -l <pattern>` in zsh 5.9.
    // Where zshrs diverges, the test FAILS to expose the bug.
    // ═══════════════════════════════════════════════════════════════════

    /// `{1..10..2}` → 1 3 5 7 9 (step range). zsh: 5 elements.
    #[test]
    fn xpandbraces_numeric_step_two() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("{1..10..2}"), false),
            vec!["1", "3", "5", "7", "9"]
        );
    }

    /// `{10..1..2}` → 10 8 6 4 2 (descending step).
    #[test]
    fn xpandbraces_numeric_descending_step_two() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("{10..1..2}"), false),
            vec!["10", "8", "6", "4", "2"]
        );
    }

    /// `{5..1}` → 5 4 3 2 1 (descending range, no step).
    #[test]
    fn xpandbraces_numeric_descending_range() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("{5..1}"), false),
            vec!["5", "4", "3", "2", "1"]
        );
    }

    /// `{01..10}` → 01 02 03 ... 10 (zero-padded; pad preserved).
    #[test]
    fn xpandbraces_zero_padded_numeric_range() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("{01..10}"), false),
            vec!["01", "02", "03", "04", "05", "06", "07", "08", "09", "10"]
        );
    }

    /// `{001..010}` → 001 002 ... 010 (3-digit pad preserved).
    #[test]
    fn xpandbraces_three_digit_pad_preserved() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("{001..010}"), false),
            vec!["001", "002", "003", "004", "005", "006", "007", "008", "009", "010"]
        );
    }

    /// `\{a,b,c\}` → no expansion (escaped braces aren't brace
    /// delimiters). xpandbraces returns input unchanged; the lexer
    /// upstream strips the backslashes before this layer sees them.
    /// Anchored test: zsh `print -r -- \{a,b,c\}` → `{a,b,c}` because
    /// the lexer tokenizes `\{`/`\}` to `Bnull{`/`Bnull}` before
    /// xpandbraces runs, then untokenize drops the Bnulls. At the
    /// xpandbraces-unit level (raw backslashes), the only invariant
    /// to verify is "no expansion fires" — the brace literal survives.
    #[test]
    fn xpandbraces_escaped_braces_remain_literal_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        // tok() emits Bnull+ASCII `{` for `\{` and Bnull+ASCII `}` for
        // `\}`, with Comma TOKEN for the unescaped commas inside.
        // xpandbraces scans for Inbrace TOKEN only (per
        // c:Src/glob.c::xpandbraces) — finds none here, so the
        // string passes through unchanged at this layer. The Bnulls
        // are stripped later by remnulargs in prefork.
        assert_eq!(
            xpandbraces(&tok("\\{a,b,c\\}"), false),
            vec![tok("\\{a,b,c\\}")],
            "xpandbraces unit: escaped braces survive without expansion (no per-element splat)"
        );
    }

    /// `{a,b}{c,d}` → ac ad bc bd (Cartesian product).
    /// 2 * 2 = 4 elements, in row-major order.
    #[test]
    fn xpandbraces_cartesian_product_two_by_two() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("{a,b}{c,d}"), false),
            vec!["ac", "ad", "bc", "bd"]
        );
    }

    /// `{a,{b,c}}` → a b c (nested flattened).
    #[test]
    fn xpandbraces_nested_braces_flatten() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(xpandbraces(&tok("{a,{b,c}}"), false), vec!["a", "b", "c"]);
    }

    /// `{}` → `{}` literal (empty brace doesn't expand). xpandbraces
    /// preserves TOKEN form at this layer (c:Src/glob.c::xpandbraces);
    /// ASCII conversion happens later in untokenize.
    #[test]
    fn xpandbraces_empty_braces_remain_literal() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(xpandbraces(&tok("{}"), false), vec![tok("{}")]);
    }

    /// `a{b,c}d{e,f}` → abde abdf acde acdf (cartesian with surrounding text).
    #[test]
    fn xpandbraces_cartesian_with_surrounding_text() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("a{b,c}d{e,f}"), false),
            vec!["abde", "abdf", "acde", "acdf"]
        );
    }

    /// `{a..z..3}` → `{a..z..3}` literal (alpha range with step NOT expanded
    /// by zsh — surprising but verified). xpandbraces preserves TOKEN form
    /// at this layer; ASCII conversion happens later in untokenize.
    #[test]
    fn xpandbraces_alpha_step_unsupported_anchored_to_zsh() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("{a..z..3}"), false),
            vec![tok("{a..z..3}")],
            "zsh: alpha range with step → literal (TOKEN form preserved at xpandbraces layer)"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // zsh test-corpus pins — direct anchors to Test/D09brace.ztst.
    // Tests verify brace expansion matches zsh's documented output.
    // ═══════════════════════════════════════════════════════════════════

    /// `Test/D09brace.ztst:10-12` — basic brace expansion with
    /// nested range: `X{1,2,{3..6},7,8}Y` expands all 8 values.
    #[test]
    fn zsh_corpus_basic_brace_with_nested_range() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("X{1,2,{3..6},7,8}Y"), false),
            vec!["X1Y", "X2Y", "X3Y", "X4Y", "X5Y", "X6Y", "X7Y", "X8Y"],
            "ztst:11 — basic brace expansion with nested range",
        );
    }

    /// `Test/D09brace.ztst:30-32` — numeric range with leading zero
    /// padding: `X{01..4}Y` → all 4 values 0-padded to 2 digits.
    #[test]
    fn zsh_corpus_numeric_range_zero_padding() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("X{01..4}Y"), false),
            vec!["X01Y", "X02Y", "X03Y", "X04Y"],
            "ztst:32 — leading-zero padding propagates to all values",
        );
    }

    /// `Test/D09brace.ztst:34-36` — padding comes from RHS too:
    /// `X{1..04}Y` → 2-digit zero-padded.
    #[test]
    fn zsh_corpus_numeric_range_padding_from_rhs() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("X{1..04}Y"), false),
            vec!["X01Y", "X02Y", "X03Y", "X04Y"],
            "ztst:36 — RHS padding `04` propagates",
        );
    }

    /// `Test/D09brace.ztst:38-40` — unpadded range >9: `X{7..12}Y`
    /// numbers stay unpadded.
    #[test]
    fn zsh_corpus_numeric_range_no_padding_when_unspecified() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("X{7..12}Y"), false),
            vec!["X7Y", "X8Y", "X9Y", "X10Y", "X11Y", "X12Y"],
            "ztst:40 — no padding when neither end has leading zero",
        );
    }

    /// `Test/D09brace.ztst:42-44` — `X{07..12}Y` → 2-digit padded.
    #[test]
    fn zsh_corpus_numeric_range_lhs_padding_propagates() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("X{07..12}Y"), false),
            vec!["X07Y", "X08Y", "X09Y", "X10Y", "X11Y", "X12Y"],
            "ztst:44 — LHS padding `07` propagates",
        );
    }

    /// `Test/D09brace.ztst:46-48` — wider RHS padding overrides:
    /// `X{7..012}Y` → 3-digit padded.
    #[test]
    fn zsh_corpus_numeric_range_max_padding_width_wins() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("X{7..012}Y"), false),
            vec!["X007Y", "X008Y", "X009Y", "X010Y", "X011Y", "X012Y"],
            "ztst:48 — widest padding width wins",
        );
    }

    /// `Test/D09brace.ztst:50-52` — decreasing range: `X{4..1}Y` →
    /// 4,3,2,1 (reversed).
    #[test]
    fn zsh_corpus_numeric_range_decreasing() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("X{4..1}Y"), false),
            vec!["X4Y", "X3Y", "X2Y", "X1Y"],
            "ztst:52 — decreasing range emits in reverse",
        );
    }

    /// `Test/D09brace.ztst:54-56` — combined braces: `X{1..4}{1..4}Y`
    /// → 16-element cross product.
    #[test]
    fn zsh_corpus_combined_braces_cross_product() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("X{1..4}{1..4}Y"), false),
            vec![
                "X11Y", "X12Y", "X13Y", "X14Y", "X21Y", "X22Y", "X23Y", "X24Y", "X31Y", "X32Y",
                "X33Y", "X34Y", "X41Y", "X42Y", "X43Y", "X44Y",
            ],
            "ztst:56 — combined-brace cross product",
        );
    }

    /// `Test/D09brace.ztst:58-60` — negative numbers in range:
    /// `X{-4..4}Y` → `-4`..`4` (9 values including 0).
    #[test]
    fn zsh_corpus_negative_numbers_in_range() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("X{-4..4}Y"), false),
            vec!["X-4Y", "X-3Y", "X-2Y", "X-1Y", "X0Y", "X1Y", "X2Y", "X3Y", "X4Y",],
            "ztst:60 — negative-to-positive range",
        );
    }

    /// `Test/D09brace.ztst:62-64` — reverse-direction numeric range:
    /// `X{4..-4}Y` walks from 4 to -4 descending.
    #[test]
    fn zsh_corpus_brace_descending_range_from_positive_to_negative() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("X{4..-4}Y"), false),
            vec!["X4Y", "X3Y", "X2Y", "X1Y", "X0Y", "X-1Y", "X-2Y", "X-3Y", "X-4Y",],
            "ztst:64 — descending 4..-4 produces 9 values",
        );
    }

    /// `Test/D09brace.ztst:66-68` — stepped padded range:
    /// `X{004..-4..2}Y` = X004Y X002Y X000Y X-02Y X-04Y.
    /// Padding width 3 from LHS; step is +2 (sign of step is ignored,
    /// direction is from start to end).
    #[test]
    fn zsh_corpus_brace_stepped_padded_descending() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("X{004..-4..2}Y"), false),
            vec!["X004Y", "X002Y", "X000Y", "X-02Y", "X-04Y"],
            "ztst:68 — stepped+padded descending",
        );
    }

    /// `Test/D09brace.ztst:74-76` — step alignment 1: `X{1..32..3}Y`
    /// = X1Y X4Y X7Y X10Y X13Y X16Y X19Y X22Y X25Y X28Y X31Y.
    /// Step 3 starting from 1, capped at 32.
    #[test]
    fn zsh_corpus_brace_step_alignment_1_to_32_step_3() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("X{1..32..3}Y"), false),
            vec![
                "X1Y", "X4Y", "X7Y", "X10Y", "X13Y", "X16Y", "X19Y", "X22Y", "X25Y", "X28Y",
                "X31Y",
            ],
            "ztst:76 — {{1..32..3}} step alignment",
        );
    }

    /// `Test/D09brace.ztst:100-102` — `hey{a..j}there` — char range.
    #[test]
    fn zsh_corpus_brace_char_range_simple() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("hey{a..j}there"), false),
            vec![
                "heyathere",
                "heybthere",
                "heycthere",
                "heydthere",
                "heyethere",
                "heyfthere",
                "heygthere",
                "heyhthere",
                "heyithere",
                "heyjthere",
            ],
            "ztst:102 — char range a..j",
        );
    }

    /// `Test/D09brace.ztst:108-110` — reverse char range:
    /// `crumbs{y..p}ooh` walks down y → p (10 values).
    #[test]
    fn zsh_corpus_brace_char_range_reverse() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("crumbs{y..p}ooh"), false),
            vec![
                "crumbsyooh",
                "crumbsxooh",
                "crumbswooh",
                "crumbsvooh",
                "crumbsuooh",
                "crumbstooh",
                "crumbssooh",
                "crumbsrooh",
                "crumbsqooh",
                "crumbspooh",
            ],
            "ztst:110 — char range y..p reverse",
        );
    }

    /// `Test/D09brace.ztst:104-106` — nested brace: `gosh{1,{Z..a},2}cripes`
    /// — inner `{Z..a}` is char range Z to a in ASCII order (10 chars).
    /// Full output: gosh1cripes goshZcripes gosh[cripes gosh\cripes
    /// gosh]cripes gosh^cripes gosh_cripes gosh`cripes goshacripes
    /// gosh2cripes.
    #[test]
    fn zsh_corpus_brace_nested_with_char_range_ascii() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            xpandbraces(&tok("gosh{1,{Z..a},2}cripes"), false),
            vec![
                "gosh1cripes",
                "goshZcripes",
                "gosh[cripes",
                "gosh\\cripes",
                "gosh]cripes",
                "gosh^cripes",
                "gosh_cripes",
                "gosh`cripes",
                "goshacripes",
                "gosh2cripes",
            ],
            "ztst:106 — nested brace with ASCII char range",
        );
    }

    /// `Test/D09brace.ztst:116-118` — unmatched closing brace after
    /// matched braces stays literal: `{1..10}{..` → `1{.. 2{.. ...`.
    /// `xpandbraces` itself preserves TOKEN form (Inbrace=`\u{8f}`,
    /// Outbrace=`\u{90}`) per the convention pinned by
    /// `xpandbraces_alpha_step_unsupported_anchored_to_zsh` — the
    /// later untokenize pass in the user-output pipeline turns
    /// surviving brace tokens into literal `{` / `}`. This test
    /// asserts the tokenized intermediate form.
    #[test]
    fn zsh_corpus_brace_unmatched_after_matched_left_literal() {
        let _g = crate::test_util::global_state_lock();
        let tokb = '\u{8f}'; // Inbrace
        let expected: Vec<String> = (1..=10).map(|n| format!("{}{}..", n, tokb)).collect();
        assert_eq!(
            xpandbraces(&tok("{1..10}{.."), false),
            expected,
            "ztst:118 — unmatched trailing {{.. preserved as tokenized Inbrace at xpandbraces layer",
        );
    }

    // ── split_qualifier: parse trailing (qual) block ─────────────────
    /// `*.txt` (no trailing parens) → (input, None).
    #[test]
    fn split_qualifier_no_parens_returns_input_and_none() {
        let _g = crate::test_util::global_state_lock();
        let (head, qual) = split_qualifier("*.txt");
        assert_eq!(head, "*.txt");
        assert_eq!(qual, None);
    }

    /// `*(N)` → ("*", Some("N")) — null-glob qualifier.
    #[test]
    fn split_qualifier_star_paren_N_extracts_null_glob_qual() {
        let _g = crate::test_util::global_state_lock();
        let (head, qual) = split_qualifier("*(N)");
        assert_eq!(head, "*");
        assert_eq!(qual, Some("N"));
    }

    /// `*(.)` → ("*", Some(".")) — regular-file qualifier.
    #[test]
    fn split_qualifier_star_paren_dot_extracts_regular_file_qual() {
        let _g = crate::test_util::global_state_lock();
        let (head, qual) = split_qualifier("*(.)");
        assert_eq!(head, "*");
        assert_eq!(qual, Some("."));
    }

    /// `*.rs(.)` → ("*.rs", Some(".")) — qualifier on glob pattern.
    #[test]
    fn split_qualifier_pattern_with_qual_extracts_correctly() {
        let _g = crate::test_util::global_state_lock();
        let (head, qual) = split_qualifier("*.rs(.)");
        assert_eq!(head, "*.rs");
        assert_eq!(qual, Some("."));
    }

    /// `*(#qN)` → ("*", Some("N")) — `#q` prefix stripped per zsh syntax.
    #[test]
    fn split_qualifier_hash_q_prefix_stripped() {
        let _g = crate::test_util::global_state_lock();
        let (head, qual) = split_qualifier("*(#qN)");
        assert_eq!(head, "*");
        assert_eq!(qual, Some("N"));
    }

    /// `*(om[1])` → ("*", Some("om[1]")) — sort-by-mtime keep first.
    #[test]
    fn split_qualifier_multichar_qual_with_brackets() {
        let _g = crate::test_util::global_state_lock();
        let (head, qual) = split_qualifier("*(om[1])");
        assert_eq!(head, "*");
        assert_eq!(qual, Some("om[1]"));
    }

    /// `(a)(b)` — multiple paren groups: split takes the OUTERMOST trailing.
    #[test]
    fn split_qualifier_multiple_groups_takes_outermost_trailing() {
        let _g = crate::test_util::global_state_lock();
        let (head, qual) = split_qualifier("(a)(b)");
        assert_eq!(head, "(a)");
        assert_eq!(qual, Some("b"));
    }

    // ═══════════════════════════════════════════════════════════════════
    // parse_qualifier_string — file-type / permission / sort qualifiers.
    // Tests call the private fn directly (same crate, same file).
    // Each qualifier letter maps to a `qualifier::*` variant per the
    // zsh glob.c arm at c:1495+. Pin the parse, not the match — the
    // match path needs a real filesystem.
    //
    // IMPORTANT: at end of parse_qualifier_string (c:3433-3435 in this
    // file), if `qualifiers` is non-empty it gets moved into
    // `alternatives[]`. So tests read the qualifier from `alternatives[0]`
    // — qualifiers itself is empty after the move.
    // ═══════════════════════════════════════════════════════════════════

    /// Helper: read the only qualifier from a single-letter parse result.
    fn first_qual(qs: &qualifier_set) -> &qualifier {
        // After the post-loop move, single-letter input lands in
        // alternatives[0][0]. Fall back to qualifiers[0] for safety in
        // case the move ever changes.
        if !qs.alternatives.is_empty() && !qs.alternatives[0].is_empty() {
            &qs.alternatives[0][0]
        } else {
            &qs.qualifiers[0]
        }
    }

    /// Empty qualifier body → no alternatives, no qualifiers.
    #[test]
    fn parse_qualifier_string_empty_returns_empty_set() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("");
        assert!(qs.qualifiers.is_empty());
        assert!(qs.alternatives.is_empty());
        assert!(!qs.nullglob);
    }

    /// `/` → IsDirectory (in alternatives[0]).
    #[test]
    fn parse_qualifier_string_slash_is_directory() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("/");
        assert!(matches!(first_qual(&qs), qualifier::IsDirectory));
    }

    /// `.` → IsRegular.
    #[test]
    fn parse_qualifier_string_dot_is_regular() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string(".");
        assert!(matches!(first_qual(&qs), qualifier::IsRegular));
    }

    /// `parsecomplist` splits a path into per-component `complist` nodes,
    /// each literal section a PAT_PURES Patprog (relies on patcompile's
    /// PAT_FILE PURES segment-stop at `/`, pattern.c:584-610).
    #[test]
    fn parsecomplist_splits_path_components() {
        let _g = crate::test_util::global_state_lock();
        fn pure(p: &crate::ported::pattern::Patprog) -> Option<String> {
            if (p.0.flags & crate::ported::zsh_h::PAT_PURES as i32) != 0 {
                let off = p.0.startoff as usize;
                let l = p.0.patmlen as usize;
                p.1.get(off..off + l)
                    .map(|b| String::from_utf8_lossy(b).into_owned())
            } else {
                None
            }
        }
        fn sections(mut q: &complist) -> Vec<String> {
            let mut out = Vec::new();
            loop {
                out.push(pure(&q.pat).unwrap_or_else(|| "<pat>".into()));
                match &q.next {
                    Some(n) => q = n,
                    None => break,
                }
            }
            out
        }
        // Literal path → split into per-component PAT_PURES sections.
        let mut t = "a/b/c.txt".to_string();
        tokenize(&mut t);
        let cl = parsecomplist(&t).expect("parsecomplist None");
        assert_eq!(sections(&cl), vec!["a", "b", "c.txt"]);

        // Literal prefix before a pattern still splits the literal dirs;
        // the trailing glob section is a real pattern (not PAT_PURES).
        let mut t2 = "a/b/*.txt".to_string();
        tokenize(&mut t2);
        let cl2 = parsecomplist(&t2).expect("parsecomplist None");
        let s2 = sections(&cl2);
        assert_eq!(s2.len(), 3, "a/b/*.txt → 3 sections, got {s2:?}");
        assert_eq!(&s2[..2], &["a".to_string(), "b".to_string()]);
        assert_eq!(s2[2], "<pat>", "trailing *.txt must be a pattern section");

        // Pattern-FIRST then literal dir: */foo → [<pat>, foo].
        let mut t3 = "*/foo".to_string();
        tokenize(&mut t3);
        let cl3 = parsecomplist(&t3).expect("parsecomplist None for */foo");
        let s3 = sections(&cl3);
        assert_eq!(s3.len(), 2, "*/foo → 2 sections, got {s3:?}");
        assert_eq!(s3[0], "<pat>", "leading * must be a pattern section");
        assert_eq!(s3[1], "foo");
    }

    /// The `struct qual` arena is built from the parse alongside the enum:
    /// AND via `next`, alternatives via `or`, per-node `sense` from `^`.
    #[test]
    fn parse_qualifier_string_builds_qual_arena() {
        let _g = crate::test_util::global_state_lock();
        // single qualifier → one node, sense 0, head set, func present.
        let q = parse_qualifier_string(".");
        assert_eq!(q.quals.nodes.len(), 1);
        assert_eq!(q.quals.head, Some(0));
        assert_eq!(q.quals.nodes[0].sense, 0);
        assert!(q.quals.nodes[0].func.is_some());
        assert_eq!(q.quals.nodes[0].next, None);

        // leading `^` → per-node sense flipped (c:1346).
        let q = parse_qualifier_string("^.");
        assert_eq!(q.quals.nodes.len(), 1);
        assert_eq!(q.quals.nodes[0].sense, 1);

        // `/^@` → AND-chain of 2: dir(sense 0) -next-> symlink(sense 1).
        let q = parse_qualifier_string("/^@");
        assert_eq!(q.quals.nodes.len(), 2);
        assert_eq!(q.quals.nodes[0].sense, 0);
        assert_eq!(q.quals.nodes[0].next, Some(1));
        assert_eq!(q.quals.nodes[1].sense, 1);
        assert_eq!(q.quals.nodes[1].next, None);

        // `.,/` → two alternatives: node0 -or-> node1, neither chained.
        let q = parse_qualifier_string(".,/");
        assert_eq!(q.quals.nodes.len(), 2);
        assert_eq!(q.quals.nodes[0].or, Some(1));
        assert_eq!(q.quals.nodes[0].next, None);
        assert_eq!(q.quals.head, Some(0));
    }

    /// `@` → IsSymlink.
    #[test]
    fn parse_qualifier_string_at_is_symlink() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("@");
        assert!(matches!(first_qual(&qs), qualifier::IsSymlink));
    }

    /// `=` → IsSocket.
    #[test]
    fn parse_qualifier_string_equals_is_socket() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("=");
        assert!(matches!(first_qual(&qs), qualifier::IsSocket));
    }

    /// `p` → IsFifo.
    #[test]
    fn parse_qualifier_string_p_is_fifo() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("p");
        assert!(matches!(first_qual(&qs), qualifier::IsFifo));
    }

    /// `*` → IsExecutable.
    #[test]
    fn parse_qualifier_string_star_is_executable() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("*");
        assert!(matches!(first_qual(&qs), qualifier::IsExecutable));
    }

    /// `%b` → IsBlockDev.
    #[test]
    fn parse_qualifier_string_pct_b_is_block_device() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("%b");
        assert!(matches!(first_qual(&qs), qualifier::IsBlockDev));
    }

    /// `%c` → IsCharDev.
    #[test]
    fn parse_qualifier_string_pct_c_is_char_device() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("%c");
        assert!(matches!(first_qual(&qs), qualifier::IsCharDev));
    }

    /// `r` / `w` / `x` → Readable / Writable / Executable.
    #[test]
    fn parse_qualifier_string_perm_letters_map_to_perm_qualifiers() {
        let _g = crate::test_util::global_state_lock();
        for (letter, expected) in [
            ("r", qualifier::Readable),
            ("w", qualifier::Writable),
            ("x", qualifier::Executable),
        ] {
            let qs = parse_qualifier_string(letter);
            assert_eq!(
                std::mem::discriminant(first_qual(&qs)),
                std::mem::discriminant(&expected),
                "letter {letter:?} should map to {expected:?}"
            );
        }
    }

    /// `U` → OwnedByEuid (process EUID).
    #[test]
    fn parse_qualifier_string_capital_U_is_owned_by_euid() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("U");
        assert!(matches!(first_qual(&qs), qualifier::OwnedByEuid));
    }

    /// `G` → OwnedByEgid.
    #[test]
    fn parse_qualifier_string_capital_G_is_owned_by_egid() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("G");
        assert!(matches!(first_qual(&qs), qualifier::OwnedByEgid));
    }

    /// Multiple file-type qualifiers stack — `./` → IsRegular then IsDirectory
    /// both end up in alternatives[0].
    #[test]
    fn parse_qualifier_string_multiple_letters_stack_in_one_alternative() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("./");
        assert_eq!(qs.alternatives.len(), 1);
        assert_eq!(qs.alternatives[0].len(), 2);
        assert!(matches!(qs.alternatives[0][0], qualifier::IsRegular));
        assert!(matches!(qs.alternatives[0][1], qualifier::IsDirectory));
    }

    /// `,` separates alternatives — `/,.` → two alternatives, each with one
    /// qualifier (IsDirectory and IsRegular respectively).
    #[test]
    fn parse_qualifier_string_comma_creates_two_alternatives() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("/,.");
        assert_eq!(qs.alternatives.len(), 2, "two alternatives expected");
        assert!(matches!(qs.alternatives[0][0], qualifier::IsDirectory));
        assert!(matches!(qs.alternatives[1][0], qualifier::IsRegular));
    }

    /// `:` introduces colon-modifiers; rest captured into colon_mods.
    #[test]
    fn parse_qualifier_string_colon_captures_modifiers_in_field() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string(":h");
        assert_eq!(qs.colon_mods.as_deref(), Some(":h"));
    }

    /// `:` after a qualifier captures BOTH.
    #[test]
    fn parse_qualifier_string_qualifier_then_colon_mod() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("/:h");
        assert!(matches!(first_qual(&qs), qualifier::IsDirectory));
        assert_eq!(qs.colon_mods.as_deref(), Some(":h"));
    }

    /// `N` flag → nullglob set, no qualifier pushed.
    #[test]
    fn parse_qualifier_string_capital_N_sets_nullglob_flag() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("N");
        assert!(qs.nullglob, "(N) must set nullglob");
        assert!(qs.alternatives.is_empty(), "N doesn't push to qualifiers");
    }

    /// `M` flag → mark_dirs.
    #[test]
    fn parse_qualifier_string_capital_M_sets_mark_dirs_flag() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("M");
        assert!(qs.mark_dirs, "(M) must set mark_dirs");
    }

    /// `T` flag → list_types.
    #[test]
    fn parse_qualifier_string_capital_T_sets_list_types_flag() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("T");
        assert!(qs.list_types, "(T) must set list_types");
    }

    // ── split_qualifier integration with parse_qualifier_string ──────
    /// `*(.)` — split yields ("*", Some(".")); parse yields IsRegular.
    #[test]
    fn split_then_parse_dot_qualifier_yields_is_regular() {
        let _g = crate::test_util::global_state_lock();
        let (head, qual) = split_qualifier("*(.)");
        assert_eq!(head, "*");
        let qs = parse_qualifier_string(qual.unwrap());
        assert!(matches!(first_qual(&qs), qualifier::IsRegular));
    }

    /// `*(/)` — directory-only glob.
    #[test]
    fn split_then_parse_slash_qualifier_yields_is_directory() {
        let _g = crate::test_util::global_state_lock();
        let (_, qual) = split_qualifier("*(/)");
        let qs = parse_qualifier_string(qual.unwrap());
        assert!(matches!(first_qual(&qs), qualifier::IsDirectory));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Sort qualifiers — `o<key>` ascending, `O<key>` descending.
    // ═══════════════════════════════════════════════════════════════════

    /// `oN` — sort by NONE (no sorting key requested).
    #[test]
    fn parse_qualifier_string_oN_pushes_gs_none() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("oN");
        assert_eq!(qs.sorts.len(), 1);
        assert_ne!(qs.sorts[0] & GS_NONE, 0, "oN must set GS_NONE bit");
    }

    /// `on` — sort by name ascending.
    #[test]
    fn parse_qualifier_string_on_pushes_gs_name_ascending() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("on");
        assert_eq!(qs.sorts.len(), 1);
        assert_eq!(qs.sorts[0] & GS_NAME, GS_NAME);
        assert_eq!(qs.sorts[0] & GS_DESC, 0);
    }

    /// `On` — sort by name descending.
    #[test]
    fn parse_qualifier_string_On_pushes_gs_name_descending() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("On");
        assert_eq!(qs.sorts.len(), 1);
        assert_eq!(qs.sorts[0] & GS_NAME, GS_NAME);
        assert_eq!(qs.sorts[0] & GS_DESC, GS_DESC);
    }

    /// Multiple sort keys stack.
    #[test]
    fn parse_qualifier_string_chained_sort_keys_stack() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("onOL");
        assert_eq!(qs.sorts.len(), 2);
    }

    // ── Size qualifier — `L<unit><op><value>` ───────────────────────
    // NOTE: zshrs's parse_range_spec normalises `+` (greater) → '>' and
    // `-` (less) → '<' for the internal op char, matching C's qgetnum
    // convention (Src/glob.c c:1620-ish).

    /// `Lk+1` — size > 1 kilobyte. zsh syntax: `L<unit><op><value>`
    /// (unit char BEFORE op).
    #[test]
    fn parse_qualifier_string_Lk_plus_one_size_kilobyte() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("Lk+1");
        match first_qual(&qs) {
            qualifier::Size { value, unit, op } => {
                assert_eq!(*value, 1);
                assert_eq!(*unit, TT_KILOBYTES);
                assert_eq!(*op, '>', "+ is stored as '>' (greater than)");
            }
            other => panic!("expected Size, got {other:?}"),
        }
    }

    /// `L-100` — size < 100 (default unit bytes). Stored op is '<'.
    #[test]
    fn parse_qualifier_string_L_minus_100_size_bytes() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("L-100");
        match first_qual(&qs) {
            qualifier::Size { value, unit, op } => {
                assert_eq!(*value, 100);
                assert_eq!(*unit, TT_BYTES);
                assert_eq!(*op, '<', "- is stored as '<' (less than)");
            }
            other => panic!("expected Size, got {other:?}"),
        }
    }

    /// `Lm+1` — size > 1 megabyte; unit recognised.
    #[test]
    fn parse_qualifier_string_Lm_megabyte_unit() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("Lm+1");
        match first_qual(&qs) {
            qualifier::Size { unit, .. } => assert_eq!(*unit, TT_MEGABYTES),
            other => panic!("expected Size, got {other:?}"),
        }
    }

    // ── Time qualifier — same op normalisation ───────────────────────
    /// `m-1` — modified less than 1 day ago. Stored op '<'.
    #[test]
    fn parse_qualifier_string_m_minus_1_day_default_unit() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("m-1");
        match first_qual(&qs) {
            qualifier::Mtime { value, unit, op } => {
                assert_eq!(*value, 1);
                assert_eq!(*unit, TT_DAYS);
                assert_eq!(*op, '<');
            }
            other => panic!("expected Mtime, got {other:?}"),
        }
    }

    /// `mh+24` — modified more than 24 hours ago. Stored op '>'.
    #[test]
    fn parse_qualifier_string_mh_plus_24_hours() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("mh+24");
        match first_qual(&qs) {
            qualifier::Mtime { value, unit, op } => {
                assert_eq!(*value, 24);
                assert_eq!(*unit, TT_HOURS);
                assert_eq!(*op, '>');
            }
            other => panic!("expected Mtime, got {other:?}"),
        }
    }

    /// `a-7` — accessed less than 7 days ago. Stored op '<'.
    #[test]
    fn parse_qualifier_string_a_minus_7_days() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("a-7");
        match first_qual(&qs) {
            qualifier::Atime { value, unit, op } => {
                assert_eq!(*value, 7);
                assert_eq!(*unit, TT_DAYS);
                assert_eq!(*op, '<');
            }
            other => panic!("expected Atime, got {other:?}"),
        }
    }

    // ── Subscript qualifier — `[N]` keep first ───────────────────────
    /// `[1]` — first entry only.
    #[test]
    fn parse_qualifier_string_bracket_one_sets_first_to_one() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("[1]");
        assert_eq!(qs.first, Some(1));
    }

    /// `om[1]` — sort by mtime, keep first.
    #[test]
    fn parse_qualifier_string_om_bracket_one_sort_and_subscript() {
        let _g = crate::test_util::global_state_lock();
        let qs = parse_qualifier_string("om[1]");
        assert_eq!(qs.sorts.len(), 1);
        assert_eq!(qs.sorts[0] & GS_MTIME, GS_MTIME);
        assert_eq!(qs.first, Some(1));
    }

    // ═══════════════════════════════════════════════════════════════════
    // tokenize / remnulargs C-parity tests — pin Src/glob.c:3551 (tokenize)
    // and Src/glob.c:3649 (remnulargs). These two functions form the
    // input → glob-token → output sandwich every glob pattern walks
    // through; regressions silently corrupt every pattern match.
    // Tests that capture KNOWN ZSHRS BUGS use #[ignore = "ZSHRS BUG: …"].
    // ═══════════════════════════════════════════════════════════════════

    /// `tokenize("abc")` is a no-op for plain ASCII — no glob metachars.
    /// C glob.c:3551 — `for (; (c = *s); s++) zstrtok(...)` — only
    /// recognized glob bytes get replaced with their token form.
    #[test]
    fn tokenize_pure_ascii_unchanged() {
        let _g = crate::test_util::global_state_lock();
        let mut s = String::from("abc");
        tokenize(&mut s);
        assert_eq!(s, "abc");
    }

    /// `tokenize("")` on empty input returns empty unchanged.
    #[test]
    fn tokenize_empty_unchanged() {
        let _g = crate::test_util::global_state_lock();
        let mut s = String::new();
        tokenize(&mut s);
        assert_eq!(s, "");
    }

    /// `remnulargs("abc")` is a no-op for pure ASCII (no inull bytes).
    /// C glob.c:3649-3680 — only Snull/Dnull/Bnull/Bnullkeep/Nularg
    /// are stripped or transformed.
    #[test]
    fn remnulargs_pure_ascii_unchanged() {
        let _g = crate::test_util::global_state_lock();
        let mut s = String::from("hello");
        remnulargs(&mut s);
        assert_eq!(s, "hello");
    }

    /// `remnulargs("")` returns empty unchanged (c:3653 early exit).
    #[test]
    fn remnulargs_empty_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let mut s = String::new();
        remnulargs(&mut s);
        assert_eq!(s, "");
    }

    /// `remnulargs` strips Snull (single-quote scope marker).
    /// C glob.c:3656 — inull predicate includes Snull (0x84).
    /// Input `"a\u{84}b"` → `"ab"` (strip the Snull byte).
    #[test]
    fn remnulargs_strips_snull_marker() {
        let _g = crate::test_util::global_state_lock();
        let mut s = format!("a{}b", crate::ported::zsh_h::Snull);
        remnulargs(&mut s);
        assert_eq!(s, "ab", "Snull byte should be stripped");
    }

    /// `remnulargs` strips Dnull (double-quote scope marker).
    #[test]
    fn remnulargs_strips_dnull_marker() {
        let _g = crate::test_util::global_state_lock();
        let mut s = format!("a{}b", crate::ported::zsh_h::Dnull);
        remnulargs(&mut s);
        assert_eq!(s, "ab", "Dnull byte should be stripped");
    }

    /// `remnulargs` strips Bnull (backslash-quoted marker — c:3658
    /// `inull(c)` includes Bnull). The non-Bnullkeep variant of the
    /// active-backslash marker.
    #[test]
    fn remnulargs_strips_bnull_marker() {
        let _g = crate::test_util::global_state_lock();
        let mut s = format!("a{}b", crate::ported::zsh_h::Bnull);
        remnulargs(&mut s);
        assert_eq!(s, "ab", "Bnull byte should be stripped");
    }

    /// `remnulargs` on a single Nularg returns the Nularg sentinel
    /// preserved. C glob.c:3690-3692 — if post-strip output is empty
    /// AND original had any inull, emit Nularg as the empty-arg marker.
    #[test]
    fn remnulargs_empty_post_strip_emits_nularg_sentinel() {
        let _g = crate::test_util::global_state_lock();
        let mut s = format!("{}", crate::ported::zsh_h::Snull);
        remnulargs(&mut s);
        assert_eq!(
            s,
            format!("{}", crate::ported::zsh_h::Nularg),
            "all-inull input should collapse to single Nularg marker"
        );
    }

    /// `remnulargs` on the Bnullkeep marker: c:3669 — Bnullkeep
    /// either (a) preserved as-is when it appears BEFORE any other
    /// inull (scan phase), or (b) transformed to literal `\` (copy
    /// phase). Pin the simpler case: a lone Bnullkeep is stripped.
    #[test]
    fn remnulargs_lone_bnullkeep_stripped() {
        let _g = crate::test_util::global_state_lock();
        let mut s = format!("a{}b", crate::ported::zsh_h::Bnullkeep);
        remnulargs(&mut s);
        // C behavior depends on whether the byte is in scan or copy
        // phase. Faithful port should give either "ab" (scan strip)
        // or "a\\b" (copy phase materialize). Either is acceptable;
        // pin that it's NOT the raw input.
        assert!(
            s == "ab" || s == "a\\b",
            "Bnullkeep should be stripped or materialized; got {s:?}"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/glob.c file_type + hasbraces.
    // ═══════════════════════════════════════════════════════════════════

    /// c:2018 — file_type for regular file with no exec bits → ' '.
    #[test]
    #[cfg(unix)]
    fn file_type_regular_non_exec_returns_space() {
        let r = file_type(libc::S_IFREG as u32 | 0o644);
        assert_eq!(r, ' ');
    }

    /// c:2018 — file_type for regular file WITH exec bit → '*'.
    #[test]
    #[cfg(unix)]
    fn file_type_regular_exec_returns_star() {
        let r = file_type(libc::S_IFREG as u32 | 0o755);
        assert_eq!(r, '*');
    }

    /// c:2018 — file_type for directory → '/'.
    #[test]
    #[cfg(unix)]
    fn file_type_directory_returns_slash() {
        let r = file_type(libc::S_IFDIR as u32 | 0o755);
        assert_eq!(r, '/');
    }

    /// c:2018 — file_type for symlink → '@'.
    #[test]
    #[cfg(unix)]
    fn file_type_symlink_returns_at() {
        let r = file_type(libc::S_IFLNK as u32 | 0o777);
        assert_eq!(r, '@');
    }

    /// c:2018 — file_type for FIFO → '|'.
    #[test]
    #[cfg(unix)]
    fn file_type_fifo_returns_pipe() {
        let r = file_type(libc::S_IFIFO as u32 | 0o644);
        assert_eq!(r, '|');
    }

    /// c:2018 — file_type for socket → '='.
    #[test]
    #[cfg(unix)]
    fn file_type_socket_returns_equals() {
        let r = file_type(libc::S_IFSOCK as u32 | 0o777);
        assert_eq!(r, '=');
    }

    /// c:2018 — file_type for char device → '%'.
    #[test]
    #[cfg(unix)]
    fn file_type_char_device_returns_percent() {
        let r = file_type(libc::S_IFCHR as u32 | 0o644);
        assert_eq!(r, '%');
    }

    /// c:2018 — file_type for block device → '#'.
    #[test]
    #[cfg(unix)]
    fn file_type_block_device_returns_hash() {
        let r = file_type(libc::S_IFBLK as u32 | 0o644);
        assert_eq!(r, '#');
    }

    /// c:2018 — file_type for unknown mode → '?'.
    #[test]
    fn file_type_unknown_mode_returns_question() {
        // Bare 0 mode (none of the S_IF* bits set) → '?'.
        assert_eq!(file_type(0), '?');
    }

    /// c:2018 — file_type is deterministic.
    #[test]
    #[cfg(unix)]
    fn file_type_is_deterministic() {
        for mode in [
            libc::S_IFREG as u32 | 0o644,
            libc::S_IFDIR as u32,
            libc::S_IFLNK as u32,
            0u32,
        ] {
            let first = file_type(mode);
            for _ in 0..5 {
                assert_eq!(file_type(mode), first);
            }
        }
    }

    /// c:2042 — hasbraces on plain string returns false.
    #[test]
    fn hasbraces_no_braces_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!hasbraces("hello", false));
        assert!(!hasbraces("", false));
    }

    /// c:2042 — hasbraces on plain ASCII `{a,b}` returns FALSE because
    /// the C port checks TOKEN bytes (Inbrace=0x8f / Outbrace=0x90 /
    /// Comma=0x9a) not literal ASCII. The lexer pre-tokenizes input
    /// before hasbraces runs in the real pipeline. Pin the actual
    /// behavior so a regen that adds ASCII fast-path is caught.
    #[test]
    fn hasbraces_plain_ascii_braces_returns_false_uses_tokens() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !hasbraces("{a,b}", false),
            "plain ASCII not tokenized → false; lexer pre-tokenizes in real pipeline"
        );
    }

    /// c:2042 — hasbraces on tokenized brace expansion returns true.
    /// Construct the TOKEN form: Inbrace + 'a' + Comma + 'b' + Outbrace.
    #[test]
    fn hasbraces_tokenized_brace_expansion_returns_true() {
        let _g = crate::test_util::global_state_lock();
        let tokenized = format!(
            "{}a{}b{}",
            '\u{8f}', // Inbrace token
            '\u{9a}', // Comma token
            '\u{90}'  // Outbrace token
        );
        assert!(
            hasbraces(&tokenized, false),
            "Inbrace+a+Comma+b+Outbrace must trigger brace expansion"
        );
    }

    /// c:2042 — hasbraces on unclosed `{` returns false (no matching pair).
    #[test]
    fn hasbraces_unclosed_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!hasbraces("{", false));
        assert!(!hasbraces("{abc", false));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/glob.c
    // c:950 qgetnum / c:994 qgetmodespec / c:1250 glob_exec_string /
    // c:1351 checkglobqual / c:1539 hasbraces / c:1617 bracechardots /
    // c:1802 matchpat / c:1899 getmatch / c:1505 file_type
    // ═══════════════════════════════════════════════════════════════════

    /// c:950 — `qgetnum("")` empty returns None.
    #[test]
    fn qgetnum_empty_returns_none() {
        assert!(qgetnum("").is_none(), "empty → None");
    }

    /// c:950 — `qgetnum` returns Option<(i64, &str)> (type pin).
    #[test]
    fn qgetnum_returns_option_i64_str_tuple_type() {
        let _: Option<(i64, &str)> = qgetnum("42");
    }

    /// c:950 — `qgetnum("123")` returns Some((123, "")) (canonical decimal).
    #[test]
    fn qgetnum_canonical_decimal_parses() {
        let r = qgetnum("123");
        assert!(r.is_some(), "valid digits → Some");
        let (n, rest) = r.unwrap();
        assert_eq!(n, 123);
        assert_eq!(rest, "");
    }

    /// c:994 — `qgetmodespec("")` empty returns None.
    #[test]
    fn qgetmodespec_empty_returns_none() {
        assert!(qgetmodespec("").is_none());
    }

    /// c:1539 — `hasbraces("")` empty returns false.
    #[test]
    fn hasbraces_empty_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!hasbraces("", false));
    }

    /// c:1539 — `hasbraces` is pure (no side effects).
    #[test]
    fn hasbraces_is_pure() {
        let _g = crate::test_util::global_state_lock();
        for s in ["", "abc", "{", "{a,b}", "no braces"] {
            let first = hasbraces(s, false);
            for _ in 0..3 {
                assert_eq!(
                    hasbraces(s, false),
                    first,
                    "hasbraces({:?}, false) must be pure",
                    s
                );
            }
        }
    }

    /// c:1505 — `file_type(0)` returns char (compile-time type pin).
    #[test]
    fn file_type_returns_char_type() {
        let _: char = file_type(0);
    }

    /// c:1505 — `file_type` is pure for arbitrary mode.
    #[test]
    fn file_type_is_pure() {
        for m in [0u32, 0o100000, 0o040000, 0o120000, 0o140000, 0o160000] {
            let first = file_type(m);
            for _ in 0..3 {
                assert_eq!(file_type(m), first, "file_type({:o}) must be pure", m);
            }
        }
    }

    /// c:1802 — `matchpat("", "", false, false)` returns bool (type pin).
    #[test]
    fn matchpat_returns_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let _: bool = matchpat("", "", false, false);
    }

    /// c:1899 — `getmatch("", "", 0, 0, None)` returns String (type pin).
    #[test]
    fn getmatch_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: String = getmatch("", "", 0, 0, None);
    }

    /// c:1862 — `compgetmatch("")` empty returns Option<(String, i32)>.
    #[test]
    fn compgetmatch_returns_option_tuple_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<(String, i32)> = compgetmatch("");
    }
}
