//! !!! RUST-ORIGINAL — no counterpart in `zsh/Src` !!!
//!
//! A pure-Rust reader for the compiled terminfo database, replacing the
//! `setupterm` / `tigetstr` / `tigetnum` / `tigetflag` / `tgetent` /
//! `tgetstr` / `tgetnum` / `tgetflag` entry points zshrs used to import from
//! ncurses (`libtinfo` on Linux, `libncursesw` on macOS).
//!
//! Why this exists: those eighteen symbols were the ONLY reason the binary
//! carried a C terminal library, which made `libtinfo.so.6` a hard install
//! dependency on Ubuntu and `libtinfo-dev` a build dependency in CI. Nothing
//! in zshrs needed the ncurses *screen* library — `zsh/curses` is already a
//! pure-Rust port and imported a single `tigetnum`. Reading the database
//! directly removes the whole dependency and keeps the build self-hosting on
//! a machine with no ncurses headers.
//!
//! The on-disk format is `term(5)` and has been frozen for decades:
//!
//! ```text
//!   header   6 × int16 LE: magic, name-size, bool-count, num-count,
//!                          str-offset-count, string-table-size
//!   names    name-size bytes, NUL-terminated, `|`-separated aliases
//!   bools    bool-count bytes (0 absent, 1 set, 2 cancelled)
//!   pad      one byte when (name-size + bool-count) is odd
//!   nums     num-count × 2 bytes (magic 0o432) or × 4 bytes (magic 0o1036)
//!   offsets  str-offset-count × int16 LE into the string table
//!   strings  string-table-size bytes, NUL-terminated entries
//! ```
//!
//! `-1` means absent and `-2` means cancelled, for numbers and offsets alike.
//! ncurses' extended section (user-defined capabilities such as `Smulx`,
//! `RGB` and the `XM`/`xm` mouse caps) follows the same shape and is parsed
//! too, since `tigetstr` resolves extended names.
//!
//! What this module does NOT do: it never writes the database, and it does
//! not read the hashed (Berkeley DB) form some ncurses builds use. Every
//! platform zshrs targets — Debian/Ubuntu, Fedora, Arch, Alpine, macOS and
//! the BSDs — ships the directory-tree form, which is what is read here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use crate::extensions::terminfo_caps::{
    BOOL_CODES, BOOL_NAMES, NUM_CODES, NUM_NAMES, STR_CODES, STR_NAMES,
};

/// `term(5)` magic for an entry whose numeric capabilities are 16-bit.
const MAGIC_16: u16 = 0o432;
/// ncurses 6 magic for the 32-bit numeric form, written when any numeric
/// capability does not fit in an `i16` (`Co` on direct-colour terminals).
const MAGIC_32: u16 = 0o1036;

/// Value stored for a capability the entry does not define.
pub const ABSENT: i32 = -1;
/// Value stored for a capability the entry explicitly cancels (`cap@`).
pub const CANCELLED: i32 = -2;

/// One compiled terminfo entry, held as the positional arrays the file
/// stores plus ncurses' extended-capability maps.
#[derive(Debug, Default, Clone)]
pub struct TermEntry {
    /// `|`-separated names from the header, first is the primary name.
    pub names: Vec<String>,
    /// Booleans, indexed by position in [`BOOL_NAMES`].
    pub bools: Vec<i8>,
    /// Numbers, indexed by position in [`NUM_NAMES`]; [`ABSENT`]/[`CANCELLED`].
    pub nums: Vec<i32>,
    /// Strings, indexed by position in [`STR_NAMES`]. `None` is absent.
    pub strs: Vec<Option<Vec<u8>>>,
    /// ncurses extended booleans, by capability name.
    pub ext_bools: HashMap<String, bool>,
    /// ncurses extended numbers, by capability name.
    pub ext_nums: HashMap<String, i32>,
    /// ncurses extended strings, by capability name.
    pub ext_strs: HashMap<String, Vec<u8>>,
    /// ncurses' `FIX_SGR0`: the termcap-facing form of `sgr0`, computed by
    /// `tgetent` and substituted for it by `tgetstr`. `None` until `tgetent`
    /// runs, or when the trimmed form equals `sgr0` itself.
    pub fix_sgr0: Option<Vec<u8>>,
}

/// Why `setupterm` could not produce an entry. The discriminants are
/// ncurses' `errret` values, which `zsh/terminfo`'s port already branches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupError {
    /// errret 0 — `$TERM` names an entry that is not in the database.
    NotFound,
    /// errret -1 — no database could be located at all.
    NoDatabase,
    /// errret 0 with an empty/unset `$TERM`.
    Unset,
}

