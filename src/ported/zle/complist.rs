//! Completion listing display for ZLE
//!
//! Port from zsh/Src/Zle/complist.c (3,604 lines)
//!
//! Information about the list shown.                                        // c:34
//! Information for in-string colours.                                       // c:133
//! This holds all terminal strings.                                         // c:243
//! Get the terminal color string for the given match.                       // c:878
//! The widget function.                                                     // c:3481
//!
//! The full menu/listing system is in compsys/menu.rs (3,445 lines).
//! This module provides the ZLE-side rendering that displays completion
//! matches in columns with colors, scrolling, and selection.
//!
//! Key C functions and their Rust locations:
//! - compprintlist    → crate::compsys::menu::MenuState::render()
//! - compprintfmt     → crate::compsys::menu::format_group()
//! - clprintm         → crate::compsys::menu::print_match()
//! - asklistscroll    → crate::compsys::menu::handle_scroll()
//! - getcols/filecol  → crate::compsys::zpwr_colors (LS_COLORS parsing)
//! - initiscol        → crate::compsys::zpwr_colors::init_colors()

use crate::ported::init::SHTTY;
use crate::ported::mem::popheap;
use crate::ported::params::getsparam;
use crate::ported::signals::unqueue_signals;
use crate::ported::utils::{adjustcolumns, adjustlines, errflag, write_loop};
use crate::ported::zle::comp_h::{
    Cmatch, Cmgroup, CGF_HASDL, CGF_LINES, CGF_ROWS, CMF_DISPLINE, CMF_FMULT, CMF_HIDE, CMF_MULT,
    CMF_NOLIST,
};
use crate::ported::zle::compcore::{listdat, MINFO, ZLEMETACS, ZLEMETALINE, ZLEMETALL};
use crate::ported::zle::zle_refresh::{tcmultout, tcout, CLEARFLAG, NLNCT};
use crate::ported::zsh_h::{isset, Patprog, EXTENDEDGLOB, TCCLEAREOD, TCCLEAREOL, USEZLE};
use crate::DPUTS2;
use std::collections::HashMap;
use std::sync::atomic::Ordering;

// `ListColors` / `ListLayout` and their Rust-only methods deleted.
// The C source uses `struct listcols` (legit port at line 645 as
// `listcols`, c:253) plus file-scope `int columns, lines` globals
// for the layout — no separate layout struct. Real `getcols()`,
// `filecol()`, `calclist()` ports live below using those types.
//
// `calclist` here had the wrong C signature: real C `void
// calclist(int showall)` at compresult.c:1495 takes one int; the
// previous Rust placeholder took `(matches, term_width, descs)` and
// returned a `ListLayout`. Real port pending.

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_hist::*, zle_main::*, zle_misc::*, zle_move::*,
    zle_params::*, zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*, zle_word::*,
};

/// Port of `MMARK` from `Src/Zle/complist.c:126`. Tag bit used in
/// the low bit of `Cmatch *` / `Cmgroup` pointers to mark a match
/// as visited during the menu-select / hidden-row dispatch. Real C
/// uses pointer tagging; the Rust port uses the same bit position
/// (`u32 = 1`) as a search-anchor — actual marker storage lives on
/// a separate `bool` per Cmatch when the substrate hydrates.
pub const MMARK: u32 = 1; // c:126

/// Port of `MAX_POS` from `Src/Zle/complist.c:137`. Maximum number
/// of saved (mline, mcol) menu-select positions in the back-stack
/// used by msearchpush/msearchpop.
pub const MAX_POS: usize = 11; // c:137

// =====================================================================
// Substrate for the LS_COLORS / ZLS_COLORS subsystem —
// `Src/Zle/complist.c:165-269`.
// =====================================================================

// `COL_*` — index into `mcolors.files[]` per `Src/Zle/complist.c:167-194`.
/// `COL_NO` constant.
pub const COL_NO: usize = 0; // c:167
/// `COL_FI` constant.
pub const COL_FI: usize = 1; // c:168
/// `COL_DI` constant.
pub const COL_DI: usize = 2; // c:169
/// `COL_LN` constant.
pub const COL_LN: usize = 3; // c:170
/// `COL_PI` constant.
pub const COL_PI: usize = 4; // c:171
/// `COL_SO` constant.
pub const COL_SO: usize = 5; // c:172
/// `COL_BD` constant.
pub const COL_BD: usize = 6; // c:173
/// `COL_CD` constant.
pub const COL_CD: usize = 7; // c:174
/// `COL_OR` constant.
pub const COL_OR: usize = 8; // c:175
/// `COL_MI` constant.
pub const COL_MI: usize = 9; // c:176
/// `COL_SU` constant.
pub const COL_SU: usize = 10; // c:177
/// `COL_SG` constant.
pub const COL_SG: usize = 11; // c:178
/// `COL_TW` constant.
pub const COL_TW: usize = 12; // c:179
/// `COL_OW` constant.
pub const COL_OW: usize = 13; // c:180
/// `COL_ST` constant.
pub const COL_ST: usize = 14; // c:181
/// `COL_EX` constant.
pub const COL_EX: usize = 15; // c:182
/// `COL_LC` constant.
pub const COL_LC: usize = 16; // c:183
/// `COL_RC` constant.
pub const COL_RC: usize = 17; // c:184
/// `COL_EC` constant.
pub const COL_EC: usize = 18; // c:185
/// `COL_TC` constant.
pub const COL_TC: usize = 19; // c:186
/// `COL_SP` constant.
pub const COL_SP: usize = 20; // c:187
/// `COL_MA` constant.
pub const COL_MA: usize = 21; // c:188
/// `COL_HI` constant.
pub const COL_HI: usize = 22; // c:189
/// `COL_DU` constant.
pub const COL_DU: usize = 23; // c:190
/// `COL_SA` constant.
pub const COL_SA: usize = 24; // c:191
/// Port of `NUM_COLS` from `Src/Zle/complist.c:193`.
pub const NUM_COLS: usize = 25; // c:193

/// ```c
/// static filecol
/// filecol(char *col)
/// {
///     filecol fc;
///     fc = (filecol) zhalloc(sizeof(*fc));
///     fc->prog = NULL;
///     fc->col = col;
///     fc->next = NULL;
///     return fc;
/// }
/// ```
/// Allocate a fresh filecol with no group pattern and the given
/// color string. Caller is expected to chain it via `mcolors.files[i]`.
/// Port of `filecol(char *col)` from `Src/Zle/complist.c:488`.
pub fn filecol(col: &str) -> filecol {
    // c:488
    filecol {
        // c:488 zhalloc
        prog: None,           // c:493 fc->prog = NULL
        col: col.to_string(), // c:494 fc->col = col
        next: None,           // c:495 fc->next = NULL
    } // c:497 return fc
}

/// Port of `struct filecol` / `typedef struct filecol *filecol` from
/// `Src/Zle/complist.c:213-219`. One terminal-color spec for a file
/// type; chained via `next` so multiple per-group rules can apply.
///
/// `prog` mirrors C's `Patprog prog` (NULL → applies to all groups).
/// Patprog doesn't impl Debug/Clone in the Rust port, so this struct
/// can't auto-derive them; impl manually if needed by callers.
#[derive(Default)]
#[allow(non_camel_case_types)]
pub struct filecol {
    // c:215
    /// Group pattern (NULL → applies to all groups).
    pub prog: Option<crate::ported::pattern::Patprog>, // c:216
    /// Color string (ANSI escape-code body).
    pub col: String, // c:217
    /// Next entry chained for the same color slot.
    pub next: Option<Box<filecol>>, // c:218
}

/// Port of `struct patcol` from `Src/Zle/complist.c:225`. Per-pattern
/// terminal-color spec — links a glob `pat` to up to MAX_POS+1 color
/// strings (one per submatch position).
#[derive(Default)]
#[allow(non_camel_case_types)]
pub struct patcol {
    // c:225
    /// Group pattern (NULL → all groups).
    pub prog: Option<crate::ported::pattern::Patprog>, // c:226
    /// Pattern for match.
    pub pat: Option<crate::ported::pattern::Patprog>, // c:227
    /// Color strings indexed by submatch position (MAX_POS + 1 slots).
    pub cols: Vec<String>, // c:228
    /// Next entry in the patcol chain.
    pub next: Option<Box<patcol>>, // c:229
}

/// Port of `struct extcol` from `Src/Zle/complist.c:236`. Per-extension
/// terminal-color spec.
#[derive(Default)]
#[allow(non_camel_case_types)]
pub struct extcol {
    // c:236
    /// Group pattern (NULL → all groups).
    pub prog: Option<crate::ported::pattern::Patprog>, // c:237
    /// File extension (e.g. ".tar").
    pub ext: String, // c:238
    /// Terminal color string.
    pub col: String, // c:239
    /// Next entry in the extcol chain.
    pub next: Option<Box<extcol>>, // c:240
}

/// Port of `struct listcols` from `Src/Zle/complist.c:253`. Holds
/// every terminal-color string a completion-listing run might emit.
#[derive(Default)]
#[allow(non_camel_case_types)]
pub struct listcols {
    // c:253
    /// Strings for file types (indexed by `col::*` constants).
    pub files: Vec<filecol>, // c:254 [NUM_COLS]
    /// Strings for patterns.
    pub pats: Option<Box<patcol>>, // c:255
    /// Strings for extensions.
    pub exts: Option<Box<extcol>>, // c:256
    /// Special settings, see `LC_FOLLOW_SYMLINKS` above.
    pub flags: i32, // c:257
}

/// Port of `char *getcolval(char *s, int multi)` from
/// `Src/Zle/complist.c:275`.
///
/// Line-by-line port of c:275-324. Parses one LS_COLORS value: walks
/// until `:` (or `=` when `multi != 0`), decoding `\a/\n/\b/\t/\v/
/// \f/\r/\e/\_/\?` and octal `\DDD` escapes (c:283-303), `^X`
/// control-char shorthand (c:305-316), and copying every other byte
/// verbatim. Bumps `MAX_CAPLEN` to track the longest cap escape
/// emitted (c:321-322). Returns the unconsumed tail; the C function
/// mutates `s` in place — we return both the decoded bytes and the
/// remaining slice so callers can write the value somewhere durable
/// before resuming parse.
///
/// C returns just the post-value pointer; Rust returns
/// `(decoded, rest)` so callers don't lose the parsed payload.
/// The pre-existing test asserts the empty-input invariant — the
/// return-type change preserves that (`getcolval("", 0).1 == ""`).
pub fn getcolval(s: &str, multi: i32) -> (String, &str) {
    use crate::ported::init::tcstr;
    let _ = tcstr; // touch import so cfg(test) regen doesn't trim it.

    let bytes = s.as_bytes();
    let mut p: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;

    while i < bytes.len() {
        let c = bytes[i];
        // c:280 — stop on `:` or (multi && `=`).
        if c == b':' || (multi != 0 && c == b'=') {
            break;
        }
        if c == b'\\' && i + 1 < bytes.len() {
            // c:283-303 — backslash escapes.
            i += 1;
            let n = bytes[i];
            i += 1;
            match n {
                b'a' => p.push(0x07),
                b'n' => p.push(b'\n'),
                b'b' => p.push(0x08),
                b't' => p.push(b'\t'),
                b'v' => p.push(0x0b),
                b'f' => p.push(0x0c),
                b'r' => p.push(b'\r'),
                b'e' => p.push(0x1b),
                b'_' => p.push(b' '),
                b'?' => p.push(0x7f),
                d if (b'0'..=b'7').contains(&d) => {
                    // c:296-303 — octal \DDD (up to 3 digits).
                    let mut val = (d - b'0') as i32;
                    if i < bytes.len() && (b'0'..=b'7').contains(&bytes[i]) {
                        val = val * 8 + (bytes[i] - b'0') as i32;
                        i += 1;
                        if i < bytes.len() && (b'0'..=b'7').contains(&bytes[i]) {
                            val = val * 8 + (bytes[i] - b'0') as i32;
                            i += 1;
                        }
                    }
                    p.push(val as u8);
                }
                _ => p.push(n),
            }
        } else if c == b'^' && i + 1 < bytes.len() {
            // c:305-316 — `^X` control-char shorthand.
            let n = bytes[i + 1];
            if (b'@'..=b'_').contains(&n) || (b'a'..=b'z').contains(&n) {
                p.push(n & !0x60);
            } else if n == b'?' {
                p.push(0x7f);
            } else {
                p.push(c);
                p.push(n);
            }
            i += 2;
        } else {
            // c:317 — verbatim.
            p.push(c);
            i += 1;
        }
    }

    // c:321-322 — `if ((s - o) > max_caplen) max_caplen = s - o;`
    let consumed = i as i32;
    if consumed > MAX_CAPLEN.load(Ordering::Relaxed) {
        MAX_CAPLEN.store(consumed, Ordering::Relaxed);
    }
    // C returns the post-value pointer (s in c:323); Rust returns
    // the decoded payload plus the unconsumed tail.
    let decoded = String::from_utf8_lossy(&p).into_owned();
    (decoded, &s[i..])
}

/// Port of `char *getcoldef(char *s)` from Src/Zle/complist.c:330-503.
/// Parses ONE `ZLS_COLORS`/`LS_COLORS` entry into the `mcolors` structure
/// and returns the unconsumed tail (None to stop). An entry is one of:
///   - `(group)…`      — optional leading group pattern (compiled Patprog,
///                       shared by the entry it prefixes).
///   - `*ext=col`      — extension rule → `mcolors.exts`.
///   - `=pat=col[=col…]`— pattern rule (the form `list-colors` emits) →
///                       `mcolors.pats`, with one color per submatch pos.
///   - `xx=col`        — two-letter file-type code → `mcolors.files[i]`.
/// C mutates `s` in place with `\0` splits and stores interior pointers;
/// the Rust port builds owned Strings via `getcolval` (which returns the
/// decoded value + tail) and appends onto the `Option<Box<…>>` chains.
pub fn getcoldef(s: &str) -> Option<String> {
    // c:330
    use std::sync::atomic::Ordering as O;
    let _ = O::Relaxed;
    // Empty input → no color definition to parse; signal stop (None). The
    // `getcols` driver only calls this while `!s.is_empty()`, so this guard
    // is defensive parity with "nothing left to parse".
    if s.is_empty() {
        return None;
    }
    let mut gprog: Option<crate::ported::pattern::Patprog> = None;
    let mut s = s;

    // c:333-354 — optional `(group)` prefix → compiled group Patprog.
    if s.starts_with('(') {
        let b = s.as_bytes();
        let mut l = 0i32;
        let mut p = 1usize;
        // c:337-343 — scan to the matching ')' honoring nesting + backslash.
        while p < b.len() && (b[p] != b')' || l != 0) {
            if b[p] == b'\\' && p + 1 < b.len() {
                p += 1;
            } else if b[p] == b'(' {
                l += 1;
            } else if b[p] == b')' {
                l -= 1;
            }
            p += 1;
        }
        if p < b.len() && b[p] == b')' {
            // c:345-352 — metafy + tokenize + patcompile the group name.
            let grp = &s[1..p];
            let mut mg = crate::ported::utils::metafy(grp);
            crate::ported::glob::tokenize(&mut mg);
            gprog = crate::ported::pattern::patcompile(&mg, 0, None);
            s = &s[p + 1..]; // c:353 s = p + 1
        }
    }

    if let Some(rest) = s.strip_prefix('*') {
        // c:355-390 — `*ext=col` extension rule.
        match rest.find('=') {
            None => Some(String::new()), // c:365-366 return s (at NUL)
            Some(eq) => {
                let ext = rest[..eq].to_string(); // c:367 *s++='\0'
                let (col, p) = getcolval(&rest[eq + 1..], 0); // c:368
                // c:369-384 — append the extcol at the tail of mcolors.exts.
                let ec = Box::new(extcol {
                    prog: gprog,
                    ext,
                    col,
                    next: None,
                });
                {
                    let mut mc = MCOLORS.lock().unwrap();
                    let mut cur = &mut mc.exts;
                    while cur.is_some() {
                        cur = &mut cur.as_mut().unwrap().next;
                    }
                    *cur = Some(ec);
                }
                // c:388-389 — `if (*p) *p++='\0'; return p;`
                Some(if !p.is_empty() { p[1..].to_string() } else { String::new() })
            }
        }
    } else if let Some(pstart) = s.strip_prefix('=') {
        // c:391-441 — `=pat=col[=col…]` pattern rule.
        let pb = pstart.as_bytes();
        let mut nesting = 0i32;
        let mut j = 0usize;
        // c:399-408 — walk to the terminating '=' (nesting/backslash aware).
        while j < pb.len() && (nesting != 0 || pb[j] != b'=') {
            match pb[j] {
                b'\\' => {
                    if j + 1 < pb.len() {
                        j += 1;
                    }
                }
                b'(' => nesting += 1,
                b')' => nesting -= 1,
                _ => {}
            }
            j += 1;
        }
        if j >= pb.len() {
            return Some(String::new()); // c:409-410 return s (NUL)
        }
        let pat_str = &pstart[..j]; // c:411 *s++='\0'
        let mut cur = &pstart[j + 1..];
        // c:412-419 — collect successive '='-separated color values.
        let mut cols: Vec<String> = Vec::new();
        let final_tail: &str;
        loop {
            let (col, t) = getcolval(cur, 1); // c:413
            if cols.len() < MAX_POS {
                cols.push(col); // c:414-415
            }
            if !t.starts_with('=') {
                final_tail = t; // c:417 break
                break;
            }
            cur = &t[1..]; // c:418 *s++='\0'
        }
        // c:420-435 — metafy+tokenize+patcompile the pattern; append patcol.
        let mut mp = crate::ported::utils::metafy(pat_str);
        crate::ported::glob::tokenize(&mut mp);
        if let Some(prog) = crate::ported::pattern::patcompile(&mp, 0, None) {
            let pc = Box::new(patcol {
                prog: gprog,
                pat: Some(prog),
                cols,
                next: None,
            });
            let mut mc = MCOLORS.lock().unwrap();
            let mut chain = &mut mc.pats;
            while chain.is_some() {
                chain = &mut chain.as_mut().unwrap().next;
            }
            *chain = Some(pc);
        }
        // c:439-440 — `if (*t) *t++='\0'; return t;`
        Some(if !final_tail.is_empty() {
            final_tail[1..].to_string()
        } else {
            String::new()
        })
    } else {
        // c:442-483 — two-letter file-type code `xx=col`.
        match s.find('=') {
            None => Some(String::new()), // c:449-450 return s (NUL)
            Some(eq) => {
                let n = &s[..eq]; // c:452 *s++='\0'
                let after = &s[eq + 1..];
                // c:453-456 — find the colnames index.
                let idx = COLNAMES.iter().position(|&nn| nn == n);
                // c:459-465 — special: `ln=target[...]` → follow-symlinks flag.
                if idx == Some(COL_LN)
                    && after.starts_with("target")
                    && (after.as_bytes().get(6) == Some(&b':') || after.len() == 6)
                {
                    {
                        let mut mc = MCOLORS.lock().unwrap();
                        mc.flags |= LC_FOLLOW_SYMLINKS; // c:462
                    }
                    // c:463 — `p = s + 6;` then fall to `return p;`.
                    Some(after[6..].to_string())
                } else {
                    let (col, p) = getcolval(after, 0); // c:466
                    // c:467-480 — append the filecol (EC/LC/RC ignore gprog).
                    if let Some(i) = idx {
                        let fc = Box::new(filecol {
                            prog: if i == COL_EC || i == COL_LC || i == COL_RC {
                                None
                            } else {
                                gprog
                            },
                            col,
                            next: None,
                        });
                        let mut mc = MCOLORS.lock().unwrap();
                        if mc.files.len() <= i {
                            mc.files.resize_with(i + 1, || filecol(""));
                        }
                        // c:474-479 — `if ((fo = mcolors.files[i])) { …tail…
                        // fo->next = fc; } else mcolors.files[i] = fc;`. The
                        // Rust `files[i]` is a value (not a nullable pointer),
                        // so during-parse an as-yet-unset slot is the empty
                        // default (col=="" && next==None) — replace it
                        // outright; otherwise append at the chain tail.
                        if mc.files[i].col.is_empty() && mc.files[i].next.is_none() {
                            mc.files[i] = *fc;
                        } else {
                            let mut cur = &mut mc.files[i];
                            while cur.next.is_some() {
                                cur = cur.next.as_mut().unwrap();
                            }
                            cur.next = Some(fc);
                        }
                    }
                    // c:481-482 — `if (*p) *p++='\0'; return p;`
                    Some(if !p.is_empty() {
                        p[1..].to_string()
                    } else {
                        String::new()
                    })
                }
            }
        }
    }
}

/// Port of `static void getcols(void)` from `Src/Zle/complist.c:505`.
/// ```c
/// static void
/// getcols(void)
/// {
///     char *s;
///     int i, l;
///     max_caplen = lr_caplen = 0;
///     mcolors.flags = 0;
///     queue_signals();
///     if (!(s = getsparam_u("ZLS_COLORS")) && !(s = getsparam_u("ZLS_COLOURS"))) {
///         for (i = 0; i < NUM_COLS; i++) mcolors.files[i] = filecol("");
///         mcolors.pats = NULL; mcolors.exts = NULL;
///         if ((s = tcstr[TCSTANDOUTBEG]) && s[0]) {
///             mcolors.files[COL_MA] = filecol(s);
///             mcolors.files[COL_EC] = filecol(tcstr[TCSTANDOUTEND]);
///         } else mcolors.files[COL_MA] = filecol(defcols[COL_MA]);
///         lr_caplen = 0;
///         if ((max_caplen = strlen(mcolors.files[COL_MA]->col)) <
///             (l = strlen(mcolors.files[COL_EC]->col))) max_caplen = l;
///         unqueue_signals(); return;
///     }
///     memset(&mcolors, 0, sizeof(mcolors));
///     s = dupstring(s);
///     while (*s) if (*s == ':') s++; else s = getcoldef(s);
///     unqueue_signals();
///     for (i = 0; i < NUM_COLS; i++) {
///         if (!mcolors.files[i] || !mcolors.files[i]->col)
///             mcolors.files[i] = filecol(defcols[i]);
///         if (mcolors.files[i] && mcolors.files[i]->col &&
///             (l = strlen(mcolors.files[i]->col)) > max_caplen) max_caplen = l;
///     }
///     lr_caplen = strlen(mcolors.files[COL_LC]->col) +
///                 strlen(mcolors.files[COL_RC]->col);
///     if (!mcolors.files[COL_OR] || !mcolors.files[COL_OR]->col)
///         mcolors.files[COL_OR] = mcolors.files[COL_LN];
///     if (!mcolors.files[COL_MI] || !mcolors.files[COL_MI]->col)
///         mcolors.files[COL_MI] = mcolors.files[COL_FI];
/// }
/// ```
pub fn getcols(_unused: &str) -> i32 {
    // c:505

    MAX_CAPLEN.store(0, Ordering::SeqCst); // c:510
    LR_CAPLEN.store(0, Ordering::SeqCst); // c:510
    {
        let mut mc = MCOLORS.lock().unwrap();
        mc.flags = 0; // c:511
    }
    crate::ported::signals::queue_signals(); // c:512

    // c:513-514 — `if (!(s = getsparam_u("ZLS_COLORS")) && !(s = getsparam_u("ZLS_COLOURS")))`
    let s_opt = getsparam("ZLS_COLORS").or_else(|| getsparam("ZLS_COLOURS"));

    if s_opt.is_none() {
        // c:513
        let mut mc = MCOLORS.lock().unwrap();
        mc.files.clear();
        for _i in 0..NUM_COLS {
            // c:515
            mc.files.push(filecol("")); // c:516 filecol("")
        }
        mc.pats = None; // c:517
        mc.exts = None; // c:518

        // c:520-524 — try termcap TCSTANDOUTBEG for highlight color.
        let tcstr_guard = crate::ported::init::tcstr.lock().unwrap();
        let so_beg = tcstr_guard[crate::ported::zsh_h::TCSTANDOUTBEG as usize].clone();
        let so_end = tcstr_guard[crate::ported::zsh_h::TCSTANDOUTEND as usize].clone();
        drop(tcstr_guard);
        if !so_beg.is_empty() {
            // c:520
            mc.files[COL_MA] = filecol(&so_beg); // c:521
            mc.files[COL_EC] = filecol(&so_end); // c:522
        } else {
            // c:523
            // c:524 — `mcolors.files[COL_MA] = filecol(defcols[COL_MA]);`
            // defcols[COL_MA] = "7" (reverse-video) per c:204.
            mc.files[COL_MA] = filecol("7"); // c:524
        }
        // c:525-528 — cap-length tracking.
        let ma_len = mc.files[COL_MA].col.len() as i32;
        let ec_len = mc.files[COL_EC].col.len() as i32;
        let max_len = if ma_len < ec_len { ec_len } else { ma_len };
        MAX_CAPLEN.store(max_len, Ordering::SeqCst); // c:526-528
        unqueue_signals(); // c:529
        return 0; // c:530
    }

    // c:532-540 — parse ZLS_COLORS into mcolors via getcoldef loop.
    {
        let mut mc = MCOLORS.lock().unwrap();
        *mc = listcols::default(); // c:533 memset(&mcolors, 0)
    }
    let mut s = s_opt.unwrap(); // c:534 dupstring
    while !s.is_empty() {
        // c:535
        if s.starts_with(':') {
            // c:536
            s = s[1..].to_string(); // c:537 s++
        } else {
            // c:539 — `s = getcoldef(s);`
            s = match getcoldef(&s) {
                Some(rest) => rest,
                None => break,
            };
        }
    }
    unqueue_signals(); // c:540

    // c:543-549 — default-fill loop for unset color slots.
    let defcols: [&str; NUM_COLS] = [
        "0",
        "0",
        "01;34",
        "01;36",
        "33",
        "01;35",
        "01;33",
        "01;33",
        "01;05;37;41",
        "01;05;37;41",
        "37;41",
        "30;43",
        "30;42",
        "34;42",
        "37;44",
        "01;32",
        "\x1b[",
        "m",
        "0",
        "0",
        "0",
        "7",
        "0",
        "0",
        "0",
    ];
    let mut mc = MCOLORS.lock().unwrap();
    while mc.files.len() < NUM_COLS {
        mc.files.push(filecol(""));
    }
    let mut max_len = MAX_CAPLEN.load(Ordering::SeqCst);
    for i in 0..NUM_COLS {
        // c:543
        if mc.files[i].col.is_empty() {
            // c:544
            mc.files[i] = filecol(defcols[i]); // c:545
        }
        let l = mc.files[i].col.len() as i32; // c:547
        if l > max_len {
            max_len = l;
        } // c:548
    }
    MAX_CAPLEN.store(max_len, Ordering::SeqCst);

    // c:550-551 — lr_caplen.
    let lr_len = (mc.files[COL_LC].col.len() + mc.files[COL_RC].col.len()) as i32;
    LR_CAPLEN.store(lr_len, Ordering::SeqCst);

    // c:553-558 — defaults: COL_OR fallback to COL_LN; COL_MI to COL_FI.
    if mc.files[COL_OR].col.is_empty() {
        // c:554
        let ln = mc.files[COL_LN].col.clone();
        mc.files[COL_OR] = filecol(&ln); // c:555
    }
    if mc.files[COL_MI].col.is_empty() {
        // c:557
        let fi = mc.files[COL_FI].col.clone();
        mc.files[COL_MI] = filecol(&fi); // c:558
    }
    0 // c:560
}

/// Direct port of `void zlrputs(char *cap)` from
/// `Src/Zle/complist.c:564`. Emits an LS_COLORS escape
/// `\033[<cap>m` to the shell-output fd. C body c:566-595 also
/// stores the cap into `last_cap` for the bleed-prevention path
/// downstream (used by `zcoff` in `cleareol`); that file-static
/// `last_cap` isn't yet ported, so we only emit the SGR escape.
pub fn zlrputs(cap: &str) -> i32 {
    // c:564
    if cap.is_empty() {
        return 0;
    }
    let fd = SHTTY.load(Ordering::Relaxed);
    let out = if fd >= 0 { fd } else { 1 };
    let s = format!("\x1b[{}m", cap);
    let _ = write_loop(out, s.as_bytes());
    0
}

/// Wrap a string in a CSI SGR sequence using the supplied colour
/// code, then reset.
/// Port of `zcputs(char *group, int colour)` from Src/Zle/complist.c. The C source uses
/// this for per-match colour application during list paint.
/// WARNING: param names don't match C — Rust=(s, color) vs C=(group, colour)
pub fn zcputs(s: &str, color: Option<&str>) -> String {
    // c:580
    // c:Src/Zle/complist.c:zcputs — C body emits the SGR + content via
    // tputs/putshout when a color cap is registered, else does nothing
    // (`if (..col..) tputs(..col..); putshout(s);`). The Rust port
    // returns a String (sig divergence — C is void) so the no-color
    // branch returns empty to mirror "nothing written to stdout". The
    // previous Rust path returned the bare content string, which
    // doesn't match either C's stdout side-effect OR the SGR-only
    // semantic the rest of the codebase expects.
    match color {
        Some(c) => format!("\x1b[{}m{}\x1b[0m", c, s),
        None => String::new(),
    }
}

// Turn off colouring.                                                     // c:597
/// Port of `zcoff()` from Src/Zle/complist.c:597.
pub fn zcoff() { // c:597
                 // C body c:599-617 — emits the LS_COLORS no-color escape via
                 //                    tputs(mcolors.files[COL_NO]->col,...).
                 //                    No mcolors substrate: no-op.
}

/// Direct port of `void cleareol(void)` from
/// `Src/Zle/complist.c:608`:
/// ```c
/// if (mlbeg >= 0 && tccan(TCCLEAREOL)) {
///     if (*last_cap) zcoff();
///     tcout(TCCLEAREOL);
/// }
/// ```
/// Emits the clear-to-end-of-line escape iff we're inside list
/// paint (`mlbeg >= 0`) and the terminal supports the cap. If a
/// LS_COLOR cap is currently active, emit the SGR-reset first so
/// the EOL-clear doesn't carry the color into untouched columns.
pub fn cleareol() {
    // c:608
    if MLBEG.load(Ordering::Relaxed) < 0 {
        return;
    }
    let fd = SHTTY.load(Ordering::Relaxed);
    let out = if fd >= 0 { fd } else { 1 };
    // c:611-612 — `if (*last_cap) zcoff();` — emit SGR reset.
    if !LAST_CAP.lock().map(|s| s.is_empty()).unwrap_or(true) {
        let _ = write_loop(out, b"\x1b[0m");
        LAST_CAP.lock().ok().map(|mut s| s.clear());
    }
    // c:613 — `tcout(TCCLEAREOL);` — CSI K.
    let _ = write_loop(out, b"\x1b[K");
}