impl SetupError {
    /// ncurses' `errret` code, so callers can reproduce its exact returns.
    pub fn errret(self) -> i32 {
        match self {
            SetupError::NoDatabase => -1,
            SetupError::NotFound | SetupError::Unset => 0,
        }
    }
}

fn read_i16(b: &[u8], at: usize) -> Option<i16> {
    let hi = *b.get(at + 1)?;
    let lo = *b.get(at)?;
    Some(i16::from_le_bytes([lo, hi]))
}

fn read_i32(b: &[u8], at: usize) -> Option<i32> {
    let s: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
    Some(i32::from_le_bytes(s))
}

/// Widen a stored numeric capability. Both widths sign-extend, so `-1`
/// (absent) and `-2` (cancelled) survive the 16-bit form unchanged.
fn num_at(b: &[u8], at: usize, wide: bool) -> Option<i32> {
    if wide {
        read_i32(b, at)
    } else {
        read_i16(b, at).map(i32::from)
    }
}

/// NUL-terminated slice of the string table starting at `off`.
fn cstr_at(table: &[u8], off: usize) -> Option<Vec<u8>> {
    let rest = table.get(off..)?;
    let end = rest.iter().position(|&c| c == 0).unwrap_or(rest.len());
    Some(rest[..end].to_vec())
}

impl TermEntry {
    /// Parse one compiled entry. Returns `None` when the bytes are not a
    /// terminfo file or are truncated — a corrupt entry is treated the same
    /// as a missing one, exactly as ncurses does.
    pub fn parse(b: &[u8]) -> Option<TermEntry> {
        let magic = read_i16(b, 0)? as u16;
        let wide = match magic {
            MAGIC_16 => false,
            MAGIC_32 => true,
            _ => return None,
        };
        let name_size = read_i16(b, 2)?.max(0) as usize;
        let bool_count = read_i16(b, 4)?.max(0) as usize;
        let num_count = read_i16(b, 6)?.max(0) as usize;
        let str_count = read_i16(b, 8)?.max(0) as usize;
        let table_size = read_i16(b, 10)?.max(0) as usize;
        let num_width = if wide { 4 } else { 2 };

        let mut p = 12;
        let names_raw = b.get(p..p + name_size)?;
        let names = String::from_utf8_lossy(names_raw)
            .trim_end_matches('\0')
            .split('|')
            .map(|s| s.to_string())
            .collect();
        p += name_size;

        let bools: Vec<i8> = b.get(p..p + bool_count)?.iter().map(|&v| v as i8).collect();
        p += bool_count;

        // term(5): numbers start on an even boundary relative to the file.
        if p % 2 != 0 {
            p += 1;
        }

        let mut nums = Vec::with_capacity(num_count);
        for i in 0..num_count {
            nums.push(num_at(b, p + i * num_width, wide)?);
        }
        p += num_count * num_width;

        let mut offsets = Vec::with_capacity(str_count);
        for i in 0..str_count {
            offsets.push(read_i16(b, p + i * 2)?);
        }
        p += str_count * 2;

        let table = b.get(p..p + table_size)?;
        p += table_size;

        let strs = offsets
            .iter()
            .map(|&o| {
                if o < 0 {
                    None
                } else {
                    cstr_at(table, o as usize)
                }
            })
            .collect();

        let mut e = TermEntry {
            names,
            bools,
            nums,
            strs,
            ..Default::default()
        };
        // The extended section is optional; a truncated or absent one just
        // leaves the maps empty rather than failing the whole entry.
        e.parse_extended(b, p, wide);
        Some(e)
    }

    /// ncurses' extended (user-definable) capability block, which follows the
    /// standard section after realignment. Layout mirrors the main one: a
    /// 5-short header, then bools, nums, and a single offset array holding
    /// the extended string VALUES followed by the NAMES of all three kinds.
    fn parse_extended(&mut self, b: &[u8], mut p: usize, wide: bool) {
        if p % 2 != 0 {
            p += 1;
        }
        let Some(ext_bool) = read_i16(b, p).map(|v| v.max(0) as usize) else {
            return;
        };
        let Some(ext_num) = read_i16(b, p + 2).map(|v| v.max(0) as usize) else {
            return;
        };
        let Some(ext_str) = read_i16(b, p + 4).map(|v| v.max(0) as usize) else {
            return;
        };
        let Some(ext_off) = read_i16(b, p + 6).map(|v| v.max(0) as usize) else {
            return;
        };
        let Some(ext_size) = read_i16(b, p + 8).map(|v| v.max(0) as usize) else {
            return;
        };
        p += 10;
        let num_width = if wide { 4 } else { 2 };

        let Some(bools) = b.get(p..p + ext_bool) else {
            return;
        };
        let bools: Vec<i8> = bools.iter().map(|&v| v as i8).collect();
        p += ext_bool;
        if p % 2 != 0 {
            p += 1;
        }

        let mut nums = Vec::with_capacity(ext_num);
        for i in 0..ext_num {
            match num_at(b, p + i * num_width, wide) {
                Some(v) => nums.push(v),
                None => return,
            }
        }
        p += ext_num * num_width;

        let mut offsets = Vec::with_capacity(ext_off);
        for i in 0..ext_off {
            match read_i16(b, p + i * 2) {
                Some(v) => offsets.push(v),
                None => return,
            }
        }
        p += ext_off * 2;

        let Some(table) = b.get(p..p + ext_size) else {
            return;
        };

        // The first `ext_str` offsets are values; everything after is the
        // name list, ordered bools, then numbers, then strings.
        let values: Vec<Option<Vec<u8>>> = offsets
            .iter()
            .take(ext_str)
            .map(|&o| {
                if o < 0 {
                    None
                } else {
                    cstr_at(table, o as usize)
                }
            })
            .collect();
        let names: Vec<String> = offsets
            .iter()
            .skip(ext_str)
            .map(|&o| {
                if o < 0 {
                    String::new()
                } else {
                    cstr_at(table, o as usize)
                        .map(|v| String::from_utf8_lossy(&v).into_owned())
                        .unwrap_or_default()
                }
            })
            .collect();

        let mut it = names.into_iter();
        for i in 0..ext_bool {
            if let Some(n) = it.next() {
                if !n.is_empty() {
                    self.ext_bools.insert(n, bools.get(i).copied().unwrap_or(0) == 1);
                }
            }
        }
        for i in 0..ext_num {
            if let Some(n) = it.next() {
                if !n.is_empty() {
                    self.ext_nums.insert(n, nums.get(i).copied().unwrap_or(ABSENT));
                }
            }
        }
        for i in 0..ext_str {
            if let Some(n) = it.next() {
                if n.is_empty() {
                    continue;
                }
                if let Some(Some(v)) = values.get(i) {
                    self.ext_strs.insert(n, v.clone());
                }
            }
        }
    }

    /// Boolean capability by terminfo long name (`am`, `xenl`, …), falling
    /// back to the extended map. ncurses returns `-1` for a name that is not
    /// a boolean at all; that is the caller's `tigetflag` contract.
    pub fn flag(&self, name: &str) -> i32 {
        if let Some(i) = BOOL_NAMES.iter().position(|&n| n == name) {
            return i32::from(self.bools.get(i).copied().unwrap_or(0) == 1);
        }
        if let Some(&v) = self.ext_bools.get(name) {
            return i32::from(v);
        }
        // `tigetflag(3)`: "-1 if capname is not a boolean capability". That
        // covers an UNKNOWN name as well as a known one of another kind —
        // returning 0 here made every two-letter termcap code answer "no"
        // when the `zsh/terminfo` scan probed it, so `$terminfo` grew ~220
        // phantom entries (`S1`, `S2`, `YA`-`YG`, `ta`, `te`, `xn`, …) on
        // terminals whose codes are not also capability names.
        -1
    }

    /// Numeric capability by terminfo long name.
    ///
    /// `tigetnum(3)`: "-1 if `capname` is not a numeric capability, or -2 if
    /// it is canceled or absent". A CANCELLED value stored in the entry
    /// (`ncv@`, `colors@`) is therefore reported as [`ABSENT`] — only an
    /// unknown-or-wrong-kind NAME yields [`CANCELLED`]. Returning the raw
    /// -2 here made `$terminfo[ncv]` read -2 where zsh reads -1 on 45 of the
    /// 394 entries in the reference database.
    pub fn num(&self, name: &str) -> i32 {
        if let Some(i) = NUM_NAMES.iter().position(|&n| n == name) {
            let v = self.nums.get(i).copied().unwrap_or(ABSENT);
            return if v == CANCELLED { ABSENT } else { v };
        }
        if let Some(&v) = self.ext_nums.get(name) {
            return v;
        }
        // `tigetnum(3)`: "-2 if capname is not a numeric capability" — an
        // unknown name included. [`ABSENT`] is reserved for a numeric
        // capability the entry does not define.
        CANCELLED
    }