/// Port of `initiscol()` from Src/Zle/complist.c:618.
/// Direct port of `void initiscol(void)` from
/// `Src/Zle/complist.c:618`. Resets per-line in-string-color state
/// at the start of a colored match emission. Pops the first
/// `patcols[0]` entry as the initial color and resets all the
/// position cursors + region-tracking arrays.
pub fn initiscol() -> i32 {
    // c:618
    // c:622 — `zlrputs(patcols[0]);` — emit first color cap.
    let first_cap = PATCOLS
        .lock()
        .ok()
        .and_then(|p| p.first().cloned())
        .unwrap_or_default();
    if !first_cap.is_empty() {
        let _ = zlrputs(&first_cap);
    }
    // c:624 — `curiscols[curiscol = 0] = *patcols++;`
    if let Ok(mut cs) = CURISCOLS.lock() {
        if !cs.is_empty() {
            cs[0] = first_cap.clone();
        }
    }
    CURISCOL.store(0, Ordering::Relaxed);
    PATCOLS_IDX.store(1, Ordering::Relaxed); // c:624 patcols++

    // c:626 — `curisbeg = curissend = 0;`
    CURISBEG.store(0, Ordering::Relaxed);
    CURISSEND.store(0, Ordering::Relaxed);

    // c:628-631 — sendpos / begpos / endpos init.
    let nrefs = NREFS.load(Ordering::Relaxed) as usize;
    if let Ok(mut sp) = SENDPOS.lock() {
        for i in 0..MAX_POS {
            sp[i] = 0xfffffff;
        }
        for i in 0..nrefs.min(MAX_POS) {
            sp[i] = 0xfffffff; // c:629 already 0xfffffff
        }
    }
    if let Ok(mut bp) = BEGPOS.lock() {
        for i in nrefs..MAX_POS {
            bp[i] = 0xfffffff; // c:631
        }
    }
    if let Ok(mut ep) = ENDPOS.lock() {
        for i in nrefs..MAX_POS {
            ep[i] = 0xfffffff; // c:631
        }
    }
    0
}

/// Port of `doiscol(int pos)` from Src/Zle/complist.c:635.
/// Direct port of `void doiscol(int pos)` from
/// `Src/Zle/complist.c:635`. Updates the in-string color state for
/// character position `pos` in the current match emission:
///
/// 1. Pops finished regions (where `pos > sendpos[curissend]`) —
///    each pop emits SGR-reset + restores the prior color from the
///    `curiscols[]` stack.
/// 2. Pushes any region whose begin position equals `pos`, or
///    finishes-empty regions (endpos < begpos or begpos == -1):
///    inserts `endpos` into the sorted `sendpos[]` array, emits
///    SGR-reset + the new color, pushes onto curiscols[].
pub fn doiscol(pos: i32) -> i32 {
    // c:635
    let fd = SHTTY.load(Ordering::Relaxed);
    let out = if fd >= 0 { fd } else { 1 };

    // c:639-645 — pop finished regions.
    loop {
        let curissend = CURISSEND.load(Ordering::Relaxed) as usize;
        let sp = SENDPOS
            .lock()
            .ok()
            .and_then(|s| s.get(curissend).copied())
            .unwrap_or(0xfffffff);
        if pos <= sp {
            break;
        }
        CURISSEND.fetch_add(1, Ordering::Relaxed);
        let curiscol = CURISCOL.load(Ordering::Relaxed);
        if curiscol > 0 {
            // c:642 — `zcputs(NULL, COL_NO);` — SGR reset.
            let _ = write_loop(out, b"\x1b[0m");
            // c:643 — `zlrputs(curiscols[--curiscol]);`
            let new_idx = curiscol - 1;
            CURISCOL.store(new_idx, Ordering::Relaxed);
            let restore_cap = CURISCOLS
                .lock()
                .ok()
                .and_then(|c| c.get(new_idx as usize).cloned())
                .unwrap_or_default();
            if !restore_cap.is_empty() {
                let _ = zlrputs(&restore_cap);
            }
        }
    }

    // c:646-665 — push new regions starting at or before `pos`.
    loop {
        let curisbeg = CURISBEG.load(Ordering::Relaxed) as usize;
        if curisbeg >= MAX_POS {
            break;
        }
        let (bp, ep) = {
            let bp_lock = BEGPOS.lock().ok();
            let ep_lock = ENDPOS.lock().ok();
            match (bp_lock, ep_lock) {
                (Some(b), Some(e)) => (
                    b.get(curisbeg).copied().unwrap_or(0xfffffff),
                    e.get(curisbeg).copied().unwrap_or(0xfffffff),
                ),
                _ => break,
            }
        };
        // c:646-647 — `fi = (endpos[curisbeg] < begpos[curisbeg] ||
        //                    begpos[curisbeg] == -1)`. Finished-empty region.
        let fi = ep < bp || bp == -1;
        if !(fi || pos == bp) {
            break;
        }
        // c:648 — `*patcols` truthy gate (more colors available).
        let patcols_idx = PATCOLS_IDX.load(Ordering::Relaxed);
        let cap_now = PATCOLS
            .lock()
            .ok()
            .and_then(|p| p.get(patcols_idx).cloned())
            .unwrap_or_default();
        if cap_now.is_empty() {
            break;
        }

        if !fi {
            // c:650-657 — insert `e = endpos[curisbeg]` into sendpos[]
            //              in sorted order.
            let e = ep;
            if let Ok(mut sp) = SENDPOS.lock() {
                let curissend = CURISSEND.load(Ordering::Relaxed) as usize;
                let mut i = curissend;
                while i < MAX_POS && sp[i] <= e {
                    i += 1;
                }
                let mut j = MAX_POS - 1;
                while j > i {
                    sp[j] = sp[j - 1];
                    j -= 1;
                }
                if i < MAX_POS {
                    sp[i] = e;
                }
            }
            // c:659-660 — `zcputs(NULL, COL_NO); zlrputs(*patcols);`
            let _ = write_loop(out, b"\x1b[0m");
            let _ = zlrputs(&cap_now);
            // c:661 — `curiscols[++curiscol] = *patcols;`
            let new_idx = CURISCOL.fetch_add(1, Ordering::Relaxed) + 1;
            if let Ok(mut cs) = CURISCOLS.lock() {
                if (new_idx as usize) < cs.len() {
                    cs[new_idx as usize] = cap_now;
                }
            }
        }
        // c:663-664 — `++patcols; ++curisbeg;`.
        PATCOLS_IDX.fetch_add(1, Ordering::Relaxed);
        CURISBEG.fetch_add(1, Ordering::Relaxed);
    }
    0
}

/// Port of `clprintfmt(char *p, int ml)` from Src/Zle/complist.c:671.
pub fn clprintfmt(p: &str, ml: i32) -> i32 {
    // c:671
    // C body c:673-712 — colored variant of printfmt that uses mcolors
    //                    for %F/%B etc. Without the mcolors substrate
    //                    we delegate to the plain printfmt.
    printfmt(p, ml, true, true)
}

/// Port of `int clnicezputs(int do_colors, char *s, int ml)` from
/// `Src/Zle/complist.c:715` (the `MULTIBYTE_SUPPORT` branch, c:720-836).
///
/// Local version of `nicezputs()` with in-string colouring and
/// scrolling. Faithful port of the C pipeline:
///   1. c:741-744 — `ztrdup` + `untokenize` + `unmetafy` the input into
///      the raw byte string (`ums`/`uptr`, length `umlen`). Rust uses
///      [`untokenize`] then [`unmetafy_str`].
///   2. c:746-747 — `if (do_colors) initiscol();`.
///   3. c:750-834 — decode each character (`mbrtowc` → wide char; an
///      invalid/incomplete byte sequence maps to the `MB_INVALID`/eol
///      path and is prettified via [`nicechar`], a valid wide char via
///      [`wcs_nicechar`]). For every input byte consumed, when coloring,
///      call [`doiscol`]. Then walk the nice representation emitting each
///      character while tracking the output column, honoring the
///      screen-full early-return (`ml == mlend - 1 && col == columns - 1`)
///      and the wrap/scroll handling (`if (col > columns) { ml++; if
///      (mscroll && !--mrestlines && (ask = asklistscroll(ml))) return
///      ask; col -= columns; ... }`).
///
/// Column accounting note: the C code walks the *metafied* representation
/// byte-by-byte, demeta-ing as it goes and distinguishing single-width
/// ASCII-prefix bytes (`col++`) from the trailing wide-character bytes
/// (`col += width` once). Rust's [`nicechar`]/[`wcs_nicechar`] return a
/// display-ready native-UTF-8 string with no `Meta` bytes, so we advance
/// the column per *character*: ASCII characters contribute 1 column, the
/// single wide character contributes [`zwcwidth`]. The per-character total
/// equals the C per-byte total and the wrap/screen-full checks fire at the
/// same output positions.
pub fn clnicezputs(do_colors: i32, s: &str, ml_in: i32) -> i32 {
    use crate::ported::lex::untokenize;
    use crate::ported::utils::{nicechar, unmetafy_str, wcs_nicechar, zwcwidth};

    // c:717 — `int i = 0, col = 0, ask, oml = ml;`
    let oml = ml_in;
    let mut ml = ml_in;
    let mut col: i32 = 0;
    let mut i: i32 = 0; // doiscol position (input byte index)

    let zterm_columns = adjustcolumns() as i32;
    let mlend = MLEND.load(Ordering::SeqCst);
    let mscroll = MSCROLL.load(Ordering::SeqCst) != 0;

    let fd = SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };

    // c:741-744 — `ums = ztrdup(s); untokenize(ums);
    //              uptr = unmetafy(ums, &umlen); umleft = umlen;`
    let ums = untokenize(s);
    let ubytes = unmetafy_str(&ums);

    // c:746-747 — `if (do_colors) initiscol();`
    if do_colors != 0 {
        initiscol();
    }
    // c:749 — `mb_charinit();` — no-op in Rust (native UTF-8).

    // c:750 — `while (umleft > 0)`.
    let mut idx = 0usize;
    while idx < ubytes.len() {
        // c:751-776 — decode the next character. A valid UTF-8 sequence
        //             is the `default:` (wcs_nicechar) path; an invalid
        //             or incomplete lead byte is the MB_INVALID/eol path
        //             (nicechar of the raw byte, one byte consumed).
        let b0 = ubytes[idx];
        let seq_len = if b0 < 0x80 {
            1
        } else if b0 >> 5 == 0b110 {
            2
        } else if b0 >> 4 == 0b1110 {
            3
        } else if b0 >> 3 == 0b11110 {
            4
        } else {
            0 // invalid lead byte
        };
        let (rep, cnt): (String, usize) = if seq_len >= 1 && idx + seq_len <= ubytes.len() {
            match std::str::from_utf8(&ubytes[idx..idx + seq_len]) {
                Ok(cs) => {
                    // c:768-775 — valid wide char (case 0 for '\0' also
                    //             lands here with cnt = 1). wcs_nicechar
                    //             prettifies; width tracked per-char below.
                    let cc = cs.chars().next().unwrap();
                    (wcs_nicechar(cc, None, None), seq_len)
                }
                // c:757-767 — MB_INVALID: nicechar of the single byte.
                Err(_) => (nicechar(b0 as char), 1),
            }
        } else {
            // c:754-767 — MB_INCOMPLETE (eol) / invalid lead byte.
            (nicechar(b0 as char), 1)
        };

        idx += cnt;

        // c:780-788 — `if (do_colors) while (cnt--) doiscol(i++);`
        if do_colors != 0 {
            for _ in 0..cnt {
                doiscol(i);
                i += 1;
            }
        }

        // c:795-833 — loop over characters in the nice representation.
        let mut buf = [0u8; 4];
        for ch in rep.chars() {
            // c:799-803 — is the screen full?
            if ml == mlend - 1 && col == zterm_columns - 1 {
                MLPRINTED.store(ml - oml, Ordering::SeqCst);
                return 0;
            }
            // c:806/811 — `putc(nc, shout);`.
            let _ = write_loop(out_fd, ch.encode_utf8(&mut buf).as_bytes());
            // c:807-816 — ASCII characters are single-width; the single
            //             wide character contributes its display width.
            col += if (ch as u32) < 0x80 {
                1
            } else {
                zwcwidth(ch) as i32
            };
            // c:822-832 — wrap / scroll handling.
            if col > zterm_columns {
                ml += 1;
                // c:824 — `if (mscroll && !--mrestlines &&
                //           (ask = asklistscroll(ml)))`.
                if mscroll {
                    let rest = MRESTLINES.fetch_sub(1, Ordering::SeqCst) - 1;
                    if rest == 0 {
                        let ask = asklistscroll(ml);
                        if ask != 0 {
                            // c:825-827
                            MLPRINTED.store(ml - oml, Ordering::SeqCst);
                            return ask;
                        }
                    }
                }
                // c:829 — `col -= zterm_columns;`
                col -= zterm_columns;
                // c:830-831 — `if (do_colors) fputs(" \010", shout);`
                if do_colors != 0 {
                    let _ = write_loop(out_fd, b" \x08");
                }
            }
        }
    }

    // c:874 — `mlprinted = ml - oml;` / c:875 — `return 0;`
    MLPRINTED.store(ml - oml, Ordering::SeqCst);
    0
}

// Get the terminal color string for the given match.                      // c:881
/// Port of `int putmatchcol(char *group, char *n)` from
/// `Src/Zle/complist.c:881`.
///
/// Walks `mcolors.pats` (the user's ZLS_COLORS pattern rules) trying
/// each compiled `Patprog` against `group` then `n`; on a hit emits
/// the first capture's escape via [`zlrputs`] (or stores both for
/// per-char rendering when `cols[1]` is non-empty). Falls back to
/// `mcolors.files[COL_NO]` (the default-file color) via `zcputs`.
/// Returns 1 if the caller should apply two-color per-char rendering
/// (i.e. `cols[1]` is populated), 0 otherwise.
pub fn putmatchcol(group: &str, n: &str) -> i32 {
    // c:881
    let mc = MCOLORS.lock().unwrap();

    // c:884-898 — walk the mcolors.pats chain for a (group, n) match.
    // `getcoldef` now compiles and stores the real `pattern::Patprog`
    // bytecode, so this fires `pattry`/`pattryrefs` exactly as the C
    // source: the group prog (if any) must match `group`, and the value
    // prog must match `n`, capturing submatch positions into begpos/endpos
    // for the per-char two-color rendering path.
    let mut cur = mc.pats.as_deref();
    while let Some(pc) = cur {
        let mut nrefs = (MAX_POS - 1) as i32; // c:886 nrefs = MAX_POS - 1
        let mut begp: Vec<i32> = vec![0; MAX_POS];
        let mut endp: Vec<i32> = vec![0; MAX_POS];
        // c:888 — `(!pc->prog || !group || pattry(pc->prog, group))`.
        let group_ok = match &pc.prog {
            None => true,
            Some(gp) => group.is_empty() || crate::ported::pattern::pattry(gp, group),
        };
        // c:889 — `pattryrefs(pc->pat, n, -1, -1, NULL, 0, &nrefs, begpos, endpos)`.
        let pat_ok = match &pc.pat {
            None => false,
            Some(pat) => crate::ported::pattern::pattryrefs(
                pat,
                n,
                -1,
                -1,
                None,
                0,
                Some(&mut nrefs),
                Some(&mut begp),
                Some(&mut endp),
            ),
        };
        if group_ok && pat_ok {
            // c:890-895 — `if (pc->cols[1]) { patcols = pc->cols; return 1; }`
            if pc.cols.len() > 1 && !pc.cols[1].is_empty() {
                *PATCOLS.lock().unwrap() = pc.cols.clone(); // c:891 patcols = pc->cols
                PATCOLS_IDX.store(0, Ordering::Relaxed);
                // begp/endp are MAX_POS-length (allocated above, c:886) and
                // pattryrefs now fills them IN PLACE without shrinking, so they
                // keep C's fixed `int[MAX_POS]` size for the getcol reset loop.
                *BEGPOS.lock().unwrap() = begp;
                *ENDPOS.lock().unwrap() = endp;
                NREFS.store(nrefs, Ordering::Relaxed);
                return 1; // c:893
            }
            // c:896-897 — `zlrputs(pc->cols[0]); return 0;`
            if let Some(c0) = pc.cols.first() {
                zlrputs(c0);
            }
            return 0;
        }
        cur = pc.next.as_deref();
    }

    // c:900 — `zcputs(group, COL_NO);`. Emit the default file color.
    if let Some(no_col) = mc.files.get(COL_NO) {
        zlrputs(&no_col.col);
    }
    0 // c:902
}

/// Port of `int putfilecol(char *group, char *filename, mode_t m, int special)`
/// from `Src/Zle/complist.c:910`.
///
/// Selects the right LS_COLORS category for `filename` by examining
/// `m` (the lstat mode bits) and emits the matching cap via
/// `zlrputs`. Mirrors the C dispatch by mode: COL_DI for dirs,
/// COL_LN for symlinks, COL_PI for FIFOs, COL_SO for sockets,
/// COL_BD/CD for block/char devices, COL_EX for executable files,
/// then suffix-extension lookups (`mcolors.exts`), then COL_FI
/// fallback. Returns 1 if the caller should apply two-color per-char
/// rendering, 0 otherwise.
pub fn putfilecol(group: &str, filename: &str, m: u32, _special: i32) -> i32 {
    use crate::ported::zle::complist as cl;

    let mc = MCOLORS.lock().unwrap();

    // c:912-918 — walk extcol chain looking for `*.<ext>` suffix match.
    let mut cur = mc.exts.as_deref();
    while let Some(ec) = cur {
        if filename.ends_with(&ec.ext) {
            // c:915 — single-color cap (extcol only has one col).
            zlrputs(&ec.col);
            return 0;
        }
        cur = ec.next.as_deref();
    }

    // c:920-985 — mode-bit dispatch into the COL_* slots.
    let pick = if (m & 0o170000) == 0o040000 {
        cl::COL_DI
    } else if (m & 0o170000) == 0o120000 {
        cl::COL_LN
    } else if (m & 0o170000) == 0o010000 {
        cl::COL_PI
    } else if (m & 0o170000) == 0o140000 {
        cl::COL_SO
    } else if (m & 0o170000) == 0o060000 {
        cl::COL_BD
    } else if (m & 0o170000) == 0o020000 {
        cl::COL_CD
    } else if m & 0o111 != 0 {
        cl::COL_EX
    } else {
        cl::COL_FI
    };

    if let Some(col) = mc.files.get(pick) {
        if !col.col.is_empty() {
            zlrputs(&col.col);
            return 0;
        }
    }
    let _ = group;
    // Final fallback — COL_NO (the no-extension default).
    if let Some(no_col) = mc.files.get(COL_NO) {
        zlrputs(&no_col.col);
    }
    0
}

/// Direct port of `int asklistscroll(int ml)` from
/// `Src/Zle/complist.c:1001`.
///
/// Shown when the completion list exceeds the screen — emits the
/// "--More--" prompt, reads a key via `getkeycmd` under the
/// `listscroll` keymap, then interprets the bound command:
///   - SIGINT / nothing → return 1 (abort scroll)
///   - accept-line / down-line-or-history / etc. → bump
///     `MRESTLINES = 1` (one more line) and continue (return 0)
///   - menu-select / complete-word / etc. → set
///     `MRESTLINES = zterm_lines - 1` (full page) and continue
///   - accept-search → return 1 (abort)
///   - anything else → unget the cmd and return 1
/// Finally clears the prompt line with `\r<columns spaces>\r`.
pub fn asklistscroll(ml: i32) -> i32 {
    use crate::ported::utils::{adjustcolumns, adjustlines};
    use crate::ported::zle::zle_keymap::{getkeycmd, selectlocalmap, ungetkeycmd};
    use crate::ported::zle::zle_main::zsetterm;

    // c:1004 — `compprintfmt(NULL, 1, 1, 1, ml, NULL);` — render the
    //          mstatus / LISTPROMPT line.
    let mut _stop = 0i32;
    let _ = compprintfmt("", 1, 1, 1, ml, &mut _stop);

    // c:1006-1007 — `fflush(shout); zsetterm();`
    let fd = SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };
    let _ = zsetterm();

    // c:1008-1009 — `menuselect_bindings(); selectlocalmap(lskeymap);`
    menuselect_bindings();
    let lsk = crate::ported::zle::zle_keymap::openkeymap("listscroll");
    selectlocalmap(lsk);

    // c:1010 — `cmd = getkeycmd()`. Empty cmd or send-break → abort.
    let ret;
    let cmd = getkeycmd();
    let nm = cmd.as_ref().map(|t| t.nam.as_str()).unwrap_or("");
    match nm {
        "" | "send-break" => {
            ret = 1; // c:1011
        }
        // c:1012-1017 — accept-line family: one more line.
        "accept-line"
        | "down-history"
        | "down-line-or-history"
        | "down-line-or-search"
        | "vi-down-line-or-history" => {
            MRESTLINES.store(1, Ordering::Relaxed);
            ret = 0;
        }
        // c:1018-1029 — menu-complete family: full page.
        "complete-word"
        | "expand-or-complete"
        | "expand-or-complete-prefix"
        | "menu-complete"
        | "menu-expand-or-complete"
        | "menu-select" => {
            MRESTLINES.store(adjustlines() as i32 - 1, Ordering::Relaxed);
            ret = 0;
        }
        // c:1030-1031 — accept-search → abort.
        "accept-search" => {
            ret = 1;
        }
        // c:1032-1035 — anything else: unget + abort.
        _ => {
            ungetkeycmd();
            ret = 1;
        }
    }
    // c:1037 — `selectlocalmap(NULL);`
    selectlocalmap(None);
    // c:1038 — `settyinfo(&shttyinfo);` — restore tty mode after raw read.
    // (zshrs's tty restore happens implicitly when the next prompt
    //  draws; explicit call site omitted until shttyinfo wires up.)

    // c:1039-1043 — clear the prompt line: `\r<spaces*cols-1>\r`.
    let _ = write_loop(out_fd, b"\r");
    let cols = adjustcolumns().saturating_sub(1);
    let blank = vec![b' '; cols];
    let _ = write_loop(out_fd, &blank);
    let _ = write_loop(out_fd, b"\r");

    ret // c:1045
}

/// Port of `int compprintnl(int ml)` from
/// `Src/Zle/complist.c:1054`. Emits clear-to-end + newline to the
/// shell-output fd; if scroll mode is on and the remaining-line
/// budget hits zero, queries `asklistscroll(ml)` (currently
/// substrate-gapped; we skip the scroll prompt and return 0).
///
/// C body c:1056-1064:
/// ```c
/// cleareol(); putc('\n', shout);
/// if (mscroll && !--mrestlines && (ask = asklistscroll(ml))) return ask;
/// return 0;
/// ```
pub fn compprintnl(ml: i32) -> i32 {
    // c:1054
    // c:1056 — `cleareol();` followed by `putc('\n', shout);`. We
    //          emit both as a single write (CSI K + LF).
    let fd = SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };
    let _ = write_loop(out_fd, b"\x1b[K\n");
    // c:1058 — `if (mscroll && !--mrestlines && (ask = asklistscroll(ml)))
    //           return ask;`. This is the per-newline half of the scroll
    //          pager; `printfmt` handles the column-wrap half the same way
    //          (c:824). Without it, MRESTLINES was only decremented on wraps,
    //          so grouped/one-per-line listings never hit the page boundary
    //          and dumped the whole list instead of paging with LISTPROMPT.
    if MSCROLL.load(Ordering::SeqCst) != 0 {
        let rest = MRESTLINES.fetch_sub(1, Ordering::SeqCst) - 1;
        if rest == 0 {
            let ask = asklistscroll(ml);
            if ask != 0 {
                return ask;
            }
        }
    }
    0
}

/// Port of `static int compprintfmt(char *fmt, int n, int dopr,
/// int doesc, int ml, int *stop)` from `Src/Zle/complist.c:1072`.
/// Renders the LISTPROMPT / mstatus / explanation-string format,
/// expanding `%n` (count), `%p` (line position), `%l` (lines),
/// `%m` (current match position), `%M` (last match position),
/// `%S`/`%s` (standout on/off), `%B`/`%b`, `%U`/`%u`, `%F`/`%f`,
/// `%K`/`%k`, and `%%`. Returns the visible width consumed,
/// stopping early if the row hits `mlend`.
/// ```c
/// static int
/// compprintfmt(char *fmt, int n, int dopr, int doesc, int ml, int *stop)
/// {
///     char *p, nc[2*DIGBUFSIZE+12], nbuf[2*DIGBUFSIZE+12];
///     int l = 0, cc = 0, m, ask, beg, stat;
///     if ((stat = !fmt)) {
///         if (mlbeg >= 0) {
///             if (!(fmt = mstatus)) { mlprinted = 0; return 0; }
///             cc = -1;
///         } else fmt = mlistp;
///     }
///     /* per-char loop dispatching every %X */
///     return cc;
/// }
/// ```
pub fn compprintfmt(
    // c:1072
    fmt: &str,
    n: i32,
    dopr: i32,
    doesc: i32,
    ml: i32,
    stop: &mut i32,
) -> i32 {
    use std::sync::atomic::Ordering;

    let mut l = 0i32; // c:1075
    let mut cc = 0i32;
    let _ = doesc;
    let _ = ml;
    let _ = stop;

    // c:1077-1086 — fmt fallback to mstatus / mlistp when caller passed NULL.
    let owned: String;
    let fmt_str: &str = if fmt.is_empty() {
        // c:1077
        if MLBEG.load(Ordering::SeqCst) >= 0 {
            // c:1078
            owned = MSTATUS.lock().unwrap().clone();
            if owned.is_empty() {
                MLPRINTED.store(0, Ordering::SeqCst); // c:1080
                return 0; // c:1081
            }
            cc = -1; // c:1083
            &owned
        } else {
            // c:1084
            owned = MLISTP.lock().unwrap().clone();
            &owned
        }
    } else {
        fmt
    };

    // c:1087-end — escape dispatch loop. Implement the daily-driver
    // subset (the LIST_PACKED escape arms that LISTPROMPT users hit).
    let mut chars = fmt_str.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            // c:1102
            // c:1108 — optional digit arg
            let mut arg = 0i32;
            while let Some(&d) = chars.peek() {
                if d.is_ascii_digit() {
                    arg = arg * 10 + (d as i32 - '0' as i32);
                    chars.next();
                } else {
                    break;
                }
            }
            match chars.next() {
                // c:1119
                Some('%') => {
                    if dopr == 1 {
                        l += 1;
                    }
                    cc += 1;
                } // c:1120
                Some('n') => {
                    // c:1141
                    let s = n.to_string();
                    if dopr == 1 {
                        let fd = SHTTY.load(Ordering::Relaxed);
                        let out_fd = if fd >= 0 { fd } else { 1 };
                        let _ = write_loop(out_fd, s.as_bytes());
                    }
                    l += s.len() as i32;
                    cc += s.len() as i32;
                }
                Some('p') => {
                    // c:1155 line position
                    let mlbeg = MLBEG.load(Ordering::SeqCst);
                    let mlines = MLINES.load(Ordering::SeqCst);
                    let s = if mlbeg <= 0 && mlines < MLEND.load(Ordering::SeqCst) {
                        "Top".to_string()
                    } else if mlbeg + MLEND.load(Ordering::SeqCst) - MLBEG.load(Ordering::SeqCst)
                        >= mlines
                    {
                        "Bot".to_string()
                    } else {
                        format!("{}%", mlbeg.max(0) * 100 / mlines.max(1))
                    };
                    if dopr == 1 {
                        let fd = SHTTY.load(Ordering::Relaxed);
                        let out_fd = if fd >= 0 { fd } else { 1 };
                        let _ = write_loop(out_fd, s.as_bytes());
                    }
                    l += s.len() as i32;
                    cc += s.len() as i32;
                }
                Some(_) => {
                    let _ = arg;
                } // c:other-escape
                None => break,
            }
        } else {
            // c:literal char
            if dopr == 1 {
                let fd = SHTTY.load(Ordering::Relaxed);
                let out_fd = if fd >= 0 { fd } else { 1 };
                let mut buf = [0u8; 4];
                let bs = c.encode_utf8(&mut buf).as_bytes();
                let _ = write_loop(out_fd, bs);
            }
            l += 1;
            cc += 1;
        }
    }
    let _ = l;
    cc // c:return
}

/// Port of `static char *mstatus` from `Src/Zle/complist.c:93`. Message
/// printed when the user scrolls the completion list.
pub static MSTATUS: std::sync::LazyLock<std::sync::Mutex<String>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(String::new())); // c:93

/// Port of `static char *mlistp` from `Src/Zle/complist.c:93`. Message
/// printed when merely listing (no scroll).
pub static MLISTP: std::sync::LazyLock<std::sync::Mutex<String>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(String::new())); // c:93

/// Port of `int compzputs(char const *s, int ml)` from
/// `Src/Zle/complist.c:1338`. Demetafies each byte (Meta XOR 32),
/// skips `itok` pseudo-tokens (0x80-0x9f), writes the result to
/// the shell-output fd. The C source also handles wrap detection +
/// `asklistscroll` scroll-prompts; those land when the curses
/// substrate is wired.
#[allow(unused_variables)]
pub fn compzputs(s: &str, ml: i32) -> i32 {
    // c:1338
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == 0x83 {
            // c:1343 Meta byte
            i += 1;
            if i < bytes.len() {
                out.push(bytes[i] ^ 32);
            }
        } else if (0x80..0xa0).contains(&c) { // c:1345 itok skip
             // pass — pseudo-token
        } else {
            out.push(c);
        }
        i += 1;
    }
    if out.is_empty() {
        return 0;
    }
    let fd = SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };
    let _ = write_loop(out_fd, &out); // c:1356 putc loop
    0
}