    /// String capability by terminfo long name. `None` is absent; the
    /// wrong-type case is reported separately by [`TermEntry::str_is_wrong_type`]
    /// so callers can reproduce ncurses' `(char *)-1`.
    pub fn string(&self, name: &str) -> Option<&[u8]> {
        if let Some(i) = STR_NAMES.iter().position(|&n| n == name) {
            return self.strs.get(i).and_then(|o| o.as_deref());
        }
        self.ext_strs.get(name).map(|v| v.as_slice())
    }

    /// Write (or clear) a string capability by terminfo long name, growing
    /// the positional array when the compiled entry was shorter than the slot.
    /// Only `tgetent`'s termcap fixups mutate a loaded entry.
    pub fn set_string(&mut self, name: &str, value: Option<Vec<u8>>) {
        let Some(i) = STR_NAMES.iter().position(|&n| n == name) else {
            return;
        };
        if self.strs.len() <= i {
            self.strs.resize(i + 1, None);
        }
        self.strs[i] = value;
    }

    /// True when `name` is not a string capability at all, which is what
    /// ncurses signals with a `(char *)-1` return from `tigetstr` — for a
    /// capability of another kind and for an unknown name alike.
    pub fn str_is_wrong_type(&self, name: &str) -> bool {
        !STR_NAMES.contains(&name) && !self.ext_strs.contains_key(name)
    }

    /// Boolean capability by two-letter TERMCAP code (`am`, `xn`, …).
    pub fn tc_flag(&self, code: &str) -> i32 {
        match BOOL_CODES.iter().position(|&c| c == code) {
            Some(i) => i32::from(self.bools.get(i).copied().unwrap_or(0) == 1),
            None => 0,
        }
    }

    /// Numeric capability by two-letter TERMCAP code (`li`, `co`, `Co`, …).
    /// `tgetnum` reports a missing capability as `-1`.
    pub fn tc_num(&self, code: &str) -> i32 {
        match NUM_CODES.iter().position(|&c| c == code) {
            // Cancelled reads as absent here too — `tgetnum` has only the
            // one failure value.
            Some(i) => self.nums.get(i).copied().unwrap_or(ABSENT).max(ABSENT),
            None => ABSENT,
        }
    }

    /// String capability by two-letter TERMCAP code (`cl`, `ce`, `up`, …).
    ///
    /// `rposition`, not `position`: `ML` is carried by two slots (`smgl` at
    /// 271 and `smglr` at 368) and ncurses' `_nc_find_type_entry` resolves it
    /// to the later one, so a `position` lookup answered with `smgl` — absent
    /// on xterm — and dropped `ML` from `$termcap` entirely.
    pub fn tc_string(&self, code: &str) -> Option<&[u8]> {
        let i = STR_CODES.iter().rposition(|&c| c == code)?;
        // lib_termcap.c `tgetstr`: "if (result == exit_attribute_mode &&
        // FIX_SGR0 != 0) result = FIX_SGR0" — the termcap view of `sgr0` is
        // the trimmed one.
        if STR_NAMES.get(i) == Some(&"sgr0") {
            if let Some(fix) = self.fix_sgr0.as_deref() {
                return Some(fix);
            }
        }
        self.strs.get(i).and_then(|o| o.as_deref())
    }
}

/// The entry `setupterm` / `tgetent` last loaded — ncurses' `cur_term`.
fn cur_term() -> &'static Mutex<Option<TermEntry>> {
    static CUR: OnceLock<Mutex<Option<TermEntry>>> = OnceLock::new();
    CUR.get_or_init(|| Mutex::new(None))
}

/// Run `f` against the loaded entry, or return `default` when none is loaded.
pub fn with_cur_term<T>(default: T, f: impl FnOnce(&TermEntry) -> T) -> T {
    match cur_term().lock() {
        Ok(g) => match g.as_ref() {
            Some(e) => f(e),
            None => default,
        },
        Err(_) => default,
    }
}

/// True once an entry has been loaded — ncurses' `cur_term != 0` test.
pub fn have_cur_term() -> bool {
    cur_term().lock().map(|g| g.is_some()).unwrap_or(false)
}

/// The directories to search, in ncurses' documented order: `$TERMINFO`,
/// `~/.terminfo`, then each entry of `$TERMINFO_DIRS` (an empty entry meaning
/// the compiled-in default), then the built-in list.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut push = |p: PathBuf| {
        if !p.as_os_str().is_empty() && !dirs.contains(&p) {
            dirs.push(p);
        }
    };
    // Built-in defaults, shared by the `$TERMINFO_DIRS` empty-entry rule.
    // These stand in for the path ncurses was COMPILED with, so the order
    // has to name whichever database the rest of the system reads.
    //
    // On Linux that is `/usr/share/terminfo` (ncurses-base), first.
    //
    // On macOS the system copy under `/usr/share/terminfo` is many years
    // old — its `xterm-256color` has no `rep`, `nel`, `oc`, `mgc`,
    // `smglr`, `ka1`… and reports `pairs` as 32767 rather than 65536 —
    // while anything linked against Homebrew's ncurses reads
    // `<prefix>/opt/ncurses/share/terminfo`. Reference zsh on such a host
    // is one of those things, so the Homebrew trees are offered FIRST
    // there: with the system copy first, `${(k)terminfo}` came back 12
    // capabilities short of the zsh running beside it.
    let defaults = || {
        let brew = [
            "/opt/homebrew/opt/ncurses/share/terminfo",
            "/usr/local/opt/ncurses/share/terminfo",
        ];
        let system = [
            "/usr/share/terminfo",
            "/lib/terminfo",
            "/etc/terminfo",
            "/usr/lib/terminfo",
            "/usr/share/lib/terminfo",
            "/usr/local/share/terminfo",
        ];
        let ordered: Vec<&str> = if cfg!(target_os = "macos") {
            brew.into_iter().chain(system).collect()
        } else {
            system.into_iter().chain(brew).collect()
        };
        ordered.into_iter().map(PathBuf::from)
    };
    if let Ok(v) = std::env::var("TERMINFO") {
        push(PathBuf::from(v));
    }
    if let Ok(h) = std::env::var("HOME") {
        push(PathBuf::from(h).join(".terminfo"));
    }
    if let Ok(v) = std::env::var("TERMINFO_DIRS") {
        for part in v.split(':') {
            if part.is_empty() {
                for d in defaults() {
                    push(d);
                }
            } else {
                push(PathBuf::from(part));
            }
        }
    }
    for d in defaults() {
        push(d);
    }
    dirs
}

/// Candidate files for `name` under `dir`. Two layouts are in use: the
/// letter-directory form (`<dir>/x/xterm`, Linux and the BSDs) and the
/// hex-directory form (`<dir>/78/xterm`, macOS). Both are tried.
fn candidates(dir: &Path, name: &str) -> Vec<PathBuf> {
    let Some(first) = name.chars().next() else {
        return Vec::new();
    };
    let mut v = vec![dir.join(first.to_string()).join(name)];
    let mut buf = [0u8; 4];
    let hex = format!("{:02x}", first.encode_utf8(&mut buf).as_bytes()[0]);
    v.push(dir.join(hex).join(name));
    v
}

/// Locate and parse the entry for `name`, without touching `cur_term`.
pub fn load_entry(name: &str) -> Result<TermEntry, SetupError> {
    if name.is_empty() || name.contains('/') {
        return Err(SetupError::Unset);
    }
    let dirs = search_dirs();
    let mut saw_dir = false;
    for dir in &dirs {
        if !dir.is_dir() {
            continue;
        }
        saw_dir = true;
        for cand in candidates(dir, name) {
            let Ok(bytes) = std::fs::read(&cand) else {
                continue;
            };
            if let Some(e) = TermEntry::parse(&bytes) {
                return Ok(e);
            }
        }
    }
    Err(if saw_dir {
        SetupError::NotFound
    } else {
        SetupError::NoDatabase
    })
}