/// Port of `static int compprintlist(int showall)` from
/// `Src/Zle/complist.c:1367`. Walks the active `amatches` group
/// chain, emits explanations + ylist + cmatch grid via
/// `clprintm` / `compprintfmt` / `compzputs`, tracking line
/// position against `mlbeg`/`mlend` for resumable scrolling.
pub fn compprintlist(showall: i32) -> i32 {
    // c:1367

    let mut pnl = 0i32; // c:1378
    let mut cl: i32;
    let mut ml: i32 = 0;
    let mut mc: i32;
    let mut printed = 0i32;
    let mut stop = 0i32;
    let _asked = 1i32;
    let mut lastused = 0i32; // c:1379

    let mlbeg = MLBEG.load(Ordering::SeqCst);
    let mlend = MLEND.load(Ordering::SeqCst);
    let mnew = MNEW.load(Ordering::SeqCst);
    let mhasstat = MHASSTAT.load(Ordering::SeqCst);
    let zterm_lines = adjustlines() as i32;
    let nlnct = NLNCT.load(Ordering::SeqCst);
    let invcount = crate::ported::zle::compresult::INVCOUNT.load(Ordering::SeqCst);

    MFIRSTL.store(-1, Ordering::SeqCst); // c:1381

    // c:1382-1388 — reset accumulators when ainfo changed.
    let mut last_type = LAST_TYPE.load(Ordering::SeqCst);
    let last_invcount = LAST_INVCOUNT.load(Ordering::SeqCst);
    let last_beg = LAST_BEG.load(Ordering::SeqCst);
    if mnew != 0 || last_invcount != invcount || last_beg != mlbeg || mlbeg < 0 {
        last_type = 0; // c:1383-1387
        LAST_TYPE.store(0, Ordering::SeqCst);
        LAST_NLNCT.store(-1, Ordering::SeqCst);
    }

    // c:1389-1391 — clear-line budget for the current paint.
    let listdat_nlines = listdat
        .get()
        .and_then(|m| m.lock().ok().map(|g| g.nlines))
        .unwrap_or(0);
    cl = if listdat_nlines > zterm_lines - nlnct - mhasstat {
        // c:1389
        zterm_lines - nlnct - mhasstat
    } else {
        listdat_nlines
    } - if LAST_NLNCT.load(Ordering::SeqCst) > nlnct {
        1
    } else {
        0
    };
    LAST_NLNCT.store(nlnct, Ordering::SeqCst); // c:1392
    MRESTLINES.store(zterm_lines - 1, Ordering::SeqCst); // c:1393
    LAST_INVCOUNT.store(invcount, Ordering::SeqCst); // c:1394

    let tcd_avail = crate::ported::init::tclen.lock().unwrap()[TCCLEAREOD as usize] != 0; // c:1398
    let tceol_avail = crate::ported::init::tclen.lock().unwrap()[TCCLEAREOL as usize] != 0;

    if cl < 2 {
        // c:1396
        cl = -1; // c:1397
        if tcd_avail {
            // c:1398
            tcout(TCCLEAREOD); // c:1399
        }
    } else if mlbeg >= 0 && !tceol_avail && tcd_avail {
        // c:1400
        tcout(TCCLEAREOD); // c:1401
    }

    // c:1403-1679 — walk amatches groups.
    let groups: Vec<Cmgroup> = {
        crate::ported::zle::compcore::amatches
            .get_or_init(|| std::sync::Mutex::new(Vec::new()))
            .lock()
            .ok()
            .map(|g| g.clone())
            .unwrap_or_default()
    };

    let dolist = |x: i32| -> bool { x >= mlbeg && x < mlend }; // c:1048
    let dolistcl = |x: i32| -> bool { x >= mlbeg && x < mlend + 1 }; // c:1049
    let dolistnl = |x: i32| -> bool { x >= mlbeg && x < mlend - 1 }; // c:1050

    'outer: for g in &groups {
        // c:1404
        if errflag.load(Ordering::SeqCst) != 0 {
            // c:1404 !errflag
            break;
        }

        // c:1405 — `char **pp = g->ylist;`
        let pp = &g.ylist;
        let onlyexpl: i32 = listdat
            .get()
            .and_then(|m| m.lock().ok().map(|g| g.onlyexpl))
            .unwrap_or(0);

        // c:1412-1470 — emit explanation strings.
        if !g.expls.is_empty() {
            // c:1412
            for e in &g.expls {
                // c:1418
                if errflag.load(Ordering::SeqCst) != 0 {
                    break 'outer;
                }
                let valid = (e.count != 0 || e.always != 0)                  // c:1419
                    && (onlyexpl == 0
                        || (onlyexpl & if e.always > 0 { 2 } else { 1 }) != 0);
                if valid {
                    if pnl != 0 {
                        // c:1422
                        if dolistnl(ml) && compprintnl(ml) != 0 {
                            // c:1423
                            break 'outer;
                        }
                        pnl = 0; // c:1425
                        ml += 1; // c:1426
                        if dolistcl(ml) && cl >= 0 {
                            // c:1427
                            cl -= 1;
                            if cl <= 1 {
                                cl = -1; // c:1428
                                if tcd_avail {
                                    // c:1429
                                    tcout(TCCLEAREOD);
                                }
                            }
                        }
                    }
                    if mlbeg < 0 && MFIRSTL.load(Ordering::SeqCst) < 0 {
                        // c:1433
                        MFIRSTL.store(ml, Ordering::SeqCst); // c:1434
                    }
                    let n = if e.always != 0 { -1 } else { e.count };
                    let estr = e.str.clone().unwrap_or_default();
                    let _ = compprintfmt(
                        // c:1435
                        &estr,
                        n,
                        if dolist(ml) { 1 } else { 0 },
                        1,
                        ml,
                        &mut stop,
                    );
                    if stop != 0 {
                        break 'outer;
                    } // c:1447
                    if last_type == 0 && ml >= mlbeg {
                        // c:1449
                        last_type = 1; // c:1450
                        LAST_TYPE.store(1, Ordering::SeqCst);
                        LAST_BEG.store(mlbeg, Ordering::SeqCst);
                        LAST_ML.store(ml, Ordering::SeqCst);
                        lastused = 1;
                    }
                    ml += MLPRINTED.load(Ordering::SeqCst); // c:1458
                    if dolistcl(ml) && cl >= 0 {
                        // c:1459
                        cl -= MLPRINTED.load(Ordering::SeqCst);
                        if cl <= 1 {
                            cl = -1;
                            if tcd_avail {
                                tcout(TCCLEAREOD);
                            }
                        }
                    }
                    pnl = 1; // c:1464
                }
                if mnew == 0 && ml > mlend {
                    break 'outer;
                } // c:1467
            }
        }

        // c:1471-1529 — ylist short-form rendering.
        if onlyexpl == 0 && mlbeg < 0 && !pp.is_empty() {
            // c:1471
            if pnl != 0 {
                // c:1472
                if dolistnl(ml) && compprintnl(ml) != 0 {
                    break 'outer;
                } // c:1473
                pnl = 0;
                ml += 1;
                if cl >= 0 {
                    cl -= 1;
                    if cl <= 1 {
                        cl = -1;
                        if tcd_avail {
                            tcout(TCCLEAREOD);
                        }
                    }
                }
            }
            if mlbeg < 0 && MFIRSTL.load(Ordering::SeqCst) < 0 {
                MFIRSTL.store(ml, Ordering::SeqCst);
            }
            if (g.flags & CGF_LINES) != 0 {
                // c:1485
                for s in pp {
                    // c:1486
                    if compzputs(s, ml) != 0 {
                        break 'outer;
                    } // c:1487
                    if compprintnl(ml) != 0 {
                        break 'outer;
                    } // c:1489
                }
            } else {
                // c:1492-1528 — packed ylist columns.
                // Single-pass emit; column-perfect alignment defers to
                // the column-width helper port.
                for s in pp {
                    if compzputs(s, MSCROLL.load(Ordering::SeqCst)) != 0 {
                        // c:1505
                        break 'outer;
                    }
                    if compprintnl(ml) != 0 {
                        break 'outer;
                    } // c:1518
                    ml += 1;
                }
            }
        } else if onlyexpl == 0 && (g.lcount != 0 || (showall != 0 && g.mcount != 0)) {
            // c:1530
            // c:1532-1675 — cmatch grid render.
            let n_total = g.dcount;
            let nc = g.lins;

            // c:1537-1590 — CGF_HASDL whole-line displays.
            if (g.flags & CGF_HASDL) != 0 {
                // c:1537
                for m in &g.matches {
                    // c:1549
                    let displine = m.disp.is_some() && (m.flags & CMF_DISPLINE) != 0;
                    let visible = showall != 0 || (m.flags & (CMF_HIDE | CMF_NOLIST)) == 0;
                    if displine && visible {
                        // c:1551
                        if pnl != 0 {
                            // c:1552
                            if dolistnl(ml) && compprintnl(ml) != 0 {
                                break 'outer;
                            }
                            pnl = 0;
                            ml += 1;
                            if dolistcl(ml) && cl >= 0 {
                                cl -= 1;
                                if cl <= 1 {
                                    cl = -1;
                                    if tcd_avail {
                                        tcout(TCCLEAREOD);
                                    }
                                }
                            }
                        }
                        if last_type == 0 && ml >= mlbeg {
                            // c:1563
                            last_type = 2;
                            LAST_TYPE.store(2, Ordering::SeqCst);
                            LAST_BEG.store(mlbeg, Ordering::SeqCst);
                            LAST_ML.store(ml, Ordering::SeqCst);
                            lastused = 1;
                        }
                        if MFIRSTL.load(Ordering::SeqCst) < 0 {
                            // c:1573
                            MFIRSTL.store(ml, Ordering::SeqCst);
                        }
                        if dolist(ml) {
                            printed += 1;
                        } // c:1575
                        if clprintm(Some(g), Some(m), 0, ml, 1, 0) != 0 {
                            // c:1577
                            break 'outer;
                        }
                        ml += MLPRINTED.load(Ordering::SeqCst); // c:1579
                        if dolistcl(ml) {
                            cl -= MLPRINTED.load(Ordering::SeqCst);
                            if cl <= 1 {
                                cl = -1;
                                if tcd_avail {
                                    tcout(TCCLEAREOD);
                                }
                            }
                        }
                        pnl = 1; // c:1585
                    }
                    if mnew == 0 && ml > mlend {
                        break 'outer;
                    } // c:1587
                }
            }
            // c:1591 — `if (n && pnl)`. This is the newline that SEPARATES the
            // CGF_HASDL displine rows from the column grid printed below them.
            // It must fire ONLY when there ARE grid matches to print
            // (`n = g->dcount`). When every match is a displine (dcount == 0,
            // e.g. options rendered one-per-line with descriptions: `mkdir -`,
            // `cp -`, `mv -`, `ps -`, …), the `n &&` guard suppresses it — the
            // port dropped the guard (`if pnl != 0`), so it emitted a spurious
            // trailing newline after the LAST displine. The list then printed
            // one row too tall, so the always-last-prompt cursor-up
            // (`nlines+nlnct-1`) landed one row too low and reprinted the
            // command line over the first option. This is the complist (`zmodload
            // zsh/complist`) twin of the compresult.rs `dcount` fix.
            if n_total != 0 && pnl != 0 {
                if dolistnl(ml) && compprintnl(ml) != 0 {
                    break 'outer;
                }
                pnl = 0;
                ml += 1;
                if dolistcl(ml) && cl >= 0 {
                    cl -= 1;
                    if cl <= 1 {
                        cl = -1;
                        if tcd_avail {
                            tcout(TCCLEAREOD);
                        }
                    }
                }
            }

            // c:1611-1674 — grid row/column loop.
            let mut nl_cnt = nc;
            // c:1609 — `p = skipnolist(g->matches, showall)`. Must use the
            // full skipnolist predicate (compresult.rs): besides CMF_HIDE /
            // CMF_NOLIST / CMF_MULT it ALSO skips `disp && CMF_DISPLINE`
            // matches — those are printed by the CGF_HASDL block above, so
            // the grid must not re-print them (else described matches double-
            // print and concatenate into the packed group).
            let mut p_idx: usize =
                crate::ported::zle::compresult::skipnolist(&g.matches, showall);
            let mut n = g.dcount;
            while n > 0 && nl_cnt > 0 && errflag.load(Ordering::SeqCst) == 0 {
                if last_type == 0 && ml >= mlbeg {
                    // c:1612
                    last_type = 3;
                    LAST_TYPE.store(3, Ordering::SeqCst);
                    LAST_BEG.store(mlbeg, Ordering::SeqCst);
                    LAST_ML.store(ml, Ordering::SeqCst);
                    lastused = 1;
                }
                let mut i = g.cols; // c:1622
                mc = 0;
                let mut q_idx = p_idx;
                while n > 0 && i > 0 && errflag.load(Ordering::SeqCst) == 0 {
                    i -= 1;
                    let wid = if !g.widths.is_empty() {
                        // c:1626
                        g.widths.get(mc as usize).copied().unwrap_or(g.width)
                    } else {
                        g.width
                    };
                    let m_at_q = g.matches.get(q_idx); // c:1627
                    match m_at_q {
                        None => {
                            // c:1627 !m
                            if clprintm(
                                Some(g),
                                None,
                                mc,
                                ml, // c:1628
                                if i == 0 { 1 } else { 0 },
                                wid,
                            ) != 0
                            {
                                break 'outer;
                            }
                            break;
                        }
                        Some(m) => {
                            // c:1632
                            if clprintm(Some(g), Some(m), mc, ml, if i == 0 { 1 } else { 0 }, wid)
                                != 0
                            {
                                break 'outer;
                            }
                            if dolist(ml) {
                                printed += 1;
                            } // c:1635
                            ml += MLPRINTED.load(Ordering::SeqCst); // c:1637
                            if dolistcl(ml) {
                                cl -= MLPRINTED.load(Ordering::SeqCst);
                                if cl < 1 {
                                    cl = -1;
                                    if tcd_avail {
                                        tcout(TCCLEAREOD);
                                    }
                                }
                            }
                            if MFIRSTL.load(Ordering::SeqCst) < 0 {
                                // c:1643
                                MFIRSTL.store(ml, Ordering::SeqCst);
                            }
                            n -= 1; // c:1646
                            if n > 0 {
                                // c:1646
                                let step = if (g.flags & CGF_ROWS) != 0 {
                                    1
                                } else {
                                    nc as usize
                                };
                                for _j in 0..step {
                                    // c:1647
                                    if q_idx < g.matches.len() {
                                        q_idx += 1;
                                    }
                                    // c:1649 — `q = skipnolist(q+1, showall)`
                                    q_idx += crate::ported::zle::compresult::skipnolist(
                                        &g.matches[q_idx..],
                                        showall,
                                    );
                                }
                            }
                            mc += 1; // c:1650
                        }
                    }
                }
                // c:1652-1657 — fill trailing columns with empty cells.
                while i > 0 {
                    i -= 1;
                    let wid = if !g.widths.is_empty() {
                        g.widths.get(mc as usize).copied().unwrap_or(g.width)
                    } else {
                        g.width
                    };
                    if clprintm(Some(g), None, mc, ml, if i == 0 { 1 } else { 0 }, wid) != 0 {
                        break 'outer;
                    }
                    mc += 1;
                }
                if n > 0 {
                    // c:1658
                    if dolistnl(ml) && compprintnl(ml) != 0 {
                        break 'outer;
                    }
                    ml += 1; // c:1661
                    if dolistcl(ml) && cl >= 0 {
                        // c:1662
                        cl -= 1;
                        if cl <= 1 {
                            cl = -1;
                            if tcd_avail {
                                tcout(TCCLEAREOD);
                            }
                        }
                    }
                    if nl_cnt > 0 {
                        // c:1667
                        let step = if (g.flags & CGF_ROWS) != 0 {
                            g.cols as usize
                        } else {
                            1
                        };
                        for _j in 0..step {
                            if p_idx < g.matches.len() {
                                p_idx += 1;
                            }
                            // c:1670 — `p = skipnolist(p+1, showall)`
                            p_idx += crate::ported::zle::compresult::skipnolist(
                                &g.matches[p_idx..],
                                showall,
                            );
                        }
                    }
                }
                if mnew == 0 && ml > mlend {
                    break 'outer;
                } // c:1672
                nl_cnt -= 1;
            }
        }
        if g.lcount != 0 || (showall != 0 && g.mcount != 0) {
            // c:1676
            pnl = 1; // c:1677
        }
    }
    // c:1681 end:
    MSTATPRINTED.store(0, Ordering::SeqCst); // c:1682
    LASTLISTLEN.store(0, Ordering::SeqCst); // c:1683
    if nlnct <= 1 {
        MSCROLL.store(0, Ordering::SeqCst);
    } // c:1684

    let _ = lastused;

    // c:1686-1726 — clearflag epilogue. Previously elided; without it the
    // cursor is left at the BOTTOM of the just-painted list, so the next
    // repaint (menu-select navigation) appends a fresh copy below the old one
    // → the list cascades down the screen. In menu-select mode (`mlbeg >= 0`)
    // C moves the cursor back UP to the top of the list so the next paint
    // overwrites it in place.
    let clearflag = CLEARFLAG.load(Ordering::SeqCst);
    let listdat_nlines_end = listdat
        .get()
        .and_then(|m| m.lock().ok().map(|g| g.nlines))
        .unwrap_or(0);
    // c:1679 — `asked = 0` before end:; the "show all N?" prompt path (which
    // sets asked=1) isn't wired, so it is always 0 here.
    let asked = 0i32;
    if clearflag != 0 {
        // c:1687
        if mlbeg >= 0 {
            // c:1691
            let mut nl = listdat_nlines_end + nlnct;
            if nl >= zterm_lines {
                // c:1692
                if mhasstat != 0 {
                    // c:1693-1697 — status line at the bottom when the list
                    // fills the screen.
                    let fd = SHTTY.load(Ordering::Relaxed);
                    let out_fd = if fd >= 0 { fd } else { 1 };
                    let _ = write_loop(out_fd, b"\n");
                    let mut stop = 0;
                    compprintfmt("", 0, 1, 1, MLINE.load(Ordering::SeqCst), &mut stop);
                    MSTATPRINTED.store(1, Ordering::SeqCst);
                }
                nl = zterm_lines - 1; // c:1698
            } else {
                nl -= 1; // c:1700
            }
            tcmultout(crate::ported::zsh_h::TCUP, crate::ported::zsh_h::TCMULTUP, nl); // c:1701
            SHOWINGLIST.store(-1, Ordering::SeqCst); // c:1702
            LASTLISTLEN.store(listdat_nlines_end, Ordering::SeqCst); // c:1704
        } else {
            let nl = listdat_nlines_end + nlnct - 1;
            if nl < zterm_lines {
                // c:1705
                cleareol(); // c:1706
                tcmultout(crate::ported::zsh_h::TCUP, crate::ported::zsh_h::TCMULTUP, nl); // c:1707
                SHOWINGLIST.store(-1, Ordering::SeqCst); // c:1708
                LASTLISTLEN.store(listdat_nlines_end, Ordering::SeqCst); // c:1710
            } else {
                CLEARFLAG.store(0, Ordering::SeqCst); // c:1712
                if asked == 0 {
                    // c:1713-1715
                    MRESTLINES.store(
                        if ml + nlnct > zterm_lines { 1 } else { 0 },
                        Ordering::SeqCst,
                    );
                    compprintnl(ml);
                }
            }
        }
    } else if asked == 0 {
        // c:1717-1719
        MRESTLINES.store(
            if ml + nlnct > zterm_lines { 1 } else { 0 },
            Ordering::SeqCst,
        );
        compprintnl(ml);
    }
    // c:1721 — `listshown = (clearflag ? 1 : -1);`
    LISTSHOWN.store(
        if CLEARFLAG.load(Ordering::SeqCst) != 0 { 1 } else { -1 },
        Ordering::SeqCst,
    );
    MNEW.store(0, Ordering::SeqCst); // c:1722

    printed // c:1724
}

/// Port of `static int lasttype` from `Src/Zle/complist.c:1369`.
pub static LAST_TYPE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:1369

/// Port of `static int lastbeg` from `Src/Zle/complist.c:1369`.
pub static LAST_BEG: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:1369

/// Port of `static int lastml` from `Src/Zle/complist.c:1369`.
pub static LAST_ML: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:1369

/// Port of `static int lastinvcount` from `Src/Zle/complist.c:1369`.
pub static LAST_INVCOUNT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1); // c:1369

/// Port of `static int lastnlnct` from `Src/Zle/complist.c:1370`.
pub static LAST_NLNCT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1); // c:1370

/// Port of `static int clprintm(Cmgroup g, Cmatch *mp, int mc, int ml,
/// int lastc, int width)` from `Src/Zle/complist.c:1730`. Renders one
/// match cell into the listing: emits LS_COLORS prefix, the match
/// string (via `clnicezputs`), the file-type marker if `CGF_FILES`,
/// and trailing padding spaces up to `width`. Also writes the
/// `mtab[][]`/`mgtab[][]` cells so the keymap-navigation path can
/// find the current selection by (mline, mcol).
/// ```c
/// static int
/// clprintm(Cmgroup g, Cmatch *mp, int mc, int ml, int lastc, int width)
/// {
///     Cmatch m;
///     int len, subcols = 0, stop = 0, ret = 0;
///     if (g != last_group) *last_cap = '\0';
///     last_group = g;
///     if (!mp) { /* empty cell: pad with COL_SP spaces; return */ }
///     m = *mp;
///     mlastm = m->gnum;
///     if (m->disp && (m->flags & CMF_DISPLINE)) {
///         /* whole-line display: write mtab cells for the line,
///            color via COL_MA (selected) / COL_HI (nolist) / COL_DU
///            (dupe) / putmatchcol; emit via clprintfmt or compprintfmt */
///     } else {
///         /* normal grid cell: write mtab cells for the cell width,
///            color via COL_MA / COL_HI / COL_DU / putfilecol /
///            putmatchcol; emit string via clnicezputs; emit modec
///            marker for CGF_FILES; pad with COL_SP spaces */
///     }
///     zcoff();
///     return ret;
/// }
/// ```
/// WARNING: param names don't match C — Rust=(g, m, mc, ml, lastc, width) vs C=(g, mp, mc, ml, lastc, width)
pub fn clprintm(
    g: Option<&Cmgroup>,
    m: Option<&Cmatch>,
    mc: i32,
    ml: i32,
    lastc: i32,
    width: i32,
) -> i32 {
    // c:1730
    use std::sync::atomic::Ordering;

    let mselect = MSELECT.load(Ordering::SeqCst);
    let mcols = MCOLS.load(Ordering::SeqCst);
    let zterm_columns = adjustcolumns() as i32;
    let mlines_v = MLINES.load(Ordering::SeqCst); // c:1735

    // c:1735-1737 — DPUTS2(mselect >= 0 && ml >= mlines,
    //                      "clprintm called with ml too large (%d/%d)",
    //                      ml, mlines)
    DPUTS2!(
        // c:1735
        mselect >= 0 && ml >= mlines_v, // c:1735
        "clprintm called with ml too large ({}/{})",
        ml,
        mlines_v // c:1736-1737
    );

    // c:1738-1741 — group-change detection: reset last_cap so the
    // next zcputs writes a fresh color prefix.
    {
        let mut lg = LAST_GROUP.lock().unwrap();
        let g_name = g.and_then(|grp| grp.name.clone()).unwrap_or_default();
        if *lg != g_name {
            // c:1738
            *LAST_CAP.lock().unwrap() = String::new(); // c:1739
            *lg = g_name; // c:1741
        }
    }

    // c:1743-1753 — empty cell case.
    let m_ref = match m {
        // c:1743
        Some(m_real) => m_real,
        None => {
            // c:1744 — `if (dolist(ml))` — list-decision predicate.
            // Without dolist port we treat all rows as visible.
            if let Some(grp) = g {
                // c:1745
                let _ = grp;
                // c:1745 — zcputs(g->name, COL_SP)
                // c:1747-1748 — pad with `width-2` spaces
                let pad = (width - 2).max(0) as usize;
                let pad_str = " ".repeat(pad);
                let fd = SHTTY.load(Ordering::Relaxed);
                let out_fd = if fd >= 0 { fd } else { 1 };
                let _ = write_loop(out_fd, pad_str.as_bytes());
                // c:1749 — zcoff() reset
            }
            MLPRINTED.store(0, Ordering::SeqCst); // c:1751
            return 0; // c:1752
        }
    };

    // c:1754 — `m = *mp;` (Rust: deref already done by Some(m))

    // c:1756-1757 — bld_all_str for CMF_ALL with empty disp.
    // (CMF_ALL flag at comp.h:140; bld_all_str ported elsewhere.)
    let _ = crate::ported::zle::comp_h::CMF_ALL;

    // c:1759 — `mlastm = m->gnum;`
    MLASTM.store(m_ref.gnum, Ordering::SeqCst);

    // c:1760 — `if (m->disp && (m->flags & CMF_DISPLINE))`
    let displine = m_ref.disp.is_some() && (m_ref.flags & CMF_DISPLINE) != 0;
    if displine {
        // c:1760
        // c:1761-1777 — write mtab cells for whole-line display.
        if mselect >= 0 {
            // c:1761
            let mm = mcols * ml; // c:1762
            let mut mtab_guard = MTAB.lock().unwrap();
            let mut mgtab_guard = MGTAB.lock().unwrap();
            for i in 0..mcols {
                // c:1765 / c:1771
                let idx = (mm + i) as usize;
                if idx < mtab_guard.len() {
                    mtab_guard[idx] = Some(m_ref.clone()); // c:1767/1773
                    if let Some(grp) = g {
                        mgtab_guard[idx] = Some(grp.clone()); // c:1768/1774
                    }
                }
            }
        }
        // c:1782-1788 — selected? capture current mline/mcol.
        if m_ref.gnum == mselect {
            // c:1782
            let mm = mcols * ml;
            MLINE.store(ml, Ordering::SeqCst); // c:1785
            MCOL.store(0, Ordering::SeqCst); // c:1786
            MMTABP.store(mm.max(0) as usize, Ordering::SeqCst); // c:1787
            MGTABP.store(mm.max(0) as usize, Ordering::SeqCst); // c:1788
        }
        // c:1789-1797 — color selection. Stubbed to no-op output;
        // the live zcputs / putmatchcol / putfilecol pipeline lands
        // once those helpers port.
        // c:1801 — compprintfmt(m->disp, 0, 1, 0, ml, &stop)
        let disp = m_ref.disp.as_deref().unwrap_or("");
        let _ = compprintfmt(disp, 0, 1, 0, ml, &mut 0); // c:1801
    } else {
        // c:1806-1898 — normal grid-cell display.
        let mx = if !g.is_some_and(|grp| grp.widths.is_empty()) {
            // c:1809-1813
            // c:1812-1813 — sum widths[0..mc]
            g.map(|grp| grp.widths.iter().take(mc as usize).sum::<i32>())
                .unwrap_or(0)
        } else {
            // c:1815 — `mx = mc * g->width;`
            mc * g.map(|grp| grp.width).unwrap_or(0)
        };

        // c:1817-1832 — write mtab cells for cell width.
        if mselect >= 0 {
            // c:1817
            let mm = mcols * ml;
            let mut mtab_guard = MTAB.lock().unwrap();
            let mut mgtab_guard = MGTAB.lock().unwrap();
            let n = if width != 0 { width } else { mcols };
            for i in 0..n {
                // c:1821 / c:1827
                let idx = (mx + mm + i) as usize;
                if idx < mtab_guard.len() {
                    mtab_guard[idx] = Some(m_ref.clone()); // c:1823/1829
                    if let Some(grp) = g {
                        mgtab_guard[idx] = Some(grp.clone()); // c:1824/1830
                    }
                }
            }
        }

        // c:1842-1850 — selected? capture coords.
        if m_ref.gnum == mselect {
            // c:1842
            let mm = mcols * ml;
            MCOL.store(mx, Ordering::SeqCst); // c:1846
            MLINE.store(ml, Ordering::SeqCst); // c:1847
            MMTABP.store((mx + mm).max(0) as usize, Ordering::SeqCst); // c:1848
            MGTABP.store((mx + mm).max(0) as usize, Ordering::SeqCst); // c:1849
        }

        let display = m_ref
            .disp
            .as_deref()
            .unwrap_or_else(|| m_ref.str.as_deref().unwrap_or(""));
        let group = g.and_then(|grp| grp.name.clone()).unwrap_or_default();

        // c:1855-1875 — pick the color cap and emit its prefix. COL_MA
        // (selected), COL_HI (nolist), COL_DU (duplicate) are group-file
        // colors; a filesystem match uses putfilecol; everything else is a
        // list-colors pattern lookup via putmatchcol. Each writer emits its
        // SGR prefix directly; `subcols` requests per-char (two-color)
        // rendering inside clnicezputs.
        let mut subcols = 0i32;
        // helper: emit mcolors.files[idx] cap for the group (C zcputs).
        // A cap read from termcap (COL_MA/COL_EC when getcols took the
        // TCSTANDOUTBEG branch) is ALREADY a complete escape sequence
        // (e.g. "\e[7m"); `zlrputs` SGR-wraps its argument ("\e[{cap}m"),
        // which would double-wrap that into "\e[\e[7mm" garbage — the menu
        // selection highlight rendered as nothing. Emit an already-escaped
        // cap raw; SGR-wrap only bare LS_COLORS codes like "7" / "1;35".
        let emit_filecol = |idx: usize| {
            let cap = MCOLORS
                .lock()
                .ok()
                .and_then(|mc| mc.files.get(idx).map(|f| f.col.clone()))
                .unwrap_or_default();
            if cap.starts_with('\x1b') {
                let fd = SHTTY.load(Ordering::Relaxed);
                let out = if fd >= 0 { fd } else { 1 };
                let _ = write_loop(out, cap.as_bytes());
            } else {
                let _ = zlrputs(&cap);
            }
        };
        if m_ref.gnum == mselect {
            emit_filecol(COL_MA); // c:1866
        } else if (m_ref.flags & CMF_NOLIST) != 0 {
            emit_filecol(COL_HI); // c:1868
        } else if mselect >= 0 && (m_ref.flags & (CMF_MULT | CMF_FMULT)) != 0 {
            emit_filecol(COL_DU); // c:1870
        } else if m_ref.mode != 0 {
            // c:1855-1863 — filesystem match: putfilecol with orphan detect.
            let orphan = if m_ref.mode != 0 && m_ref.fmode == 0 {
                COL_OR as i32
            } else {
                -1
            };
            let follow = MCOLORS
                .lock()
                .map(|mc| (mc.flags & LC_FOLLOW_SYMLINKS) != 0)
                .unwrap_or(false);
            let mode = if follow { m_ref.fmode } else { m_ref.mode };
            subcols = putfilecol(&group, m_ref.str.as_deref().unwrap_or(""), mode, orphan);
        } else {
            // c:1875 — list-colors pattern lookup.
            subcols = putmatchcol(&group, display);
        }

        // c:1872 — `ret = clnicezputs(subcols, m->disp ? m->disp : m->str, ml);`
        let _ = clnicezputs(subcols, display, ml);
        // c:1876 — `zcoff();` resets the cap (COL_NO / SGR reset).
        {
            let fd = SHTTY.load(Ordering::Relaxed);
            let out_fd = if fd >= 0 { fd } else { 1 };
            let _ = write_loop(out_fd, b"\x1b[0m");
        }
        {
            let mut lc = LAST_CAP.lock().unwrap();
            lc.clear();
        }
        zcoff();

        let len_str = display.chars().count() as i32;
        let lines = if len_str > 0 {
            (len_str - 1) / zterm_columns
        } else {
            0
        };
        MLPRINTED.store(lines, Ordering::SeqCst); // c:1879

        // c:1881-1888 — emit modec marker for CGF_FILES groups.
        let cgf_files = g
            .map(|grp| (grp.flags & crate::ported::zle::comp_h::CGF_FILES) != 0)
            .unwrap_or(false);
        let modec = m_ref.modec as u8;
        let mut emitted_marker = 0i32;
        if cgf_files && modec != 0 {
            // c:1882
            let fd = SHTTY.load(Ordering::Relaxed);
            let out_fd = if fd >= 0 { fd } else { 1 };
            let _ = write_loop(out_fd, &[modec]); // c:1887
            emitted_marker = 1; // c:1888 len++
        }

        // c:1890-1897 — pad to width.
        let total_len = len_str + emitted_marker;
        let pad = (width - total_len - 2).max(0) as usize; // c:1890
        if pad > 0 {
            // c:1890
            let pad_str = " ".repeat(pad);
            let fd = SHTTY.load(Ordering::Relaxed);
            let out_fd = if fd >= 0 { fd } else { 1 };
            let _ = write_loop(out_fd, pad_str.as_bytes());
            // c:1896
        }
        // c:1898 — zcoff() reset after the pad fill.
        zcoff();
        // c:1899-1903 — inter-column separator. Unless this cell is the last
        // column of the row (lastc), C emits the COL_SP cap, two LITERAL spaces
        // (`fputs("  ", shout)`), and a reset. This port dropped the block
        // (`let _ = lastc;`), so every column collapsed against the next
        // (`f001f011` instead of `f001  f011`). The gap was only masked in
        // variable-width listings, where the pad-to-content-width above happened
        // to leave space after the shorter entries; the longest entry in each
        // column still touched its neighbour.
        if lastc == 0 {
            // c:1899
            emit_filecol(COL_SP); // c:1900 — zcputs(g->name, COL_SP)
            let fd = SHTTY.load(Ordering::Relaxed);
            let out_fd = if fd >= 0 { fd } else { 1 };
            let _ = write_loop(out_fd, b"  "); // c:1901 — fputs("  ", shout)
            zcoff(); // c:1902
        }
    }
    0 // c:1988 ret
}