/// ncurses `setupterm(term, fd, errret)` — load `term` (or `$TERM` when
/// `None`) and install it as the current entry.
pub fn setupterm(term: Option<&str>) -> Result<(), SetupError> {
    let name = match term {
        Some(t) => t.to_string(),
        None => std::env::var("TERM").unwrap_or_default(),
    };
    if name.is_empty() {
        return Err(SetupError::Unset);
    }
    // ncurses keeps ONE `cur_term`; calling `setupterm` again for the terminal
    // that is already loaded does not discard it. That matters because
    // `tgetent` mutates the loaded entry (see `apply_termcap_fixups`) and
    // `init_term` runs it long before `zmodload zsh/terminfo` calls
    // `setupterm`: reloading from disk here threw the fixups away, so
    // `$terminfo[OTbs]` reported the entry's stored value where zsh reports
    // the `cub1 == "\b"` answer ncurses computed.
    if let Ok(g) = cur_term().lock() {
        if let Some(cur) = g.as_ref() {
            if cur.names.iter().any(|n| *n == name) {
                return Ok(());
            }
        }
    }
    let mut e = load_entry(&name)?;
    apply_screensize(&mut e);
    // The termcap fixups belong to `tgetent` in ncurses, but every zsh
    // process reaches them: `init_term` (Src/init.c) calls `tgetent` during
    // startup, so by the time anything reads `$terminfo` the entry has been
    // through them. zshrs's `init_term` does not run for a non-interactive
    // `-c` script, which left `$terminfo[OTbs]` reporting the entry's stored
    // value there while zsh reported ncurses' computed one. Applying them on
    // load keeps the two agreeing on every path; they are idempotent and
    // derived only from the entry itself.
    apply_termcap_fixups(&mut e);
    if let Ok(mut g) = cur_term().lock() {
        *g = Some(e);
    }
    Ok(())
}

/// ncurses `_nc_get_screensize`, which `setupterm` runs before publishing
/// `cur_term`: the `lines` / `cols` capabilities are overridden by the real
/// terminal size, then by `$LINES` / `$COLUMNS`, and finally defaulted to
/// 24x80. Without it `$terminfo[lines]` reports whatever the entry happens
/// to carry — and 72 of the reference database's 394 entries carry nothing
/// at all, where ncurses answers 24 and 80.
fn apply_screensize(e: &mut TermEntry) {
    let idx = |n: &str| NUM_NAMES.iter().position(|&x| x == n);
    let (Some(li), Some(co)) = (idx("lines"), idx("cols")) else {
        return;
    };
    if e.nums.len() <= li.max(co) {
        e.nums.resize(li.max(co) + 1, ABSENT);
    }
    let mut lines = e.nums[li];
    let mut cols = e.nums[co];

    // TIOCGWINSZ on whichever of the three standard descriptors is a tty.
    // A zero field means "unknown" and must not clobber the capability.
    for fd in [libc::STDOUT_FILENO, libc::STDERR_FILENO, libc::STDIN_FILENO] {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } == 0 {
            if ws.ws_row > 0 {
                lines = i32::from(ws.ws_row);
            }
            if ws.ws_col > 0 {
                cols = i32::from(ws.ws_col);
            }
            break;
        }
    }
    // `use_env(TRUE)` is ncurses' default, so the environment wins last.
    if let Ok(v) = std::env::var("LINES") {
        if let Ok(n) = v.parse::<i32>() {
            if n > 0 {
                lines = n;
            }
        }
    }
    if let Ok(v) = std::env::var("COLUMNS") {
        if let Ok(n) = v.parse::<i32>() {
            if n > 0 {
                cols = n;
            }
        }
    }
    e.nums[li] = if lines > 0 { lines } else { 24 };
    e.nums[co] = if cols > 0 { cols } else { 80 };
}

/// ncurses `tigetflag` — `1` set, `0` absent, `-1` not a boolean capability.
pub fn tigetflag(name: &str) -> i32 {
    with_cur_term(0, |e| e.flag(name))
}

/// ncurses `tigetnum` — the value, `-1` absent, `-2` not a numeric capability.
pub fn tigetnum(name: &str) -> i32 {
    with_cur_term(ABSENT, |e| e.num(name))
}

/// ncurses `tigetstr` — `Ok(Some(v))` set, `Ok(None)` absent, `Err(())` for
/// the wrong-type case ncurses reports as `(char *)-1`.
pub fn tigetstr(name: &str) -> Result<Option<Vec<u8>>, ()> {
    let wrong = with_cur_term(false, |e| e.str_is_wrong_type(name));
    if wrong {
        return Err(());
    }
    Ok(with_cur_term(None, |e| e.string(name).map(|s| s.to_vec())))
}

/// ncurses `tgetent(NULL, name)` — `1` on success, `0` when the entry is not
/// in the database, `-1` when no database was found. Installs `cur_term` the
/// same way `setupterm` does, since the termcap layer reads the same entry.
///
/// `tgetent` is not merely `setupterm` with a different return convention:
/// lib_termcap.c MUTATES the loaded entry with two termcap-era fixups, and
/// both are observable through `$termcap` and `$terminfo` afterwards. zsh
/// runs `tgetent` from `init_term`, so every zsh process sees the mutated
/// entry — which is why `$terminfo[OTbs]` differs between a shell that has
/// called `tgetent` and one that has only called `setupterm`.
pub fn tgetent(name: &str) -> i32 {
    if let Err(e) = setupterm(Some(name)) {
        return e.errret();
    }
    1
}