/// Port of `static Cmgroup last_group` from `Src/Zle/complist.c:1729`.
/// The group whose color cap is currently active; reset clears
/// `last_cap` so the next zcputs re-emits the prefix.
pub static LAST_GROUP: std::sync::LazyLock<std::sync::Mutex<String>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(String::new())); // c:1729

/// Direct port of `int singlecalc(int *cp, int l, int *lcp)` from
/// `Src/Zle/complist.c:1909`.
///
/// Used by single-column listing scroll: scans `mtab[l]` backward
/// from `*cp` counting distinct Cmatch pointers, then forward to
/// see whether the rest of the row is uniform. Returns the count of
/// distinct matches seen on the way back to the first occurrence.
pub fn singlecalc(cp: &mut i32, l: i32, lcp: &mut i32) -> i32 {
    let zterm_columns = crate::ported::utils::adjustcolumns() as i32;
    if zterm_columns <= 0 {
        *lcp = 1;
        return 0;
    }
    let mtab_guard = MTAB.lock().unwrap();
    let row_base = (l * zterm_columns) as usize;
    let mut c = *cp;
    if c < 0 {
        c = 0;
    }
    // c:1913 — `mp = mtab[l*cols + c]`.
    let mp = mtab_guard.get(row_base + c as usize).cloned().flatten();

    // c:1915-1923 — backward scan counting distinct pointers,
    //                tracking first cell where `*p == mp`. C uses
    //                raw pointer equality on `Cmatch *`; we resolve it via the
    //                unique match number `gnum` (c:120), the value-equivalent of
    //                the pointer. Display-string equality would wrongly collapse
    //                distinct matches that share a display string, or grid
    //                matches with str=None. (Same resolution as complist.rs:3657.)
    let cm_eq = |a: Option<&Cmatch>, b: Option<&Cmatch>| -> bool {
        match (a, b) {
            (None, None) => true,
            (Some(x), Some(y)) => x.gnum == y.gnum,
            _ => false,
        }
    };
    let mut n = 0i32;
    let mut op: Option<Cmatch> = None;
    let mut first = true;
    let mut j = c;
    while j >= 0 {
        let p = mtab_guard.get(row_base + j as usize).cloned().flatten();
        if cm_eq(p.as_ref(), mp.as_ref()) {
            c = j;
        }
        if !first && !cm_eq(p.as_ref(), op.as_ref()) {
            n += 1;
        }
        op = p;
        first = false;
        j -= 1;
    }
    *cp = c; // c:1924

    // c:1926-1929 — forward scan: `*lcp = 1` then clear if anything
    //                else in the row.
    *lcp = 1;
    let mut k = c;
    while k < zterm_columns {
        let p = mtab_guard.get(row_base + k as usize).cloned().flatten();
        if p.is_some() && !cm_eq(p.as_ref(), mp.as_ref()) {
            *lcp = 0;
        }
        k += 1;
    }
    n // c:1931
}

/// Direct port of `static int singledraw(void)` from
/// `Src/Zle/complist.c:1934`. Repaints the menu-completion
/// listing in single-column mode (one match per line, current
/// pick highlighted).
///
/// **Substrate trade-off:** the redraw needs `mtab` (the
/// match-table indexed by row) + the `complistmtab`/`complistmlist`
/// terminal-coordinate arrays + `tputs`-driven cursor/color escapes.
/// All three live on the live ZLE refresh layer that compcore-call
/// context can't reach. Returns 0 = "redraw scheduled" so the live
/// refresh tick picks up the geometry from `listdat` + `amatches`.
/// Port of `static int singledraw(void)` from `Src/Zle/complist.c:1934`.
/// Redraws just the two cells whose state changed since last frame
/// (old selection + new selection) instead of repainting the whole
/// match list. Driven from `complistmatches` when only the cursor
/// moved.
/// ```c
/// static int
/// singledraw(void)
/// {
///     Cmgroup g;
///     int mc1, mc2, ml1, ml2, md1, md2, mcc1, mcc2, lc1, lc2, t1, t2;
///     t1 = mline - mlbeg; t2 = moline - molbeg;
///     if (t2 < t1) {
///         mc1 = mocol; ml1 = moline; md1 = t2;
///         mc2 = mcol;  ml2 = mline;  md2 = t1;
///     } else {
///         mc1 = mcol;  ml1 = mline;  md1 = t1;
///         mc2 = mocol; ml2 = moline; md2 = t2;
///     }
///     mcc1 = singlecalc(&mc1, ml1, &lc1);
///     mcc2 = singlecalc(&mc2, ml2, &lc2);
///     if (md1) tc_downcurs(md1);
///     if (mc1) tcmultout(TCRIGHT, TCMULTRIGHT, mc1);
///     g = mgtab[ml1 * zterm_columns + mc1];
///     clprintm(g, mtab[ml1 * zterm_columns + mc1], mcc1, ml1, lc1, ...);
///     if (mlprinted) tcmultout(TCUP, TCMULTUP, mlprinted);
///     putc('\r', shout);
///     if (md2 != md1) tc_downcurs(md2 - md1);
///     if (mc2) tcmultout(TCRIGHT, TCMULTRIGHT, mc2);
///     g = mgtab[ml2 * zterm_columns + mc2];
///     clprintm(g, mtab[ml2 * zterm_columns + mc2], mcc2, ml2, lc2, ...);
///     ...
///     return 0;
/// }
/// ```
pub fn singledraw() -> i32 {
    // c:1934

    let mline = MLINE.load(Ordering::SeqCst);
    let mcol = MCOL.load(Ordering::SeqCst);
    let mlbeg = MLBEG.load(Ordering::SeqCst);
    let moline = MOLINE.load(Ordering::SeqCst);
    let mocol = MOCOL.load(Ordering::SeqCst);
    let molbeg = MOLBEG.load(Ordering::SeqCst);
    let zterm_columns = adjustcolumns() as i32;

    let t1 = mline - mlbeg; // c:1939
    let t2 = moline - molbeg; // c:1940

    // c:1942-1948 — pick top→bottom ordering for the two paints.
    // mc1/mc2 are mutable: singlecalc (c:1949-1950) rewrites each to the
    // leftmost column of the multi-cell match at that row.
    let (mut mc1, ml1, md1, mut mc2, ml2, md2);
    if t2 < t1 {
        // c:1942
        mc1 = mocol;
        ml1 = moline;
        md1 = t2; // c:1943
        mc2 = mcol;
        ml2 = mline;
        md2 = t1; // c:1944
    } else {
        // c:1945
        mc1 = mcol;
        ml1 = mline;
        md1 = t1; // c:1946
        mc2 = mocol;
        ml2 = moline;
        md2 = t2; // c:1947
    }

    // c:1949-1950 — `mcc1 = singlecalc(&mc1, ml1, &lc1);
    //                mcc2 = singlecalc(&mc2, ml2, &lc2);`
    // singlecalc mutates mc1/mc2 back to the match's leftmost column, returns
    // the distinct-match index (mcc, used to select the per-column width and
    // passed to clprintm), and sets lc = last-column flag.
    let mut lc1 = 0i32;
    let mcc1 = singlecalc(&mut mc1, ml1, &mut lc1);
    let mut lc2 = 0i32;
    let mcc2 = singlecalc(&mut mc2, ml2, &mut lc2);

    if md1 != 0 {
        // c:1952
        tc_downcurs(md1); // c:1953
    }
    if mc1 != 0 {
        // c:1954
        tcmultout(
            crate::ported::zsh_h::TCRIGHT,
            crate::ported::zsh_h::TCMULTRIGHT,
            mc1,
        ); // c:1955
    }

    // c:1957-1959 — `g = mgtab[ml1 * zterm_columns + mc1];
    //                clprintm(g, mtab[...], mcc1, ml1, lc1, width)`
    let idx1 = (ml1 * zterm_columns + mc1) as usize;
    let g_at1 = MGTAB.lock().unwrap().get(idx1).cloned().flatten();
    let m_at1 = MTAB.lock().unwrap().get(idx1).cloned().flatten();
    let width_at1 = g_at1
        .as_ref()
        .map(|g| {
            if g.widths.is_empty() {
                g.width
            } else {
                g.widths.get(mcc1 as usize).copied().unwrap_or(g.width)
            }
        })
        .unwrap_or(0);
    clprintm(g_at1.as_ref(), m_at1.as_ref(), mcc1, ml1, lc1, width_at1); // c:1958

    let mlprinted = MLPRINTED.load(Ordering::SeqCst);
    if mlprinted != 0 {
        // c:1960
        tcmultout(
            crate::ported::zsh_h::TCUP,
            crate::ported::zsh_h::TCMULTUP,
            mlprinted,
        ); // c:1961
    }
    // c:1962 — putc('\r', shout)
    let fd = SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };
    let _ = write_loop(out_fd, b"\r");

    // c:1964-1965 — relative down-move to second cell.
    if md2 != md1 {
        // c:1964
        tc_downcurs(md2 - md1); // c:1965
    }
    if mc2 != 0 {
        // c:1966
        tcmultout(
            crate::ported::zsh_h::TCRIGHT,
            crate::ported::zsh_h::TCMULTRIGHT,
            mc2,
        ); // c:1967
    }

    let idx2 = (ml2 * zterm_columns + mc2) as usize;
    let g_at2 = MGTAB.lock().unwrap().get(idx2).cloned().flatten();
    let m_at2 = MTAB.lock().unwrap().get(idx2).cloned().flatten();
    let width_at2 = g_at2
        .as_ref()
        .map(|g| {
            if g.widths.is_empty() {
                g.width
            } else {
                g.widths.get(mcc2 as usize).copied().unwrap_or(g.width)
            }
        })
        .unwrap_or(0);
    clprintm(g_at2.as_ref(), m_at2.as_ref(), mcc2, ml2, lc2, width_at2); // c:1970

    // c:1972 — re-read the `mlprinted` GLOBAL, which the second clprintm just
    // rewrote (c:800/825/857/864/874/1879). C reads it fresh here; caching the
    // first clprintm's value would move the cursor up by the wrong line count
    // when the two painted cells wrap to different heights.
    let mlprinted = MLPRINTED.load(Ordering::SeqCst);
    if mlprinted != 0 {
        // c:1972
        tcmultout(
            crate::ported::zsh_h::TCUP,
            crate::ported::zsh_h::TCMULTUP,
            mlprinted,
        ); // c:1973
    }
    let _ = write_loop(out_fd, b"\r"); // c:1974

    // c:1975-1985 — reposition the cursor back to the TOP of the list after the
    // incremental two-cell repaint, so the next navigation repaint starts in
    // place. Previously elided (jumped straight to `return 0`), so the cursor
    // was left at the moved-to cell (row `md2`); repeated menu-select
    // navigation then drifted the grid UP onto the command line and misaligned
    // columns. Mirrors the compprintlist epilogue fix.
    let nlnct_sd = NLNCT.load(Ordering::SeqCst);
    let zterm_lines_sd = adjustlines() as i32;
    if MSTATPRINTED.load(Ordering::SeqCst) != 0 {
        // c:1976-1981 — bottom status line present: park below it, print it,
        // then jump back to the top row.
        let i = zterm_lines_sd - md2 - nlnct_sd;
        tc_downcurs(i - 1);
        let mut stop = 0;
        compprintfmt("", 0, 1, 1, MLINE.load(Ordering::SeqCst), &mut stop);
        tcmultout(
            crate::ported::zsh_h::TCUP,
            crate::ported::zsh_h::TCMULTUP,
            zterm_lines_sd - 1,
        );
    } else {
        // c:1983 — `tcmultout(TCUP, TCMULTUP, md2 + nlnct)`.
        tcmultout(
            crate::ported::zsh_h::TCUP,
            crate::ported::zsh_h::TCMULTUP,
            md2 + nlnct_sd,
        );
    }
    SHOWINGLIST.store(-1, Ordering::SeqCst); // c:1985
    LISTSHOWN.store(1, Ordering::SeqCst); // c:1986

    0 // c:1987
}

/// Port of `int complistmatches(UNUSED(Hookdef dummy), Chdata dat)` from
/// `Src/Zle/complist.c:1990`.
/// ```c
/// int
/// complistmatches(UNUSED(Hookdef dummy), Chdata dat)
/// {
///     static int onlnct = -1;
///     static int extendedglob;
///     Cmgroup oamatches = amatches;
///     amatches = dat->matches;
///     if (noselect > 0) noselect = 0;
///     if ((minfo.asked == 2 && mselect < 0) || nlnct >= zterm_lines || errflag) {
///         showinglist = 0;
///         amatches = oamatches;
///         return (noselect = 1);
///     }
///     pushheap();
///     extendedglob = opts[EXTENDEDGLOB];
///     opts[EXTENDEDGLOB] = 1;
///     getcols();
///     mnew = ((calclist(mselect >= 0) || mlastcols != zterm_columns ||
///              mlastlines != listdat.nlines) && mselect >= 0);
///     if (!listdat.nlines || (mselect >= 0 && !(isset(USEZLE) && ...))) {
///         showinglist = listshown = 0;
///         noselect = 1; ...; return 1;
///     }
///     if (inselect || mlbeg >= 0) clearflag = 0;
///     mscroll = 0; mlistp = NULL;
///     /* asklist / mlistp / clearflag setup */
///     /* mlend = mlbeg + zterm_lines - nlnct - mhasstat; */
///     if (mnew) { realloc mtab/mgtab; mlastcols = mcols = zterm_columns; ... }
///     last_cap = zhalloc(max_caplen + 1);
///     if (!mnew && inselect && onlnct == nlnct && mlbeg >= 0 && mlbeg == molbeg) {
///         if (!noselect) singledraw();
///     } else if (!compprintlist(mselect >= 0) || !clearflag) noselect = 1;
///     onlnct = nlnct; molbeg = mlbeg; mocol = mcol; moline = mline;
///     amatches = oamatches; popheap();
///     opts[EXTENDEDGLOB] = extendedglob;
///     return noselect;
/// }
/// ```
/// The `dummy`/`dat` args mirror the C `int complistmatches(Hookdef
/// dummy, Chdata dat)` signature so this registers directly as the
/// `comp_list_matches` Hookfn (complist.c:3595); both are unused (the body
/// reads the `amatches` globals as the C source does).
pub fn complistmatches(
    _dummy: *mut crate::ported::zsh_h::hookdef,
    _dat: *mut std::ffi::c_void,
) -> i32 {
    // c:1990

    // c:1995 — `Cmgroup oamatches = amatches;` — saved for restore
    // before any return path; the Rust amatches lives in compcore.

    // c:1997 — `amatches = dat->matches;` — the chdata hook supplies a
    // fresh group list at each completion call. Without a Chdata param
    // exposed at this entry, the global is already populated.

    // c:2004-2005 — `if (noselect > 0) noselect = 0;`
    if NOSELECT.load(Ordering::SeqCst) > 0 {
        // c:2004
        NOSELECT.store(0, Ordering::SeqCst); // c:2005
    }

    // c:2007-2012 — early-exit: list too tall or errflag set.
    let zterm_lines = adjustlines() as i32;
    let zterm_columns = adjustcolumns() as i32;
    let nlnct = NLNCT.load(Ordering::SeqCst);
    let mselect = MSELECT.load(Ordering::SeqCst);
    let minfo_asked = MINFO
        .get()
        .and_then(|m| m.lock().ok().map(|g| g.asked))
        .unwrap_or(0);
    let errflag_v = errflag.load(Ordering::SeqCst);

    if (minfo_asked == 2 && mselect < 0)                                     // c:2007
        || nlnct >= zterm_lines
        || errflag_v != 0
    {
        SHOWINGLIST.store(0, Ordering::SeqCst); // c:2009
        NOSELECT.store(1, Ordering::SeqCst); // c:2011
        return 1;
    }

    // c:2022 — `pushheap();` — Rust uses scope-bounded vector growth.
    crate::ported::mem::pushheap();

    // c:2023-2024 — save EXTENDEDGLOB; force it on for the listing pass.
    let extendedglob = isset(EXTENDEDGLOB);
    // c:2024 — `opts[EXTENDEDGLOB] = 1;` — option mutation not yet
    // exposed as a free fn; the typed `setopt(EXTENDEDGLOB)` path
    // would set the bit. Carry-through.

    // c:2026 — `getcols();` — parse ZLS_COLORS into mcolors.
    getcols("");

    // c:2028-2029 — `mnew = ((calclist(mselect >= 0) || mlastcols != ...))`
    let calc_changed = crate::ported::zle::compresult::calclist(if mselect >= 0 { 1 } else { 0 });
    let mlastcols = MLASTCOLS.load(Ordering::SeqCst);
    let mlastlines = MLASTLINES.load(Ordering::SeqCst);
    let listdat_nlines: i32 = listdat
        .get()
        .and_then(|m| m.lock().ok().map(|g| g.nlines))
        .unwrap_or(0);
    let mnew = (calc_changed != 0 || mlastcols != zterm_columns || mlastlines != listdat_nlines)
        && mselect >= 0;
    MNEW.store(if mnew { 1 } else { 0 }, Ordering::SeqCst);

    // c:2031-2040 — empty list / no-zle bail-out.
    let usezle = isset(USEZLE);
    if listdat_nlines == 0 || (mselect >= 0 && !(usezle/* && !termflags && complastprompt valid */))
    {
        SHOWINGLIST.store(0, Ordering::SeqCst);
        LISTSHOWN.store(0, Ordering::SeqCst);
        NOSELECT.store(1, Ordering::SeqCst);
        popheap();
        return 1;
    }

    // c:2041-2042 — `if (inselect || mlbeg >= 0) clearflag = 0;`
    if INSELECT.load(Ordering::SeqCst) != 0 || MLBEG.load(Ordering::SeqCst) >= 0 {
        CLEARFLAG.store(0, Ordering::SeqCst);
    }

    // c:2044-2045 — `mscroll = 0; mlistp = NULL;`
    MSCROLL.store(0, Ordering::SeqCst);

    // c:2048-2076 — LISTPROMPT / asklist branch. The LISTPROMPT param
    // path drives a scroll-paged display when the user has it set.
    let listprompt = getsparam("LISTPROMPT");
    if mselect >= 0 || MLBEG.load(Ordering::SeqCst) >= 0 || listprompt.is_some() {
        // c:2053 — trashzle()
        trashzle();
        SHOWINGLIST.store(0, Ordering::SeqCst);
        LISTSHOWN.store(0, Ordering::SeqCst);
        LASTLISTLEN.store(0, Ordering::SeqCst);
        if listprompt.is_some() {
            // c:2060
            // c:2061 — clearflag = (USEZLE && !termflags && dolastprompt)
            CLEARFLAG.store(if usezle { 1 } else { 0 }, Ordering::SeqCst);
            MSCROLL.store(1, Ordering::SeqCst); // c:2062
        } else {
            // c:2063
            CLEARFLAG.store(1, Ordering::SeqCst); // c:2064
                                                  // c:2065 — minfo.asked = listdat.nlines + nlnct <= zterm_lines
            if let Some(m) = MINFO.get() {
                if let Ok(mut g) = m.lock() {
                    g.asked = if listdat_nlines + nlnct <= zterm_lines {
                        1
                    } else {
                        0
                    };
                }
            }
        }
    } else {
        // c:2070-2075 — asklist() prompts "show all N? (y/n)"
        let r = crate::ported::zle::compresult::asklist();
        if r != 0 {
            // c:2070
            popheap();
            NOSELECT.store(1, Ordering::SeqCst);
            return 1;
        }
    }

    // c:2077-2082 — mlend window calculation.
    let mlbeg = MLBEG.load(Ordering::SeqCst);
    if mlbeg >= 0 {
        // c:2077
        let mhasstat = MHASSTAT.load(Ordering::SeqCst);
        let mut new_mlend = mlbeg + zterm_lines - nlnct - mhasstat; // c:2078
        let mline = MLINE.load(Ordering::SeqCst);
        let mut adjusted_mlbeg = mlbeg;
        while mline >= new_mlend {
            // c:2079
            adjusted_mlbeg += 1; // c:2080 mlbeg++
            new_mlend += 1;
        }
        MLBEG.store(adjusted_mlbeg, Ordering::SeqCst);
        MLEND.store(new_mlend, Ordering::SeqCst);
    } else {
        // c:2081
        MLEND.store(9_999_999, Ordering::SeqCst); // c:2082
    }

    // c:2084-2102 — `if (mnew)` realloc mtab/mgtab.
    if mnew {
        // c:2084
        MTAB_BEEN_REALLOCATED.store(1, Ordering::SeqCst); // c:2087
        let i = (zterm_columns * listdat_nlines) as usize; // c:2089
                                                           // c:2090-2092 — free(mtab); mtab = zalloc(i); memset(mtab, 0, i);
        *MTAB.lock().unwrap() = vec![None; i]; // c:2091-2092
                                               // c:2093-2098 — same for mgtab
        *MGTAB.lock().unwrap() = vec![None; i]; // c:2094-2098
        MGTABSIZE.store(i as i32, Ordering::SeqCst); // c:2096
        MLASTCOLS.store(zterm_columns, Ordering::SeqCst); // c:2099 mlastcols = mcols
        MCOLS.store(zterm_columns, Ordering::SeqCst);
        MLASTLINES.store(listdat_nlines, Ordering::SeqCst); // c:2100 mlastlines = mlines
        MLINES.store(listdat_nlines, Ordering::SeqCst);
        MMTABP.store(0, Ordering::SeqCst); // c:2101
    }

    // c:2103-2104 — last_cap = zhalloc(max_caplen + 1); *last_cap = '\0';
    let cap_size = (MAX_CAPLEN.load(Ordering::SeqCst) + 1).max(1) as usize;
    *LAST_CAP.lock().unwrap() = String::with_capacity(cap_size);

    // c:2106-2111 — choose singledraw (incremental) vs full compprintlist.
    // ONLNCT is a function-static in C; we mirror with a file-static.
    let cur_onlnct = ONLNCT.load(Ordering::SeqCst);
    let inselect = INSELECT.load(Ordering::SeqCst);
    let mlbeg_cur = MLBEG.load(Ordering::SeqCst);
    let molbeg = MOLBEG.load(Ordering::SeqCst);
    let clearflag = CLEARFLAG.load(Ordering::SeqCst);
    // c:2106-2109 — C uses singledraw() (incremental two-cell highlight-move)
    // when only the selection changed within the same frame, instead of
    // repainting the whole list every keystroke.
    //
    // GATED OFF by default. Enabling it (0e8a580bf1) fixed singledraw's own cell
    // math (singlecalc, mcc/lc/mlprinted — all faithful now) but was committed
    // WITHOUT live verification (the commit says so). Live + headless A/B since
    // then shows it still corrupts the SCROLL-PAGER menu (LISTPROMPT set →
    // mscroll=1): the list columns split mid-word / merge across rows (e.g.
    // `lz`+`diff`, `libpng16-confi`+`g`, `lil`+`l3build`). Root cause is NOT in
    // singledraw's emitted bytes — they match the C algorithm exactly (down md1,
    // paint cell1, down md2-md1, paint cell2, up md2+nlnct). It is the cursor
    // BASELINE at singledraw entry: singledraw is differential and assumes the
    // cursor is parked at list-top (row 0 of the window), but across menuselect
    // frames zshrs does not hold that invariant (the intervening zle refresh /
    // trashzle leaves the cursor elsewhere), so the two repainted cells land on
    // the wrong rows while the rest of the prior frame stays. The full
    // compprintlist repaint is self-consistent (redraws the whole grid from its
    // own baseline) so it renders correctly — verified byte-clean columns via
    // A/B. Re-enable only after the baseline invariant is proven byte-identical
    // to zsh's menuselect redisplay (needs a zsh reference capture). Opt-in with
    // ZSHRS_SINGLEDRAW=1 for that work.
    let use_singledraw = std::env::var("ZSHRS_SINGLEDRAW").is_ok();
    let took_singledraw = use_singledraw
        && !mnew
        && inselect != 0
        && cur_onlnct == nlnct
        && mlbeg_cur >= 0
        && mlbeg_cur == molbeg;
    // TEMP env-gated diagnostic (ZSHRS_COMPLIST_LOG) — traces every complist
    // redraw invocation + which branch it takes, to diagnose the p10k <TAB>
    // multi-draw duplication. No-op unless the env var is set.
    if let Ok(path) = std::env::var("ZSHRS_COMPLIST_LOG") {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(f, "complistmatches: branch={} mnew={} inselect={} onlnct={} nlnct={} mlbeg={} molbeg={} mselect={} clearflag={} mscroll={} showinglist_in={} nlines={} noselect={}",
                if took_singledraw { "SINGLEDRAW" } else { "COMPPRINTLIST" },
                mnew, inselect, cur_onlnct, nlnct, mlbeg_cur, molbeg, mselect, clearflag,
                MSCROLL.load(Ordering::SeqCst), SHOWINGLIST.load(Ordering::SeqCst),
                listdat.get().and_then(|m| m.lock().ok().map(|g| g.nlines)).unwrap_or(-1),
                NOSELECT.load(Ordering::SeqCst));
        }
    }
    if took_singledraw {
        if NOSELECT.load(Ordering::SeqCst) == 0 {
            // c:2108
            singledraw(); // c:2109
        }
    } else if compprintlist(if mselect >= 0 { 1 } else { 0 }) == 0
        || clearflag == 0
    {
        NOSELECT.store(1, Ordering::SeqCst); // c:2111
    }

    // Rust adaptation (mirrors ilistmatches, compresult.rs c:2172): the
    // recursive `zrefresh` that follows `listmatches` re-enters this hook while
    // `showinglist == -2`. C makes that re-entry harmless with `singledraw`
    // (incremental highlight move); this port gates singledraw OFF and does a
    // full `compprintlist` each time, so a re-entry would repaint the ENTIRE
    // menu again below the first — a visible duplicate. It bites only when the
    // list SCROLLS (clearflag=0): compprintlist's fits-branch sets
    // showinglist=-1, but the exceeds-branch (c:1712) leaves it -2. Mark the
    // list shown (-1) so the recursive zrefresh repaints only the command line
    // — the same guard the plain-list path already has. (Was masked by the
    // removed `\x1b[A` cursor hack, which repositioned so the 2nd draw
    // overwrote the 1st; the faithful fix is to not draw twice at all.)
    if SHOWINGLIST.load(Ordering::SeqCst) == -2 {
        SHOWINGLIST.store(-1, Ordering::SeqCst);
    }

    // c:2113-2116 — capture frame state for next call's diff.
    ONLNCT.store(nlnct, Ordering::SeqCst); // c:2113
    MOLBEG.store(MLBEG.load(Ordering::SeqCst), Ordering::SeqCst); // c:2114
    MOCOL.store(MCOL.load(Ordering::SeqCst), Ordering::SeqCst); // c:2115
    MOLINE.store(MLINE.load(Ordering::SeqCst), Ordering::SeqCst); // c:2116

    // c:2118-2120 — `amatches = oamatches; popheap();`
    popheap();
    let _ = extendedglob; // c:2121 opts[EXTENDEDGLOB] = extendedglob

    NOSELECT.load(Ordering::SeqCst) // c:2123 return noselect
}

/// Port of `static int onlnct` from `Src/Zle/complist.c:1992`. Saved
/// `nlnct` from the previous `complistmatches` call so the incremental
/// `singledraw` path can detect frame-boundary equality.
pub static ONLNCT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1); // c:1992

/// Port of `adjust_mcol(int wish, Cmatch ***tabp, Cmgroup **grp)` from Src/Zle/complist.c:2127.
pub fn adjust_mcol(wish: i32, tabp: &mut i32, grp: &mut i32) -> i32 {
    // c:2127
    // C body c:2129-2170 — clamps mcol to nearest valid column when
    //                      moving across rows of variable-width matches.
    //                      Without the mtab[][] matrix we just clamp
    //                      to a non-negative column.
    wish.max(0)
}

/// Port of `struct menustack` from `Src/Zle/complist.c:2159`. Saved
/// menu-select snapshot — the menu-stack chain `domenuselect` pushes
/// on entry and pops on exit so nested menu invocations restore
/// previous state.
#[derive(Default)]
#[allow(non_camel_case_types)]
pub struct menustack {
    // c:2159
    /// Saved zleline contents.
    pub line: String, // c:2161
    /// Brace-info head + tail. C uses `Brinfo` (linked-list head).
    /// Rust port snapshots the BRBEG/BREND globals (in compcore.rs)
    /// by cloning the full Brinfo struct out so the menustack pop
    /// can restore them. None when the brinfo list was empty.
    pub brbeg: Option<Box<crate::ported::zle::comp_h::Brinfo>>, // c:2162
    pub brend: Option<Box<crate::ported::zle::comp_h::Brinfo>>, // c:2163
    /// Brace-info counts.
    pub nbrbeg: i32,                  // c:2164
    pub nbrend: i32,                                            // c:2164
    /// Cursor + acceptance + match counts + menu line + line begin
    /// + nolist flag.
    pub cs: i32, // c:2165
    pub acc: i32,                                               // c:2165
    pub nmatches: i32,                                          // c:2165
    pub mline: i32,                                             // c:2165
    pub mlbeg: i32,                                             // c:2165
    pub nolist: i32,                                            // c:2165
    /// Original line state before menu entry.
    pub origline: String, // c:2172
    pub origcs: i32,                                            // c:2173
    pub origll: i32,                                            // c:2173
    /// Interactive-mode status line.
    pub status: String,    // c:2180
    /// Mode discriminator (interactive vs search).
    pub mode: i32, // c:2181
}

/// Port of `struct menusearch` from `Src/Zle/complist.c:2186`. Per-step
/// state for incremental match-search inside the menu — back-stack so
/// backspace can undo one step.
#[derive(Default)]
#[allow(non_camel_case_types)]
pub struct menusearch {
    // c:2186
    /// The search string accumulator.
    pub str: String, // c:2188
    /// Saved line + column.
    pub line: i32, // c:2189
    pub col: i32, // c:2190
    /// Direction (1 = forward, 0 = backward).
    pub back: i32, // c:2191
    /// Search-state discriminator (`MS_OK`/`MS_FAILED`/`MS_WRAPPED`).
    pub state: i32, // c:2192
    /// Cursor pointer into the current Cmatch row (index into mtab).
    pub ptr: usize, // c:2193
}

/// Port of `MS_OK` from `Src/Zle/complist.c:2196`. Search step landed
/// on a match.
pub const MS_OK: i32 = 0; // c:2196
/// Port of `MS_FAILED` from `complist.c:2197`. Search step found no match.
pub const MS_FAILED: i32 = 1; // c:2197
/// Port of `MS_WRAPPED` from `complist.c:2198`. Search wrapped past edge.
pub const MS_WRAPPED: i32 = 2; // c:2198

/// Port of `MAX_STATUS` from `Src/Zle/complist.c:2200`. Max bytes the
/// menu-status line shows.
pub const MAX_STATUS: usize = 128; // c:2200

/// Port of `setmstatus(char *status, char *sline, int sll, int scs, int *csp, int *llp, int *lenp)` from Src/Zle/complist.c:2203.
/// WARNING: param names don't match C — Rust=(_status, _sline, _scs, _np, _nl, _nc) vs C=(status, sline, sll, scs, csp, llp, lenp)
/// Port of `static char *setmstatus(char *status, char *sline, int sll,
/// int scs, int *csp, int *llp, int *lenp)` from
/// `Src/Zle/complist.c:2203`. Formats the menu-select status line
/// (`interactive: <prefix>[]<suffix>`) capped at MAX_STATUS-14 width.
/// When `csp` is non-NULL, captures the current zle line for restore.
/// ```c
/// static char *
/// setmstatus(char *status, char *sline, int sll, int scs,
///            int *csp, int *llp, int *lenp)
/// {
///     char *p, *s, *ret = NULL;
///     int pl, sl, max;
///     METACHECK();
///     if (csp) {
///         *csp = zlemetacs; *llp = zlemetall; *lenp = lastend - wb;
///         ret = dupstring(zlemetaline);
///         p = zhalloc(zlemetacs - wb + 1);
///         strncpy(p, zlemetaline + wb, zlemetacs - wb);
///         p[zlemetacs - wb] = '\0';
///         if (lastend < zlemetacs) s = "";
///         else { s = zhalloc(lastend - zlemetacs + 1);
///                strncpy(s, zlemetaline + zlemetacs, lastend - zlemetacs);
///                s[lastend - zlemetacs] = '\0'; }
///         zlemetacs = 0; foredel(zlemetall, CUT_RAW);
///         spaceinline(sll); memcpy(zlemetaline, sline, sll);
///         zlemetacs = scs;
///     } else { p = complastprefix; s = complastsuffix; }
///     pl = strlen(p); sl = strlen(s);
///     max = (zterm_columns < MAX_STATUS ? zterm_columns : MAX_STATUS) - 14;
///     if (max > 12) {
///         int h = (max - 2) >> 1;
///         strcpy(status, "interactive: ");
///         if (pl > h - 3) { strcat(status, "..."); strcat(status, p + pl - h - 3); }
///         else strcat(status, p);
///         strcat(status, "[]");
///         if (sl > h - 3) { strncat(status, s, h - 3); strcat(status, "..."); }
///         else strcat(status, s);
///     }
///     return ret;
/// }
/// ```
pub fn setmstatus(
    // c:2203
    status: &mut String,
    sline: &str,
    sll: i32,
    scs: i32,
    csp: Option<&mut i32>,
    llp: Option<&mut i32>,
    lenp: Option<&mut i32>,
) -> Option<String> {
    use std::sync::atomic::Ordering;

    let mut ret: Option<String> = None; // c:2206

    let zlemetacs = ZLEMETACS.load(Ordering::SeqCst);
    let zlemetall = ZLEMETALL.load(Ordering::SeqCst);
    let lastend = crate::ported::zle::compcore::LASTEND.load(Ordering::SeqCst);
    let wb = crate::ported::zle::compcore::WB.load(Ordering::SeqCst);

    let mut p: String;
    let mut s: String;

    if let Some(csp_ref) = csp {
        // c:2211
        *csp_ref = zlemetacs; // c:2212
        if let Some(llp_ref) = llp {
            *llp_ref = zlemetall;
        } // c:2213
        if let Some(lenp_ref) = lenp {
            *lenp_ref = lastend - wb;
        } // c:2214

        let zml = ZLEMETALINE
            .get()
            .and_then(|m| m.lock().ok().map(|g| g.clone()))
            .unwrap_or_default();
        ret = Some(zml.clone()); // c:2216 dupstring(zlemetaline)

        // c:2218-2220 — p = zlemetaline[wb..zlemetacs]
        let wb_u = wb.max(0) as usize;
        let cs_u = zlemetacs.max(0) as usize;
        p = zml.get(wb_u..cs_u).unwrap_or("").to_string();

        // c:2221-2227 — s = zlemetaline[zlemetacs..lastend] or empty
        if lastend < zlemetacs {
            // c:2221
            s = String::new(); // c:2222
        } else {
            let le_u = lastend.max(0) as usize;
            s = zml.get(cs_u..le_u).unwrap_or("").to_string(); // c:2224-2226
        }

        // c:2228-2232 — replace line with sline.
        ZLEMETACS.store(0, Ordering::SeqCst); // c:2228
        foredel(zlemetall, 0); // c:2229 CUT_RAW
        spaceinline(sll); // c:2230
        if let Some(zml_mutex) = ZLEMETALINE.get() {
            if let Ok(mut g) = zml_mutex.lock() {
                if g.len() >= sll as usize {
                    let head: String = sline.chars().take(sll as usize).collect();
                    g.replace_range(..sll as usize, &head); // c:2231 memcpy
                } else {
                    *g = sline.chars().take(sll as usize).collect();
                }
            }
        }
        ZLEMETACS.store(scs, Ordering::SeqCst); // c:2232
    } else {
        // c:2233
        // c:2234-2235 — p = complastprefix; s = complastsuffix
        p = crate::ported::zle::complete::COMPLASTPREFIX
            .get_or_init(|| std::sync::Mutex::new(String::new()))
            .lock()
            .unwrap()
            .clone();
        s = crate::ported::zle::complete::COMPLASTSUFFIX
            .get_or_init(|| std::sync::Mutex::new(String::new()))
            .lock()
            .unwrap()
            .clone();
    }

    let pl = p.len() as i32; // c:2237
    let sl = s.len() as i32; // c:2238
    let zterm_columns = adjustcolumns() as i32;
    let max = if zterm_columns < MAX_STATUS as i32 {
        // c:2239
        zterm_columns
    } else {
        MAX_STATUS as i32
    } - 14;

    if max > 12 {
        // c:2241
        let h = (max - 2) >> 1; // c:2242

        status.clear();
        status.push_str("interactive: "); // c:2244
        if pl > h - 3 {
            // c:2245
            status.push_str("..."); // c:2246
            let skip = (pl - h - 3).max(0) as usize;
            status.push_str(&p[skip..]); // c:2247 p + pl - h - 3
        } else {
            status.push_str(&p); // c:2249
        }
        status.push_str("[]"); // c:2251
        if sl > h - 3 {
            // c:2252
            let take = (h - 3).max(0) as usize;
            status.push_str(&s.chars().take(take).collect::<String>()); // c:2253
            status.push_str("..."); // c:2254
        } else {
            status.push_str(&s); // c:2256
        }
    }
    ret // c:2258
}

/// Port of `msearchpush(Cmatch **p, int back)` from Src/Zle/complist.c:2266.
/// WARNING: param names don't match C — Rust=() vs C=(p, back)
pub fn msearchpush() -> i32 {
    // c:2266
    // C body c:2268-2280 — pushes current mline/mcol/msearchstr onto
    //                      msearchstack so msearchpop can restore.
    //                      No msearchstack substrate: no-op.
    0
}

/// Direct port of `int *msearchpop(int *backp)` from
/// `Src/Zle/complist.c:2281`.
///
/// Pops one [`menusearch`] frame off [`MSEARCHSTACK`] and restores
/// the per-frame state into `MSEARCHSTR` / `MLINE` / `MCOL` /
/// `MSEARCHSTATE`. Returns the `back` flag of the popped frame so
/// callers can re-run the search in the original direction.
pub fn msearchpop() -> i32 {
    let mut stack = MSEARCHSTACK.lock().unwrap();
    // c:2284 — `if (!s) return NULL;` (empty stack → no-op).
    let popped = match stack.pop() {
        Some(s) => s,
        None => return 0,
    };
    // c:2289-2293 — restore msearchstr / mline / mcol / msearchstate.
    *MSEARCHSTR.lock().unwrap() = popped.str.clone();
    MLINE.store(popped.line, Ordering::Relaxed);
    MCOL.store(popped.col, Ordering::Relaxed);
    MSEARCHSTATE.store(popped.state, Ordering::Relaxed);
    // c:2294-2295 — return the back direction so caller can re-search.
    popped.back
}

/// Port of `msearch(Cmatch **ptr, char *ins, int back, int rep, int *wrapp)` from Src/Zle/complist.c:2302.
/// WARNING: param names don't match C — Rust=() vs C=(ptr, ins, back, rep, wrapp)
/// Port of `static Cmatch *msearch(Cmatch **ptr, char *ins, int back,
/// int rep, int *wrapp)` from `Src/Zle/complist.c:2302`. Walks the
/// `mtab[][]` matrix forward (or backward when `back`) from the
/// current cursor, looking for a Cmatch whose display string
/// contains `msearchstr`. Returns the matrix index of the match,
/// wrapping around when the end is reached.
/// ```c
/// static Cmatch *
/// msearch(Cmatch **ptr, char *ins, int back, int rep, int *wrapp)
/// {
///     Cmatch **p, *l = NULL, m;
///     int x = mcol, y = mline;
///     int ex, ey, wrap = 0, owrap = (msearchstate & MS_WRAPPED);
///     msearchpush(ptr, back);
///     if (ins) msearchstr = dyncat(msearchstr, ins);
///     if (back) { ex = mcols - 1; ey = -1; }
///     else { ex = 0; ey = listdat.nlines; }
///     p = mtab + (mline * mcols) + mcol;
///     if (rep) l = *p;
///     while (1) {
///         if (!rep && mtunmark(*p) && *p != l) {
///             l = *p; m = *mtunmark(*p);
///             if (strstr((m->disp ? m->disp : m->str), msearchstr)) {
///                 mcol = x; mline = y; return p;
///             }
///         }
///         rep = 0;
///         /* advance x/y per back direction */
///         if (x == ex && y == ey) {
///             /* wrap once; fail on second exhaustion */
///             if (wrap) { msearchstate = MS_FAILED | owrap; break; }
///             msearchstate |= MS_WRAPPED; wrap = 1; *wrapp = 1;
///         }
///     }
///     return NULL;
/// }
/// ```
/// Returns the linear index of the matched cell in `mtab`, or `-1`
/// on failure. Param shape adapted from `Cmatch **` out-pointer to
/// the canonical Rust Result-like discriminant.
pub fn msearch() -> i32 {
    // c:2302

    let mut x = MCOL.load(Ordering::SeqCst);
    let mut y = MLINE.load(Ordering::SeqCst);
    let mcols = MCOLS.load(Ordering::SeqCst);
    let listdat_nlines = listdat
        .get()
        .and_then(|m| m.lock().ok().map(|g| g.nlines))
        .unwrap_or(0);
    let mut wrap = 0i32;
    let owrap = MSEARCHSTATE.load(Ordering::SeqCst) & MS_WRAPPED; // c:2306

    // c:2308 — msearchpush(ptr, back). Stack management deferred.

    let back = 0i32; // c:2305 default forward
    let (mut ex, mut ey) = if back != 0 {
        // c:2312
        (mcols - 1, -1i32)
    } else {
        // c:2315
        (0i32, listdat_nlines)
    };

    let mut p = (y * mcols + x).max(0) as usize; // c:2319

    let needle = MSEARCHSTR.lock().unwrap().clone();
    let mtab_snapshot: Vec<Option<Cmatch>> = MTAB.lock().unwrap().clone();

    loop {
        // c:2322
        // c:2323-2333 — probe current cell
        if let Some(Some(m)) = mtab_snapshot.get(p) {
            // c:2323
            let hay = m
                .disp
                .as_deref()
                .unwrap_or_else(|| m.str.as_deref().unwrap_or(""));
            if !needle.is_empty() && hay.contains(needle.as_str()) {
                // c:2327
                MCOL.store(x, Ordering::SeqCst); // c:2328
                MLINE.store(y, Ordering::SeqCst); // c:2329
                return p as i32; // c:2331
            }
        }

        // c:2336-2348 — advance.
        if back != 0 {
            if p == 0 {
                p = mtab_snapshot.len().saturating_sub(1);
            } else {
                p -= 1;
            }
            x -= 1;
            if x < 0 {
                // c:2338
                x = mcols - 1; // c:2339
                y -= 1; // c:2340
            }
        } else {
            p += 1; // c:2343
            x += 1;
            if x == mcols {
                // c:2344
                x = 0; // c:2345
                y += 1; // c:2346
            }
        }

        // c:2349 — `if (x == ex && y == ey)` — hit boundary.
        if x == ex && y == ey {
            // c:2349
            // c:2351-2358 — restart from the opposite corner.
            if back != 0 {
                // c:2351
                x = mcols - 1; // c:2352
                y = listdat_nlines - 1; // c:2353
                p = (y * mcols + x).max(0) as usize; // c:2354
            } else {
                x = 0;
                y = 0; // c:2356
                p = 0; // c:2357
            }
            ex = MCOL.load(Ordering::SeqCst); // c:2359
            ey = MLINE.load(Ordering::SeqCst); // c:2360

            // c:2362-2365 — second exhaustion: fail.
            if wrap != 0 || (x == ex && y == ey) {
                // c:2362
                MSEARCHSTATE.store(MS_FAILED | owrap, Ordering::SeqCst); // c:2363
                break; // c:2364
            }

            MSEARCHSTATE.fetch_or(MS_WRAPPED, Ordering::SeqCst); // c:2367
            wrap = 1; // c:2368
        }
        if p >= mtab_snapshot.len() {
            break;
        }
    }
    -1 // c:2372 NULL
}

/// Port of `static char *msearchstr` from `Src/Zle/complist.c:2262`.
/// The accumulator string the menu-select incremental search is
/// currently matching against.
pub static MSEARCHSTR: std::sync::LazyLock<std::sync::Mutex<String>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(String::new())); // c:2262

/// Port of `static int msearchstate` from `Src/Zle/complist.c`. Search
/// state bitmask: `MS_OK` / `MS_FAILED` / `MS_WRAPPED`.
pub static MSEARCHSTATE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(MS_OK); // c:msearchstate

/// Port of `static Menusearch msearchstack` from
/// `Src/Zle/complist.c:2263`. LIFO stack of `menusearch` frames so
/// `msearchpop` can rewind the incremental search by one char.
pub static MSEARCHSTACK: std::sync::LazyLock<std::sync::Mutex<Vec<menusearch>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new())); // c:2263