/// The two entry mutations lib_termcap.c's `tgetent` performs.
fn apply_termcap_fixups(e: &mut TermEntry) {
    // `if (cursor_left) backspaces_with_bs = !strcmp(cursor_left, "\b")`.
    // `backspaces_with_bs` IS the `OTbs` boolean, so the entry's stored value
    // is replaced by whether `cub1` is a literal backspace. Measured: `ansi`
    // ships OTbs set but `cub1=\E[D`, so ncurses reports it unset; `Eterm`
    // ships it unset but `cub1=^H`, so ncurses reports it set.
    if let Some(cub1) = e.string("cub1").map(|s| s.to_vec()) {
        let is_bs = cub1 == b"\x08";
        if let Some(i) = BOOL_NAMES.iter().position(|&n| n == "OTbs") {
            if e.bools.len() <= i {
                e.bools.resize(i + 1, 0);
            }
            e.bools[i] = i8::from(is_bs);
        }
        // The other half of the same `if` — `backspace_if_not_bs = cursor_left`
        // — is NOT reproduced: it assigns ncurses' `BC` global, not the
        // entry, so `$terminfo[OTbc]` stays unset in the reference shell.
        let _ = cub1;
    }
    // `FIX_SGR0 = _nc_trim_sgr0(...)`, cleared again when it matches `sgr0`.
    let trimmed = trim_sgr0(e);
    if let Some(t) = trimmed {
        if e.string("sgr0") != Some(t.as_slice()) {
            e.fix_sgr0 = Some(t);
        }
    }
}