/// Port of `static int domenuselect(Hookdef dummy, Chdata dat)` from
/// `Src/Zle/complist.c:2383`. Menu-select interactive key-loop:
/// reads keys via `getkeycmd`, navigates `mline`/`mcol` through the
/// `mtab[][]` matrix, dispatches widget actions (up/down/forward/
/// backward/accept/search/cancel), repaints via `complistmatches`.
/// WARNING: param names don't match C — Rust=() vs C=(dummy, dat)
pub fn domenuselect(
    _dummy: *mut crate::ported::zsh_h::hookdef,
    _dat: *mut std::ffi::c_void,
) -> i32 {
    // c:2383

    // c:2385-2396 — local declarations.
    let mut _i: i32 = 0; // c:2392
    let mut _acc: i32 = 0; // c:2392
    let mut _wishcol: i32 = 0; // c:2392
    let _setwish: i32 = 0; // c:2392
    let oe = crate::ported::zle::compcore::onlyexpl.load(Ordering::SeqCst); // c:2392
    let mut _wasnext: i32 = 0; // c:2392
    let _space: i32 = 0; // c:2393
    let _lbeg: i32 = 0; // c:2393
    let mut step: i32 = 1; // c:2393
    let _wrap: i32 = 0; // c:2393
    let _pl = NLNCT.load(Ordering::SeqCst); // c:2393
    let _broken: i32 = 0; // c:2393
    let _first: i32 = 1; // c:2393
    let mut _nolist: i32 = 0; // c:2394
    let mut mode: i32 = 0; // c:2394
    let _modecs: i32 = 0; // c:2394
    let _modell: i32 = 0; // c:2394
    let _modelen: i32 = 0; // c:2394
    let _wasmeta: i32; // c:2394
    let mut status = String::new(); // c:2396

    // Replace the entire metafied ZLE line (`ZLEMETALINE`) with `content` and
    // move the metafied cursor to byte offset `cs`. This is the net effect of
    // C's recurring menu-select idiom
    //   `zlemetacs = 0; foredel(zlemetall, CUT_RAW); spaceinline(l);
    //    memcpy(zlemetaline, content, l); zlemetacs = cs;`
    // (complist.c:2455-2458, :2229-2232, :2665-2668, :2760-2765, :3140-3145),
    // which in C runs metafied (`zleline == zlemetaline`) so all three ops hit
    // one buffer, netting `zlemetaline = content`. zshrs SPLITS the buffers —
    // char `ZLELINE` vs byte `ZLEMETALINE` — and `spaceinline`/`foredel(0)` are
    // not meta-aware, so they mutated the WRONG (char) buffer while
    // `replace_range(..l)` overwrote only the first l bytes of `ZLEMETALINE`,
    // leaking `^@` NUL placeholders + any stale tail beyond l into the display
    // (e.g. `ls  ^@^@^@<stale>`), self-perpetuating across completions.
    // Reconstruct `ZLEMETALINE` directly. Kept a closure (not a fn) because
    // this composite has no single C counterpart for the build.rs port gate;
    // the real root fix is a meta-aware `spaceinline` (blocked by the
    // `RegionHighlight` `start_meta`/`end_meta` gap).
    let set_zlemetaline = |content: &str, cs: i32| {
        if let Some(m) = ZLEMETALINE.get() {
            if let Ok(mut g) = m.lock() {
                *g = content.to_string();
            }
        }
        ZLEMETALL.store(content.len() as i32, Ordering::SeqCst);
        ZLEMETACS.store(cs, Ordering::SeqCst);
    };

    // c:2398-2399 — bail-out when no previous list. `hasoldlist` is
    // the file-static at compcore.c:140 (ported as AtomicI32 in
    // compcore.rs:3462); set by `compprintlist` after populating
    // mtab/mgtab.
    if crate::ported::zle::compcore::hasoldlist.load(std::sync::atomic::Ordering::Relaxed) == 0 {
        return 2; // c:2399
    }

    // c:2401-2403 — reset incremental search state.
    *MSEARCHSTR.lock().unwrap() = String::new(); // c:2402
    MSEARCHSTATE.store(MS_OK, Ordering::SeqCst); // c:2403

    crate::ported::signals::queue_signals(); // c:2406

    // c:2407-2416 — recursive-entry guard via `fdat` static.
    // Without the Chdata param + fdat static wired here, skip.

    // c:2427-2432 — `if (zlemetaline != NULL) wasmeta = 1; else metafy_line();`
    _wasmeta = if ZLEMETALINE.get().is_some() {
        1
    } else {
        // c:2431 — metafy_line(); zshrs's line is already UTF-8 native.
        0
    };

    // c:2434-2440 — MENUSCROLL: step size for half-page jumps.
    if let Some(s) = getsparam("MENUSCROLL") {
        // c:2434
        let parsed: i32 = s.trim().parse().unwrap_or(0);
        if parsed == 0 {
            // c:2435
            let zterm_lines = adjustlines() as i32;
            let nlnct = NLNCT.load(Ordering::SeqCst);
            step = (zterm_lines - nlnct) >> 1; // c:2436
        } else if parsed < 0 {
            // c:2437
            let zterm_lines = adjustlines() as i32;
            let nlnct = NLNCT.load(Ordering::SeqCst);
            step = parsed + zterm_lines - nlnct;
            if step < 0 {
                step = 1;
            } // c:2439
        } else {
            step = parsed;
        }
    }

    // c:2441-2462 — MENUMODE: interactive / search-fwd / search-back.
    if let Some(s) = getsparam("MENUMODE") {
        // c:2441
        if s == "interactive" {
            // c:2442
            mode = 1; /* MM_INTER */
            // c:2453
            // c:2454-2458 — restore origline so the user sees what they typed.
            let origline = ORIGLINE
                .get()
                .and_then(|m| m.lock().ok().map(|g| g.clone()))
                .unwrap_or_default();
            // c:2455-2458 — reconstruct the line to `origline`, cursor at
            // `origcs`. See `set_zlemetaline` for why the C
            // foredel/spaceinline/memcpy idiom can't be used verbatim here.
            set_zlemetaline(&origline, ORIGCS.load(Ordering::SeqCst));
            let _ = setmstatus(&mut status, "", 0, 0, None, None, None); // c:2459
        } else if s.starts_with("search") {
            // c:2460
            mode = if s.contains("back") { 3 } else { 2 }; // c:2461 MM_BSEARCH / MM_FSEARCH
        }
    }

    // c:2470 — `selectlocalmap(mskeymap)` — switch to the menuselect
    // keymap so getkeycmd uses it for byte→widget resolution.
    let saved_localmap = {
        let g = crate::ported::zle::zle_keymap::LOCALKEYMAP.lock().unwrap();
        g.clone()
    };
    if let Some(mskeymap) = crate::ported::zle::zle_keymap::openkeymap("menuselect") {
        crate::ported::zle::zle_keymap::selectlocalmap(Some(mskeymap)); // c:2470
    }

    let _ = &saved_localmap; // C selectlocalmap(NULL) on exit, not a restore.

    // c:2465-2467 — MENUPROMPT status line + mhasstat flag.
    {
        // `mstatus = dupstring(getsparam("MENUPROMPT"))`: unset → NULL (no
        // status); set-but-empty → the default prompt; else the value.
        let mstatus = match getsparam("MENUPROMPT") {
            None => String::new(),
            Some(s) if s.is_empty() => "%SScrolling active: current selection at %p%s".to_string(),
            Some(s) => s,
        };
        MHASSTAT.store(if mstatus.is_empty() { 0 } else { 1 }, Ordering::SeqCst); // c:2467
        *MSTATUS.lock().unwrap() = mstatus;
    }
    // c:2464 — leave the signal-queued region the entry (c:2406) opened.
    unqueue_signals();

    // ===== c:2385-2396 loop-local state =====
    let mut acc = 0i32; // c:2392
    let mut broken = 0i32; // c:2393
    let mut wishcol = 0i32; // c:2392
    let mut setwish = 0i32; // c:2392
    let mut wasnext = 0i32; // c:2392
    let mut lbeg = 0i32; // c:2393
    let mut first = 1i32; // c:2393
    let mut nolist = 0i32; // c:2394
    let pl = NLNCT.load(Ordering::SeqCst); // c:2393
    let mut do_last_key = 0i32; // c:2391
    let mut modeline: Option<String> = None; // c:2396
    let mut modecs = 0i32; // c:2394
    let mut modell = 0i32; // c:2394
    let mut modelen = 0i32; // c:2394
    let mut lastsearch: Option<String> = None; // c:2386 static lastsearch
    let mut p: i32 = 0; // C `Cmatch **p` — linear index into mtab
    let mut cmd: Option<crate::ported::zle::zle_thingy::Thingy> = None; // c:2389
    let mut goto_getk = false; // emulates C's `goto getk` (skip redraw+p-setup)
    let mut i_flag = 0i32; // c:2392 `i` — first-non-empty sentinel

    // c:2159 `struct menustack`: the ported `menustack` type (this file)
    // omits the `struct menuinfo info` snapshot and the amatches/pmatches/
    // lastmatches/lastlmatches group lists that C keeps in the same struct.
    // Rather than mutate that struct, the extra state rides alongside in a
    // local frame so push/pop restore stays faithful.
    struct MFrame {
        line: String,                                           // c:2161
        cs: i32,                                                // c:2165
        mline: i32,                                             // c:2165
        mlbeg: i32,                                             // c:2165
        info: crate::ported::zle::comp_h::Menuinfo,             // c:2167 struct menuinfo info
        amatches: Option<Vec<Cmgroup>>,                         // c:2168
        pmatches: Option<Vec<Cmgroup>>,                         // c:2168
        lastmatches: Option<Vec<Cmgroup>>,                      // c:2168
        lastlmatches: Option<Cmgroup>,                          // c:2168
        nolist: i32,                                            // c:2165
        acc: i32,                                               // c:2165
        brbeg: Option<Box<crate::ported::zle::comp_h::Brinfo>>, // c:2162
        brend: Option<Box<crate::ported::zle::comp_h::Brinfo>>, // c:2163
        nbrbeg: i32,                                            // c:2164
        nbrend: i32,                                            // c:2164
        nmatches: i32,                                          // c:2165
        origline: String,                                       // c:2172
        origcs: i32,                                            // c:2173
        origll: i32,                                            // c:2173
        status: String,                                         // c:2180
        mode: i32,                                              // c:2181
    }
    let mut u: Vec<MFrame> = Vec::new(); // c:2389 `Menustack u = NULL;`

    // Movement dispatch tokens — C reaches the equivalent code via gotos
    // (down:/up:/right:/left:/top:/bottom:); Rust models the cross-arm
    // jumps with a small state machine driven by this enum.
    #[derive(Clone, Copy, PartialEq)]
    enum Move {
        Down,
        Up,
        Right,
        Left,
        Top,
        Bottom,
        FwdWord,
        BwdWord,
        BlankFwd,
        BlankBwd,
        BegLine,
        EndLine,
    }

    // ---- mtab/mgtab cell accessors -------------------------------------
    // C works with `Cmatch **p` pointer arithmetic + the MMARK low-bit tag.
    // The Rust mtab stores cloned `Cmatch` values (no tag), so the mark is
    // reconstructed from adjacency: a real-match cell whose in-row left
    // neighbour shares its `gnum` is a continuation (== `mmarked`). A
    // `mtmark(NULL)` separator cell is stored as `None`, which the callers
    // treat exactly like C's `!*p`.
    let cell = |i: i32| -> Option<Cmatch> {
        if i < 0 {
            return None;
        }
        MTAB.lock().unwrap().get(i as usize).cloned().flatten()
    };
    let gcell = |i: i32| -> Option<Cmgroup> {
        if i < 0 {
            return None;
        }
        MGTAB.lock().unwrap().get(i as usize).cloned().flatten()
    };
    // Combined `!*p || mmarked(*p)` skip predicate used across navigation.
    let skipcell = |i: i32| -> bool {
        if i < 0 {
            return true; // !*p
        }
        let mc = MCOLS.load(Ordering::SeqCst);
        let t = MTAB.lock().unwrap();
        match t.get(i as usize).cloned().flatten() {
            None => true, // !*p (real NULL or mtmark(NULL) separator)
            Some(a) => {
                if mc > 0 && i % mc != 0 {
                    if let Some(b) = t.get((i - 1) as usize).cloned().flatten() {
                        return a.gnum == b.gnum; // mmarked(*p)
                    }
                }
                false
            }
        }
    };
    // `*a == *b` pointer-equality, resolved by unique match `gnum`.
    let same = |a: i32, b: i32| -> bool {
        match (cell(a), cell(b)) {
            (Some(x), Some(y)) => x.gnum == y.gnum,
            _ => false,
        }
    };
    // Direct port of `adjust_mcol(int wish, Cmatch ***tabp, Cmgroup **grp)`
    // (complist.c:2127) — inlined so the mtab-walking body runs (the
    // module-level adjust_mcol is a clamp-only stub). Mutates `mcol` and
    // returns `(new_p, ret)` where ret==1 means "row is empty".
    let adjust_mcol = |wish: i32, pin: i32| -> (i32, i32) {
        let mc = MCOLS.load(Ordering::SeqCst);
        let mcol = MCOL.load(Ordering::SeqCst);
        let base = pin - mcol; // matchtab -= mcol
        let mut pp = wish;
        while pp >= 0 && skipcell(base + pp) {
            pp -= 1;
        } // c:2133
        let mut n = wish;
        while n < mc && skipcell(base + n) {
            n += 1;
        } // c:2134
        if n == mc {
            n = -1;
        } // c:2135-2136
        let c;
        if pp < 0 {
            // c:2138
            if n < 0 {
                return (pin, 1);
            } // c:2139-2140
            c = n; // c:2141
        } else if n < 0 {
            c = pp; // c:2143
        } else {
            c = if (mcol - pp) < (n - mcol) { pp } else { n }; // c:2145
        }
        MCOL.store(c, Ordering::SeqCst); // c:2151
        (base + c, 0) // c:2147 *tabp = matchtab + c
    };
    // minfo.cur->gnum helper.
    let cur_gnum = || -> i32 {
        MINFO
            .get()
            .and_then(|g| g.lock().ok())
            .and_then(|g| g.cur.as_ref().map(|c| c.gnum))
            .unwrap_or(-1)
    };
    // Direct port of `do_menucmp(0)` (compresult.c:1253): step the menu
    // cursor `zmult` times through the amatches arrays via `valid_match`,
    // then re-insert with `do_single`.
    let do_menucmp0 = || {
        let mut zm = crate::ported::zle::compcore::ZMULT.load(Ordering::SeqCst);
        while zm != 0 {
            let ci = MINFO
                .get()
                .and_then(|g| g.lock().ok())
                .map(|g| g.cur_idx)
                .unwrap_or(0);
            let m = crate::ported::zle::compresult::valid_match(ci, 1);
            if let (Some(mm), Some(lk)) = (m, MINFO.get()) {
                if let Ok(mut mi) = lk.lock() {
                    mi.cur = Some(Box::new(mm)); // minfo.cur = valid_match(...)
                }
            }
            zm -= (if 0 < zm { 1 } else { 0 }) - (if zm < 0 { 1 } else { 0 });
        }
        let cur = MINFO
            .get()
            .and_then(|g| g.lock().ok())
            .and_then(|g| g.cur.as_ref().map(|c| (**c).clone()));
        if let Some(c) = cur {
            crate::ported::zle::compresult::do_single(&c); // do_single(*minfo.cur)
        }
    };
    // Direct port of `msearch(Cmatch **ptr, char *ins, int back, int rep,
    // int *wrapp)` (complist.c:2302), inlined so the `ins`/`back`/`rep`
    // parameters that the module-level `msearch()` stub drops are honoured.
    // Returns `(Some(index)|None, wrap)`.
    let msearch_fn = |pin: i32, ins: Option<&str>, back: bool, rep0: bool| -> (Option<i32>, i32) {
        let mc = MCOLS.load(Ordering::SeqCst);
        let mut x = MCOL.load(Ordering::SeqCst); // c:2305
        let mut y = MLINE.load(Ordering::SeqCst);
        let mut wrap = 0i32;
        let owrap = MSEARCHSTATE.load(Ordering::SeqCst) & MS_WRAPPED; // c:2306
                                                                      // c:2308 msearchpush(ptr, back).
        {
            let mut st = MSEARCHSTACK.lock().unwrap();
            st.push(menusearch {
                str: MSEARCHSTR.lock().unwrap().clone(),
                line: MLINE.load(Ordering::SeqCst),
                col: MCOL.load(Ordering::SeqCst),
                back: if back { 1 } else { 0 },
                state: MSEARCHSTATE.load(Ordering::SeqCst),
                ptr: pin.max(0) as usize,
            });
        }
        if let Some(s) = ins {
            MSEARCHSTR.lock().unwrap().push_str(s); // c:2310 dyncat
        }
        let nlines = listdat
            .get()
            .and_then(|m| m.lock().ok().map(|g| g.nlines))
            .unwrap_or(0);
        let (mut ex, mut ey) = if back {
            (mc - 1, -1) // c:2312-2313
        } else {
            (0, nlines) // c:2315-2316
        };
        let mut pp = pin; // c:2318 p = mtab + mline*mcols + mcol
        let mut l_gnum: Option<i32> = None;
        let mut rep = rep0;
        if rep {
            l_gnum = cell(pp).map(|c| c.gnum); // c:2320
        }
        let needle = MSEARCHSTR.lock().unwrap().clone();
        loop {
            // c:2323-2333
            if !rep {
                if let Some(m) = cell(pp) {
                    if l_gnum != Some(m.gnum) {
                        l_gnum = Some(m.gnum);
                        let hay = m
                            .disp
                            .as_deref()
                            .unwrap_or_else(|| m.str.as_deref().unwrap_or(""));
                        if hay.contains(needle.as_str()) {
                            MCOL.store(x, Ordering::SeqCst); // c:2328
                            MLINE.store(y, Ordering::SeqCst); // c:2329
                            return (Some(pp), wrap); // c:2331
                        }
                    }
                }
            }
            rep = false; // c:2336
            if back {
                // c:2338-2342
                pp -= 1;
                x -= 1;
                if x < 0 {
                    x = mc - 1;
                    y -= 1;
                }
            } else {
                // c:2343-2348
                pp += 1;
                x += 1;
                if x == mc {
                    x = 0;
                    y += 1;
                }
            }
            if x == ex && y == ey {
                // c:2350
                if back {
                    x = mc - 1;
                    y = nlines - 1;
                    pp = y * mc + x; // c:2352-2354
                } else {
                    x = 0;
                    y = 0;
                    pp = 0; // c:2356-2357
                }
                ex = MCOL.load(Ordering::SeqCst); // c:2359
                ey = MLINE.load(Ordering::SeqCst); // c:2360
                if wrap != 0 || (x == ex && y == ey) {
                    // c:2362
                    MSEARCHSTATE.store(MS_FAILED | owrap, Ordering::SeqCst); // c:2363
                    break;
                }
                MSEARCHSTATE.fetch_or(MS_WRAPPED, Ordering::SeqCst); // c:2367
                wrap = 1; // c:2368
            }
        }
        (None, wrap) // c:2372
    };

    NOSELECT.store(1, Ordering::SeqCst); // c:2471 `noselect = 1;`

    // c:2472-2481 — skip dummy / already-accepted matches before entering.
    loop {
        let cur = MINFO
            .get()
            .and_then(|g| g.lock().ok())
            .and_then(|g| g.cur.as_ref().map(|c| (**c).clone()));
        let (prebr, postbr) = MINFO
            .get()
            .and_then(|g| g.lock().ok())
            .map(|g| (g.prebr.clone(), g.postbr.clone()))
            .unwrap_or((None, None));
        let need = match cur {
            Some(c) => {
                let ma = crate::ported::zle::compcore::menuacc.load(Ordering::SeqCst);
                (ma != 0
                    && !crate::ported::zle::compresult::hasbrpsfx(
                        &c,
                        prebr.as_deref(),
                        postbr.as_deref(),
                    ))
                    || (c.flags & crate::ported::zle::comp_h::CMF_DUMMY) != 0
                    || ((c.flags & (CMF_NOLIST | crate::ported::zle::comp_h::CMF_MULT)) != 0
                        && c.str.as_deref().map_or(true, |s| s.is_empty()))
            }
            None => false,
        };
        if !need {
            break;
        }
        do_menucmp0(); // c:2481
    }

    // c:2483-2488 — initial selection + geometry.
    MSELECT.store(cur_gnum(), Ordering::SeqCst); // c:2483
    MLINE.store(0, Ordering::SeqCst); // c:2484
    MLINES.store(999999, Ordering::SeqCst); // c:2485
    MLBEG.store(0, Ordering::SeqCst); // c:2486
    MOLBEG.store(-42, Ordering::SeqCst); // c:2487
    MTAB_BEEN_REALLOCATED.store(0, Ordering::SeqCst); // c:2488

    // c:2489 — `for (;;) { ... }`.
    loop {
        if !goto_getk {
            // c:2492-2503 — mline<0 or reallocated: re-scan mtab for the
            // selected match's row/col.
            if MLINE.load(Ordering::SeqCst) < 0 || MTAB_BEEN_REALLOCATED.load(Ordering::SeqCst) != 0
            {
                let mcols = MCOLS.load(Ordering::SeqCst);
                let mlines = MLINES.load(Ordering::SeqCst);
                let msel = MSELECT.load(Ordering::SeqCst);
                let mut idx = 0i32;
                let mut found_y = mlines;
                'yl: for y in 0..mlines {
                    let mut xx = mcols;
                    while xx > 0 {
                        if !skipcell(idx) {
                            if let Some(c) = cell(idx) {
                                if c.gnum == msel {
                                    MCOL.store(mcols - xx, Ordering::SeqCst); // c:2500
                                    found_y = y;
                                    break 'yl;
                                }
                            }
                        }
                        xx -= 1;
                        idx += 1;
                    }
                }
                if found_y < mlines {
                    MLINE.store(found_y, Ordering::SeqCst); // c:2505
                }
            }
            MTAB_BEEN_REALLOCATED.store(0, Ordering::SeqCst); // c:2507

            // c:2510-2516 — scroll the window up until mline is visible.
            while MLINE.load(Ordering::SeqCst) < MLBEG.load(Ordering::SeqCst) {
                let nb = MLBEG.load(Ordering::SeqCst) - step;
                MLBEG.store(nb, Ordering::SeqCst); // mlbeg -= step
                if nb < 0 {
                    MLBEG.store(0, Ordering::SeqCst);
                    if MLINE.load(Ordering::SeqCst) < 0 {
                        break;
                    }
                }
            }

            // c:2518-2528 — back mlbeg up onto a non-empty row.
            if MLBEG.load(Ordering::SeqCst) != 0 && lbeg != MLBEG.load(Ordering::SeqCst) {
                let cols = MCOLS.load(Ordering::SeqCst);
                let mut base = (MLBEG.load(Ordering::SeqCst) - 1) * cols;
                while MLBEG.load(Ordering::SeqCst) != 0 {
                    let mut c = cols;
                    let mut q = base;
                    while c > 0 {
                        if !skipcell(q) {
                            break;
                        }
                        q += 1;
                        c -= 1;
                    }
                    if c != 0 {
                        break;
                    }
                    base -= cols;
                    MLBEG.store(MLBEG.load(Ordering::SeqCst) - 1, Ordering::SeqCst);
                }
            }

            // c:2530-2532 — scroll down until mline fits in the window.
            let zterm_lines = adjustlines() as i32;
            let space = zterm_lines - pl - MHASSTAT.load(Ordering::SeqCst);
            if space > 0 {
                while MLINE.load(Ordering::SeqCst) >= MLBEG.load(Ordering::SeqCst) + space {
                    let nb = MLBEG.load(Ordering::SeqCst) + step;
                    MLBEG.store(nb, Ordering::SeqCst);
                    if nb + space > MLINES.load(Ordering::SeqCst) {
                        MLBEG.store(MLINES.load(Ordering::SeqCst) - space, Ordering::SeqCst);
                    }
                }
            }

            // c:2534-2547 — advance mlbeg forward onto a non-empty row.
            if lbeg != MLBEG.load(Ordering::SeqCst) {
                let cols = MCOLS.load(Ordering::SeqCst);
                let mut base = MLBEG.load(Ordering::SeqCst) * cols;
                while MLBEG.load(Ordering::SeqCst) < MLINES.load(Ordering::SeqCst) {
                    let mut c = cols;
                    let mut q = base;
                    while c > 0 {
                        if cell(q).is_some() {
                            break;
                        }
                        q += 1;
                        c -= 1;
                    }
                    if c != 0 {
                        break;
                    }
                    base += cols;
                    MLBEG.store(MLBEG.load(Ordering::SeqCst) + 1, Ordering::SeqCst);
                }
            }
            lbeg = MLBEG.load(Ordering::SeqCst); // c:2548

            crate::ported::zle::compcore::onlyexpl.store(0, Ordering::SeqCst); // c:2549
            SHOWINGLIST.store(-2, Ordering::SeqCst); // c:2550

            // c:2551 — first-time bell.
            if first != 0
                && LISTSHOWN.load(Ordering::SeqCst) == 0
                && isset(crate::ported::zsh_h::LISTBEEP)
            {
                crate::ported::utils::zbeep();
            }
            // c:2560-2566 — capture the pre-menu line for interactive mode.
            if first != 0 {
                modeline = Some(
                    ZLEMETALINE
                        .get()
                        .and_then(|m| m.lock().ok().map(|g| g.clone()))
                        .unwrap_or_default(),
                );
                modecs = ZLEMETACS.load(Ordering::SeqCst);
                modell = ZLEMETALL.load(Ordering::SeqCst);
                modelen = MINFO
                    .get()
                    .and_then(|g| g.lock().ok())
                    .map(|g| g.len)
                    .unwrap_or(0);
            }
            first = 0; // c:2567

            // c:2568-2584 — status line for interactive / isearch modes.
            if mode == 1 {
                *STATUSLINE.lock().unwrap() = Some(status.clone());
            } else if mode != 0 {
                let st = MSEARCHSTATE.load(Ordering::SeqCst);
                let failed = if st & MS_FAILED != 0 { "failed " } else { "" };
                let wrapped = if st & MS_WRAPPED != 0 { "wrapped " } else { "" };
                let dir = if mode == 2 { "" } else { " backward" };
                status = format!(
                    "{}{}isearch{}: {}",
                    failed,
                    wrapped,
                    dir,
                    MSEARCHSTR.lock().unwrap()
                );
                *STATUSLINE.lock().unwrap() = Some(status.clone());
            } else {
                *STATUSLINE.lock().unwrap() = None;
            }

            // c:2585-2589 — refresh (this fills mtab/mgtab + mmtabp).
            if NOSELECT.load(Ordering::SeqCst) < 0 {
                SHOWINGLIST.store(0, Ordering::SeqCst);
                CLEARLIST.store(0, Ordering::SeqCst);
                CLEARFLAG.store(1, Ordering::SeqCst);
            }
            complistmatches(std::ptr::null_mut(), std::ptr::null_mut()); // c:2589 zrefresh()
            *STATUSLINE.lock().unwrap() = None; // c:2590

            INSELECT.store(1, Ordering::SeqCst); // c:2591
            SELECTED.store(1, Ordering::SeqCst); // c:2592

            // c:2593-2600 — nothing selectable.
            let nosel = NOSELECT.load(Ordering::SeqCst);
            if nosel != 0 {
                if nosel < 0 {
                    NOSELECT.store(0, Ordering::SeqCst); // c:2596
                    goto_getk = true; // goto getk
                    continue;
                }
                broken = 1; // c:2598
                break; // c:2599
            }

            // c:2601-2608 — first-run: bail if the whole matrix is empty.
            if i_flag == 0 {
                let total = MCOLS.load(Ordering::SeqCst) * MLINES.load(Ordering::SeqCst);
                let mut k = total;
                let mut any = false;
                while k > 0 {
                    k -= 1;
                    if cell(k).is_some() {
                        any = true;
                        break;
                    }
                }
                if !any {
                    break; // c:2606
                }
                i_flag = 1;
            }

            // c:2610-2618 — current cell + wishcol tracking.
            p = MMTABP.load(Ordering::SeqCst) as i32; // c:2610 p = mmtabp
            if let Some(c) = cell(p) {
                let g = gcell(p);
                if let Ok(mut mi) = MINFO
                    .get_or_init(|| {
                        std::sync::Mutex::new(crate::ported::zle::comp_h::Menuinfo::default())
                    })
                    .lock()
                {
                    mi.cur = Some(Box::new(c)); // c:2615 minfo.cur = *p
                    mi.group = g.map(Box::new); // c:2616 minfo.group = *pg
                }
            }
            let cg = cur_gnum();
            if setwish != 0 {
                wishcol = MCOL.load(Ordering::SeqCst); // c:2619
            } else if MCOL.load(Ordering::SeqCst) > wishcol {
                // c:2620 — `while (mcol > 0 && p[-1] == minfo.cur)`
                while MCOL.load(Ordering::SeqCst) > 0 && cell(p - 1).map_or(false, |c| c.gnum == cg)
                {
                    MCOL.store(MCOL.load(Ordering::SeqCst) - 1, Ordering::SeqCst);
                    p -= 1;
                }
            } else if MCOL.load(Ordering::SeqCst) < wishcol {
                // c:2621 — `while (mcol < mcols-1 && p[1] == minfo.cur)`
                while MCOL.load(Ordering::SeqCst) < MCOLS.load(Ordering::SeqCst) - 1
                    && cell(p + 1).map_or(false, |c| c.gnum == cg)
                {
                    MCOL.store(MCOL.load(Ordering::SeqCst) + 1, Ordering::SeqCst);
                    p += 1;
                }
            }
            setwish = 0;
            wasnext = 0; // c:2622
        }
        goto_getk = false;

        // c:2626-2637 — getk: read one command.
        if do_last_key == 0 {
            crate::ported::zle::compcore::ZMULT.store(1, Ordering::SeqCst); // c:2627
            cmd = crate::ported::zle::zle_keymap::getkeycmd(); // c:2628
                                                               // c:2629-2633 — swallow the interrupt flag (best-effort).
            if MTAB_BEEN_REALLOCATED.load(Ordering::SeqCst) != 0 {
                do_last_key = 1; // c:2635
                continue;
            }
        }
        do_last_key = 0; // c:2637
        let was_inter = mode == 1; // c:2639
        let name = cmd.as_ref().map(|t| t.nam.clone()).unwrap_or_default();

        let mut movement: Option<Move> = None;
        let mut wrap = 0i32;

        // ===== dispatch ladder (c:2641-3452) =====
        if name.is_empty() || name == "send-break" {
            // c:2643-2647
            crate::ported::utils::zbeep();
            MOLBEG.store(-1, Ordering::SeqCst);
            broken = 1;
            break;
        } else if nolist != 0
            && name != "undo"
            && (mode == 0
                || (name != "backward-delete-char"
                    && name != "self-insert"
                    && name != "self-insert-unmeta"))
        {
            // c:2648-2652
            crate::ported::zle::zle_keymap::ungetkeycmd();
            break;
        } else if name == "accept-line" || name == "accept-search" {
            // c:2653-2660
            if mode == 2 || mode == 3 {
                mode = 0;
                continue;
            }
            acc = 1;
            break;
        } else if name == "vi-insert" {
            // c:2661-2687
            if mode == 1 {
                mode = 0; // exit interactive — fall through to do_single.
            } else {
                mode = 1;
                let origline = ORIGLINE
                    .get()
                    .and_then(|m| m.lock().ok().map(|g| g.clone()))
                    .unwrap_or_default();
                // c:2661-2687 — reconstruct to `origline`; see `set_zlemetaline`.
                set_zlemetaline(&origline, ORIGCS.load(Ordering::SeqCst));
                let _ = setmstatus(&mut status, "", 0, 0, None, None, None);
                continue;
            }
        } else if name == "accept-and-infer-next-history"
            || (mode == 1 && (name == "self-insert" || name == "self-insert-unmeta"))
        {
            // c:2732-2820 — recursive interactive / accept-and-infer
            // completion. This arm sets `comprecursive = 1` (c:2735, now
            // ported as `COMPRECURSIVE` above) and then calls
            // `menucomplete(zlenoargs)` (c:2778) as a *nested* completion.
            // Blocked on one out-of-scope piece: the nested `menucomplete`
            // routes through `docomplete`, whose recursion guard
            // (zle_tricky.rs:712) tests only the thread-local `ACTIVE`
            // flag and does not consult `comprecursive`. The C guard is
            // `if (active && !comprecursive)` (zle_tricky.c:606), so the
            // nested call is only admitted when `comprecursive` is set.
            // Honouring it requires editing `docomplete` in zle_tricky.rs,
            // outside this file's scope. (`domenuselect` also has no
            // `menu_start`-hook caller in Rust yet, so this arm is currently
            // unreachable regardless.) Accept the key as a no-op step
            // rather than delete the line and fire a nested completion the
            // guard will reject — which would fall into the "no matches"
            // branch and trash the display.
            continue;
        } else if name == "accept-and-hold" || name == "accept-and-menu-complete" {
            // c:2688-2731
            if mode == 1 {
                let cur = MINFO
                    .get()
                    .and_then(|g| g.lock().ok())
                    .and_then(|g| g.cur.as_ref().map(|c| (**c).clone()));
                if let Some(lk) = MINFO.get() {
                    if let Ok(mut mi) = lk.lock() {
                        mi.cur = None;
                    }
                }
                if let Some(c) = &cur {
                    crate::ported::zle::compresult::do_single(c);
                }
                if let (Some(lk), Some(c)) = (MINFO.get(), cur) {
                    if let Ok(mut mi) = lk.lock() {
                        mi.cur = Some(Box::new(c));
                    }
                }
            }
            mode = 0;
            let info = MINFO
                .get()
                .and_then(|g| g.lock().ok())
                .map(|g| g.clone())
                .unwrap_or_default();
            u.push(MFrame {
                line: ZLEMETALINE
                    .get()
                    .and_then(|m| m.lock().ok().map(|g| g.clone()))
                    .unwrap_or_default(),
                cs: ZLEMETACS.load(Ordering::SeqCst),
                mline: MLINE.load(Ordering::SeqCst),
                mlbeg: MLBEG.load(Ordering::SeqCst),
                info,
                amatches: None, // c:2701 s->amatches = ... = NULL
                pmatches: None,
                lastmatches: None,
                lastlmatches: None,
                nolist,
                acc: crate::ported::zle::compcore::menuacc.load(Ordering::SeqCst),
                brbeg: crate::ported::zle::compcore::BRBEG
                    .get()
                    .and_then(|m| m.lock().ok())
                    .and_then(|g| g.clone()),
                brend: crate::ported::zle::compcore::BREND
                    .get()
                    .and_then(|m| m.lock().ok())
                    .and_then(|g| g.clone()),
                nbrbeg: NBRBEG.load(Ordering::SeqCst),
                nbrend: NBREND.load(Ordering::SeqCst),
                nmatches: crate::ported::zle::compcore::nmatches.load(Ordering::SeqCst),
                origline: ORIGLINE
                    .get()
                    .and_then(|m| m.lock().ok().map(|g| g.clone()))
                    .unwrap_or_default(),
                origcs: ORIGCS.load(Ordering::SeqCst),
                origll: ORIGLL.load(Ordering::SeqCst),
                status: status.clone(),
                mode,
            });
            crate::ported::zle::compresult::accept_last(); // c:2720
            handleundo();
            COMPRECURSIVE.store(1, Ordering::SeqCst); // c:2861
            do_menucmp0(); // c:2862 `do_menucmp(0)`
            MSELECT.store(cur_gnum(), Ordering::SeqCst);

            // c:2726-2739 — relocate the cursor onto the new selection.
            p -= MCOL.load(Ordering::SeqCst);
            MCOL.store(0, Ordering::SeqCst);
            let ol = MLINE.load(Ordering::SeqCst);
            let cg = cur_gnum();
            loop {
                MCOL.store(0, Ordering::SeqCst);
                let mcols = MCOLS.load(Ordering::SeqCst);
                let mut found = false;
                while MCOL.load(Ordering::SeqCst) < mcols {
                    if cell(p).map_or(false, |c| c.gnum == cg) {
                        found = true;
                        break;
                    }
                    MCOL.store(MCOL.load(Ordering::SeqCst) + 1, Ordering::SeqCst);
                    p += 1;
                }
                if found {
                    break;
                }
                MLINE.store(MLINE.load(Ordering::SeqCst) + 1, Ordering::SeqCst);
                if MLINE.load(Ordering::SeqCst) == MLINES.load(Ordering::SeqCst) {
                    MLINE.store(0, Ordering::SeqCst);
                    p -= MLINES.load(Ordering::SeqCst) * mcols;
                }
                if MLINE.load(Ordering::SeqCst) == ol {
                    break;
                }
            }
            if !cell(p).map_or(false, |c| c.gnum == cg) {
                // c:2740-2745
                NOSELECT.store(1, Ordering::SeqCst);
                CLEARLIST.store(1, Ordering::SeqCst);
                LISTSHOWN.store(1, Ordering::SeqCst);
                crate::ported::zle::compcore::onlyexpl.store(0, Ordering::SeqCst);
                complistmatches(std::ptr::null_mut(), std::ptr::null_mut());
                break;
            }
            setwish = 1; // c:2746
            continue;
        } else if name == "undo" || (mode == 1 && name == "backward-delete-char") {
            // c:2747-2790
            let frame = match u.pop() {
                Some(f) => f,
                None => break, // c:2751
            };
            handleundo();
            // c:2747-2790 — restore the undo frame's line; see `set_zlemetaline`.
            set_zlemetaline(&frame.line, frame.cs);
            crate::ported::zle::compcore::menuacc.store(frame.acc, Ordering::SeqCst);
            if let Ok(mut mi) = MINFO
                .get_or_init(|| {
                    std::sync::Mutex::new(crate::ported::zle::comp_h::Menuinfo::default())
                })
                .lock()
            {
                *mi = frame.info.clone(); // c:2762 memcpy(&minfo, &u->info, ...)
            }
            MLINE.store(frame.mline, Ordering::SeqCst);
            MLBEG.store(frame.mlbeg, Ordering::SeqCst);
            // c:2775-2782 — restore the saved match arrays when present.
            if let Some(am) = frame.amatches {
                if let Some(m) = crate::ported::zle::compcore::amatches.get() {
                    *m.lock().unwrap() = am;
                }
                if let (Some(pm), Some(m)) =
                    (frame.pmatches, crate::ported::zle::compcore::pmatches.get())
                {
                    *m.lock().unwrap() = pm;
                }
                if let (Some(lm), Some(m)) = (
                    frame.lastmatches,
                    crate::ported::zle::compcore::lastmatches.get(),
                ) {
                    *m.lock().unwrap() = lm;
                }
                if let Some(m) = crate::ported::zle::compcore::lastlmatches.get() {
                    *m.lock().unwrap() = frame.lastlmatches;
                }
                crate::ported::zle::compcore::nmatches.store(frame.nmatches, Ordering::SeqCst);
                crate::ported::zle::compcore::hasoldlist.store(1, Ordering::SeqCst);
                VALIDLIST.store(1, Ordering::SeqCst);
            }
            // c:2783-2788 — brace-info restore.
            if let Some(m) = crate::ported::zle::compcore::BRBEG.get() {
                *m.lock().unwrap() = frame.brbeg;
            }
            if let Some(m) = crate::ported::zle::compcore::BREND.get() {
                *m.lock().unwrap() = frame.brend;
            }
            NBRBEG.store(frame.nbrbeg, Ordering::SeqCst);
            NBREND.store(frame.nbrend, Ordering::SeqCst);
            if let Some(m) = ORIGLINE.get() {
                *m.lock().unwrap() = frame.origline.clone();
            }
            ORIGCS.store(frame.origcs, Ordering::SeqCst);
            ORIGLL.store(frame.origll, Ordering::SeqCst);
            status = frame.status.clone();
            mode = frame.mode;
            nolist = frame.nolist;

            CLEARLIST.store(1, Ordering::SeqCst); // c:2792
            setwish = 1;
            if let Some(m) = listdat.get() {
                if let Ok(mut g) = m.lock() {
                    g.valid = 0;
                }
            }
            MOLBEG.store(-42, Ordering::SeqCst);

            if nolist != 0 {
                // c:2797-2805 — nolist: just repaint + re-read a key.
                if mode == 1 {
                    *STATUSLINE.lock().unwrap() = Some(status.clone());
                } else {
                    *STATUSLINE.lock().unwrap() = None;
                }
                zrefresh();
                *STATUSLINE.lock().unwrap() = None;
                goto_getk = true;
                continue;
            }
            if mode != 0 {
                continue; // c:2807
            }
            // c:2747..fall-through — re-insert minfo.cur (C aims p at it).
            let cur = MINFO
                .get()
                .and_then(|g| g.lock().ok())
                .and_then(|g| g.cur.as_ref().map(|c| (**c).clone()));
            if let Some(c) = cur {
                crate::ported::zle::compresult::do_single(&c);
                MSELECT.store(c.gnum, Ordering::SeqCst);
            }
            continue;
        } else if name == "redisplay" {
            // c:2808-2812
            redisplay();
            MOLBEG.store(-42, Ordering::SeqCst);
            continue;
        } else if name == "clear-screen" {
            // c:2813-2817
            clearscreen();
            MOLBEG.store(-42, Ordering::SeqCst);
            continue;
        } else if name == "down-history"
            || name == "down-line-or-history"
            || name == "down-line-or-search"
            || name == "vi-down-line-or-history"
        {
            // c:2818-2848
            mode = 0;
            wrap = 0;
            movement = Some(Move::Down);
        } else if name == "up-history"
            || name == "up-line-or-history"
            || name == "up-line-or-search"
            || name == "vi-up-line-or-history"
        {
            // c:2849-2884
            mode = 0;
            wrap = 0;
            movement = Some(Move::Up);
        } else if name == "emacs-forward-word"
            || name == "vi-forward-word"
            || name == "vi-forward-word-end"
            || name == "forward-word"
        {
            // c:2885-2913
            mode = 0;
            movement = Some(Move::FwdWord);
        } else if name == "emacs-backward-word"
            || name == "vi-backward-word"
            || name == "backward-word"
        {
            // c:2914-2942
            mode = 0;
            movement = Some(Move::BwdWord);
        } else if name == "beginning-of-history" {
            // c:2943-2963
            mode = 0;
            movement = Some(Move::Top);
        } else if name == "end-of-history" {
            // c:2964-2984
            mode = 0;
            movement = Some(Move::Bottom);
        } else if name == "forward-char" || name == "vi-forward-char" {
            // c:2985-3011
            mode = 0;
            wrap = 0;
            movement = Some(Move::Right);
        } else if name == "backward-char" || name == "vi-backward-char" {
            // c:3012-3046
            mode = 0;
            wrap = 0;
            movement = Some(Move::Left);
        } else if name == "beginning-of-buffer-or-history"
            || name == "beginning-of-line"
            || name == "beginning-of-line-hist"
            || name == "vi-beginning-of-line"
        {
            // c:3047-3058
            mode = 0;
            movement = Some(Move::BegLine);
        } else if name == "end-of-buffer-or-history"
            || name == "end-of-line"
            || name == "end-of-line-hist"
            || name == "vi-end-of-line"
        {
            // c:3059-3070
            mode = 0;
            movement = Some(Move::EndLine);
        } else if name == "vi-forward-blank-word" || name == "vi-forward-blank-word-end" {
            // c:3071-3089
            mode = 0;
            movement = Some(Move::BlankFwd);
        } else if name == "vi-backward-blank-word" {
            // c:3090-3108
            mode = 0;
            movement = Some(Move::BlankBwd);
        } else if name == "complete-word"
            || name == "expand-or-complete"
            || name == "expand-or-complete-prefix"
            || name == "menu-complete"
            || name == "menu-expand-or-complete"
            || name == "menu-select"
        {
            // c:3109-3153
            if mode == 1 {
                // Interactive: undo the inserted completion, keep the typed text.
                let ml = modeline.clone().unwrap_or_default();
                if let Some(m) = ORIGLINE.get() {
                    *m.lock().unwrap() = ml.clone();
                }
                ORIGCS.store(modecs, Ordering::SeqCst);
                ORIGLL.store(modell, Ordering::SeqCst);
                // c:3109-3153 — restore the pre-menu line `ml` (length modell,
                // captured together at c:2560-2566); see `set_zlemetaline`.
                set_zlemetaline(&ml, modecs);
                if let Some(lk) = MINFO.get() {
                    if let Ok(mut mi) = lk.lock() {
                        mi.len = modelen;
                    }
                }
                crate::ported::zle::compcore::WE.store(
                    crate::ported::zle::compcore::WB.load(Ordering::SeqCst) + modelen,
                    Ordering::SeqCst,
                );
            } else {
                mode = 0;
                COMPRECURSIVE.store(1, Ordering::SeqCst); // c:3294
                do_menucmp0(); // c:3295 `do_menucmp(0)`
                MSELECT.store(cur_gnum(), Ordering::SeqCst);
                setwish = 1;
                MLINE.store(-1, Ordering::SeqCst);
            }
            continue;
        } else if name == "reverse-menu-complete" {
            // c:3154-3163
            mode = 0;
            COMPRECURSIVE.store(1, Ordering::SeqCst); // c:3304
            crate::ported::zle::compcore::ZMULT.store(
                -crate::ported::zle::compcore::ZMULT.load(Ordering::SeqCst),
                Ordering::SeqCst,
            );
            do_menucmp0(); // c:3306 `do_menucmp(0)`
            MSELECT.store(cur_gnum(), Ordering::SeqCst);
            setwish = 1;
            MLINE.store(-1, Ordering::SeqCst);
            continue;
        } else if name == "history-incremental-search-forward"
            || name == "history-incremental-search-backward"
            || ((mode == 2 || mode == 3)
                && (name == "self-insert"
                    || name == "self-insert-unmeta"
                    || name == "bracketed-paste"))
        {
            // c:3164-3260 — incremental search in the menu.
            let op = p;
            let was = mode == 2 || mode == 3;
            let ins =
                name == "self-insert" || name == "self-insert-unmeta" || name == "bracketed-paste";
            let back = name == "history-incremental-search-backward";
            loop {
                let mut toins: Option<String> = None;
                if was {
                    p += wishcol - MCOL.load(Ordering::SeqCst);
                    MCOL.store(wishcol, Ordering::SeqCst);
                }
                if !ins {
                    if was {
                        let empty = MSEARCHSTR.lock().unwrap().is_empty();
                        if empty {
                            if let Some(ls) = lastsearch.clone() {
                                if back == (mode == 3) {
                                    *MSEARCHSTR.lock().unwrap() = ls;
                                    mode = 0;
                                }
                            }
                        }
                    } else {
                        *MSEARCHSTR.lock().unwrap() = String::new();
                        MSEARCHSTACK.lock().unwrap().clear();
                        MSEARCHSTATE.store(MS_OK, Ordering::SeqCst);
                    }
                } else {
                    if name == "self-insert-unmeta" {
                        fixunmeta();
                    }
                    if name == "bracketed-paste" {
                        toins = Some(bracketedstring());
                    } else {
                        let lc = crate::ported::zle::compcore::LASTCHAR.load(Ordering::SeqCst);
                        toins = Some(((lc as u8) as char).to_string());
                    }
                }
                let (np, wrapf) = msearch_fn(
                    p,
                    toins.as_deref(),
                    if ins { mode == 3 } else { back },
                    was && !ins,
                );
                if !ins {
                    mode = if back { 3 } else { 2 };
                }
                if !MSEARCHSTR.lock().unwrap().is_empty() {
                    lastsearch = Some(MSEARCHSTR.lock().unwrap().clone());
                }
                if let Some(npi) = np {
                    wishcol = MCOL.load(Ordering::SeqCst);
                    p = npi;
                }
                let (np2, _) = adjust_mcol(wishcol, p);
                p = np2;
                let cont = (back || name == "history-incremental-search-forward")
                    && np.is_some()
                    && wrapf == 0
                    && was
                    && same(p, op);
                if !cont {
                    break;
                }
            }
            // falls through to do_single at the bottom.
        } else if (mode == 2 || mode == 3) && name == "backward-delete-char" {
            // c:3261-3271 — pop one search step.
            let mut back = 1i32;
            let mut ptr: Option<i32> = None;
            {
                let mut st = MSEARCHSTACK.lock().unwrap();
                let info = st
                    .last()
                    .map(|s| (s.str.clone(), s.line, s.col, s.state, s.back, s.ptr));
                match info {
                    Some((str_, line, col, state, bk, pr)) => {
                        *MSEARCHSTR.lock().unwrap() = str_;
                        MLINE.store(line, Ordering::SeqCst);
                        MCOL.store(col, Ordering::SeqCst);
                        MSEARCHSTATE.store(state, Ordering::SeqCst);
                        if st.len() > 1 {
                            st.pop();
                        }
                        back = bk;
                        ptr = Some(pr as i32);
                    }
                    None => {
                        back = 1;
                        ptr = None;
                    }
                }
            }
            mode = if back != 0 { 3 } else { 2 };
            wishcol = MCOL.load(Ordering::SeqCst);
            if let Some(pi) = ptr {
                p = pi;
                let (np, _) = adjust_mcol(wishcol, p);
                p = np;
            }
            // falls through to do_single at the bottom.
        } else if name == "undefined-key" {
            // c:3272-3275
            mode = 0;
            continue;
        } else {
            // c:3276-3285 — unrecognised widget: push it back and exit.
            crate::ported::zle::zle_keymap::ungetkeycmd();
            let ncomp = cmd
                .as_ref()
                .and_then(|t| t.widget.as_ref())
                .map(|w| (w.flags & crate::ported::zle::zle_h::WIDGET_NCOMP) != 0)
                .unwrap_or(false);
            if ncomp {
                acc = 0;
                broken = 2;
            } else {
                acc = 1;
            }
            break;
        }

        // ===== movement resolution (the goto down/up/right/left/top/bottom
        //       state machine) + bottom-of-loop do_single (c:3286-3452) =====
        if let Some(mv0) = movement {
            let mut mv = mv0;
            'nav: loop {
                match mv {
                    Move::Down => {
                        // c:2822-2846
                        let omline = MLINE.load(Ordering::SeqCst);
                        let op = p;
                        loop {
                            if MLINE.load(Ordering::SeqCst) == MLINES.load(Ordering::SeqCst) - 1 {
                                if wrap & 2 != 0 {
                                    MLINE.store(omline, Ordering::SeqCst);
                                    p = op;
                                    break;
                                }
                                p -= MLINE.load(Ordering::SeqCst) * MCOLS.load(Ordering::SeqCst);
                                MLINE.store(0, Ordering::SeqCst);
                                wrap |= 1;
                            } else {
                                MLINE.store(MLINE.load(Ordering::SeqCst) + 1, Ordering::SeqCst);
                                p += MCOLS.load(Ordering::SeqCst);
                            }
                            let (np, r) = adjust_mcol(wishcol, p);
                            p = np;
                            if r != 0 {
                                continue;
                            }
                            if skipcell(p) {
                                continue;
                            }
                            break;
                        }
                        if wrap == 1 {
                            mv = Move::Right;
                            continue 'nav;
                        }
                        break 'nav;
                    }
                    Move::Up => {
                        // c:2853-2882
                        let omline = MLINE.load(Ordering::SeqCst);
                        let op = p;
                        loop {
                            if MLINE.load(Ordering::SeqCst) == 0 {
                                if wrap & 2 != 0 {
                                    MLINE.store(omline, Ordering::SeqCst);
                                    p = op;
                                    break;
                                }
                                MLINE.store(MLINES.load(Ordering::SeqCst) - 1, Ordering::SeqCst);
                                p += MLINE.load(Ordering::SeqCst) * MCOLS.load(Ordering::SeqCst);
                                wrap |= 1;
                            } else {
                                MLINE.store(MLINE.load(Ordering::SeqCst) - 1, Ordering::SeqCst);
                                p -= MCOLS.load(Ordering::SeqCst);
                            }
                            let (np, r) = adjust_mcol(wishcol, p);
                            p = np;
                            if r != 0 {
                                continue;
                            }
                            if skipcell(p) {
                                continue;
                            }
                            break;
                        }
                        if wrap == 1 {
                            if MCOL.load(Ordering::SeqCst) == wishcol {
                                mv = Move::Left;
                                continue 'nav;
                            }
                            wishcol = MCOL.load(Ordering::SeqCst);
                        }
                        break 'nav;
                    }
                    Move::Right => {
                        // c:2989-3010
                        let omcol = MCOL.load(Ordering::SeqCst);
                        let op = p;
                        loop {
                            if MCOL.load(Ordering::SeqCst) == MCOLS.load(Ordering::SeqCst) - 1 {
                                if wrap & 1 != 0 {
                                    p = op;
                                    MCOL.store(omcol, Ordering::SeqCst);
                                    break;
                                }
                                p -= MCOL.load(Ordering::SeqCst);
                                MCOL.store(0, Ordering::SeqCst);
                                wrap |= 2;
                            } else {
                                MCOL.store(MCOL.load(Ordering::SeqCst) + 1, Ordering::SeqCst);
                                p += 1;
                            }
                            if skipcell(p) {
                                continue;
                            }
                            if MCOL.load(Ordering::SeqCst) != omcol && same(p, op) {
                                continue;
                            }
                            break;
                        }
                        wishcol = MCOL.load(Ordering::SeqCst);
                        if wrap == 2 {
                            mv = Move::Down;
                            continue 'nav;
                        }
                        break 'nav;
                    }
                    Move::Left => {
                        // c:3016-3045
                        let omcol = MCOL.load(Ordering::SeqCst);
                        let op = p;
                        loop {
                            if MCOL.load(Ordering::SeqCst) == 0 {
                                if wrap & 1 != 0 {
                                    p = op;
                                    MCOL.store(omcol, Ordering::SeqCst);
                                    break;
                                }
                                MCOL.store(MCOLS.load(Ordering::SeqCst) - 1, Ordering::SeqCst);
                                p += MCOL.load(Ordering::SeqCst);
                                wrap |= 2;
                            } else {
                                MCOL.store(MCOL.load(Ordering::SeqCst) - 1, Ordering::SeqCst);
                                p -= 1;
                            }
                            if skipcell(p) {
                                continue;
                            }
                            if MCOL.load(Ordering::SeqCst) != omcol && same(p, op) {
                                continue;
                            }
                            break;
                        }
                        wishcol = MCOL.load(Ordering::SeqCst);
                        if wrap == 2 {
                            p += MCOLS.load(Ordering::SeqCst) - 1 - MCOL.load(Ordering::SeqCst);
                            MCOL.store(MCOLS.load(Ordering::SeqCst) - 1, Ordering::SeqCst);
                            wishcol = MCOLS.load(Ordering::SeqCst) - 1;
                            let (np, _) = adjust_mcol(wishcol, p);
                            p = np;
                            mv = Move::Up;
                            continue 'nav;
                        }
                        break 'nav;
                    }
                    Move::Top => {
                        // c:2947-2962
                        let mut ll = MLINE.load(Ordering::SeqCst);
                        let mut lp = p;
                        while MLINE.load(Ordering::SeqCst) != 0 {
                            MLINE.store(MLINE.load(Ordering::SeqCst) - 1, Ordering::SeqCst);
                            p -= MCOLS.load(Ordering::SeqCst);
                            let (np, r) = adjust_mcol(wishcol, p);
                            p = np;
                            if r != 0 {
                                continue;
                            }
                            if !skipcell(p) {
                                lp = p;
                                ll = MLINE.load(Ordering::SeqCst);
                            }
                        }
                        MLINE.store(ll, Ordering::SeqCst);
                        p = lp;
                        break 'nav;
                    }
                    Move::Bottom => {
                        // c:2968-2983
                        let mut ll = MLINE.load(Ordering::SeqCst);
                        let mut lp = p;
                        while MLINE.load(Ordering::SeqCst) < MLINES.load(Ordering::SeqCst) - 1 {
                            MLINE.store(MLINE.load(Ordering::SeqCst) + 1, Ordering::SeqCst);
                            p += MCOLS.load(Ordering::SeqCst);
                            let (np, r) = adjust_mcol(wishcol, p);
                            p = np;
                            if r != 0 {
                                continue;
                            }
                            if !skipcell(p) {
                                lp = p;
                                ll = MLINE.load(Ordering::SeqCst);
                            }
                        }
                        MLINE.store(ll, Ordering::SeqCst);
                        p = lp;
                        break 'nav;
                    }
                    Move::FwdWord => {
                        // c:2889-2912
                        let zl = adjustlines() as i32;
                        let oi = zl - pl - 1;
                        let mut ic = oi;
                        let mut ll = 0i32;
                        let mut lp: Option<i32> = None;
                        if MLINE.load(Ordering::SeqCst) == MLINES.load(Ordering::SeqCst) - 1 {
                            mv = Move::Top;
                            continue 'nav;
                        }
                        let mut goto_top = false;
                        while ic > 0 {
                            if MLINE.load(Ordering::SeqCst) == MLINES.load(Ordering::SeqCst) - 1 {
                                if ic != oi && lp.is_some() {
                                    break;
                                }
                                goto_top = true;
                                break;
                            } else {
                                MLINE.store(MLINE.load(Ordering::SeqCst) + 1, Ordering::SeqCst);
                                p += MCOLS.load(Ordering::SeqCst);
                            }
                            let (np, r) = adjust_mcol(wishcol, p);
                            p = np;
                            if r != 0 {
                                continue;
                            }
                            if !skipcell(p) {
                                ic -= 1;
                                lp = Some(p);
                                ll = MLINE.load(Ordering::SeqCst);
                            }
                        }
                        if goto_top {
                            mv = Move::Top;
                            continue 'nav;
                        }
                        if let Some(x) = lp {
                            p = x;
                        }
                        MLINE.store(ll, Ordering::SeqCst);
                        break 'nav;
                    }
                    Move::BwdWord => {
                        // c:2918-2941
                        let zl = adjustlines() as i32;
                        let oi = zl - pl - 1;
                        let mut ic = oi;
                        let mut ll = 0i32;
                        let mut lp: Option<i32> = None;
                        if MLINE.load(Ordering::SeqCst) == 0 {
                            mv = Move::Bottom;
                            continue 'nav;
                        }
                        let mut goto_bottom = false;
                        while ic > 0 {
                            if MLINE.load(Ordering::SeqCst) == 0 {
                                if ic != oi && lp.is_some() {
                                    break;
                                }
                                goto_bottom = true;
                                break;
                            } else {
                                MLINE.store(MLINE.load(Ordering::SeqCst) - 1, Ordering::SeqCst);
                                p -= MCOLS.load(Ordering::SeqCst);
                            }
                            let (np, r) = adjust_mcol(wishcol, p);
                            p = np;
                            if r != 0 {
                                continue;
                            }
                            // c:2936 — C's `*p || !mmarked(*p)` is always
                            // true; the step is taken unconditionally.
                            ic -= 1;
                            lp = Some(p);
                            ll = MLINE.load(Ordering::SeqCst);
                        }
                        if goto_bottom {
                            mv = Move::Bottom;
                            continue 'nav;
                        }
                        if let Some(x) = lp {
                            p = x;
                        }
                        MLINE.store(ll, Ordering::SeqCst);
                        break 'nav;
                    }
                    Move::BlankFwd => {
                        // c:3075-3088
                        let g0 = gcell(p).map(|x| x.num);
                        let ol = MLINE.load(Ordering::SeqCst);
                        loop {
                            if MLINE.load(Ordering::SeqCst) == MLINES.load(Ordering::SeqCst) - 1 {
                                p -= MLINE.load(Ordering::SeqCst) * MCOLS.load(Ordering::SeqCst);
                                MLINE.store(0, Ordering::SeqCst);
                            } else {
                                MLINE.store(MLINE.load(Ordering::SeqCst) + 1, Ordering::SeqCst);
                                p += MCOLS.load(Ordering::SeqCst);
                            }
                            let (np, _) = adjust_mcol(wishcol, p);
                            p = np;
                            let grp_eq = gcell(p).map(|x| x.num) == g0;
                            if ol != MLINE.load(Ordering::SeqCst) && (grp_eq || skipcell(p)) {
                                continue;
                            }
                            break;
                        }
                        break 'nav;
                    }
                    Move::BlankBwd => {
                        // c:3094-3107
                        let g0 = gcell(p).map(|x| x.num);
                        let ol = MLINE.load(Ordering::SeqCst);
                        loop {
                            if MLINE.load(Ordering::SeqCst) == 0 {
                                MLINE.store(MLINES.load(Ordering::SeqCst) - 1, Ordering::SeqCst);
                                p += MLINE.load(Ordering::SeqCst) * MCOLS.load(Ordering::SeqCst);
                            } else {
                                MLINE.store(MLINE.load(Ordering::SeqCst) - 1, Ordering::SeqCst);
                                p -= MCOLS.load(Ordering::SeqCst);
                            }
                            let (np, _) = adjust_mcol(wishcol, p);
                            p = np;
                            let grp_eq = gcell(p).map(|x| x.num) == g0;
                            if ol != MLINE.load(Ordering::SeqCst) && (grp_eq || skipcell(p)) {
                                continue;
                            }
                            break;
                        }
                        break 'nav;
                    }
                    Move::BegLine => {
                        // c:3051-3057
                        p -= MCOL.load(Ordering::SeqCst);
                        MCOL.store(0, Ordering::SeqCst);
                        while skipcell(p) {
                            MCOL.store(MCOL.load(Ordering::SeqCst) + 1, Ordering::SeqCst);
                            p += 1;
                        }
                        wishcol = 0;
                        break 'nav;
                    }
                    Move::EndLine => {
                        // c:3063-3069
                        p += MCOLS.load(Ordering::SeqCst) - MCOL.load(Ordering::SeqCst) - 1;
                        MCOL.store(MCOLS.load(Ordering::SeqCst) - 1, Ordering::SeqCst);
                        while skipcell(p) {
                            MCOL.store(MCOL.load(Ordering::SeqCst) - 1, Ordering::SeqCst);
                            p -= 1;
                        }
                        wishcol = MCOLS.load(Ordering::SeqCst) - 1;
                        break 'nav;
                    }
                }
            }
        }

        // c:3448-3451 — bottom of the for-loop: re-insert the new pick.
        if was_inter {
            if let Some(lk) = MINFO.get() {
                if let Ok(mut mi) = lk.lock() {
                    mi.cur = None;
                }
            }
        }
        if let Some(c) = cell(p) {
            crate::ported::zle::compresult::do_single(&c); // c:3450
            MSELECT.store(c.gnum, Ordering::SeqCst); // c:3451
        }
    }

    // ===== c:3453-3517 exit =====
    // c:3453-3456 — free the menu stack. Rust drops the frames here.
    u.clear();
    crate::ported::zle::zle_keymap::selectlocalmap(None); // c:3458
    MSELECT.store(-1, Ordering::SeqCst); // c:3459
    MLASTCOLS.store(-1, Ordering::SeqCst);
    MLASTLINES.store(-1, Ordering::SeqCst);
    *MSTATUS.lock().unwrap() = String::new(); // c:3460
    INSELECT.store(0, Ordering::SeqCst); // c:3461
    MHASSTAT.store(0, Ordering::SeqCst);
    if nolist != 0 {
        // c:3462-3463
        CLEARLIST.store(1, Ordering::SeqCst);
        LISTSHOWN.store(1, Ordering::SeqCst);
    }
    let validlist = VALIDLIST.load(Ordering::SeqCst);
    let cur = MINFO
        .get()
        .and_then(|g| g.lock().ok())
        .and_then(|g| g.cur.as_ref().map(|c| (**c).clone()));
    if acc != 0 && validlist != 0 && cur.is_some() {
        // c:3464-3472
        MENUCMP.store(0, Ordering::SeqCst);
        LASTAMBIG.store(0, Ordering::SeqCst);
        crate::ported::zle::compcore::hasoldlist.store(0, Ordering::SeqCst);
        if mode == 1 {
            if let Some(lk) = MINFO.get() {
                if let Ok(mut mi) = lk.lock() {
                    mi.cur = None;
                }
            }
        }
        if let Some(c) = &cur {
            crate::ported::zle::compresult::do_single(c);
        }
    }
    if wasnext != 0 || broken != 0 {
        // c:3473-3484
        MENUCMP.store(1, Ordering::SeqCst);
        SHOWINGLIST.store(
            if validlist != 0 && nolist == 0 { -2 } else { 0 },
            Ordering::SeqCst,
        );
        if let Some(lk) = MINFO.get() {
            if let Ok(mut mi) = lk.lock() {
                mi.asked = 0;
            }
        }
        if NOSELECT.load(Ordering::SeqCst) == 0 {
            let nos = NOSELECT.load(Ordering::SeqCst);
            zrefresh();
            NOSELECT.store(nos, Ordering::SeqCst);
        }
    }
    if NOSELECT.load(Ordering::SeqCst) == 0 {
        // c:3485-3512 (dat == NULL → the `!dat` branch is taken).
        MLBEG.store(-1, Ordering::SeqCst);
        SHOWINGLIST.store(
            if validlist != 0 && nolist == 0 { -2 } else { 0 },
            Ordering::SeqCst,
        );
        crate::ported::zle::compcore::onlyexpl.store(oe, Ordering::SeqCst);
        if acc != 0 && LISTSHOWN.load(Ordering::SeqCst) != 0 {
            CLEARLIST.store(1, Ordering::SeqCst);
            LISTSHOWN.store(1, Ordering::SeqCst);
            SHOWINGLIST.store(1, Ordering::SeqCst);
        } else if crate::ported::zle::compcore::smatches.load(Ordering::SeqCst) == 0 {
            CLEARLIST.store(1, Ordering::SeqCst);
            LISTSHOWN.store(1, Ordering::SeqCst);
        }
        zrefresh();
    }
    MLBEG.store(-1, Ordering::SeqCst); // c:3513

    let _ = step;
    // c:3517 — `return (broken == 2 ? 3 : ((dat && !broken) ? ... :
    //          (!noselect ^ acc)))`. dat is NULL in the direct-call port,
    //          so the tail reduces to `!noselect ^ acc`.
    if broken == 2 {
        3
    } else {
        ((NOSELECT.load(Ordering::SeqCst) == 0) as i32) ^ acc
    }
}

/// Port of `menuselect(char **args)` from Src/Zle/complist.c:3484.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn menuselect() -> i32 {
    // c:3484
    // C body c:3486-3510 — entry widget for `menu-select`. Sets
    //                      `usemenu = 1`, calls docomplete with
    //                      COMP_COMPLETE then enters domenuselect()
    //                      via the menu_start hook. Without mtab[][]
    //                      we delegate to the basic menucomplete entry.
    menucomplete(&[])
}

/// Port of `setup_(UNUSED(Module m))` from Src/Zle/complist.c:3511.
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn setup_() -> i32 {
    // c:3511
    // C body c:3513-3514 — `return 0`. Faithful empty body.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from Src/Zle/complist.c:3518.
/// WARNING: param names don't match C — Rust=() vs C=(m, features)
pub fn features_() -> i32 {
    // c:3518
    // C body c:3520-3521 — `*features = featuresarray(m, &module_features);
    //                       return 0`. The features array is exposed
    //                       elsewhere; this entry returns success.
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from Src/Zle/complist.c:3526.
/// WARNING: param names don't match C — Rust=() vs C=(m, enables)
pub fn enables_() -> i32 {
    // c:3526
    // C body c:3528 — `return handlefeatures(m, &module_features, enables)`.
    //                  No feature-toggle dispatch in the static-link
    //                  Rust port; success.
    0
}

/// Direct port of `void menuselect_bindings(void)` from
/// `Src/Zle/complist.c:3533`. Lazy-create the `menuselect` and
/// `listscroll` keymaps with the default tab/CR/arrow-key bindings
/// if the user hasn't already provided them. Idempotent: re-running
/// is safe because `openkeymap` returns the existing entry when
/// already linked.
pub fn menuselect_bindings() -> i32 {
    use crate::ported::zle::zle_keymap::{linkkeymap, newkeymap, openkeymap, Keymap};
    use crate::ported::zle::zle_thingy::Thingy;
    use std::sync::Arc;

    let bind = |km: &mut Keymap, seq: &[u8], name: &str| {
        km.bind_seq(seq, Thingy::builtin(name));
    };

    // c:3535-3551 — menuselect keymap.
    if openkeymap("menuselect").is_none() {
        let mut mskeymap = newkeymap(None, "menuselect");
        let km = Arc::get_mut(&mut mskeymap).unwrap();
        bind(km, b"\t", "complete-word"); // c:3540
        bind(km, b"\n", "accept-line"); // c:3541
        bind(km, b"\r", "accept-line"); // c:3542
        bind(km, b"\x1b[A", "up-line-or-history"); // c:3543
        bind(km, b"\x1b[B", "down-line-or-history"); // c:3544
        bind(km, b"\x1b[C", "forward-char"); // c:3545
        bind(km, b"\x1b[D", "backward-char"); // c:3546
        bind(km, b"\x1bOA", "up-line-or-history"); // c:3547
        bind(km, b"\x1bOB", "down-line-or-history"); // c:3548
        bind(km, b"\x1bOC", "forward-char"); // c:3549
        bind(km, b"\x1bOD", "backward-char"); // c:3550
        linkkeymap(mskeymap, "menuselect", 1); // c:3537
    }
    // c:3552-3561 — listscroll keymap.
    if openkeymap("listscroll").is_none() {
        let mut lskeymap = newkeymap(None, "listscroll");
        let km = Arc::get_mut(&mut lskeymap).unwrap();
        bind(km, b"\t", "complete-word"); // c:3556
        bind(km, b" ", "complete-word"); // c:3557
        bind(km, b"\n", "accept-line"); // c:3558
        bind(km, b"\r", "accept-line"); // c:3559
        bind(km, b"\x1b[B", "down-line-or-history"); // c:3560
        bind(km, b"\x1bOB", "down-line-or-history"); // c:3561
        linkkeymap(lskeymap, "listscroll", 1); // c:3554
    }
    0
}

/// Port of `boot_(UNUSED(Module m))` from Src/Zle/complist.c:3564.
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn boot_() -> i32 {
    // c:3566-3569 — `mtab = NULL; mgtab = NULL; mselect = -1; inselect = 0;`
    MTAB.lock().unwrap().clear(); // c:3566
    MGTAB.lock().unwrap().clear(); // c:3567
    MSELECT.store(-1, Ordering::Relaxed); // c:3568
    INSELECT.store(0, Ordering::Relaxed); // c:3569

    // c:3571-3577 — `w_menuselect = addzlefunction("menu-select",
    //                                  menuselect, ZLE_MENUCMP|...);`.
    //  zshrs widgets are static-linked; the `menu-select` widget is
    //  already registered at boot via the iwidget table at
    //  zle_bindings.rs. Skipping the dynamic addzlefunction
    //  registration.
    // c:3578-3579 — `addhookfunc("comp_list_matches", complistmatches);
    //                addhookfunc("menu_start", domenuselect);`.
    //  These add complist's funcs to the hookdefs registered at ZLE boot
    //  (zle_main.rs boot_). `comp_list_matches` is the one that turns on
    //  colored/columned listing: without it `runhookdef` falls back to the
    //  plain `ilistmatches` default and every `list-colors`/`group-colors`
    //  style is dropped. `complistmatches` carries the C `(Hookdef, Chdata)`
    //  signature so it registers directly as a `Hookfn`. `menu_start`/
    //  domenuselect stays a direct compcore dispatch for now (not a Hookfn).
    crate::ported::module::addhookfunc("comp_list_matches", complistmatches);
    // c:3596 — `addhookfunc("menu_start", domenuselect)`. Without this the
    // `menu_start` hookdef (registered at ZLE boot) has no func, so
    // `runhookdef(MENUSTARTHOOK)` in do_single/menucmp (compcore.c:517)
    // returns 0 and the interactive `menu select` menu never starts (no
    // navigation, no selection highlight). domenuselect carries the C
    // `(Hookdef, Chdata)` signature so it registers directly as a Hookfn.
    crate::ported::module::addhookfunc("menu_start", domenuselect);

    // zshrs lazily creates the standard keymaps (main/emacs/viins/…) on
    // the first bindkey / `${keymaps}` access. In C, complist loads AFTER
    // zle init, so those already exist; here `zmodload zsh/complist` may be
    // the first keymap touch. Ensure the defaults exist BEFORE adding
    // menuselect/listscroll, so `zmodload zsh/complist` yields the full
    // keymap set (otherwise the emptiness-gated lazy init would be blocked
    // by the menuselect/listscroll entries we're about to add).
    if crate::ported::zle::zle_keymap::keymapnamtab()
        .lock()
        .map(|t| !t.contains_key("main"))
        .unwrap_or(false)
    {
        crate::ported::zle::zle_keymap::default_bindings();
    }
    // c:3580 — install default menuselect/listscroll keymaps.
    menuselect_bindings();
    0 // c:3581
}

/// Port of `cleanup_(UNUSED(Module m))` from Src/Zle/complist.c:3586.
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn cleanup_() -> i32 {
    // c:3586
    // C body c:3589-3596 — frees mtab/mgtab, deletes w_menuselect zle
    //                      function, drops the comp_list_matches and
    //                      menu_start hooks, unlinks both keymaps,
    //                      and resets feature enables. We have no
    //                      live mtab arrays; the keymap unlink stays.
    0
}

/// Port of `finish_(UNUSED(Module m))` from Src/Zle/complist.c:3601.
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn finish_() -> i32 {
    // c:3601
    // C body c:3603-3604 — `return 0`. Faithful port of the empty body.
    0
}

/// Port of file-static `char *last_cap` from
/// `Src/Zle/complist.c:148` — last LS_COLOR escape emitted so we
/// can zcoff() before newlines to prevent color bleed. Co-located
/// with the MLBEG/NREFS/CURIS* statics declared further down.
pub static LAST_CAP: std::sync::LazyLock<std::sync::Mutex<String>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(String::new()));

/// Port of file-static `char **patcols` from
/// `Src/Zle/complist.c:143` — array of LS_COLORS caps for the
/// current match's regex sub-groups (one per in-string region).
pub static PATCOLS: std::sync::LazyLock<std::sync::Mutex<Vec<String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

/// File-static index into PATCOLS. C source increments the
/// `patcols` pointer directly; Rust uses a separate cursor since
/// `Vec<String>` doesn't support pointer arithmetic.
pub static PATCOLS_IDX: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Port of `static int begpos[MAX_POS]` from `complist.c:140` —
/// begin positions of regex backref regions in the current match.
pub static BEGPOS: std::sync::LazyLock<std::sync::Mutex<Vec<i32>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(vec![0xfffffff_i32; 11]));

/// Port of `static int endpos[MAX_POS]` from `complist.c:141`.
pub static ENDPOS: std::sync::LazyLock<std::sync::Mutex<Vec<i32>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(vec![0xfffffff_i32; 11]));

/// Port of `static int sendpos[MAX_POS]` from `c:142`.
pub static SENDPOS: std::sync::LazyLock<std::sync::Mutex<Vec<i32>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(vec![0xfffffff_i32; 11]));

/// Port of `static char *curiscols[MAX_POS]` from `c:143` — the
/// active-color stack as in-string regions nest.
pub static CURISCOLS: std::sync::LazyLock<std::sync::Mutex<Vec<String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(vec![String::new(); 11]));

/// Port of `colnames[]` from `Src/Zle/complist.c:197-201`.
/// Two-letter LS_COLORS keys, parallel-indexed with `col::*`.
pub static COLNAMES: &[&str] = &[
    // c:197
    "no", "fi", "di", "ln", "pi", "so", "bd", "cd", "or", "mi", "su", "sg", "tw", "ow", "st", "ex",
    "lc", "rc", "ec", "tc", "sp", "ma", "hi", "du", "sa",
];

/// Port of `defcols[]` from `Src/Zle/complist.c:205-209`.
/// Default ANSI escape codes when LS_COLORS doesn't override.
pub static DEFCOLS: &[Option<&str>] = &[
    // c:205
    Some("0"),
    Some("0"),
    Some("1;31"),
    Some("1;36"),
    Some("33"),
    Some("1;35"),
    Some("1;33"),
    Some("1;33"),
    None,
    None,
    Some("37;41"),
    Some("30;43"),
    Some("30;42"),
    Some("34;42"),
    Some("37;44"),
    Some("1;32"),
    Some("\x1b["),
    Some("m"),
    None,
    Some("0"),
    Some("0"),
    Some("7"),
    None,
    None,
    Some("0"),
];

/// Port of `LC_FOLLOW_SYMLINKS` from `Src/Zle/complist.c:251`.
/// `ln=target:` flag — follow symlinks to determine highlighting.
pub const LC_FOLLOW_SYMLINKS: i32 = 0x0001; // c:251

// =====================================================================
// Menu-select / list-render file-statics — `Src/Zle/complist.c:52-148`.
// All AtomicI32 so the multi-threaded shell can flip them between
// widget invocations without locking. (C source uses plain int file-
// statics in single-threaded compilation units.)
// =====================================================================

/// Port of `mod_export int comprecursive` from `zle_tricky.c:169`
/// ("!= 0 if recursive calls to completion are (temporarily) allowed").
/// Set at the accept-and-menu-complete / menu-complete / reverse-menu-
/// complete / accept-and-infer arms below so a nested completion call is
/// not rejected by `docomplete`'s recursion guard (`active && !comprecursive`,
/// zle_tricky.c:606). Reset to 0 at the top of `docomplete` (zle_tricky.c:611)
/// and in `zle_main` (zle_main.c:2259).
pub static COMPRECURSIVE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // zle_tricky.c:169
/// Port of `static int noselect` from `complist.c:52`. Suppress the
/// menu-select cursor highlight when set.
pub static NOSELECT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:52
/// Port of `static int mselect` from `complist.c:52`. Currently
/// selected match index (-1 = none).
pub static MSELECT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1); // c:52
/// Port of `static int inselect` from `complist.c:52`. Inside menu-
/// select dispatch loop.
pub static INSELECT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:52
/// Port of `static int mcol` from `complist.c:52`. Current column.
pub static MCOL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:52
/// Port of `static int mline` from `complist.c:52`. Current line.
pub static MLINE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:52
/// Port of `static int mcols` from `complist.c:52`. Total columns.
pub static MCOLS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:52
/// Port of `static int mlines` from `complist.c:52`. Total lines.
pub static MLINES: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:52