/// ncurses' `_nc_trim_sgr0`: the termcap-facing "turn every attribute off"
/// string. It starts from `sgr` evaluated with all-zero parameters when the
/// entry has one (else from `sgr0`) and removes the parts termcap's `me` must
/// not carry — the alternate-character-set selection (`\E(B`, `\E(0`, and the
/// SO/SI bytes `\016`/`\017`), the `10` "primary font" SGR parameter, and any
/// padding spec.
///
/// Measured against ncurses: `xterm` `\E(B\E[m` → `\E[0m`, `ansi`
/// `\E[0;10m` → `\E[0m`, `vt100` `\E[m\017$<2>` → `\E[0m`.
fn trim_sgr0(e: &TermEntry) -> Option<Vec<u8>> {
    let base = match e.string("sgr") {
        Some(sgr) => crate::tparm::tparm(sgr, &[0; 9]),
        None => e.string("sgr0")?.to_vec(),
    };
    let base = crate::tparm::tputs_strip_padding(&base);
    let mut out: Vec<u8> = Vec::with_capacity(base.len());
    let mut i = 0usize;
    while i < base.len() {
        // `\E(` + one designator selects a character set — never part of `me`.
        if base[i] == 0x1b && base.get(i + 1) == Some(&b'(') && i + 2 < base.len() {
            i += 3;
            continue;
        }
        // Shift-out / shift-in.
        if base[i] == 0x0e || base[i] == 0x0f {
            i += 1;
            continue;
        }
        out.push(base[i]);
        i += 1;
    }
    // Drop a `10` parameter from the SGR sequence (`\E[0;10m` → `\E[0m`).
    let cleaned = drop_sgr_font_param(&out);
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

/// Remove the `10` (primary font) parameter from every `CSI … m` sequence.
fn drop_sgr_font_param(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0usize;
    while i < s.len() {
        if s[i] == 0x1b && s.get(i + 1) == Some(&b'[') {
            // Find the terminating `m`; anything else is left alone.
            let mut j = i + 2;
            while j < s.len() && (s[j].is_ascii_digit() || s[j] == b';') {
                j += 1;
            }
            if s.get(j) == Some(&b'm') {
                let params: Vec<&[u8]> = s[i + 2..j].split(|&c| c == b';').collect();
                let kept: Vec<&[u8]> = params.into_iter().filter(|p| *p != b"10").collect();
                out.extend_from_slice(b"\x1b[");
                for (k, p) in kept.iter().enumerate() {
                    if k > 0 {
                        out.push(b';');
                    }
                    out.extend_from_slice(p);
                }
                out.push(b'm');
                i = j + 1;
                continue;
            }
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

/// ncurses `tgetflag(code)` for a two-letter termcap code.
pub fn tgetflag(code: &str) -> i32 {
    with_cur_term(0, |e| e.tc_flag(code))
}

/// ncurses `tgetnum(code)` for a two-letter termcap code.
pub fn tgetnum(code: &str) -> i32 {
    with_cur_term(ABSENT, |e| e.tc_num(code))
}

/// ncurses `tgetstr(code, area)` for a two-letter termcap code. The `area`
/// out-parameter of the C call is not reproduced: callers copy the returned
/// bytes, which is all the zsh ports ever did with it.
pub fn tgetstr(code: &str) -> Option<Vec<u8>> {
    with_cur_term(None, |e| e.tc_string(code).map(|s| s.to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A hand-built entry exercising both numeric widths and the absent /
    /// cancelled encodings, so the parser is checked without a database.
    fn synth(wide: bool) -> Vec<u8> {
        let names = b"zz-test|synthetic\0";
        let bools: [i8; 3] = [1, 0, 1];
        let nums: [i32; 2] = [80, -1];
        let table = b"\x1b[H\0\x1b[J\0";
        let offsets: [i16; 3] = [0, 4, -1];

        let mut v = Vec::new();
        let magic: u16 = if wide { MAGIC_32 } else { MAGIC_16 };
        for x in [
            magic as i16,
            names.len() as i16,
            bools.len() as i16,
            nums.len() as i16,
            offsets.len() as i16,
            table.len() as i16,
        ] {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v.extend_from_slice(names);
        v.extend(bools.iter().map(|&b| b as u8));
        if v.len() % 2 != 0 {
            v.push(0);
        }
        for n in nums {
            if wide {
                v.extend_from_slice(&n.to_le_bytes());
            } else {
                v.extend_from_slice(&(n as i16).to_le_bytes());
            }
        }
        for o in offsets {
            v.extend_from_slice(&o.to_le_bytes());
        }
        v.extend_from_slice(table);
        v
    }

    #[test]
    fn parses_both_numeric_widths_identically() {
        for wide in [false, true] {
            let e = TermEntry::parse(&synth(wide)).expect("parses");
            assert_eq!(e.names[0], "zz-test");
            assert_eq!(e.names[1], "synthetic");
            assert_eq!(e.bools, vec![1, 0, 1]);
            assert_eq!(e.nums, vec![80, ABSENT]);
            assert_eq!(e.strs[0].as_deref(), Some(&b"\x1b[H"[..]));
            assert_eq!(e.strs[1].as_deref(), Some(&b"\x1b[J"[..]));
            assert_eq!(e.strs[2], None, "offset -1 is absent, not empty");
        }
    }

    #[test]
    fn rejects_non_terminfo_bytes() {
        assert!(TermEntry::parse(b"not a terminfo file at all").is_none());
        assert!(TermEntry::parse(&[]).is_none());
        // Right magic, truncated body — must not panic or half-parse.
        let mut t = synth(false);
        t.truncate(20);
        assert!(TermEntry::parse(&t).is_none());
    }

    /// The three cap tables are positional indexes into the file format, so a
    /// length change silently misreads every entry past the changed slot.
    #[test]
    fn cap_tables_have_the_frozen_ncurses6_lengths() {
        assert_eq!(BOOL_NAMES.len(), 44);
        assert_eq!(BOOL_CODES.len(), 44);
        assert_eq!(NUM_NAMES.len(), 39);
        assert_eq!(NUM_CODES.len(), 39);
        assert_eq!(STR_NAMES.len(), 414);
        assert_eq!(STR_CODES.len(), 414);
    }

    /// Spot-check the anchors the zsh ports look up by name and by code, so a
    /// regenerated table that shifted a row is caught immediately.
    #[test]
    fn well_known_capabilities_sit_at_their_documented_indexes() {
        assert_eq!(BOOL_NAMES[0], "bw");
        assert_eq!(BOOL_NAMES.iter().position(|&n| n == "am"), Some(1));
        assert_eq!(BOOL_CODES[1], "am");
        assert_eq!(NUM_NAMES.iter().position(|&n| n == "cols"), Some(0));
        assert_eq!(NUM_CODES[0], "co");
        assert_eq!(NUM_NAMES.iter().position(|&n| n == "lines"), Some(2));
        assert_eq!(NUM_CODES[2], "li");
        assert_eq!(STR_NAMES.iter().position(|&n| n == "cup"), Some(10));
        assert_eq!(STR_CODES[10], "cm");
    }
}