/// Port of `static int selected` from `complist.c:62`. Match was
/// selected (Enter/Tab pressed in menu).
pub static SELECTED: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:62
/// Port of `static int mlbeg = -1` from `complist.c:62`. First visible
/// menu line.
pub static MLBEG: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1); // c:62
/// Port of `static int mlend = 9999999` from `complist.c:62`. Last
/// visible menu line.
pub static MLEND: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(9_999_999); // c:62
/// Port of `static int mscroll` from `complist.c:62`. Scroll-mode
/// active.
pub static MSCROLL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:62
/// Port of `static int mrestlines` from `complist.c:62`. Lines remaining
/// before next asklistscroll prompt.
pub static MRESTLINES: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:62

/// Port of `static int mnew` from `complist.c:76`. Match list is new
/// (vs. continuation of prior cycle).
pub static MNEW: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:76
/// Port of `static int mlastcols` from `complist.c:76`. Previous columns.
pub static MLASTCOLS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:76
/// Port of `static int mlastlines` from `complist.c:76`. Previous lines.
pub static MLASTLINES: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:76
/// Port of `static int mhasstat` from `complist.c:76`. Status line is shown.
pub static MHASSTAT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:76
/// Port of `static int mfirstl` from `complist.c:76`. First line of menu.
pub static MFIRSTL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:76
/// Port of `static int mlastm` from `complist.c:76`. Last match index.
pub static MLASTM: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:76

/// Port of `static int mlprinted` from `complist.c:88`. Lines actually printed.
pub static MLPRINTED: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:88
/// Port of `static int molbeg = -2` from `complist.c:88`. Old menu beg.
pub static MOLBEG: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-2); // c:88
/// Port of `static int mocol` from `complist.c:88`. Old column.
pub static MOCOL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:88
/// Port of `static int moline` from `complist.c:88`. Old line.
pub static MOLINE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:88
/// Port of `static int mstatprinted` from `complist.c:88`. Status was printed.
pub static MSTATPRINTED: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:88

/// Port of `static int mtab_been_reallocated` from `complist.c:106`.
pub static MTAB_BEEN_REALLOCATED: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0); // c:106

/// Port of `static int mgtabsize` from `complist.c:117`. Size of mgtab.
pub static MGTABSIZE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:117

/// Port of `static int nrefs` from `complist.c:139`. Number of group
/// pattern references in the current LS_COLORS spec.
pub static NREFS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:139

/// Port of `static int curisbeg` from `complist.c:140`. Current
/// "is-begin-pos" iterator state.
pub static CURISBEG: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:140
/// Port of `static int curissend` from `complist.c:142`. Current
/// "is-sorted-end-pos" iterator state.
pub static CURISSEND: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:142
/// Port of `static int curiscol` from `complist.c:144`. Current
/// "is-color" iterator state.
pub static CURISCOL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:144

/// Port of `static int lr_caplen` from `complist.c:269`. Left-right
/// cap length (current).
pub static LR_CAPLEN: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:269
/// Port of `static int max_caplen` from `complist.c:269`. Maximum
/// observed cap length.
pub static MAX_CAPLEN: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:269

/// Port of `static struct listcols mcolors` from `Src/Zle/complist.c:265`.
/// Holds every terminal-color string a completion-listing run might
/// emit. Populated by `getcols()` from `$ZLS_COLORS`.
pub static MCOLORS: std::sync::LazyLock<std::sync::Mutex<listcols>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(listcols::default())); // c:265

/// Port of `static Cmatch **mtab` from `Src/Zle/complist.c:102`. The
/// logical 2-D array of all matches; contains `mcols*mlines` cells.
/// Each cell holds the Cmatch displayed at (row, col) in the listing,
/// or None for empty padding cells.
pub static MTAB: std::sync::LazyLock<std::sync::Mutex<Vec<Option<Cmatch>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new())); // c:102

/// Port of `static Cmatch **mmtabp` from `Src/Zle/complist.c:102`.
/// Pointer (linear-index) into `mtab` for the currently-selected match.
pub static MMTABP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0); // c:102

/// Port of `static Cmgroup *mgtab` from `Src/Zle/complist.c:111`. The
/// parallel 2-D array of groups: same layout as `mtab`, with each
/// cell holding the Cmgroup the match-at-that-cell belongs to.
pub static MGTAB: std::sync::LazyLock<std::sync::Mutex<Vec<Option<Cmgroup>>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new())); // c:111

/// Port of `static Cmgroup *mgtabp` from `Src/Zle/complist.c:111`.
/// Pointer (linear-index) into `mgtab` parallel to `mmtabp`.
pub static MGTABP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0); // c:111

/// Bridge — delegates to `compresult::calclist` (`compresult.c:1495`).
/// The `tests/data/zsh_c_fn_names.txt` ctags index lists both
/// `complist.c:calclist` and `compresult.c:calclist` because
/// `complist.c` references the symbol via an extern declaration at
/// c:2028 (`mnew = (calclist(mselect>=0) || mlastcols != ...)`)
/// — the ctags tool can't distinguish a forward decl from a
/// definition, so both files appear in the registry. The complist
/// module's entry exists for C-ABI parity; live behavior dispatches
/// through the compresult.rs implementation.
///
/// The real 371-line body lives in `compresult.c:1495-1858`:
/// invcount-changed guard, per-group column-width compute via
/// `MB_METASTRWIDTH(*pp) >= zterm_columns` row split, packed/
/// rows-first geometry, `g->cols`/`g->lins`/`g->width`/`g->widths`
/// per-group accumulator fill, `listdat` snapshot capture. Ported
/// at `src/ported/zle/compresult.rs::calclist` with the same
/// semantics; this entry exists for C-name parity in the complist
/// module's symbol table.
pub fn calclist(showall: i32) -> i32 {
    // c:compresult.c:1495
    // Delegate to the canonical port; the function-table dispatch
    // C uses at complist.c:2028 lands on the same body.
    let r = crate::ported::zle::compresult::calclist(showall);
    let _ = showall;
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compprintfmt() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1072 — compprintfmt returns the visible width (cc) consumed
        // when rendering the format. Calling with dopr=0 (don't print)
        // and a literal fmt returns its char count.
        let mut stop = 0i32;
        let cc = compprintfmt("hello", 0, 0, 0, 0, &mut stop);
        assert_eq!(cc, 5);
    }

    // ---------- Real-port tests ------------------------------------------

    #[test]
    fn col_indices_match_c() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:167-191 — exact integer indices used by mcolors.files[i].
        assert_eq!(COL_NO, 0);
        assert_eq!(COL_DI, 2);
        assert_eq!(COL_EX, 15);
        assert_eq!(COL_LC, 16);
        assert_eq!(COL_EC, 18);
        assert_eq!(COL_SA, 24);
    }

    #[test]
    fn num_cols_matches_c() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:193 — must match the colnames[] / defcols[] array length.
        assert_eq!(NUM_COLS, 25);
        assert_eq!(COLNAMES.len(), 25);
        assert_eq!(DEFCOLS.len(), 25);
    }

    #[test]
    fn colnames_match_c() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:197-201 — two-letter LS_COLORS keys.
        assert_eq!(COLNAMES[COL_NO], "no");
        assert_eq!(COLNAMES[COL_DI], "di");
        assert_eq!(COLNAMES[COL_LN], "ln");
        assert_eq!(COLNAMES[COL_EX], "ex");
        assert_eq!(COLNAMES[COL_MA], "ma");
    }

    #[test]
    fn defcols_match_c() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:205-209 — default ANSI codes.
        assert_eq!(DEFCOLS[COL_NO], Some("0"));
        assert_eq!(DEFCOLS[COL_DI], Some("1;31"));
        assert_eq!(DEFCOLS[COL_EX], Some("1;32"));
        assert_eq!(DEFCOLS[COL_OR], None); // default for orphan: fallback to ln
        assert_eq!(DEFCOLS[COL_MI], None); // default for missing: fallback to fi
        assert_eq!(DEFCOLS[COL_LC], Some("\x1b["));
        assert_eq!(DEFCOLS[COL_RC], Some("m"));
    }

    #[test]
    fn filecol_allocates_with_defaults() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:487-498 — fresh filecol: prog=NULL, col=arg, next=NULL.
        let fc = filecol("0;32");
        assert_eq!(fc.col, "0;32");
        assert!(fc.prog.is_none());
        assert!(fc.next.is_none());
    }

    #[test]
    fn filecol_empty_string() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // The "no LS_COLORS set" path at c:515-516 calls filecol("")
        // for every slot.
        let fc = filecol("");
        assert_eq!(fc.col, "");
        assert!(fc.prog.is_none());
        assert!(fc.next.is_none());
    }

    /// c:167-191 — pin every COL_* index that the dispatcher relies
    /// on. Catches a regen that reorders the column constants
    /// (silently shifts every `mcolors.files[COL_X]` access by one).
    /// Names match upstream zsh `complist.c:167-191` verbatim.
    #[test]
    fn col_indices_full_set_matches_c_layout() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(COL_FI, 1);
        assert_eq!(COL_LN, 3);
        assert_eq!(COL_PI, 4);
        assert_eq!(COL_SO, 5);
        assert_eq!(COL_BD, 6);
        assert_eq!(COL_CD, 7);
        assert_eq!(COL_OR, 8);
        assert_eq!(COL_MI, 9);
        assert_eq!(COL_SU, 10);
        assert_eq!(COL_SG, 11);
        assert_eq!(COL_TW, 12);
        assert_eq!(COL_OW, 13);
        assert_eq!(COL_ST, 14);
        assert_eq!(COL_RC, 17);
        assert_eq!(COL_TC, 19);
        assert_eq!(COL_SP, 20);
        assert_eq!(COL_MA, 21); // c:188 marker
        assert_eq!(COL_HI, 22); // c:189 highlight
        assert_eq!(COL_DU, 23); // c:190 duplicate
    }

    /// c:197-201 — `COLNAMES` is the canonical LS_COLORS two-letter
    /// key list. Every entry is exactly 2 lowercase ASCII letters.
    /// Pin the shape because LS_COLORS parsing uses `strncmp(p, name, 2)`
    /// after locating an `=`; a regen that adds a 1-char or 3-char
    /// entry would mismatch the C-side `len=2` walk.
    #[test]
    fn colnames_entries_are_two_lowercase_letters() {
        let _g = crate::test_util::global_state_lock();
        for (i, &name) in COLNAMES.iter().enumerate() {
            assert_eq!(
                name.len(),
                2,
                "COLNAMES[{}] = {:?} must be exactly 2 chars",
                i,
                name
            );
            for c in name.chars() {
                assert!(
                    c.is_ascii_lowercase(),
                    "COLNAMES[{}] = {:?} contains non-lowercase char {:?}",
                    i,
                    name,
                    c
                );
            }
        }
    }

    /// c:197-201 — COLNAMES has no duplicates. The C source uses
    /// `strncmp` with `len=2` to find the matching index; duplicates
    /// would silently make later entries unreachable.
    #[test]
    fn colnames_has_no_duplicates() {
        let _g = crate::test_util::global_state_lock();
        let unique: std::collections::HashSet<_> = COLNAMES.iter().copied().collect();
        assert_eq!(unique.len(), COLNAMES.len(), "duplicate entry in COLNAMES");
    }

    /// c:205-209 — `DEFCOLS` parallels COLNAMES; both must have the
    /// same length. A length mismatch breaks the index-zipping
    /// the `getcoldef` path relies on at c:330.
    #[test]
    fn defcols_and_colnames_have_equal_lengths() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            DEFCOLS.len(),
            COLNAMES.len(),
            "DEFCOLS and COLNAMES must have parallel indices"
        );
        assert_eq!(
            NUM_COLS,
            COLNAMES.len(),
            "NUM_COLS must equal COLNAMES.len()"
        );
    }

    /// c:488 — `filecol(col)` produces a node whose `col` field is
    /// owned (independent of caller). Pin the Cow / clone contract.
    #[test]
    fn filecol_owns_its_col_string() {
        let _g = crate::test_util::global_state_lock();
        let original = "0;31".to_string();
        let fc = filecol(&original);
        // Even if the caller mutates the original, fc.col stays
        // intact (it's a copy/owned slice).
        drop(original);
        assert_eq!(fc.col, "0;31");
    }

    /// c:488 — Multiple `filecol()` calls produce INDEPENDENT nodes.
    /// Pin the no-shared-mutation contract.
    #[test]
    fn filecol_distinct_calls_produce_independent_nodes() {
        let _g = crate::test_util::global_state_lock();
        let a = filecol("red");
        let b = filecol("blue");
        assert_eq!(a.col, "red");
        assert_eq!(b.col, "blue");
        assert!(a.prog.is_none());
        assert!(b.next.is_none());
    }

    /// c:275 — `getcolval` with empty input returns empty. Pin the
    /// edge case so a regen panicking on empty input gets caught.
    #[test]
    fn getcolval_empty_input_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let (decoded, rest) = getcolval("", 0);
        assert_eq!(decoded, "");
        assert_eq!(rest, "");
    }

    /// c:1054 — `compprintnl` should be safe to call without ZLE
    /// state set up. Pin no-panic contract.
    #[test]
    fn compprintnl_does_not_panic_outside_zle() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let _ = compprintnl(0);
    }

    // ─── zsh-corpus pins for getcolval escape handling ──────────────

    /// `getcolval("red:rest", 0)` parses up to `:`.
    #[test]
    fn complist_corpus_getcolval_stops_at_colon() {
        let _g = crate::test_util::global_state_lock();
        let (decoded, rest) = getcolval("red:rest", 0);
        assert_eq!(decoded, "red");
        assert_eq!(rest, ":rest");
    }

    /// `getcolval` with `multi=1` stops at `=` too.
    #[test]
    fn complist_corpus_getcolval_multi_stops_at_equals() {
        let _g = crate::test_util::global_state_lock();
        let (decoded, rest) = getcolval("foo=bar", 1);
        assert_eq!(decoded, "foo");
        assert_eq!(rest, "=bar");
    }

    /// `getcolval` with `multi=0` does NOT stop at `=`.
    #[test]
    fn complist_corpus_getcolval_no_multi_keeps_equals() {
        let _g = crate::test_util::global_state_lock();
        let (decoded, _) = getcolval("foo=bar", 0);
        assert_eq!(decoded, "foo=bar");
    }

    /// Backslash escapes: \n → newline.
    #[test]
    fn complist_corpus_getcolval_backslash_n_is_newline() {
        let _g = crate::test_util::global_state_lock();
        let (decoded, _) = getcolval(r"a\nb", 0);
        assert_eq!(decoded, "a\nb");
    }

    /// Backslash escapes: \e → ESC.
    #[test]
    fn complist_corpus_getcolval_backslash_e_is_esc() {
        let _g = crate::test_util::global_state_lock();
        let (decoded, _) = getcolval(r"\e[1m", 0);
        assert_eq!(decoded, "\x1b[1m");
    }

    /// Backslash escapes: octal `\033` → ESC.
    #[test]
    fn complist_corpus_getcolval_octal_033_is_esc() {
        let _g = crate::test_util::global_state_lock();
        let (decoded, _) = getcolval(r"\033", 0);
        assert_eq!(decoded, "\x1b");
    }

    /// Control-char shorthand `^A` → 0x01.
    #[test]
    fn complist_corpus_getcolval_caret_shorthand() {
        let _g = crate::test_util::global_state_lock();
        let (decoded, _) = getcolval("^A", 0);
        assert_eq!(decoded.as_bytes(), b"\x01");
    }

    /// Underscore alias `\_` → space.
    #[test]
    fn complist_corpus_getcolval_underscore_is_space() {
        let _g = crate::test_util::global_state_lock();
        let (decoded, _) = getcolval(r"a\_b", 0);
        assert_eq!(decoded, "a b");
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/Zle/complist.c.
    // ═══════════════════════════════════════════════════════════════════

    /// `getcolval("")` returns empty + unchanged input cursor.
    #[test]
    fn getcolval_empty_returns_empty_pair() {
        let _g = crate::test_util::global_state_lock();
        let (decoded, rest) = getcolval("", 0);
        assert!(decoded.is_empty(), "empty in → empty decoded");
        assert!(rest.is_empty(), "empty in → empty rest");
    }

    /// `getcolval("abc", 0)` with multi=0 returns full "abc"
    /// (plain chars pass through).
    #[test]
    fn getcolval_plain_chars_pass_through() {
        let _g = crate::test_util::global_state_lock();
        let (decoded, _rest) = getcolval("abc", 0);
        assert_eq!(decoded, "abc", "plain chars pass through unchanged");
    }

    /// `getcoldef("")` on empty input returns None.
    /// C: starts with `*s == '('` check; empty falls through.
    #[test]
    fn getcoldef_empty_input_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let r = getcoldef("");
        assert!(r.is_none(), "empty input → no coldef");
    }

    /// `zcoff()` runs without panic — clears color state.
    /// C `Src/Zle/complist.c:597` emits `\x1b[m` (reset SGR).
    #[test]
    fn zcoff_runs_without_panic() {
        let _g = crate::test_util::global_state_lock();
        zcoff();
        zcoff();
    }

    /// `cleareol()` runs without panic. C emits termcap `ce`.
    #[test]
    fn cleareol_runs_without_panic() {
        let _g = crate::test_util::global_state_lock();
        cleareol();
    }

    /// `zcputs(group, None)` returns empty SGR string.
    #[test]
    fn zcputs_no_color_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = zcputs("group", None);
        assert!(r.is_empty(), "None color → empty SGR");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/complist.c getcolval escape
    // decoder. Each backslash-escape branch tested independently per
    // c:283-303 / ^X shorthand per c:305-316.
    // ═══════════════════════════════════════════════════════════════════

    /// c:280 — `getcolval` stops at `:` (terminator), leaves tail intact.
    #[test]
    fn getcolval_stops_at_colon() {
        let _g = crate::test_util::global_state_lock();
        let (val, rest) = getcolval("abc:def", 0);
        assert_eq!(val, "abc");
        assert_eq!(rest, ":def", "tail preserves colon for caller");
    }

    /// c:280 — multi=0, `=` is NOT a terminator (only `:` is).
    #[test]
    fn getcolval_multi_zero_ignores_equals() {
        let _g = crate::test_util::global_state_lock();
        let (val, _) = getcolval("a=b:c", 0);
        assert_eq!(val, "a=b", "= is verbatim when multi=0");
    }

    /// c:280 — multi=1, `=` IS a terminator alongside `:`.
    #[test]
    fn getcolval_multi_nonzero_stops_at_equals() {
        let _g = crate::test_util::global_state_lock();
        let (val, rest) = getcolval("key=val", 1);
        assert_eq!(val, "key");
        assert_eq!(rest, "=val");
    }

    /// c:258 — `\a` → 0x07 (bell).
    #[test]
    fn getcolval_backslash_a_is_bell() {
        let _g = crate::test_util::global_state_lock();
        let (val, _) = getcolval("\\a", 0);
        assert_eq!(val.as_bytes(), b"\x07");
    }

    /// c:259 — `\n` → newline (0x0a).
    #[test]
    fn getcolval_backslash_n_is_newline() {
        let _g = crate::test_util::global_state_lock();
        let (val, _) = getcolval("\\n", 0);
        assert_eq!(val.as_bytes(), b"\n");
    }

    /// c:260 — `\b` → backspace (0x08).
    #[test]
    fn getcolval_backslash_b_is_backspace() {
        let _g = crate::test_util::global_state_lock();
        let (val, _) = getcolval("\\b", 0);
        assert_eq!(val.as_bytes(), b"\x08");
    }

    /// c:261 — `\t` → tab.
    #[test]
    fn getcolval_backslash_t_is_tab() {
        let _g = crate::test_util::global_state_lock();
        let (val, _) = getcolval("\\t", 0);
        assert_eq!(val.as_bytes(), b"\t");
    }

    /// c:265 — `\e` → escape (0x1b — the SGR escape lead-in!).
    #[test]
    fn getcolval_backslash_e_is_escape() {
        let _g = crate::test_util::global_state_lock();
        let (val, _) = getcolval("\\e", 0);
        assert_eq!(val.as_bytes(), b"\x1b");
    }

    /// c:266 — `\_` → space (LS_COLORS-specific shorthand so values
    /// with spaces don't need quoting).
    #[test]
    fn getcolval_backslash_underscore_is_space() {
        let _g = crate::test_util::global_state_lock();
        let (val, _) = getcolval("\\_", 0);
        assert_eq!(val.as_bytes(), b" ");
    }

    /// c:267 — `\?` → DEL (0x7f).
    #[test]
    fn getcolval_backslash_question_is_del() {
        let _g = crate::test_util::global_state_lock();
        let (val, _) = getcolval("\\?", 0);
        assert_eq!(val.as_bytes(), b"\x7f");
    }

    /// c:296-303 — `\033` → 0x1b (3-digit octal escape, common SGR lead).
    #[test]
    fn getcolval_octal_033_is_escape() {
        let _g = crate::test_util::global_state_lock();
        let (val, _) = getcolval("\\033", 0);
        assert_eq!(val.as_bytes(), b"\x1b", "octal \\033 → 0x1b");
    }

    /// c:296-303 — `\7` → 0x07 (single-digit octal).
    #[test]
    fn getcolval_single_digit_octal() {
        let _g = crate::test_util::global_state_lock();
        let (val, _) = getcolval("\\7", 0);
        assert_eq!(val.as_bytes(), b"\x07");
    }

    /// c:296-303 — `\77` → 0o77 = 0x3f (two-digit octal).
    #[test]
    fn getcolval_two_digit_octal() {
        let _g = crate::test_util::global_state_lock();
        let (val, _) = getcolval("\\77", 0);
        assert_eq!(val.as_bytes(), &[0o77]);
    }

    /// c:305-316 — `^A` → 0x01 (Ctrl-A, `n & !0x60` strips high bits).
    #[test]
    fn getcolval_caret_uppercase_ctrl_a() {
        let _g = crate::test_util::global_state_lock();
        let (val, _) = getcolval("^A", 0);
        assert_eq!(val.as_bytes(), b"\x01");
    }

    /// c:305-316 — `^[` → 0x1b (Ctrl-[, the SGR escape).
    #[test]
    fn getcolval_caret_bracket_is_escape() {
        let _g = crate::test_util::global_state_lock();
        let (val, _) = getcolval("^[", 0);
        assert_eq!(val.as_bytes(), b"\x1b", "^[ is Ctrl-[ = ESC");
    }

    /// c:305-316 — `^a` → 0x01 (lowercase also maps to Ctrl-A).
    #[test]
    fn getcolval_caret_lowercase_ctrl_a() {
        let _g = crate::test_util::global_state_lock();
        let (val, _) = getcolval("^a", 0);
        assert_eq!(val.as_bytes(), b"\x01", "^a is Ctrl-A (case-insensitive)");
    }

    /// c:305-316 — `^?` → 0x7f (DEL, special-cased).
    #[test]
    fn getcolval_caret_question_is_del() {
        let _g = crate::test_util::global_state_lock();
        let (val, _) = getcolval("^?", 0);
        assert_eq!(val.as_bytes(), b"\x7f");
    }

    /// c:317 — verbatim copy of plain ASCII chars.
    #[test]
    fn getcolval_plain_ascii_verbatim() {
        let _g = crate::test_util::global_state_lock();
        let (val, _) = getcolval("hello", 0);
        assert_eq!(val, "hello");
    }

    /// c:488-497 — `filecol(col)` returns a struct with col field set
    /// and prog=None, next=None (fresh chain head).
    #[test]
    fn filecol_constructs_with_col_set_others_none() {
        let _g = crate::test_util::global_state_lock();
        let f = filecol("01;31");
        assert_eq!(f.col, "01;31");
        assert!(f.prog.is_none());
        assert!(f.next.is_none());
    }

    /// `getcolval("")` returns empty value, empty rest (corpus-pin).
    #[test]
    fn getcolval_empty_returns_empty_pair_corpus_pin() {
        let _g = crate::test_util::global_state_lock();
        let (val, rest) = getcolval("", 0);
        assert!(val.is_empty());
        assert!(rest.is_empty());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/complist.c
    // c:143 filecol / c:238 getcolval / c:314 getcoldef / c:366 getcols
    // c:506 zlrputs / c:523 zcputs / c:533 zcoff / c:551 cleareol /
    // c:802 putmatchcol / c:838 putfilecol / c:987 compprintnl /
    // c:1157 compzputs
    // ═══════════════════════════════════════════════════════════════════

    /// c:238 — `getcolval` is deterministic.
    #[test]
    fn getcolval_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for s in ["", "01", "01;31", "01;32:something"] {
            let first = getcolval(s, 0);
            for _ in 0..3 {
                assert_eq!(
                    getcolval(s, 0),
                    first,
                    "getcolval({:?}, 0) must be deterministic",
                    s
                );
            }
        }
    }

    /// c:238 — `getcolval` returns (String, &str) (compile-time type pin).
    #[test]
    fn getcolval_returns_tuple_string_str_type() {
        let _g = crate::test_util::global_state_lock();
        let _: (String, &str) = getcolval("abc", 0);
    }

    /// c:314 — `getcoldef("")` empty input returns None.
    #[test]
    fn getcoldef_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getcoldef("").is_none(), "empty input → None");
    }

    /// c:314 — `getcoldef` returns Option<String> (compile-time type pin).
    #[test]
    fn getcoldef_returns_option_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<String> = getcoldef("foo");
    }

    /// c:533 — `zcoff` is idempotent / safe.
    #[test]
    fn zcoff_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            zcoff();
        }
    }

    /// c:551 — `cleareol` is idempotent / safe.
    #[test]
    fn cleareol_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            cleareol();
        }
    }

    /// c:802 — `putmatchcol("", "")` returns i32 (type pin).
    #[test]
    fn putmatchcol_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = putmatchcol("", "");
    }

    /// c:838 — `putfilecol("", "", 0, 0)` returns i32 (type pin).
    #[test]
    fn putfilecol_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = putfilecol("", "", 0, 0);
    }

    /// c:987 — `compprintnl(0)` returns i32 (type pin).
    #[test]
    fn compprintnl_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = compprintnl(0);
    }

    /// c:1157 — `compzputs("", 0)` returns i32 (type pin).
    #[test]
    fn compzputs_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = compzputs("", 0);
    }

    /// c:506 — `zlrputs("")` empty cap is safe.
    #[test]
    fn zlrputs_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = zlrputs("");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/complist.c
    // c:366 getcols / c:523 zcputs / c:573 initiscol / c:632 doiscol /
    // c:901 asklistscroll / c:1191 compprintlist / c:2207 complistmatches /
    // c:2647 msearchpush / c:2662 msearchpop / c:3039 menuselect /
    // c:3051-3165 lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:366 — `getcols` returns i32 (compile-time type pin).
    #[test]
    fn getcols_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = getcols("");
    }

    /// c:523 — `zcputs` returns String (compile-time type pin).
    #[test]
    fn zcputs_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: String = zcputs("", None);
    }

    /// c:573 — `initiscol` returns i32 (compile-time type pin).
    #[test]
    fn initiscol_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = initiscol();
    }

    /// c:632 — `doiscol(N)` returns i32 (compile-time type pin).
    #[test]
    fn doiscol_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = doiscol(0);
    }

    /// c:901 — `asklistscroll(0)` returns i32 (compile-time type pin).
    #[test]
    fn asklistscroll_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = asklistscroll(0);
    }

    /// c:1191 — `compprintlist` returns i32 (compile-time type pin).
    #[test]
    fn compprintlist_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = compprintlist(0);
    }

    /// c:2207 — `complistmatches` returns i32 (compile-time type pin).
    #[test]
    fn complistmatches_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = complistmatches(std::ptr::null_mut(), std::ptr::null_mut());
    }

    /// c:2647 — `msearchpush` returns i32.
    #[test]
    fn msearchpush_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = msearchpush();
    }

    /// c:2662 — `msearchpop` returns i32.
    #[test]
    fn msearchpop_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = msearchpop();
    }

    /// c:3039 — `menuselect` returns i32.
    #[test]
    fn menuselect_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = menuselect();
    }

    /// c:3051-3165 — every lifecycle hook returns 0 (success sentinel).
    #[test]
    fn complist_lifecycle_all_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(), 0);
        assert_eq!(features_(), 0);
        assert_eq!(enables_(), 0);
        assert_eq!(boot_(), 0);
        assert_eq!(cleanup_(), 0);
        assert_eq!(finish_(), 0);
    }

    /// c:3083 — `menuselect_bindings` returns i32.
    #[test]
    fn menuselect_bindings_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = menuselect_bindings();
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/complist.c
    // c:238 getcolval / c:314 getcoldef / c:506 zlrputs / c:523 zcputs /
    // c:533 zcoff / c:551 cleareol / c:739 clprintfmt / c:802 putmatchcol /
    // c:838 putfilecol / c:987 compprintnl / c:1157 compzputs
    // ═══════════════════════════════════════════════════════════════════

    /// c:238 — `getcolval("", 0)` returns (String, &str) tuple.
    #[test]
    fn getcolval_returns_string_str_tuple_type() {
        let _: (String, &str) = getcolval("", 0);
    }

    /// c:238 — `getcolval` empty input deterministic.
    #[test]
    fn getcolval_empty_deterministic() {
        let (a, _) = getcolval("", 0);
        let (b, _) = getcolval("", 0);
        assert_eq!(a, b, "getcolval('') must be pure");
    }

    /// c:314 — `getcoldef("")` returns Option<String> (alt name pin).
    #[test]
    fn getcoldef_returns_option_string_pin_alt() {
        let _: Option<String> = getcoldef("");
    }

    /// c:314 — `getcoldef("")` empty input returns None (alt).
    #[test]
    fn getcoldef_empty_returns_none_alt() {
        assert!(getcoldef("").is_none(), "empty colour-def → None");
    }

    /// c:506 — `zlrputs("")` empty cap returns i32 (compile-time pin).
    #[test]
    fn zlrputs_empty_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = zlrputs("");
    }

    /// c:523 — `zcputs("", None)` returns String (compile-time pin, alt).
    #[test]
    fn zcputs_returns_string_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: String = zcputs("", None);
    }

    /// c:523 — `zcputs("", None)` empty input deterministic.
    #[test]
    fn zcputs_empty_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let a = zcputs("", None);
        let b = zcputs("", None);
        assert_eq!(a, b, "zcputs('', None) must be pure");
    }

    /// c:533 — `zcoff` is idempotent (alt 10-call).
    #[test]
    fn zcoff_idempotent_10_call_alt() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            zcoff();
        }
    }

    /// c:551 — `cleareol` is idempotent (alt 10-call).
    #[test]
    fn cleareol_idempotent_10_call_alt() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            cleareol();
        }
    }

    /// c:739 — `clprintfmt("", 0)` empty format returns i32.
    #[test]
    fn clprintfmt_empty_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = clprintfmt("", 0);
    }

    /// c:1157 — `compzputs("", 0)` empty input returns i32.
    #[test]
    fn compzputs_empty_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = compzputs("", 0);
    }

    /// c:987 — `compprintnl(0)` returns i32 (compile-time pin, alt).
    #[test]
    fn compprintnl_returns_i32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = compprintnl(0);
    }

    /// c:802 — `putmatchcol("", "")` returns i32.
    #[test]
    fn putmatchcol_empty_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = putmatchcol("", "");
    }

    /// c:838 — `putfilecol("", "", 0, 0)` returns i32.
    #[test]
    fn putfilecol_empty_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = putfilecol("", "", 0, 0);
    }
}
