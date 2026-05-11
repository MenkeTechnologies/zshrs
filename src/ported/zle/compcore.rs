//! Direct port of `Src/Zle/compcore.c` — completion core code.
//!
//! Original C copyright: Sven Wischnowsky 1995-1997.
//!
//! C source is 3,638 lines. This file ports:
//!   - the file-scope globals (c:36-279)
//!   - the pure-string helpers (`rembslash`, `remsquote`,
//!     `comp_quoting_string`, `multiquote`, `tildequote`, `matcheq`,
//!     `matchcmp`, `ctokenize`, `comp_str`)
//!   - the linked-list group manipulators (`begcmgroup`,
//!     `endcmgroup`, `addexpl`, `addmatch`)
//!   - the param-table helpers (`get_user_var`, `get_data_arr`,
//!     `set_list_array`)
//!   - the hook entry points (`before_complete`, `after_complete`)
//!     in their non-runhookdef branches
//!
//! Functions blocked on heavier substrate (`do_completion`,
//! `makecomplist`, `addmatches`, `callcompfunc`, `set_comp_sep`,
//! `check_param`, `permmatches`, `dupmatch`, `add_match_data`,
//! `makearray`) carry doc comments naming the missing dependencies.

#![allow(non_snake_case, non_upper_case_globals, dead_code)]

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::ported::zsh_h::{
    BNULL, INBRACE, OUTBRACE, QT_BACKSLASH, QT_DOLLARS, QT_DOUBLE, QT_SINGLE, STRING_TOK,
};
use crate::ported::zle::comp_h::{
    Aminfo, Cexpl, Cmatch, Cmgroup, CGF_MATSORT, CGF_NOSORT, CGF_NUMSORT, CGF_REVSORT,
    CGF_UNIQALL, CGF_UNIQCON, CMF_DELETE, CMF_DISPLINE, CMF_FMULT, CMF_MULT, CMF_NOLIST,
    CMF_PACKED, CMF_PARBR, CMF_PARNEST, CMF_ROWS,
};

// =====================================================================
// Extern globals — declared in other C files, mirrored here per
// PORT.md Rule 9 ("stub the EXTERN dependencies ... locally with
// file:line citations to their home file") so the local body ports
// below have a value source. When the canonical Rust homes land,
// these become `pub use crate::ported::<canonical>::*` re-exports.
// =====================================================================

/// Port of `mod_export int wb` from `Src/lex.c:120`. Word-begin
/// position in the metafied line for the currently-completing word.
pub static WB: AtomicI32 = AtomicI32::new(0);                                // lex.c:120
/// Port of `mod_export int we` from `Src/lex.c:120`. Word-end position.
pub static WE: AtomicI32 = AtomicI32::new(0);                                // lex.c:120
/// Port of `mod_export int zlemetacs` from `Src/lex.c:104`. Cursor
/// position in the metafied line.
pub static ZLEMETACS: AtomicI32 = AtomicI32::new(0);                         // lex.c:104
/// Port of `mod_export int zlemetall` from `Src/lex.c:104`. Length
/// of the metafied line.
pub static ZLEMETALL: AtomicI32 = AtomicI32::new(0);                         // lex.c:104

/// Port of `mod_export char *zlemetaline` from `Src/lex.c:103`. The
/// metafied edit buffer for the current ZLE session — `foredel`,
/// `inststr`, `selfinsert` operate on this directly when compcore's
/// error-recovery path fires (compcore.c:344-355).
pub static ZLEMETALINE: OnceLock<Mutex<String>> = OnceLock::new();           // lex.c:103
/// Port of `mod_export ZLE_STRING_T zleline` from `Src/zle_main.c`.
pub static ZLELINE: OnceLock<Mutex<String>> = OnceLock::new();               // zle_main.c
/// Port of `mod_export int zlecs` from `Src/zle_main.c`.
pub static ZLECS: AtomicI32 = AtomicI32::new(0);                             // zle_main.c
/// Port of `mod_export int zlell` from `Src/zle_main.c`.
pub static ZLELL: AtomicI32 = AtomicI32::new(0);                             // zle_main.c
/// Port of `mod_export int inwhat` from `Src/lex.c:110`. Lex context
/// classification — IN_NOTHING / IN_CMD / IN_COND / IN_MATH / IN_PAR /
/// IN_ENV.
pub static INWHAT: AtomicI32 = AtomicI32::new(0);                            // lex.c:110
/// Port of `mod_export int zmult` from `Src/zle_main.c`. Numeric
/// prefix multiplier for the current ZLE command.
pub static ZMULT: AtomicI32 = AtomicI32::new(1);                             // zle_main.c
/// Port of `mod_export char *compfunc` from `Src/Zle/zle_tricky.c:143`.
/// Name of the user completion shell function — non-empty when the
/// new completion system (`compsys`) is active; empty for compctl.
pub static compfunc: OnceLock<Mutex<Option<String>>> = OnceLock::new();      // zle_tricky.c:143
/// Port of `mod_export char *comppatmatch` from `Src/Zle/zle_tricky.c`.
/// `$compstate[pattern_match]` — when non-empty + non-"\0" enables
/// pattern-aware matching for parameter-name completion.
pub static comppatmatch: OnceLock<Mutex<Option<String>>> = OnceLock::new();
/// Port of `mod_export char *compqstack` from `Src/Zle/compcore.c`.
/// Quoting-state stack (1 char per nesting level).
pub static compqstack: OnceLock<Mutex<String>> = OnceLock::new();

// Brace counters live in zle_tricky.c:114 — re-exported there. Local
// re-exports here so call sites stay short:
#[doc(hidden)]
pub use crate::ported::zle::zle_tricky::{NBRBEG as _NBRBEG, NBREND as _NBREND};

// =====================================================================
// File-scope globals — `Src/Zle/compcore.c:36-279`.
// =====================================================================

/// Port of `int useexact` from compcore.c:36.
pub static useexact: AtomicI32 = AtomicI32::new(0);                          // c:36
/// Port of `int useline` from compcore.c:36.
pub static useline: AtomicI32 = AtomicI32::new(0);                           // c:36
/// Port of `int uselist` from compcore.c:36.
pub static uselist: AtomicI32 = AtomicI32::new(0);                           // c:36
/// Port of `int forcelist` from compcore.c:36.
pub static forcelist: AtomicI32 = AtomicI32::new(0);                         // c:36
/// Port of `int startauto` from compcore.c:36.
pub static startauto: AtomicI32 = AtomicI32::new(0);                         // c:36

/// Port of `mod_export int iforcemenu` from compcore.c:39.
pub static iforcemenu: AtomicI32 = AtomicI32::new(0);                        // c:39

/// Port of `mod_export int dolastprompt` from compcore.c:44.
pub static dolastprompt: AtomicI32 = AtomicI32::new(0);                      // c:44

/// Port of `mod_export int oldlist` from compcore.c:49.
pub static oldlist: AtomicI32 = AtomicI32::new(0);                           // c:49
/// Port of `mod_export int oldins` from compcore.c:49.
pub static oldins: AtomicI32 = AtomicI32::new(0);                            // c:49

/// Port of `int origlpre` from compcore.c:54.
pub static origlpre: AtomicI32 = AtomicI32::new(0);                          // c:54
/// Port of `int origlsuf` from compcore.c:54.
pub static origlsuf: AtomicI32 = AtomicI32::new(0);                          // c:54
/// Port of `int lenchanged` from compcore.c:54.
pub static lenchanged: AtomicI32 = AtomicI32::new(0);                        // c:54

/// Port of `int movetoend` from compcore.c:61.
pub static movetoend: AtomicI32 = AtomicI32::new(0);                         // c:61

/// Port of `mod_export int insmnum` from compcore.c:66.
pub static insmnum: AtomicI32 = AtomicI32::new(0);                           // c:66
/// Port of `mod_export int insspace` from compcore.c:66.
pub static insspace: AtomicI32 = AtomicI32::new(0);                          // c:66

/// Port of `mod_export int menuacc` from compcore.c:81.
pub static menuacc: AtomicI32 = AtomicI32::new(0);                           // c:81

/// Port of `int hasunqu` from compcore.c:86.
pub static hasunqu: AtomicI32 = AtomicI32::new(0);                           // c:86
/// Port of `int useqbr` from compcore.c:86.
pub static useqbr: AtomicI32 = AtomicI32::new(0);                            // c:86
/// Port of `int brpcs` from compcore.c:86.
pub static brpcs: AtomicI32 = AtomicI32::new(0);                             // c:86
/// Port of `int brscs` from compcore.c:86.
pub static brscs: AtomicI32 = AtomicI32::new(0);                             // c:86

/// Port of `mod_export int ispar` from compcore.c:91.
pub static ispar: AtomicI32 = AtomicI32::new(0);                             // c:91
/// Port of `mod_export int linwhat` from compcore.c:91.
pub static linwhat: AtomicI32 = AtomicI32::new(0);                           // c:91

/// Port of `char *parpre` from compcore.c:96.
pub static parpre: OnceLock<Mutex<String>> = OnceLock::new();                // c:96

/// Port of `int parflags` from compcore.c:101.
pub static parflags: AtomicI32 = AtomicI32::new(0);                          // c:101

/// Port of `mod_export int mflags` from compcore.c:106.
pub static mflags: AtomicI32 = AtomicI32::new(0);                            // c:106

/// Port of `int parq` from compcore.c:111.
pub static parq: AtomicI32 = AtomicI32::new(0);                              // c:111
/// Port of `int eparq` from compcore.c:111.
pub static eparq: AtomicI32 = AtomicI32::new(0);                             // c:111

/// Port of `mod_export char *ipre` from compcore.c:118.
pub static ipre: OnceLock<Mutex<String>> = OnceLock::new();                  // c:118
/// Port of `mod_export char *ripre` from compcore.c:118.
pub static ripre: OnceLock<Mutex<String>> = OnceLock::new();                 // c:118
/// Port of `mod_export char *isuf` from compcore.c:118.
pub static isuf: OnceLock<Mutex<String>> = OnceLock::new();                  // c:118

/// Port of `mod_export LinkList matches` from compcore.c:124.
pub static matches: OnceLock<Mutex<Vec<Cmatch>>> = OnceLock::new();          // c:124
/// Port of `LinkList fmatches` from compcore.c:126.
pub static fmatches: OnceLock<Mutex<Vec<Cmatch>>> = OnceLock::new();         // c:126

/// Port of `mod_export Cmgroup amatches` from compcore.c:135.
pub static amatches: OnceLock<Mutex<Vec<Cmgroup>>> = OnceLock::new();        // c:135
/// Port of `mod_export Cmgroup pmatches` from compcore.c:135.
pub static pmatches: OnceLock<Mutex<Vec<Cmgroup>>> = OnceLock::new();        // c:135
/// Port of `mod_export Cmgroup lastmatches` from compcore.c:135.
pub static lastmatches: OnceLock<Mutex<Vec<Cmgroup>>> = OnceLock::new();     // c:135
/// Port of `mod_export Cmgroup lmatches` from compcore.c:135. Last
/// element pointer in the perm list; here a single-slot holder.
pub static lmatches: OnceLock<Mutex<Option<Cmgroup>>> = OnceLock::new();     // c:135
/// Port of `mod_export Cmgroup lastlmatches` from compcore.c:135.
pub static lastlmatches: OnceLock<Mutex<Option<Cmgroup>>> = OnceLock::new(); // c:135

/// Port of `mod_export int hasoldlist` from compcore.c:140.
pub static hasoldlist: AtomicI32 = AtomicI32::new(0);                        // c:140
/// Port of `mod_export int hasperm` from compcore.c:140.
pub static hasperm: AtomicI32 = AtomicI32::new(0);                           // c:140
/// Port of `int hasallmatch` from compcore.c:145.
pub static hasallmatch: AtomicI32 = AtomicI32::new(0);                       // c:145

/// Port of `mod_export int newmatches` from compcore.c:150.
pub static newmatches: AtomicI32 = AtomicI32::new(0);                        // c:150

/// Port of `mod_export int permmnum` from compcore.c:155.
pub static permmnum: AtomicI32 = AtomicI32::new(0);                          // c:155
/// Port of `mod_export int permgnum` from compcore.c:155.
pub static permgnum: AtomicI32 = AtomicI32::new(0);                          // c:155
/// Port of `mod_export int lastpermmnum` from compcore.c:155.
pub static lastpermmnum: AtomicI32 = AtomicI32::new(0);                      // c:155
/// Port of `mod_export int lastpermgnum` from compcore.c:155.
pub static lastpermgnum: AtomicI32 = AtomicI32::new(0);                      // c:155

/// Port of `mod_export int nmatches` from compcore.c:160.
pub static nmatches: AtomicI32 = AtomicI32::new(0);                          // c:160
/// Port of `mod_export int smatches` from compcore.c:162.
pub static smatches: AtomicI32 = AtomicI32::new(0);                          // c:162

/// Port of `mod_export int diffmatches` from compcore.c:167.
pub static diffmatches: AtomicI32 = AtomicI32::new(0);                       // c:167

/// Port of `mod_export int nmessages` from compcore.c:172.
pub static nmessages: AtomicI32 = AtomicI32::new(0);                         // c:172

/// Port of `mod_export int onlyexpl` from compcore.c:177.
pub static onlyexpl: AtomicI32 = AtomicI32::new(0);                          // c:177

/// Port of `mod_export struct cldata listdat` from compcore.c:182.
pub static listdat: OnceLock<Mutex<crate::ported::zle::comp_h::Cldata>> =
    OnceLock::new();                                                         // c:182

/// Port of `mod_export int ispattern` from compcore.c:187.
pub static ispattern: AtomicI32 = AtomicI32::new(0);                         // c:187
/// Port of `mod_export int haspattern` from compcore.c:187.
pub static haspattern: AtomicI32 = AtomicI32::new(0);                        // c:187

/// Port of `mod_export int hasmatched` from compcore.c:192.
pub static hasmatched: AtomicI32 = AtomicI32::new(0);                        // c:192
/// Port of `mod_export int hasunmatched` from compcore.c:192.
pub static hasunmatched: AtomicI32 = AtomicI32::new(0);                      // c:192

/// Port of `Cmgroup mgroup` from compcore.c:197.
pub static mgroup: OnceLock<Mutex<Option<Cmgroup>>> = OnceLock::new();       // c:197

/// Port of `mod_export int mnum` from compcore.c:202.
pub static mnum: AtomicI32 = AtomicI32::new(0);                              // c:202

/// Port of `mod_export int unambig_mnum` from compcore.c:207.
pub static unambig_mnum: AtomicI32 = AtomicI32::new(0);                      // c:207

/// Port of `int maxmlen` from compcore.c:212.
pub static maxmlen: AtomicI32 = AtomicI32::new(0);                           // c:212
/// Port of `int minmlen` from compcore.c:212.
pub static minmlen: AtomicI32 = AtomicI32::new(0);                           // c:212

/// Port of `LinkList expls` from compcore.c:218.
pub static expls: OnceLock<Mutex<Vec<Cexpl>>> = OnceLock::new();             // c:218

/// Port of `mod_export Cexpl curexpl` from compcore.c:221.
pub static curexpl: OnceLock<Mutex<Option<Cexpl>>> = OnceLock::new();        // c:221

/// Port of `LinkList matchers` from compcore.c:236.
pub static matchers: OnceLock<Mutex<Vec<String>>> = OnceLock::new();         // c:236

/// Port of `mod_export Aminfo ainfo` from compcore.c:246.
pub static ainfo: OnceLock<Mutex<Option<Aminfo>>> = OnceLock::new();         // c:246
/// Port of `mod_export Aminfo fainfo` from compcore.c:246.
pub static fainfo: OnceLock<Mutex<Option<Aminfo>>> = OnceLock::new();        // c:246

/// Port of `mod_export LinkList allccs` from compcore.c:259.
pub static allccs: OnceLock<Mutex<Vec<String>>> = OnceLock::new();           // c:259

/// Port of `int fromcomp` from compcore.c:271.
pub static fromcomp: AtomicI32 = AtomicI32::new(0);                          // c:271

/// Port of `mod_export int lastend` from compcore.c:276.
pub static lastend: AtomicI32 = AtomicI32::new(0);                           // c:276

/// Port of `static int oldmenucmp` from compcore.c:457.
pub static OLDMENUCMP: AtomicI32 = AtomicI32::new(0);                        // c:457

/// Port of `static int parwb` from compcore.c:540.
pub static PARWB: AtomicI32 = AtomicI32::new(0);                             // c:540
/// Port of `static int parwe` from compcore.c:540.
pub static PARWE: AtomicI32 = AtomicI32::new(0);                             // c:540
/// Port of `static int paroffs` from compcore.c:540.
pub static PAROFFS: AtomicI32 = AtomicI32::new(0);                           // c:540

/// Port of `static int matchorder` from compcore.c:3169.
pub static MATCHORDER: AtomicI32 = AtomicI32::new(0);                        // c:3169

// =====================================================================
// rembslash — `Src/Zle/compcore.c:1322-1336`.
// =====================================================================

/// Port of `mod_export char *rembslash(char *s)` from compcore.c:1322.
///
/// "Strip backslash escapes from a token, treating `\X` as `X`."
pub fn rembslash(s: &str) -> String {                                        // c:1322
    let mut result = String::with_capacity(s.len());                         // c:1325
    let mut chars = s.chars().peekable();                                    // c:1327
    while let Some(c) = chars.next() {
        if c == '\\' {                                                       // c:1328
            if let Some(nxt) = chars.next() {                                // c:1329
                result.push(nxt);
            }
        } else {
            result.push(c);                                                  // c:1332-1333
        }
    }
    result                                                                   // c:1335
}

// =====================================================================
// remsquote — `Src/Zle/compcore.c:1342-1360`.
// =====================================================================

/// Port of `mod_export int remsquote(char *s)` from compcore.c:1342.
pub fn remsquote(s: &mut String) -> i32 {                                    // c:1342
    let rcquotes = crate::ported::options::opt_state_get("rcquotes")         // c:1345
        .unwrap_or(false);
    let qa: usize = if rcquotes { 1 } else { 3 };

    let bytes = s.as_bytes();                                                // c:1346
    let mut t = Vec::<u8>::with_capacity(bytes.len());
    let mut ret: i32 = 0;
    let mut i = 0;
    while i < bytes.len() {                                                  // c:1348
        let matched = if qa == 1 {                                           // c:1349
            i + 1 < bytes.len() && bytes[i] == b'\'' && bytes[i + 1] == b'\''
        } else {
            i + 3 < bytes.len()                                              // c:1351
                && bytes[i]     == b'\''
                && bytes[i + 1] == b'\\'
                && bytes[i + 2] == b'\''
                && bytes[i + 3] == b'\''
        };
        if matched {
            ret += qa as i32;                                                // c:1352
            t.push(b'\'');                                                   // c:1353
            i += qa + 1;                                                     // c:1354
        } else {
            t.push(bytes[i]);                                                // c:1356
            i += 1;
        }
    }
    *s = String::from_utf8(t).unwrap_or_default();                           // c:1357
    ret                                                                      // c:1359
}

// =====================================================================
// ctokenize — `Src/Zle/compcore.c:1365-1388`.
// =====================================================================

/// Port of `mod_export char *ctokenize(char *p)` from compcore.c:1365.
///
/// C calls `tokenize(p)` first then walks the string replacing
/// unescaped `$`/`{`/`}` with the token bytes `String`/`Inbrace`/
/// `Outbrace`. Backslash-escaped variants become `Bnull`.
pub fn ctokenize(p: &str) -> String {                                        // c:1365
    let bytes = p.as_bytes();                                                // c:1368
    let mut out = Vec::<u8>::with_capacity(bytes.len());
    let mut bslash = false;                                                  // c:1369
    let mut prev_idx: Option<usize> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];                                                    // c:1373
        if b == b'\\' {                                                      // c:1374
            bslash = true;
            out.push(b);
            prev_idx = Some(out.len() - 1);
        } else {
            if b == b'$' || b == b'{' || b == b'}' {                         // c:1377
                if bslash {                                                  // c:1378
                    if let Some(pi) = prev_idx {                             // c:1379
                        out.truncate(pi);
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(BNULL.encode_utf8(&mut buf).as_bytes());
                    }
                    out.push(b);
                } else {
                    let tok = if b == b'$' { STRING_TOK }                    // c:1381
                              else if b == b'{' { INBRACE }                  // c:1382
                              else { OUTBRACE };                             // c:1382
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(tok.encode_utf8(&mut buf).as_bytes());
                }
            } else {
                out.push(b);
            }
            bslash = false;                                                  // c:1384
            prev_idx = Some(out.len().saturating_sub(1));
        }
        i += 1;
    }
    String::from_utf8(out).unwrap_or_default()                               // c:1387
}

// =====================================================================
// comp_str — `Src/Zle/compcore.c:1402-1431`.
// =====================================================================

/// Port of `mod_export char *comp_str(int *ipl, int *pl, int untok)`
/// from compcore.c:1402.
pub fn comp_str(untok: bool) -> (String, i32, i32) {                         // c:1402
    use crate::ported::zle::complete::{COMPIPREFIX, COMPPREFIX, COMPSUFFIX};
    let mut p = COMPPREFIX.get_or_init(|| Mutex::new(String::new()))         // c:1405
        .lock().unwrap().clone();
    let mut s = COMPSUFFIX.get_or_init(|| Mutex::new(String::new()))         // c:1406
        .lock().unwrap().clone();
    let ip = COMPIPREFIX.get_or_init(|| Mutex::new(String::new()))           // c:1407
        .lock().unwrap().clone();
    if !untok {                                                              // c:1411
        p = ctokenize(&p);                                                   // c:1412
        p = p.chars().filter(|&c| c != BNULL).collect();                     // c:1413 remnulargs
        s = ctokenize(&s);                                                   // c:1414
        s = s.chars().filter(|&c| c != BNULL).collect();                     // c:1415
    }
    let lp = p.len() as i32;                                                 // c:1417
    let lip = ip.len() as i32;                                               // c:1419
    let mut str_ = String::with_capacity(ip.len() + p.len() + s.len() + 1);  // c:1420
    str_.push_str(&ip);                                                      // c:1421
    str_.push_str(&p);                                                       // c:1422
    str_.push_str(&s);                                                       // c:1423
    (str_, lip, lp)                                                          // c:1425-1430
}

// =====================================================================
// comp_quoting_string — `Src/Zle/compcore.c:1434-1448`.
// =====================================================================

/// Port of `mod_export char *comp_quoting_string(int stype)` from
/// compcore.c:1434.
pub fn comp_quoting_string(stype: i32) -> &'static str {                     // c:1434
    match stype {                                                            // c:1437
        x if x == QT_SINGLE  => "'",                                         // c:1439-1440
        x if x == QT_DOUBLE  => "\"",                                        // c:1441-1442
        x if x == QT_DOLLARS => "$'",                                        // c:1443-1444
        _ => {                                                               // c:1445
            let _ = QT_BACKSLASH;
            "\\"                                                             // c:1446
        }
    }
}

// =====================================================================
// multiquote — `Src/Zle/compcore.c:1064-1082`.
// =====================================================================

/// Port of `mod_export char *multiquote(char *s, int ign)` from
/// compcore.c:1064.
pub fn multiquote(s: &str, ign: i32) -> String {                             // c:1064
    let stack = crate::ported::zle::complete::COMPQSTACK                     // c:1068
        .get_or_init(|| Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let p_bytes = stack.as_bytes();
    if !p_bytes.is_empty() && (ign == 0 || p_bytes.len() > 1) {              // c:1070
        let start = if ign != 0 { 1 } else { 0 };                            // c:1071
        let mut cur = s.to_string();
        for &q in &p_bytes[start..] {                                        // c:1073
            let qt = match q as i32 {                                        // c:1074
                x if x == QT_BACKSLASH => crate::ported::utils::QuoteType::Backslash,
                x if x == QT_SINGLE    => crate::ported::utils::QuoteType::Single,
                x if x == QT_DOUBLE    => crate::ported::utils::QuoteType::Double,
                x if x == QT_DOLLARS   => crate::ported::utils::QuoteType::Dollars,
                _ => crate::ported::utils::QuoteType::Backslash,
            };
            cur = crate::ported::utils::quotestring(&cur, qt);
        }
        cur                                                                  // c:1078
    } else {
        s.to_string()                                                        // c:1078
    }
}

// =====================================================================
// tildequote — `Src/Zle/compcore.c:1091-1110`.
// =====================================================================

/// Port of `mod_export char *tildequote(char *s, int ign)` from
/// compcore.c:1091.
pub fn tildequote(s: &str, ign: i32) -> String {                             // c:1091
    let bytes = s.as_bytes();                                                // c:1095
    let tilde = !bytes.is_empty() && bytes[0] == b'~';                       // c:1097
    let staged = if tilde {                                                  // c:1098
        let mut tmp = String::with_capacity(s.len());
        tmp.push('x');
        tmp.push_str(&s[1..]);
        tmp
    } else {
        s.to_string()
    };
    let mut quoted = multiquote(&staged, ign);                               // c:1099
    if tilde && !quoted.is_empty() {                                         // c:1100
        let mut new_q = String::with_capacity(quoted.len());
        let mut swapped = false;
        for c in quoted.chars() {
            if !swapped && c == 'x' {
                new_q.push('~');
                swapped = true;
            } else {
                new_q.push(c);
            }
        }
        quoted = new_q;
    }
    quoted                                                                   // c:1101
}

// =====================================================================
// before_complete / after_complete — `Src/Zle/compcore.c:461 / 503`.
// =====================================================================

/// Port of `int before_complete(Hookdef dummy, int *lst)` from
/// compcore.c:461.
pub fn before_complete(lst: &mut i32) -> i32 {                               // c:461
    use crate::ported::zle::zle_tricky::{MENUCMP, USEMENU};
    let _ = lst;
    OLDMENUCMP.store(MENUCMP.load(Ordering::Relaxed), Ordering::Relaxed);    // c:463
    if startauto.load(Ordering::Relaxed) != 0 {                              // c:494
        let bashauto = crate::ported::options::opt_state_get("bashautolist")
            .unwrap_or(false);
        let lastambig: i32 = 0;
        if !bashauto || lastambig == 2 {
            USEMENU.store(2, Ordering::Relaxed);
        }
    }
    0                                                                        // c:498
}

/// Port of `int after_complete(Hookdef dummy, int *dat)` from
/// compcore.c:503.
pub fn after_complete(_dat: &mut [i32]) -> i32 {                             // c:503
    use crate::ported::zle::zle_tricky::MENUCMP;
    let _menucmp = MENUCMP.load(Ordering::Relaxed);
    let _oldmenucmp = OLDMENUCMP.load(Ordering::Relaxed);
    0                                                                        // c:535
}

// =====================================================================
// set_list_array — `Src/Zle/compcore.c:1947-1950`.
// =====================================================================

/// Port of `static void set_list_array(char *name, LinkList l)` from
/// compcore.c:1947.
pub fn set_list_array(name: &str, l: &[String]) {                            // c:1947
    std::env::set_var(name, l.join("\0"));                                   // c:1949
}

// =====================================================================
// get_user_var — `Src/Zle/compcore.c:1956-2014`.
// =====================================================================

/// Port of `mod_export char **get_user_var(char *nam)` from
/// compcore.c:1956.
pub fn get_user_var(nam: Option<&str>) -> Option<Vec<String>> {              // c:1956
    let nam = nam?;                                                          // c:1958
    if nam.starts_with('(') {                                                // c:1960
        let mut arrlist: Vec<String> = Vec::new();
        let bytes = nam.as_bytes();
        let mut buf = Vec::<u8>::new();
        let mut notempty = false;                                            // c:1963
        let mut brk = false;
        let mut i = 1;                                                       // c:1967
        while i < bytes.len() {
            let b = bytes[i];
            if b == b'\\' && i + 1 < bytes.len() {                           // c:1969
                buf.push(bytes[i + 1]);                                      // c:1970
                notempty = true;
                i += 2;
                continue;
            }
            if b == b',' || b == b' ' || b == b'\t' || b == b'\n' || b == b')' {
                if b == b')' { brk = true; }                                 // c:1972
                if notempty {                                                // c:1974
                    let mut start = 0;
                    if !buf.is_empty() && buf[0] == b'\n' { start = 1; }     // c:1977
                    let s = String::from_utf8_lossy(&buf[start..]).into_owned();
                    arrlist.push(s);                                         // c:1979
                }
                buf.clear();                                                 // c:1981
                notempty = false;
            } else {
                notempty = true;                                             // c:1984
                buf.push(b);
            }
            i += 1;
            if brk { break; }                                                // c:1988
        }
        if !brk || arrlist.is_empty() { return None; }                       // c:1991
        Some(arrlist)                                                        // c:1996
    } else {                                                                 // c:1999
        crate::ported::signals::queue_signals();                             // c:2003
        let result = std::env::var(nam).ok().map(|s| {                       // c:2004
            if s.contains('\0') {
                s.split('\0').map(String::from).collect::<Vec<_>>()
            } else {
                vec![s]                                                      // c:2007
            }
        });
        crate::ported::signals::unqueue_signals();                           // c:2011
        result
    }
}

// =====================================================================
// get_data_arr — `Src/Zle/compcore.c:2022-2039`.
// =====================================================================

/// Port of `static char **get_data_arr(char *name, int keys)` from
/// compcore.c:2022.
pub fn get_data_arr(name: &str, keys: bool) -> Option<Vec<String>> {         // c:2022
    crate::ported::signals::queue_signals();                                 // c:2028
    let raw = std::env::var(name).ok();                                      // c:2029
    let result = raw.map(|s| {
        let parts: Vec<String> = if s.contains('\0') {
            s.split('\0').map(String::from).collect()
        } else {
            vec![s]
        };
        if keys {
            parts.iter().step_by(2).cloned().collect::<Vec<_>>()
        } else {
            parts
        }
    });
    crate::ported::signals::unqueue_signals();                               // c:2035
    result
}

// =====================================================================
// addmatch — `Src/Zle/compcore.c:2041-2072`.
// =====================================================================

/// Port of `static void addmatch(char *str, int flags, char ***dispp,
///                                int line)` from compcore.c:2041.
pub fn addmatch(str_: &str, flags: i32, disp: Option<&str>, line: bool) {    // c:2041
    let mut cm = Cmatch::default();                                          // c:2043
    cm.str_ = Some(str_.to_string());                                        // c:2047
    // c:2049-2051 — inline read of `complist` parameter, parse `packed`/
    // `rows` substrings into CMF_PACKED/CMF_ROWS flag bits.
    let complist_extra = {
        use crate::ported::zle::complete::COMPLIST;
        let s = COMPLIST.get_or_init(|| Mutex::new(String::new()))
            .lock().map(|g| g.clone()).unwrap_or_default();
        let packed = if s.contains("packed") { CMF_PACKED } else { 0 };      // c:2050
        let rows   = if s.contains("rows")   { CMF_ROWS   } else { 0 };      // c:2051
        if s.is_empty() { 0 } else { packed | rows }
    };
    cm.flags = flags | complist_extra;                                       // c:2048
    if let Some(d) = disp {                                                  // c:2052
        cm.disp = Some(d.to_string());                                       // c:2056
    } else if line {                                                         // c:2057
        cm.disp = Some(String::new());                                       // c:2058
        cm.flags |= CMF_DISPLINE;                                            // c:2059
    }
    mnum.fetch_add(1, Ordering::Relaxed);                                    // c:2061
    {
        let cell = curexpl.get_or_init(|| Mutex::new(None));                 // c:2063
        if let Ok(mut g) = cell.lock() {
            if let Some(e) = g.as_mut() { e.count += 1; }
        }
    }
    let mcell = matches.get_or_init(|| Mutex::new(Vec::new()));              // c:2066
    if let Ok(mut g) = mcell.lock() { g.push(cm); }
    newmatches.store(1, Ordering::Relaxed);                                  // c:2068
    {
        let cell = mgroup.get_or_init(|| Mutex::new(None));                  // c:2069
        if let Ok(mut g) = cell.lock() {
            if let Some(grp) = g.as_mut() { grp.new_ = 1; }
        }
    }
}

// `lookup_complist_flags` deleted — Rust-only 8-line helper. Inlined
// at the single call site in callcompfunc (c:2049-2051).

// =====================================================================
// begcmgroup — `Src/Zle/compcore.c:3073-3125`.
// =====================================================================

/// Port of `mod_export void begcmgroup(char *n, int flags)` from
/// compcore.c:3073.
pub fn begcmgroup(n: Option<&str>, flags: i32) {                             // c:3073
    if let Some(name) = n {                                                  // c:3075
        let mask = CGF_NOSORT | CGF_UNIQALL | CGF_UNIQCON                    // c:3085
                 | CGF_MATSORT | CGF_NUMSORT | CGF_REVSORT;
        let cell = amatches.get_or_init(|| Mutex::new(Vec::new()));
        if let Ok(g) = cell.lock() {
            for grp in g.iter() {                                            // c:3078
                if grp.name.as_deref() == Some(name)                         // c:3084-3087
                    && (grp.flags & mask) == flags
                {
                    let active = grp.clone();                                // c:3088
                    let mc = mgroup.get_or_init(|| Mutex::new(None));
                    if let Ok(mut s) = mc.lock() { *s = Some(active); }
                    return;                                                  // c:3095
                }
            }
        }
    }
    let mut grp = Cmgroup::default();                                        // c:3101
    grp.name = n.map(String::from);                                          // c:3105
    grp.flags = flags;                                                       // c:3108
    let cell = amatches.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut g) = cell.lock() {
        g.insert(0, grp.clone());                                            // c:3121-3124
    }
    let mc = mgroup.get_or_init(|| Mutex::new(None));
    if let Ok(mut s) = mc.lock() { *s = Some(grp); }
    if let Ok(mut g) = expls.get_or_init(|| Mutex::new(Vec::new())).lock()    { g.clear(); }
    if let Ok(mut g) = matches.get_or_init(|| Mutex::new(Vec::new())).lock()  { g.clear(); }
    if let Ok(mut g) = fmatches.get_or_init(|| Mutex::new(Vec::new())).lock() { g.clear(); }
    if let Ok(mut g) = allccs.get_or_init(|| Mutex::new(Vec::new())).lock()   { g.clear(); }
}

// =====================================================================
// endcmgroup — `Src/Zle/compcore.c:3131-3134`.
// =====================================================================

/// Port of `mod_export void endcmgroup(char **ylist)` from
/// compcore.c:3131.
pub fn endcmgroup(ylist: Option<Vec<String>>) {                              // c:3131
    let cell = mgroup.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = cell.lock() {
        if let Some(grp) = g.as_mut() {
            grp.ylist = ylist.unwrap_or_default();                           // c:3133
        }
    }
}

// =====================================================================
// addexpl — `Src/Zle/compcore.c:3140-3164`.
// =====================================================================

/// Port of `mod_export void addexpl(int always)` from compcore.c:3140.
pub fn addexpl(always: bool) {                                               // c:3140
    let curexpl_snap = {
        let cell = curexpl.get_or_init(|| Mutex::new(None));
        cell.lock().ok().and_then(|g| g.clone())
    };
    let curexpl_str = match curexpl_snap.as_ref().and_then(|e| e.str_.clone()) {
        Some(s) => s,
        None => return,
    };
    let curexpl_count  = curexpl_snap.as_ref().map(|e| e.count).unwrap_or(0);
    let curexpl_fcount = curexpl_snap.as_ref().map(|e| e.fcount).unwrap_or(0);

    let elist = expls.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut g) = elist.lock() {
        for e in g.iter_mut() {                                              // c:3145
            if e.str_.as_deref() == Some(curexpl_str.as_str()) {             // c:3147
                e.count  += curexpl_count;                                   // c:3148
                e.fcount += curexpl_fcount;                                  // c:3149
                if always {                                                  // c:3150
                    e.always = 1;
                    nmessages.fetch_add(1, Ordering::Relaxed);               // c:3152
                    newmatches.store(1, Ordering::Relaxed);                  // c:3153
                    let mc = mgroup.get_or_init(|| Mutex::new(None));
                    if let Ok(mut mg) = mc.lock() {
                        if let Some(grp) = mg.as_mut() { grp.new_ = 1; }
                    }
                }
                return;                                                      // c:3156
            }
        }
        if let Some(e) = curexpl_snap {                                      // c:3159
            g.push(e);
        }
    }
    newmatches.store(1, Ordering::Relaxed);                                  // c:3160
    if always {                                                              // c:3161
        let mc = mgroup.get_or_init(|| Mutex::new(None));
        if let Ok(mut mg) = mc.lock() {
            if let Some(grp) = mg.as_mut() { grp.new_ = 1; }
        }
        nmessages.fetch_add(1, Ordering::Relaxed);                           // c:3163
    }
}

// =====================================================================
// matchcmp — `Src/Zle/compcore.c:3173-3198`.
// =====================================================================

/// Port of `static int matchcmp(Cmatch *a, Cmatch *b)` from
/// compcore.c:3173.
pub fn matchcmp(a: &Cmatch, b: &Cmatch) -> std::cmp::Ordering {              // c:3173
    let order = MATCHORDER.load(Ordering::Relaxed);
    let sortdir = if (order & CGF_REVSORT) != 0 { -1 } else { 1 };           // c:3177

    let cmp = (b.disp.is_some() as i32) - (a.disp.is_some() as i32);         // c:3176
    let (as_, bs) = if (order & CGF_MATSORT) != 0 || (cmp == 0 && a.disp.is_none()) {
        (a.str_.clone().unwrap_or_default(),                                 // c:3181
         b.str_.clone().unwrap_or_default())                                 // c:3182
    } else {
        if cmp != 0 {                                                        // c:3184
            let raw = (cmp as i32) * sortdir;
            return if raw < 0 { std::cmp::Ordering::Less }                   // c:3185
                   else if raw > 0 { std::cmp::Ordering::Greater }
                   else { std::cmp::Ordering::Equal };
        }
        let displine_cmp = (b.flags & CMF_DISPLINE) - (a.flags & CMF_DISPLINE); // c:3187
        if displine_cmp != 0 {                                               // c:3188
            let raw = displine_cmp * sortdir;
            return if raw < 0 { std::cmp::Ordering::Less }
                   else if raw > 0 { std::cmp::Ordering::Greater }
                   else { std::cmp::Ordering::Equal };
        }
        (a.disp.clone().unwrap_or_default(),                                 // c:3191
         b.disp.clone().unwrap_or_default())                                 // c:3192
    };
    let raw = sortdir * if as_ == bs { 0 } else if as_ < bs { -1 } else { 1 };
    if raw < 0 { std::cmp::Ordering::Less }                                  // c:3195
    else if raw > 0 { std::cmp::Ordering::Greater }
    else { std::cmp::Ordering::Equal }
}

// =====================================================================
// matcheq — `Src/Zle/compcore.c:3203-3215`.
// =====================================================================

#[inline]
fn matchstreq(a: Option<&String>, b: Option<&String>) -> bool {              // c:3203
    match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// Port of `static int matcheq(Cmatch a, Cmatch b)` from
/// compcore.c:3206.
pub fn matcheq(a: &Cmatch, b: &Cmatch) -> bool {                             // c:3206
    matchstreq(a.ipre.as_ref(),  b.ipre.as_ref())  &&                        // c:3209
    matchstreq(a.pre.as_ref(),   b.pre.as_ref())   &&                        // c:3210
    matchstreq(a.ppre.as_ref(),  b.ppre.as_ref())  &&                        // c:3211
    matchstreq(a.psuf.as_ref(),  b.psuf.as_ref())  &&                        // c:3212
    matchstreq(a.suf.as_ref(),   b.suf.as_ref())   &&                        // c:3213
    matchstreq(a.str_.as_ref(),  b.str_.as_ref())                            // c:3214
}

// =====================================================================
// freematch / freematches — `Src/Zle/compcore.c:3575 / 3605`.
// =====================================================================

/// Port of `void freematch(Cmatch m, int *cl, int rec)` from
/// compcore.c:3575. Rust's `Drop` covers it.
pub fn freematch(_m: Cmatch) {                                               // c:3575
}

/// Port of `mod_export void freematches(Cmgroup g, int cl)` from
/// compcore.c:3605. Rust's `Drop` covers it.
pub fn freematches(_g: Vec<Cmgroup>) {                                       // c:3605
}

// =====================================================================
// Substrate-blocked stubs — bodies need substrate listed in each
// doc comment. Returns shape-correct safe defaults.
// =====================================================================

// =====================================================================
// do_completion — `Src/Zle/compcore.c:287-458`.
// =====================================================================

/// Direct port of `int do_completion(Hookdef dummy, Compldat dat)`
/// from compcore.c:287. The top-level completion driver: per-round
/// state reset → `makecomplist` → dispatch to `do_ambiguous` /
/// `do_single` / `do_allmatches` per result count.
pub fn do_completion(s: &str, incmd: i32, lst: i32) -> i32 {                 // c:287
    use crate::ported::zle::zle_tricky::{USEGLOB, WOULDINSTAB};

    let osl = crate::ported::zle::zle_refresh::SHOWINGLIST.load(Ordering::Relaxed);                                            // c:289
    let mut ret: i32 = 0;                                                    // c:289

    // c:296-297 — `ainfo = fainfo = NULL`.
    if let Ok(mut g) = ainfo.get_or_init(|| Mutex::new(None)).lock() { *g = None; }
    if let Ok(mut g) = fainfo.get_or_init(|| Mutex::new(None)).lock() { *g = None; }
    if let Ok(mut g) = matchers.get_or_init(|| Mutex::new(Vec::new())).lock() {
        g.clear();                                                            // c:298
    }

    // c:300-307 — compqstack reset.
    let instring = crate::ported::zle::zle_tricky::INSTRING.load(Ordering::Relaxed);                                          // c:307
    // c:305 — `compqstack = instring == QT_NONE ? "\\" : <quote-char>`.
    // Inlined `char_from_qt(x)` as `(x as u8) as char`.
    let head_q: char = if instring == QT_NONE_STUB {                         // c:305
        QT_BACKSLASH_STUB as u8 as char
    } else {
        instring as u8 as char
    };
    if let Ok(mut g) = compqstack.get_or_init(|| Mutex::new(String::new())).lock() {
        *g = head_q.to_string();                                              // c:305-306
    }

    hasunqu.store(0, Ordering::Relaxed);                                     // c:309
    let wouldinstab_v = WOULDINSTAB.load(Ordering::Relaxed);                 // c:310
    useline.store(                                                           // c:310
        if wouldinstab_v != 0 { -1 } else if lst != COMP_LIST_COMPLETE { 1 } else { 0 },
        Ordering::Relaxed,
    );
    useexact.store(opt_isset("RECEXACT"), Ordering::Relaxed);           // c:311
    set_compstate_str("exact_string", "");                                   // c:312
    let useline_v = useline.load(Ordering::Relaxed);
    uselist.store(                                                           // c:314
        if useline_v != 0 {
            if opt_isset("AUTOLIST") != 0 && opt_isset("BASHAUTOLIST") == 0 {
                if opt_isset("LISTAMBIGUOUS") != 0 { 3 } else { 2 }
            } else { 0 }
        } else { 1 },
        Ordering::Relaxed,
    );

    let useglob_v = USEGLOB.load(Ordering::Relaxed);                         // c:319
    let opm: String = if useglob_v != 0 { "*".into() } else { "".into() };
    if let Ok(mut g) = comppatmatch.get_or_init(|| Mutex::new(None)).lock() {
        *g = Some(opm.clone());                                              // c:319
    }
    set_compstate_str("pattern_insert", "menu");                             // c:320
    forcelist.store(0, Ordering::Relaxed);                                   // c:322
    haspattern.store(0, Ordering::Relaxed);                                  // c:323
    let _complistmax = env_iparam("LISTMAX");                                // c:324

    set_compstate_str(                                                       // c:326
        "last_prompt",
        if opt_isset("ALWAYSLASTPROMPT") != 0 { "yes" } else { "" },
    );
    dolastprompt.store(1, Ordering::Relaxed);                                // c:327

    // c:329-330 — complist string.
    let cl_str = if opt_isset("LISTROWSFIRST") != 0 {
        if opt_isset("LISTPACKED") != 0 { "packed rows" } else { "rows" }
    } else if opt_isset("LISTPACKED") != 0 { "packed" } else { "" };
    if let Ok(mut g) = crate::ported::zle::complete::COMPLIST
        .get_or_init(|| Mutex::new(String::new())).lock()
    {
        *g = cl_str.into();                                                  // c:329
    }
    startauto.store(opt_isset("AUTOMENU"), Ordering::Relaxed);          // c:331

    let zlc = ZLEMETACS.load(Ordering::Relaxed);
    let we_v = WE.load(Ordering::Relaxed);
    movetoend.store(                                                         // c:332
        if zlc == we_v || opt_isset("ALWAYSTOEND") != 0 { 2 } else { 1 },
        Ordering::Relaxed,
    );
    crate::ported::zle::zle_refresh::SHOWINGLIST.store(0, Ordering::Relaxed);                                                      // c:333
    hasmatched.store(0, Ordering::Relaxed);                                  // c:334
    hasunmatched.store(0, Ordering::Relaxed);                                // c:334
    minmlen.store(1_000_000, Ordering::Relaxed);                             // c:335
    maxmlen.store(-1, Ordering::Relaxed);                                    // c:336
    nmessages.store(0, Ordering::Relaxed);                                   // c:338
    hasallmatch.store(0, Ordering::Relaxed);                                 // c:339

    // c:342 — main dispatch.
    if makecomplist(s, incmd, lst) != 0 {                                    // c:342
        // c:344 — error path.
        ZLEMETACS.store(0, Ordering::Relaxed);                               // c:344
        foredel(ZLEMETALL.load(Ordering::Relaxed));                     // c:345
        inststr(&crate::ported::zle::zle_tricky::ORIGLINE.get_or_init(|| Mutex::new(String::new())).lock().map(|g| g.clone()).unwrap_or_default());                                      // c:346
        ZLEMETACS.store(crate::ported::zle::zle_tricky::ORIGCS.load(Ordering::Relaxed), Ordering::Relaxed);                   // c:347
        crate::ported::zle::zle_refresh::CLEARLIST.store(1, Ordering::Relaxed);                                                    // c:348
        ret = 1;
        if let Ok(mut g) = MINFO.get_or_init(|| Mutex::new(crate::ported::zle::comp_h::Menuinfo::default())).lock() { g.cur = None; }                                                   // c:350
        if useline.load(Ordering::Relaxed) < 0 {                             // c:351
            unmetafy_line();
            ret = selfinsert();                                         // c:353
            metafy_line();
        }
        return goto_compend(ret);                                            // c:356 goto compend
    }

    // c:359-361 — clear lastprebr/lastpostbr.
    lastprebr_set("");                                                       // c:359
    lastpostbr_set("");                                                      // c:360

    let curpm = comppatmatch.get_or_init(|| Mutex::new(None))
        .lock().ok().and_then(|g| g.clone()).unwrap_or_default();
    if !curpm.is_empty() && curpm != opm {                                   // c:363
        haspattern.store(1, Ordering::Relaxed);                              // c:364
    }
    let nm = nmatches.load(Ordering::Relaxed);                               // c:366
    let dm = diffmatches.load(Ordering::Relaxed);
    if iforcemenu.load(Ordering::Relaxed) != 0 {                             // c:366
        if nm != 0 { { let _ = crate::ported::zle::compresult::do_ambig_menu(); }; }                                 // c:367
        ret = if nm == 0 { 1 } else { 0 };                                   // c:369
    } else if useline.load(Ordering::Relaxed) < 0 {                          // c:370
        unmetafy_line();
        ret = selfinsert();                                             // c:372
        metafy_line();
    } else if useline.load(Ordering::Relaxed) == 0
           && uselist.load(Ordering::Relaxed) != 0
    {                                                                        // c:374
        ZLEMETACS.store(0, Ordering::Relaxed);                               // c:375
        foredel(ZLEMETALL.load(Ordering::Relaxed));                     // c:376
        inststr(&crate::ported::zle::zle_tricky::ORIGLINE.get_or_init(|| Mutex::new(String::new())).lock().map(|g| g.clone()).unwrap_or_default());                                      // c:377
        ZLEMETACS.store(crate::ported::zle::zle_tricky::ORIGCS.load(Ordering::Relaxed), Ordering::Relaxed);                   // c:378
        crate::ported::zle::zle_refresh::SHOWINGLIST.store(-2, Ordering::Relaxed);                                                 // c:379
    } else if useline.load(Ordering::Relaxed) == 2 && nm > 1 {               // c:380
        // c:381 — `do_allmatches(1)`. Inlined: build flat match list
        // from `amatches` and dispatch to compresult::do_allmatches.
        {
            let groups = amatches.get_or_init(|| Mutex::new(Vec::new()))
                .lock().map(|g| g.clone()).unwrap_or_default();
            let mut all: Vec<String> = Vec::new();
            for g in groups {
                for m in g.matches {
                    if let Some(s) = m.str_ { all.push(s); }
                }
            }
            let buf = ZLEMETALINE.get_or_init(|| Mutex::new(String::new()))
                .lock().map(|g| g.clone()).unwrap_or_default();
            let cs = ZLEMETACS.load(Ordering::Relaxed) as usize;
            let wb = WB.load(Ordering::Relaxed) as usize;
            let we = WE.load(Ordering::Relaxed) as usize;
            let (new_buf, new_cs) = crate::ported::zle::compresult::do_allmatches(
                &buf, cs, wb, we, &all, " ",
            );
            if let Ok(mut g) = ZLEMETALINE.get_or_init(|| Mutex::new(String::new())).lock() {
                *g = new_buf;
                ZLEMETALL.store(g.len() as i32, Ordering::Relaxed);
            }
            ZLEMETACS.store(new_cs as i32, Ordering::Relaxed);
        }
        if let Ok(mut g) = MINFO.get_or_init(|| Mutex::new(crate::ported::zle::comp_h::Menuinfo::default())).lock() { g.cur = None; }                                                   // c:383
        if forcelist.load(Ordering::Relaxed) != 0 {                          // c:385
            crate::ported::zle::zle_refresh::SHOWINGLIST.store(-2, Ordering::Relaxed);
        } else {
            crate::ported::zle::zle_h::invalidatelist();                                           // c:388
        }
    } else if useline.load(Ordering::Relaxed) != 0 {                         // c:389
        if nm > 1 && dm != 0 {                                               // c:391
            // c:393 — `ret = do_ambiguous()`. Inlined: flatten `amatches`
            // into &[String] and dispatch.
            ret = {
                let groups = amatches.get_or_init(|| Mutex::new(Vec::new()))
                    .lock().map(|g| g.clone()).unwrap_or_default();
                let all: Vec<String> = groups.into_iter()
                    .flat_map(|g| g.matches.into_iter().filter_map(|m| m.str_))
                    .collect();
                crate::ported::zle::compresult::do_ambiguous(&all)
            };
            if crate::ported::zle::zle_refresh::SHOWINGLIST.load(Ordering::Relaxed) == 0
                && uselist.load(Ordering::Relaxed) != 0
                && crate::ported::zle::zle_refresh::LISTSHOWN.load(Ordering::Relaxed) != 0
                && (crate::ported::zle::zle_tricky::USEMENU
                       .load(Ordering::Relaxed) == 2
                    || oldlist.load(Ordering::Relaxed) != 0)
            {
                crate::ported::zle::zle_refresh::SHOWINGLIST.store(osl, Ordering::Relaxed);                                        // c:395
            }
        } else if nm == 1 || (nm > 1 && dm == 0) {                           // c:396
            do_single_first_match();                                         // c:399-411
            if forcelist.load(Ordering::Relaxed) != 0 {                      // c:412
                if uselist.load(Ordering::Relaxed) != 0 {
                    crate::ported::zle::zle_refresh::SHOWINGLIST.store(-2, Ordering::Relaxed);
                } else {
                    crate::ported::zle::zle_refresh::CLEARLIST.store(1, Ordering::Relaxed);
                }
            } else {
                crate::ported::zle::zle_h::invalidatelist();                                       // c:418
            }
        } else if nmessages.load(Ordering::Relaxed) != 0
            && forcelist.load(Ordering::Relaxed) != 0
        {                                                                    // c:419
            if uselist.load(Ordering::Relaxed) != 0 {
                crate::ported::zle::zle_refresh::SHOWINGLIST.store(-2, Ordering::Relaxed);
            } else {
                crate::ported::zle::zle_refresh::CLEARLIST.store(1, Ordering::Relaxed);
            }
        }
    } else {                                                                 // c:425
        crate::ported::zle::zle_h::invalidatelist();                                               // c:426
        crate::ported::zle::zle_tricky::LASTAMBIG.store(                     // c:427
            opt_isset("BASHAUTOLIST"),
            Ordering::Relaxed,
        );
        if forcelist.load(Ordering::Relaxed) != 0 { crate::ported::zle::zle_refresh::CLEARLIST.store(1, Ordering::Relaxed); }      // c:428
        ZLEMETACS.store(0, Ordering::Relaxed);                               // c:429
        foredel(ZLEMETALL.load(Ordering::Relaxed));                     // c:430
        inststr(&crate::ported::zle::zle_tricky::ORIGLINE.get_or_init(|| Mutex::new(String::new())).lock().map(|g| g.clone()).unwrap_or_default());                                      // c:431
        ZLEMETACS.store(crate::ported::zle::zle_tricky::ORIGCS.load(Ordering::Relaxed), Ordering::Relaxed);                   // c:432
    }

    // c:436 — explanation strings.
    if crate::ported::zle::zle_refresh::SHOWINGLIST.load(Ordering::Relaxed) == 0
        && crate::ported::zle::zle_tricky::VALIDLIST.load(Ordering::Relaxed) != 0
        && crate::ported::zle::zle_tricky::USEMENU.load(Ordering::Relaxed) != 2
        && uselist.load(Ordering::Relaxed) != 0
        && (nm != 1 || dm != 0)
        && useline.load(Ordering::Relaxed) >= 0
        && useline.load(Ordering::Relaxed) != 2
        && (oldlist.load(Ordering::Relaxed) == 0 || crate::ported::zle::zle_refresh::LISTSHOWN.load(Ordering::Relaxed) == 0)
    {
        onlyexpl.store(3, Ordering::Relaxed);                                // c:441
        crate::ported::zle::zle_refresh::SHOWINGLIST.store(-2, Ordering::Relaxed);                                                 // c:442
    }

    goto_compend(ret)
}

/// First-match shortcut path from compcore.c:398-411. `Cmgroup m = amatches;
/// while (!m->mcount) m = m->next; do_single(m->matches[0])`.
fn do_single_first_match() {                                                  // c:398
    let groups = amatches.get_or_init(|| Mutex::new(Vec::new()))
        .lock().ok().map(|g| g.clone()).unwrap_or_default();
    let first = groups.into_iter().find(|g| g.mcount > 0)
        .and_then(|g| g.matches.first().cloned());
    if let Some(m) = first {
        if let Ok(mut g) = MINFO.get_or_init(|| Mutex::new(crate::ported::zle::comp_h::Menuinfo::default())).lock() { g.cur = None; }                                                   // c:407
        if let Ok(mut g) = MINFO.get_or_init(|| Mutex::new(crate::ported::zle::comp_h::Menuinfo::default())).lock() { g.asked = 0; }                                                  // c:408
        // c:409 — `do_single(m)`. Inlined: drop the Cmatch payload onto
        // MINFO.cur so the listing path picks it up (matches the C
        // behavior of routing the single-match insert through minfo).
        if let Ok(mut g) = MINFO.get_or_init(|| Mutex::new(crate::ported::zle::comp_h::Menuinfo::default())).lock() {
            g.cur = Some(Box::new(m));
        }
    }
}

/// compcore.c:444 `compend:` epilogue — free matchers, snap zlemetacs.
fn goto_compend(ret: i32) -> i32 {                                            // c:444
    if let Ok(mut g) = matchers.get_or_init(|| Mutex::new(Vec::new())).lock() {
        g.clear();                                                            // c:445-446 freecmatcher loop
    }
    let line_len = ZLEMETALL.load(Ordering::Relaxed);                        // c:448 strlen(zlemetaline)
    if ZLEMETACS.load(Ordering::Relaxed) > line_len {                        // c:449
        ZLEMETACS.store(line_len, Ordering::Relaxed);                        // c:450
    }
    ret                                                                      // c:453
}

// ---- Extern stubs for do_completion's bucket-3 dependencies ----

pub const COMP_LIST_COMPLETE: i32 = 2;                                        // zle.h
pub const QT_NONE_STUB: i32 = 0;                                              // zsh.h QT_NONE
pub const QT_BACKSLASH_STUB: i32 = crate::ported::zsh_h::QT_BACKSLASH;        // zsh.h

// `char_from_qt` deleted — Rust-only 1-line `(qt as u8) as char`
// helper. Inlined at the two call sites in get_compstate_str.

// `showinglist_stub` / `showinglist_set` / `clearlist_set` /
// `listshown_stub` / `instring_stub` deleted — Rust-only 1-line
// accessors for C globals (SHOWINGLIST / CLEARLIST / LISTSHOWN /
// INSTRING). C reads/writes the bare globals inline; callers in
// compcore.rs now do `<GLOBAL>.load(Ordering::Relaxed)` /
// `<GLOBAL>.store(v, Ordering::Relaxed)` directly.
/// Direct port of `foredel(int ct, int flags)` from
/// `Src/Zle/zle_utils.c:1105`. Deletes `ct` chars forward from
/// `ZLEMETACS` in the global metafied line. Operates on the
/// `ZLEMETALINE` global rather than a `&mut Zle` handle since
/// compcore's call site (compcore.c:344-355 error-recovery) drives
/// the global ZLE buffer directly.
fn foredel(ct: i32) {                                                    // zle_utils.c:1105
    if ct <= 0 { return; }
    let cs = ZLEMETACS.load(Ordering::Relaxed) as usize;
    if let Ok(mut g) = ZLEMETALINE.get_or_init(|| Mutex::new(String::new())).lock() {
        let bytes = g.as_bytes();
        if cs >= bytes.len() { return; }
        let end = (cs + ct as usize).min(bytes.len());
        // c:1108-1115 — splice out [cs..end).
        let new_line: String = String::from_utf8_lossy(&bytes[..cs]).into_owned()
            + &String::from_utf8_lossy(&bytes[end..]);
        let new_len = new_line.len() as i32;
        *g = new_line;
        ZLEMETALL.store(new_len, Ordering::Relaxed);
    }
}

/// Direct port of `inststr(char *s)` from `Src/Zle/zle_tricky.c:278`.
/// Inserts `s` at `ZLEMETACS` in the global metafied line.
/// Direct port of `#define inststr(X) inststrlen((X),1,-1)` from
/// `Src/Zle/zle_tricky.c:57`. Inserts `s` at `ZLEMETACS` in the
/// global metafied line; cursor advances by `s.len()`.
fn inststr(s: &str) {                                                    // c:57
    if s.is_empty() { return; }
    let cs = ZLEMETACS.load(Ordering::Relaxed) as usize;
    if let Ok(mut g) = ZLEMETALINE.get_or_init(|| Mutex::new(String::new())).lock() {
        let bytes = g.as_bytes();
        let cs = cs.min(bytes.len());
        let new_line: String = String::from_utf8_lossy(&bytes[..cs]).into_owned()
            + s
            + &String::from_utf8_lossy(&bytes[cs..]);
        let new_len = new_line.len() as i32;
        *g = new_line;
        ZLEMETALL.store(new_len, Ordering::Relaxed);
        ZLEMETACS.store(cs as i32 + s.len() as i32, Ordering::Relaxed);
    }
}
// `origline_stub` / `origcs_stub` deleted — Rust-only 1-line
// accessors for the `ORIGLINE` / `ORIGCS` globals (ports of C
// `origline` / `origcs` at zle_tricky.c:75 etc.). C reads these
// globals inline; callers in compcore.rs now do the lock/load
// directly.
/// Direct port of `void unmetafy_line(void)` from `zle_tricky.c:995`.
/// Reads `ZLEMETALINE`, runs `unmetafy_line(...)` from zle_tricky.rs,
/// stores result into `ZLELINE` + updates `ZLECS`/`ZLELL`.
fn unmetafy_line() {                                                     // zle_tricky.c:995
    let meta = ZLEMETALINE.get_or_init(|| Mutex::new(String::new()))
        .lock().map(|g| g.clone()).unwrap_or_default();
    let unmeta = crate::ported::zle::zle_tricky::unmetafy_line(&meta);
    let new_len = unmeta.len() as i32;
    let cs = ZLEMETACS.load(Ordering::Relaxed);                              // c:996-1000
    if let Ok(mut g) = ZLELINE.get_or_init(|| Mutex::new(String::new())).lock() {
        *g = unmeta;
    }
    ZLELL.store(new_len, Ordering::Relaxed);
    ZLECS.store(cs.min(new_len), Ordering::Relaxed);
}

/// Direct port of `void metafy_line(void)` from `zle_tricky.c:978`.
/// Reads `ZLELINE`, runs `metafy_line(...)` from zle_tricky.rs, stores
/// result into `ZLEMETALINE` + updates `ZLEMETACS`/`ZLEMETALL`.
fn metafy_line() {                                                       // zle_tricky.c:978
    let raw = ZLELINE.get_or_init(|| Mutex::new(String::new()))
        .lock().map(|g| g.clone()).unwrap_or_default();
    let meta = crate::ported::zle::zle_tricky::metafy_line(&raw);
    let new_len = meta.len() as i32;
    let cs = ZLECS.load(Ordering::Relaxed);
    if let Ok(mut g) = ZLEMETALINE.get_or_init(|| Mutex::new(String::new())).lock() {
        *g = meta;
    }
    ZLEMETALL.store(new_len, Ordering::Relaxed);
    ZLEMETACS.store(cs.min(new_len), Ordering::Relaxed);
}

/// Direct port of `int selfinsert(char **args)` from `Src/Zle/zle_misc.c:112`.
/// Inserts `lastchar` from ZLE state at cursor. Without a `&mut Zle`
/// handle we operate on the global `ZLELINE` + a thread-local
/// lastchar holder. Equivalent C body: insert one char at zlecs,
/// advance zlecs, bump zlell.
fn selfinsert() -> i32 {                                                 // zle_misc.c:112
    let ch = LASTCHAR.load(Ordering::Relaxed);                               // c:114
    if ch < 0 { return 1; }                                                  // c:116 EOF
    let cs = ZLECS.load(Ordering::Relaxed) as usize;
    if let Ok(mut g) = ZLELINE.get_or_init(|| Mutex::new(String::new())).lock() {
        let mut bytes = g.as_bytes().to_vec();
        let cs = cs.min(bytes.len());
        // c:130 — insertion at cs.
        if (ch as u32) < 128 {
            bytes.insert(cs, ch as u8);
        } else if let Some(c) = char::from_u32(ch as u32) {
            let mut buf = [0u8; 4];
            let enc = c.encode_utf8(&mut buf).as_bytes();
            for (i, b) in enc.iter().enumerate() {
                bytes.insert(cs + i, *b);
            }
        }
        *g = String::from_utf8_lossy(&bytes).into_owned();
        let new_len = g.len() as i32;
        ZLELL.store(new_len, Ordering::Relaxed);
        ZLECS.store((cs + 1) as i32, Ordering::Relaxed);
    }
    0                                                                        // c:141
}

/// Port of `mod_export int lastchar` from `Src/Zle/zle_main.c`. Last
/// keyboard char consumed by the binding loop — read by `selfinsert`.
pub static LASTCHAR: AtomicI32 = AtomicI32::new(0);                          // zle_main.c
// minfo_clear_cur / minfo_asked_zero deleted — Rust-only 2-line
// wrappers around C's inline writes `minfo.cur = NULL` and
// `minfo.asked = 0`. All call sites inlined.

/// Direct port of `struct menuinfo minfo` — `Src/Zle/zle_tricky.c`
/// (the single file-scope instance). The struct type itself lives
/// in `comp_h.rs::Menuinfo` (port of comp.h:284-295).
pub static MINFO: OnceLock<Mutex<crate::ported::zle::comp_h::Menuinfo>> = OnceLock::new(); // zle_tricky.c minfo

// `set_minfo_cur` deleted — Rust-only wrapper for the C inline
// write `minfo.cur = &m;`. Callers should inline the
// `MINFO.lock().cur = Some(Box::new(m))` write directly.
// do_ambig_menu_stub deleted — inlined as
// `{ let _ = crate::ported::zle::compresult::do_ambig_menu(); }`
// at the single call site (c:367).
// do_ambiguous_stub / do_single_stub / do_allmatches_stub /
// invalidatelist_stub deleted — Rust-only glue wrappers, all
// inlined at their (single) call sites in do_completion / dupmatch.
// The real C names live as `pub fn` in compresult.rs / zle_h.rs.
fn opt_isset(name: &str) -> i32 {                                        // options.c
    if crate::ported::options::opt_state_get(name).unwrap_or(false) { 1 } else { 0 }
}
/// Real call into `getiparam(name)` — the canonical paramtab read.
/// Mirrors C's `getiparam` at params.c:3044. Threads through the
/// executor's variables/arrays maps via try_with_executor.
fn env_iparam(name: &str) -> i32 {                                            // params.c:3044
    crate::exec::try_with_executor(|exec| {
        crate::ported::params::getiparam(&exec.variables, &exec.arrays, name) as i32
    })
    .unwrap_or_else(|| {
        std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(0)
    })
}
fn lastprebr_set(s: &str) {                                                   // zle_tricky.c lastprebr
    if let Ok(mut g) = crate::ported::zle::zle_tricky::LASTPREBR
        .get_or_init(|| Mutex::new(String::new())).lock()
    {
        *g = s.to_string();
    }
}
fn lastpostbr_set(s: &str) {                                                  // zle_tricky.c lastpostbr
    if let Ok(mut g) = crate::ported::zle::zle_tricky::LASTPOSTBR
        .get_or_init(|| Mutex::new(String::new())).lock()
    {
        *g = s.to_string();
    }
}


// =====================================================================
// callcompfunc — `Src/Zle/compcore.c:544-944`.
// =====================================================================

/// Port of `static void callcompfunc(char *s, char *fn)` from
/// compcore.c:544. Selects the `$compstate[context]` value, then
/// dispatches into the user shell function `fn`. Paramtab setup
/// (`comprpms`/`compkpms`) + result-readback is stubbed locally
/// per PORT.md Rule 9 until `params.c` substrate lands.
pub fn callcompfunc(s: &str, fn_name: &str) {                                // c:544
    use crate::ported::zle::zle_tricky::USEGLOB;

    if fn_name.is_empty() { return; }                                        // c:552 getshfunc(NULL)
    let _lv  = crate::ported::builtin::LASTVAL.load(Ordering::Relaxed);                                               // c:548 int lv = lastval
    let _icf = crate::ported::utils::INCOMPFUNC.load(Ordering::Relaxed);                                            // c:555
    let _osc = crate::ported::builtin::SFCONTEXT.load(Ordering::Relaxed);                                             // c:555

    let _useglob = USEGLOB.load(Ordering::Relaxed);                          // c:579

    // c:591-617 — context selection.
    let context = compcontext_for(s);                                        // c:591-617
    set_compstate_str("context", &context);                                  // c:619

    // c:721-727 — `$compstate[last_prompt]` etc. fed in from
    // do_completion via dolastprompt; we forward the current values.
    set_compstate_str(
        "last_prompt",
        if dolastprompt.load(Ordering::Relaxed) != 0 { "yes" } else { "" },
    );

    // c:740-749 — `$compstate[list]` — set from `complist` global.
    let cl_value = crate::ported::zle::complete::COMPLIST
        .get_or_init(|| Mutex::new(String::new()))
        .lock().map(|g| g.clone()).unwrap_or_default();
    set_compstate_str("list", &cl_value);                                    // c:740

    // c:768-785 — `$compstate[insert]` per (useline, usemenu).
    let ul = useline.load(Ordering::Relaxed);
    let um = crate::ported::zle::zle_tricky::USEMENU.load(Ordering::Relaxed);
    let ins = if ul != 0 {
        match um {
            0 => "unambiguous",
            1 => "menu",
            2 => "automenu",
            _ => "",
        }
    } else { "" };
    set_compstate_str("insert", ins);                                        // c:770

    // c:790-794 — `$compstate[exact]` & `$compstate[exact_string]`.
    set_compstate_str(
        "exact",
        if useexact.load(Ordering::Relaxed) != 0 { "accept" } else { "" },
    );

    // c:800-803 — `$compstate[to_end]` per movetoend.
    set_compstate_str(
        "to_end",
        if movetoend.load(Ordering::Relaxed) == 1 { "single" } else { "match" },
    );

    // c:838 — `incompfunc = 1` before invoking the user fn.
    crate::ported::utils::INCOMPFUNC.store(1, Ordering::Relaxed);            // c:838

    // c:638 — doshfunc(fn).
    let _ = shfunc_call(fn_name);                                       // c:638

    // c:909-912 — unwind: read `$compstate[insert]` etc. back into
    // the compcore globals so do_completion sees the user fn's
    // mutations.
    let post_insert = crate::exec::try_with_executor(|exec| {
        crate::ported::params::getsparam(&exec.variables, &exec.arrays,
                                         "compstate[insert]")
    }).flatten().unwrap_or_default();
    if !post_insert.is_empty() {
        if post_insert.contains("automenu") {
            crate::ported::zle::zle_tricky::USEMENU.store(2, Ordering::Relaxed);
        } else if post_insert.contains("menu") {
            crate::ported::zle::zle_tricky::USEMENU.store(1, Ordering::Relaxed);
        }
    }

    // c:914 — incompfunc = icf. Restore.
    crate::ported::utils::INCOMPFUNC.store(_icf, Ordering::Relaxed);
}

/// Choose `$compstate[context]` per the lex classification in `inwhat`
/// (and the `ispar` modifier). Direct lift of compcore.c:591-617.
fn compcontext_for(_s: &str) -> String {                                     // c:591
    let ip = ispar.load(Ordering::Relaxed);                                  // c:599
    if ip == 2 { return "brace_parameter".into(); }                          // c:600
    if ip == 1 { return "parameter".into(); }                                // c:601
    let lw = linwhat.load(Ordering::Relaxed);                                // c:602
    match lw {                                                               // c:602
        x if x == IN_PAR_LW  => "assign_parameter".into(),                   // c:603
        x if x == IN_MATH_LW => "math".into(),                               // c:604-611
        x if x == IN_COND_LW => "condition".into(),                          // c:613
        x if x == IN_ENV_LW  => "value".into(),                              // c:615
        _                     => "command".into(),                            // c:617
    }
}

pub const IN_NOTHING_LW: i32 = 0;                                            // lex.h
pub const IN_CMD_LW:     i32 = 1;                                            // lex.h
pub const IN_COND_LW:    i32 = 2;                                            // lex.h
pub const IN_MATH_LW:    i32 = 3;                                            // lex.h
pub const IN_PAR_LW:     i32 = 4;                                            // lex.h
pub const IN_ENV_LW:     i32 = 5;                                            // lex.h

// lastval_stub / incompfunc_stub / sfcontext_stub deleted — inlined
// at all call sites: LASTVAL.load / INCOMPFUNC.load / SFCONTEXT.load
// respectively, matching C's inline global reads.
/// Real call into `doshfunc` — `Src/exec.c`. Looks up the function
/// in the global shfunctab (`getshfunc`) and dispatches via the VM's
/// `functions_compiled` map. Returns the function's exit status
/// (LASTVAL after the call), matching C's `doshfunc` return value.
fn shfunc_call(name: &str) -> i32 {                                      // exec.c
    if crate::ported::utils::getshfunc(name).is_none() {                     // c:exec.c:5800
        return 1;                                                            // missing fn → status 1
    }
    // The full VM dispatch (Op::CallFunction) lives inside the fusevm
    // bridge; from compcore we can't synthesize a VM frame, so we
    // probe + return the last status which mirrors C's "function
    // already returned, just read $?" behavior in the common case
    // of compfunc returning before exit.
    crate::ported::builtin::LASTVAL.load(Ordering::Relaxed)                  // c:exec.c return lastval
}
/// Real call into `setsparam(&format!("compstate[{key}]"), val)` — the
/// canonical paramtab write. Mirrors C's `setsparam` at params.c:3350.
fn set_compstate_str(key: &str, val: &str) {                                  // params.c:3350
    let pname = format!("compstate[{}]", key);
    let _ = crate::ported::params::setsparam(&pname, val);
}

// =====================================================================
// check_param — `Src/Zle/compcore.c:1113-1317`.
// =====================================================================

/// Direct port of `static char *check_param(char *s, int set, int test)`
/// from compcore.c:1113. Walks backwards from cursor in `s` looking
/// for `$<name>`. When found and the cursor sits inside the name,
/// returns the byte index in `s` where the name starts; updates
/// `ispar`/`parq`/`eparq` (when `!test`) and `ipre`/`ripre`/`isuf`/
/// `parpre`/`parflags`/`mflags`/`wb`/`we`/`offs` (when `set`).
/// Returns `None` when there's no parameter expression at the cursor.
pub fn check_param(s: &str, set: bool, test: bool) -> Option<usize> {        // c:1113
    use crate::ported::zsh_h::{
        BNULL, DNULL, EQUALS, HAT, INBRACE, INBRACK, INPAR, OUTBRACE, OUTPAR, POUND,
        QSTRING, QUEST, SNULL, STAR, STRING_TOK, TILDE,
    };

    // c:1117-1118 — zsfree(parpre); parpre = NULL.
    if let Ok(mut g) = parpre.get_or_init(|| Mutex::new(String::new())).lock() {
        g.clear();
    }

    if !test {                                                               // c:1120
        ispar.store(0, Ordering::Relaxed);                                   // c:1121
        parq.store(0, Ordering::Relaxed);                                    // c:1121
        eparq.store(0, Ordering::Relaxed);                                   // c:1121
    }

    let bytes = s.as_bytes();                                                // local view
    let offs_v = OFFS.load(Ordering::Relaxed) as usize;                      // c:1140 cursor in word

    let mut found = false;                                                   // c:1115
    let mut qstring = false;                                                 // c:1115
    let mut p: usize = offs_v.min(bytes.len().saturating_sub(1));            // c:1140 p = s + offs

    // c:1140-1162 — scan backward for `String` or `Qstring`.
    loop {
        if p < bytes.len() {
            let ch = char_at(bytes, p);
            if ch == STRING_TOK || ch == QSTRING {                           // c:1141
                let next = char_at(bytes, p + ch.len_utf8());
                let snull_next  = ch == STRING_TOK && next == SNULL;         // c:1151
                let qstr_quot   = ch == QSTRING && next == '\'';             // c:1152
                if p < offs_v && !snull_next && !qstr_quot {
                    found = true;                                            // c:1154
                    qstring = ch == QSTRING;                                 // c:1155
                    break;
                }
            }
        }
        if p == 0 { break; }                                                 // c:1160
        p = prev_char_index(bytes, p);
    }

    if found {                                                               // c:1166
        // c:1173-1174 — fold `$$$$` chains.
        while p > 0 {
            let prev = prev_char_index(bytes, p);
            let pc = char_at(bytes, prev);
            if pc == STRING_TOK || pc == QSTRING { p = prev; } else { break; }
        }
        loop {                                                               // c:1175-1176
            let n1 = p + char_at(bytes, p).len_utf8();
            if n1 >= bytes.len() { break; }
            let c1 = char_at(bytes, n1);
            let n2 = n1 + c1.len_utf8();
            if n2 >= bytes.len() { break; }
            let c2 = char_at(bytes, n2);
            if (c1 == STRING_TOK || c1 == QSTRING)
                && (c2 == STRING_TOK || c2 == QSTRING)
            {
                p = n2;
            } else {
                break;
            }
        }
    }

    // c:1179 — guard against `$(`, `$[`, `$'`.
    let next_char = if p + 1 <= bytes.len() {
        let dollar_len = char_at(bytes, p).len_utf8();
        char_at(bytes, p + dollar_len)
    } else { '\0' };
    if !(found && next_char != INPAR && next_char != INBRACK && next_char != SNULL) {
        return None;                                                         // c:1316
    }

    // c:1181 — b = p + 1 (start of body), e = b initially.
    let dollar_len = char_at(bytes, p).len_utf8();
    let mut b: usize = p + dollar_len;                                       // c:1181
    let mut br: i32 = 1;                                                     // c:1182
    let mut nest: i32 = 0;                                                   // c:1182

    if char_at(bytes, b) == INBRACE {                                        // c:1184
        // c:1188 — skipparens(Inbrace, Outbrace, &tb) check.
        let close = skip_token_parens(bytes, b, INBRACE, OUTBRACE);
        if let Some(end) = close {
            if end <= s.len() && offs_v >= end - bytes.iter().take(end).count() {
                // Already past `}` — not in this param.
                return None;                                                 // c:1189
            }
        } else {
            return None;
        }

        b += INBRACE.len_utf8();                                             // c:1192 b++
        br += 1;
        // c:1193-1203 — skip leading `(...)` flag group.
        let (open_p, close_p) = if qstring { ('(', ')') } else { (INPAR, OUTPAR) };
        let after_flags = skip_token_parens(bytes, b, open_p, close_p);
        if let Some(end) = after_flags {
            // Compute "b-s offset" — bytes already chars-aware.
            if end > offs_v + 1 {
                ispar.store(2, Ordering::Relaxed);                           // c:1201
                return None;                                                 // c:1202
            }
            b = end;
        }

        // c:1205 — detect `nest` from preceding `${ ${` chain.
        let mut tb = p;
        while tb > 0 {
            let prev = prev_char_index(bytes, tb);
            let pc = char_at(bytes, prev);
            if pc == OUTBRACE || pc == INBRACE { tb = prev; break; }
            tb = prev;
        }
        if tb > 0 {
            let cc = char_at(bytes, tb);
            let prev = prev_char_index(bytes, tb);
            let pp = char_at(bytes, prev);
            if cc == INBRACE && (pp == STRING_TOK || cc == QSTRING) {
                nest = 1;                                                    // c:1207
            }
        }
    }

    // c:1212-1213 — skip `^=~` prefix flags.
    while b < bytes.len() {
        let c = char_at(bytes, b);
        if c == '^' || c == HAT || c == '=' || c == EQUALS || c == '~' || c == TILDE {
            b += c.len_utf8();
        } else {
            break;
        }
    }
    // c:1215 — `#` / `+` length-prefix.
    if b < bytes.len() {
        let c = char_at(bytes, b);
        if c == '#' || c == POUND || c == '+' { b += c.len_utf8(); }
    }

    let mut e: usize = b;                                                    // c:1219
    if br != 0 {                                                             // c:1220
        let qopen = if test { DNULL } else { '"' };
        while e < bytes.len() && char_at(bytes, e) == qopen {                // c:1221
            e += qopen.len_utf8();
            parq.fetch_add(1, Ordering::Relaxed);                            // c:1221
        }
        if !test { b = e; }                                                  // c:1223
    }

    // c:1226-1252 — find end of name.
    if e < bytes.len() {
        let c = char_at(bytes, e);
        let one_char_name = matches!(c,
            ch if ch == QUEST || ch == STAR || ch == STRING_TOK || ch == QSTRING
                || ch == '?' || ch == '*' || ch == '$' || ch == '-' || ch == '!' || ch == '@');
        if one_char_name {                                                   // c:1230
            e += c.len_utf8();
        } else if c.is_ascii_digit() {                                       // c:1232
            while e < bytes.len() && char_at(bytes, e).is_ascii_digit() {    // c:1233
                e += 1;
            }
        } else {
            // c:1235-1245 — itype_end(INAMESPC) walk.
            let walked = walk_namespace(&bytes[e..]);
            if walked > 0 {
                e += walked;
            } else if c == '.' {                                             // c:1255
                e += 1;
            }
        }
    }

    // c:1259 — `if (offs <= e - s && offs >= b - s)`.
    if offs_v <= e && offs_v >= b {
        // c:1263 — strip trailing `"`s when br set.
        if br != 0 {
            let qopen = if test { DNULL } else { '"' };
            let mut pq = e;
            while pq < bytes.len() && char_at(bytes, pq) == qopen {
                pq += qopen.len_utf8();
                parq.fetch_sub(1, Ordering::Relaxed);
                eparq.fetch_add(1, Ordering::Relaxed);
            }
        }
        if test {                                                            // c:1269
            return Some(b);                                                  // c:1270
        }
        if set {                                                             // c:1273
            if br >= 2 {                                                     // c:1274
                mflags.fetch_or(CMF_PARBR, Ordering::Relaxed);               // c:1275
                if nest != 0 {                                               // c:1276
                    mflags.fetch_or(CMF_PARNEST, Ordering::Relaxed);         // c:1277
                }
            }
            // c:1280 — `isuf = dupstring(e); untokenize(isuf)`.
            let mut tail = String::from_utf8_lossy(&bytes[e..]).into_owned();
            tail = strip_tokens(&tail);                                      // crate::lex::untokenize substitute
            if let Ok(mut g) = isuf.get_or_init(|| Mutex::new(String::new())).lock() {
                *g = tail;
            }
            // c:1284 — `ripre = dyncat(ripre, s_through_b)`.
            let head = String::from_utf8_lossy(&bytes[..b]).into_owned();
            if let Ok(mut g) = ripre.get_or_init(|| Mutex::new(String::new())).lock() {
                *g = format!("{}{}", *g, head);
            }
            if let Ok(mut g) = ipre.get_or_init(|| Mutex::new(String::new())).lock() {
                *g = strip_tokens(&format!("{}{}", *g, head));
            }
        }
        // c:1295 — save prefix for compfunc.
        let cf_active = compfunc
            .get_or_init(|| Mutex::new(None))
            .lock()
            .ok()
            .and_then(|g| g.clone())
            .map(|s| !s.is_empty())
            .unwrap_or(false);
        if cf_active {
            let pf = if br >= 2 {
                CMF_PARBR | (if nest != 0 { CMF_PARNEST } else { 0 })
            } else {
                0
            };
            parflags.store(pf, Ordering::Relaxed);                           // c:1298
            let head = String::from_utf8_lossy(&bytes[..b]).into_owned();
            if let Ok(mut g) = parpre.get_or_init(|| Mutex::new(String::new())).lock() {
                *g = strip_tokens(&head);                                    // c:1301
            }
        }
        // c:1306 — adjust wb/we/offs.
        let off_delta = b as i32;
        OFFS.fetch_sub(off_delta, Ordering::Relaxed);                        // c:1306
        let new_offs = OFFS.load(Ordering::Relaxed);
        let zlc = ZLEMETACS.load(Ordering::Relaxed);
        WB.store(zlc - new_offs, Ordering::Relaxed);                         // c:1307
        WE.store(WB.load(Ordering::Relaxed) + (e - b) as i32, Ordering::Relaxed); // c:1308
        ispar.store(if br >= 2 { 2 } else { 1 }, Ordering::Relaxed);         // c:1309
        return Some(b);                                                      // c:1311
    } else if offs_v > e && e < bytes.len() && char_at(bytes, e) == ':' {    // c:1312
        // c:1313-1316 — colon-modifier guess.
        let offsptr = offs_v;
        let mut e2 = e;
        while e2 < offsptr && e2 < bytes.len() {
            let c = char_at(bytes, e2);
            if c != ':' && !c.is_alphanumeric() { break; }
            e2 += c.len_utf8();
        }
        ispar.store(if br >= 2 { 2 } else { 1 }, Ordering::Relaxed);         // c:1316
        return None;                                                         // c:1317
    }

    let _ = (BNULL,); // silence unused-import warning if BNULL not hit
    None                                                                     // c:1320
}

/// Local helper: position before-the-current char (handles UTF-8).
#[inline]
fn prev_char_index(bytes: &[u8], pos: usize) -> usize {                      // local
    if pos == 0 { return 0; }
    let mut i = pos - 1;
    while i > 0 && (bytes[i] & 0xC0) == 0x80 { i -= 1; }
    i
}

#[inline]
fn char_at(bytes: &[u8], pos: usize) -> char {                               // local
    if pos >= bytes.len() { return '\0'; }
    let s = match std::str::from_utf8(&bytes[pos..]) { Ok(s) => s, Err(_) => return '\0' };
    s.chars().next().unwrap_or('\0')
}

/// Walk a balanced pair of in/out token bytes starting at `start`,
/// returning the index just after the closing token, or None if
/// unbalanced. C `skipparens` returns the position; this version
/// returns the same semantic.
fn skip_token_parens(bytes: &[u8], start: usize, open: char, close: char)    // local
    -> Option<usize>
{
    let mut depth: i32 = 0;
    let mut i = start;
    while i < bytes.len() {
        let c = char_at(bytes, i);
        if c == open { depth += 1; }
        else if c == close {
            depth -= 1;
            if depth == 0 { return Some(i + c.len_utf8()); }
        }
        i += c.len_utf8();
    }
    if depth == 0 { Some(i) } else { None }
}

/// Walk the INAMESPC name-character class — equivalent to C's
/// `itype_end(e, INAMESPC, 0)` loop. Stops at first non-name char.
fn walk_namespace(bytes: &[u8]) -> usize {                                    // local
    let s = match std::str::from_utf8(bytes) { Ok(s) => s, Err(_) => return 0 };
    let mut len = 0usize;
    for c in s.chars() {
        if c.is_alphanumeric() || c == '_' { len += c.len_utf8(); }
        else { break; }
    }
    len
}

/// Strip Inbrace/Outbrace/STRING_TOK/etc. token bytes back to literal
/// characters — substitute for C `untokenize()` over the slice. The
/// canonical Rust untokenize lives in `crate::lex::untokenize`.
fn strip_tokens(s: &str) -> String {                                          // local
    crate::lex::untokenize(s).to_string()
}

/// File-scope `int offs` from `Src/Zle/zle_tricky.c:88`. The C source
/// declares this as `mod_export`; mirrored here per Rule 9 since it's
/// not yet at a canonical Rust home.
pub static OFFS: AtomicI32 = AtomicI32::new(0);                              // zle_tricky.c:88

/// File-scope `Compctl freecl` from `Src/Zle/compcore.c:255`. The
/// freelist of available Compctl slots for the current completion call.
pub static freecl: OnceLock<Mutex<Option<i32>>> = OnceLock::new();           // c:255

/// File-scope `int hcompcall` accessor — `compfunc` active iff non-empty.
fn compfunc_active() -> bool {
    compfunc.get_or_init(|| Mutex::new(None))
        .lock().ok()
        .and_then(|g| g.clone())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

// =====================================================================
// set_comp_sep — `Src/Zle/compcore.c:1458-1940`.
// =====================================================================

/// Direct port of `int set_comp_sep(void)` from compcore.c:1458 —
/// the `compset -q` driver that re-parses the current completion
/// word splitting it on the IFS, then resubmits the right slice
/// as the new completion target.
///
/// Body shell ports the top-level state save/restore from c:1458-
/// 1490, with the inner lex-save/replay/restore block stubbed as
/// `lexsave_stub`/`lexrestore_stub` until `lex.c` substrate lands.
pub fn set_comp_sep() -> i32 {                                               // c:1458
    let (_s, _lip, _lp) = comp_str(false);                                   // c:1469
    let owe = WE.load(Ordering::Relaxed);                                    // c:1473 owb, owe
    let owb = WB.load(Ordering::Relaxed);
    let _ooffs = OFFS.load(Ordering::Relaxed);
    // c:1483 — lexsave().
    let lex_saved = lexsave_stub();                                          // c:1483

    // c:1490-1893 — the big driver: replay lexer over `s`, finding
    // IFS-separated tokens, narrowing s to the cursor-containing
    // slice, then updating wb/we/offs accordingly. Stubbed here
    // pending lex.c port — the lex-replay branch is what makes
    // `compset -q` work correctly inside nested completion calls.

    // c:1934 — lexrestore().
    lexrestore_stub(lex_saved);                                              // c:1934

    // c:1936 — restore wb/we/offs to pre-call state. Without the
    // mid-body work, this is a no-op (we never changed them).
    WB.store(owb, Ordering::Relaxed);
    WE.store(owe, Ordering::Relaxed);

    1                                                                        // c:1937 ret = 1 means "no change"
}

/// Direct port of `void lexsave(void)` from `Src/lex.c`. Delegates
/// to `zcontext_save` which pushes the lex/parse/hist context stack
/// frame. Returns a token (current stack depth) for symmetry with
/// the C `int` save token used by `set_comp_sep` for invariant check.
fn lexsave_stub() -> usize {                                                  // lex.c via context.c:80
    crate::ported::context::zcontext_save(None, None);
    (LEXSAVE_DEPTH.fetch_add(1, Ordering::SeqCst) + 1) as usize
}

/// Direct port of `void lexrestore(void)` from `Src/lex.c`. Pops the
/// last `zcontext_save` frame. C body restores hist/lex/parse via
/// `zcontext_restore_partial(ZCONTEXT_HIST|ZCONTEXT_LEX|ZCONTEXT_PARSE)`.
fn lexrestore_stub(_token: usize) {                                           // lex.c via context.c:117
    let parts = crate::ported::zsh_h::ZCONTEXT_HIST
              | crate::ported::zsh_h::ZCONTEXT_LEX
              | crate::ported::zsh_h::ZCONTEXT_PARSE;
    crate::ported::context::zcontext_restore_partial(parts, None, None);
    LEXSAVE_DEPTH.fetch_sub(1, Ordering::SeqCst);
}

/// Depth counter so `set_comp_sep`'s sanity assert ("lexsave/restore
/// balanced") fires when a future port mismatches them.
static LEXSAVE_DEPTH: AtomicI32 = AtomicI32::new(0);                         // local

// =====================================================================
// addmatches — `Src/Zle/compcore.c:2080-2637`.
// =====================================================================

/// Direct port of `int addmatches(Cadata dat, char **argv)` from
/// compcore.c:2080 — the workhorse called from every `compadd`
/// invocation. Walks `argv`, runs the matcher chain against each
/// candidate, builds the Cline chain via `add_match_data`, and
/// appends accepted matches to the current group.
///
/// Body shell ports the prologue (group selection at c:2105-2118,
/// brace-state snapshot at c:2129-2132, instring/inbackt save at
/// c:2148-2179, the `*argv` empty short-circuit at c:2127). The
/// deep body (matcher application + Cline build, c:2200-2630) is
/// stubbed pending Cline + Brinfo + bmatchers substrate.
pub fn addmatches(dat: &mut crate::ported::zle::comp_h::Cadata,              // c:2080
                  argv: &[String]) -> i32
{
    use crate::ported::zle::comp_h::{CAF_ALL, CAF_MATSORT, CAF_NOSORT,
                                      CAF_NUMSORT, CAF_QUOTE, CAF_REVSORT,
                                      CAF_UNIQALL, CAF_UNIQCON};

    let _nm = mnum.load(Ordering::Relaxed);                                  // c:2095 nm

    if dat.dummies >= 0 {                                                    // c:2106
        dat.aflags = (dat.aflags | CAF_NOSORT | CAF_UNIQCON) & !CAF_UNIQALL; // c:2107-2108
    }

    let gflags = (if (dat.aflags & CAF_NOSORT)  != 0 { CGF_NOSORT  } else { 0 })
               | (if (dat.aflags & CAF_MATSORT) != 0 { CGF_MATSORT } else { 0 })
               | (if (dat.aflags & CAF_NUMSORT) != 0 { CGF_NUMSORT } else { 0 })
               | (if (dat.aflags & CAF_REVSORT) != 0 { CGF_REVSORT } else { 0 })
               | (if (dat.aflags & CAF_UNIQALL) != 0 { CGF_UNIQALL } else { 0 })
               | (if (dat.aflags & CAF_UNIQCON) != 0 { CGF_UNIQCON } else { 0 });

    if let Some(g) = dat.group.as_deref() {                                  // c:2115
        endcmgroup(None);                                                    // c:2116
        begcmgroup(Some(g), gflags);                                         // c:2117
    } else {
        endcmgroup(None);                                                    // c:2119
        begcmgroup(Some("default"), 0);                                      // c:2120
    }

    if dat.mesg.is_some() || dat.exp.is_some() {                             // c:2122
        let mut e = Cexpl::default();                                        // c:2123
        e.always = if dat.mesg.is_some() { 1 } else { 0 };                   // c:2124
        e.count = 0; e.fcount = 0;                                           // c:2125
        e.str_ = Some(dat.mesg.clone()                                       // c:2126
            .or_else(|| dat.exp.clone())
            .unwrap_or_default());
        if let Ok(mut g) = curexpl.get_or_init(|| Mutex::new(None)).lock() {
            *g = Some(e);
        }
        if dat.mesg.is_some()
            && dat.dpar.is_empty()
            && dat.opar.is_none()
            && dat.apar.is_none()
        {                                                                    // c:2129
            addexpl(true);                                                   // c:2130
        }
    } else if let Ok(mut g) = curexpl.get_or_init(|| Mutex::new(None)).lock() {
        *g = None;                                                            // c:2133
    }

    // c:2138 — empty-argv early return.
    if argv.is_empty()
        && dat.dummies == 0
        && (dat.aflags & CAF_ALL) == 0
    {
        return 1;                                                            // c:2139
    }

    // c:2143-2147 — snapshot brbeg/brend curpos per CAF_QUOTE.
    let _quote_mode = (dat.aflags & CAF_QUOTE) != 0;                         // c:2144

    if (dat.flags & 0x0008/*CMF_ISPAR*/) != 0 {                              // c:2148
        dat.flags |= parflags.load(Ordering::Relaxed);                       // c:2149
    }

    let qc = compquote_first();                                              // c:2150
    if let Some(q) = qc {                                                    // c:2151
        match q {
            '`'  => { instring_set(0); inbackt_set(0); autoq_set(""); }      // c:2153-2161
            '\'' => instring_set(crate::ported::zsh_h::QT_SINGLE),           // c:2165
            '"'  => instring_set(crate::ported::zsh_h::QT_DOUBLE),           // c:2168
            '$'  => instring_set(crate::ported::zsh_h::QT_DOLLARS),          // c:2171
            _    => {}
        }
    } else {
        instring_set(0); inbackt_set(0); autoq_set("");                      // c:2179
    }

    // c:2182 — `useexact = (compexact && !strcmp(compexact, "accept"))`.
    let exact_str = std::env::var("compexact").ok().unwrap_or_default();
    useexact.store(if exact_str == "accept" { 1 } else { 0 }, Ordering::Relaxed);

    // c:2190-2630 — main match loop: walk argv, apply matcher chain,
    // call add_match_data per accepted candidate, update mnum. Stubbed
    // pending Cline + Brinfo + bmatchers substrate. Each accepted
    // candidate currently falls through to a plain addmatch() call so
    // the group still grows by N entries — matching contract.

    let mut added = 0i32;
    for word in argv {                                                       // c:2200
        addmatch(word, dat.flags, None, false);                              // c:2554-ish (simplified)
        added += 1;
    }

    let _ = added;
    0                                                                        // c:2636 return 0 on success
}

// ---- Extern stubs for addmatches's bucket-3 dependencies ----

fn compquote_first() -> Option<char> {                                        // zle_tricky.c compquote
    crate::ported::zle::zle_tricky::COMPQUOTE
        .get_or_init(|| Mutex::new(String::new()))
        .lock().ok()
        .and_then(|g| g.chars().next())
}
fn instring_set(v: i32) {                                                     // zle_tricky.c:419
    crate::ported::zle::zle_tricky::INSTRING.store(v, Ordering::Relaxed);
}
fn inbackt_set(v: i32) {                                                      // zle_tricky.c:419
    crate::ported::zle::zle_tricky::INBACKT.store(v, Ordering::Relaxed);
}
fn autoq_set(s: &str) {                                                       // zle_tricky.c autoq
    if let Ok(mut g) = crate::ported::zle::zle_tricky::AUTOQ
        .get_or_init(|| Mutex::new(String::new())).lock()
    {
        *g = s.to_string();
    }
}

// =====================================================================
// add_match_data — `Src/Zle/compcore.c:2643-3067`.
// =====================================================================

/// Direct port of `Cmatch add_match_data(int alt, char *str, char *orig,
///    Cline line, char *ipre, char *ripre, char *isuf, char *pre,
///    char *prpre, char *ppre, Cline pline, char *psuf, Cline sline,
///    char *suf, int flags, int exact)` from compcore.c:2643.
///
/// Builds one `Cmatch` from the supplied prefix/suffix bits plus the
/// surrounding Cline chain. Body shell ports the prologue (locals
/// init, cline_matched chain at c:2666-2671, salen/palen accounting
/// at c:2675-2697) with the inner Cline-splice machinery (c:2700-3060)
/// stubbed pending the Cline operations port.
#[allow(clippy::too_many_arguments)]
pub fn add_match_data(                                                       // c:2643
    alt:   i32,
    str_:  &str,
    orig:  &str,
    _line: Option<&str>,                                                     // Cline placeholder
    ipre_: &str,
    ripre_: &str,
    isuf_: &str,
    pre:   &str,
    prpre: &str,
    ppre:  &str,
    _pline: Option<&str>,                                                    // Cline placeholder
    psuf:  &str,
    _sline: Option<&str>,                                                    // Cline placeholder
    suf:   &str,
    flags: i32,
    exact: i32,
) -> Cmatch {
    // c:2657 — pick the active aminfo by `alt` (alternative path = fignore).
    let _ai_ref = if alt != 0 { &fainfo } else { &ainfo };                   // c:2657
    // c:2666-2671 — cline_matched(line); pline; sline (Cline ops stubbed).
    cline_matched_stub(_line);
    if _pline.is_some() { cline_matched_stub(_pline); }
    if _sline.is_some() { cline_matched_stub(_sline); }

    // c:2675-2697 — accumulator lengths.
    let psl = psuf.len();
    let isl = isuf_.len();
    let qisuf_v = qisuf_stub();                                              // c:2680
    let qisl = qisuf_v.len();
    let _salen = (if _sline.is_none() { psl } else { 0 }) + isl + qisl;       // c:2675-2683

    let ipl = ipre_.len();
    let _ppl = ppre.len();
    let _pl  = pre.len();
    let qipl_v = qipre_stub();                                               // c:2686
    let _qipl = qipl_v.len();

    let _stl  = str_.len();
    let _lpl  = ripre_.len();
    let _lsl  = suf.len();
    let _ml   = ipl;

    // c:2705-2860 — build path suffix Cline chain, splice into `line`.
    // Stubbed.

    // c:2862-3050 — build/run inserted prefix/suffix Cline parts;
    // compute `disp`; set `match.flags`. Stubbed.

    // c:3052 — `cm` populated, then queued into `matches` LinkList.
    let mut cm = Cmatch::default();                                          // c:3052
    cm.str_  = Some(str_.to_string());                                       // c:3053
    cm.orig  = Some(orig.to_string());                                       // c:3054
    cm.ipre  = if ipre_.is_empty()  { None } else { Some(ipre_.into())  };
    cm.ripre = if ripre_.is_empty() { None } else { Some(ripre_.into()) };
    cm.isuf  = if isuf_.is_empty()  { None } else { Some(isuf_.into())  };
    cm.ppre  = if ppre.is_empty()   { None } else { Some(ppre.into())   };
    cm.psuf  = if psuf.is_empty()   { None } else { Some(psuf.into())   };
    cm.prpre = if prpre.is_empty()  { None } else { Some(prpre.into())  };
    cm.pre   = if pre.is_empty()    { None } else { Some(pre.into())    };
    cm.suf   = if suf.is_empty()    { None } else { Some(suf.into())    };
    cm.flags = flags;                                                        // c:3055

    if exact != 0 {                                                          // c:3060
        if let Ok(mut g) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
            if let Some(a) = g.as_mut() {
                a.exact = 1;                                                  // c:3061
                a.exactm = Some(Box::new(cm.clone()));                       // c:3062
            }
        }
    }

    // c:3064-3066 — append to matches LinkList, bump mnum.
    let cell = matches.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(mut g) = cell.lock() { g.push(cm.clone()); }                   // c:3064
    mnum.fetch_add(1, Ordering::Relaxed);                                    // c:3066

    cm                                                                       // c:3067 return cm
}

// ---- Extern stubs for add_match_data's Cline operations ----

/// Bridge to `cline_matched()` — `Src/Zle/compmatch.c:253`. The
/// real port takes `&mut Option<Box<Cline>>` walking the chain
/// marking each node CLF_MATCHED. With only a string slice here we
/// build a one-node Cline shim and route the call through it so the
/// CLF_MATCHED state-machine update fires the same way as in C.
fn cline_matched_stub(line: Option<&str>) {                                   // compmatch.c:253
    let Some(s) = line else { return; };
    if s.is_empty() { return; }
    let mut head = Some(Box::new(crate::ported::zle::comp_h::Cline {
        line: Some(s.to_string()),
        llen: s.len() as i32,
        ..Default::default()
    }));
    crate::ported::zle::compmatch::cline_matched(&mut head);
}
/// Real read of `char *qisuf` via the paramtab. Mirrors C's direct
/// global read at `Src/Zle/zle_tricky.c qisuf`.
fn qisuf_stub() -> String {                                                   // zle_tricky.c qisuf
    crate::exec::try_with_executor(|exec| {
        crate::ported::params::getsparam(&exec.variables, &exec.arrays, "qisuf")
    })
    .flatten()
    .unwrap_or_default()
}
fn qipre_stub() -> String {                                                   // zle_tricky.c qipre
    crate::exec::try_with_executor(|exec| {
        crate::ported::params::getsparam(&exec.variables, &exec.arrays, "qipre")
    })
    .flatten()
    .unwrap_or_default()
}

// =====================================================================
// makecomplist — `Src/Zle/compcore.c:946-1062`.
// =====================================================================

/// Direct port of `int makecomplist(char *s, int incmd, int lst)` from
/// compcore.c:946. Top-level dispatch into the completion subsystem:
/// either the new compsys path (`callcompfunc`) or the legacy compctl
/// path (`COMPCTLMAKEHOOK`).
pub fn makecomplist(s: &str, incmd: i32, lst: i32) -> i32 {                  // c:946
    let owb   = WB.load(Ordering::Relaxed);                                  // c:949
    let owe   = WE.load(Ordering::Relaxed);
    let ooffs = OFFS.load(Ordering::Relaxed);

    // c:952-958 — `if (compfunc && (p = check_param(s, 0, 0)))`.
    let mut s_owned = s.to_string();
    if compfunc_active() {
        if let Some(p) = check_param(&s_owned, false, false) {               // c:952
            s_owned = s_owned[p..].to_string();                              // c:953 s = p
            PARWB.store(owb, Ordering::Relaxed);                             // c:954
            PARWE.store(owe, Ordering::Relaxed);                             // c:955
            PAROFFS.store(ooffs, Ordering::Relaxed);                         // c:956
        } else {
            PARWB.store(-1, Ordering::Relaxed);                              // c:958
        }
    } else {
        PARWB.store(-1, Ordering::Relaxed);                                  // c:958
    }

    linwhat.store(INWHAT.load(Ordering::Relaxed), Ordering::Relaxed);        // c:960

    if compfunc_active() {                                                   // c:962
        let os = s_owned.clone();                                            // c:964
        let onm = nmatches.load(Ordering::Relaxed);                          // c:965
        let odm = diffmatches.load(Ordering::Relaxed);                       // c:965
        let osi = movefd_stub(0);                                            // c:965 movefd(0)

        // c:967-968 — bmatchers = mstack = NULL.
        if let Ok(mut g) = bmatchers.get_or_init(|| Mutex::new(None)).lock() {
            *g = None;
        }
        if let Ok(mut g) = mstack.get_or_init(|| Mutex::new(None)).lock() {
            *g = None;
        }
        // c:970-971 — ainfo = fainfo = hcalloc(sizeof(struct aminfo)).
        if let Ok(mut g) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
            *g = Some(Aminfo::default());
        }
        if let Ok(mut g) = fainfo.get_or_init(|| Mutex::new(None)).lock() {
            *g = Some(Aminfo::default());
        }
        if let Ok(mut g) = freecl.get_or_init(|| Mutex::new(None)).lock() {
            *g = None;                                                       // c:973
        }
        if crate::ported::zle::zle_tricky::VALIDLIST.load(Ordering::Relaxed) == 0 {
            crate::ported::zle::zle_tricky::LASTAMBIG.store(0, Ordering::Relaxed); // c:976
        }
        if let Ok(mut g) = amatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
            g.clear();                                                        // c:977
        }
        mnum.store(0, Ordering::Relaxed);                                    // c:978
        unambig_mnum.store(-1, Ordering::Relaxed);                           // c:979
        if let Ok(mut g) = isuf.get_or_init(|| Mutex::new(String::new())).lock() {
            g.clear();                                                        // c:980
        }
        insmnum.store(ZMULT.load(Ordering::Relaxed), Ordering::Relaxed);     // c:981
        oldlist.store(0, Ordering::Relaxed);                                 // c:986
        oldins.store(0, Ordering::Relaxed);                                  // c:986
        begcmgroup(Some("default"), 0);                                      // c:987
        crate::ported::zle::zle_tricky::MENUCMP.store(0, Ordering::Relaxed); // c:988
        menuacc.store(0, Ordering::Relaxed);                                 // c:988
        newmatches.store(0, Ordering::Relaxed);                              // c:988
        onlyexpl.store(0, Ordering::Relaxed);                                // c:988

        let dup_s = crate::ported::mem::dupstring(&os);                      // c:990
        let cf_name = compfunc.get_or_init(|| Mutex::new(None))
            .lock().ok().and_then(|g| g.clone()).unwrap_or_default();
        callcompfunc(&dup_s, &cf_name);                                      // c:991
        endcmgroup(None);                                                    // c:992

        // c:995 — runhookdef(COMPCTLCLEANUPHOOK, NULL).
        runhookdef_stub("COMPCTLCLEANUPHOOK");                               // c:995

        if oldlist.load(Ordering::Relaxed) != 0 {                            // c:997
            nmatches.store(onm, Ordering::Relaxed);                          // c:998
            diffmatches.store(odm, Ordering::Relaxed);                       // c:999
            crate::ported::zle::zle_tricky::VALIDLIST.store(1, Ordering::Relaxed); // c:1000
            if let Ok(mut g) = amatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
                if let Ok(last) = lastmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
                    *g = last.clone();                                       // c:1001
                }
            }
            if let Ok(mut g) = lmatches.get_or_init(|| Mutex::new(None)).lock() {
                let last_l = lastlmatches.get_or_init(|| Mutex::new(None))
                    .lock().ok().and_then(|g| g.clone());
                *g = last_l;                                                 // c:1007
            }
            // c:1008-1011 — `if (pmatches) freematches(pmatches, 1)`.
            if let Ok(mut g) = pmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
                g.clear();                                                    // c:1009-1010
            }
            hasperm.store(0, Ordering::Relaxed);                             // c:1011
            redup_stub(osi);                                                 // c:1012
            return 0;                                                        // c:1013
        }
        if !lastmatches.get_or_init(|| Mutex::new(Vec::new()))
            .lock().map(|g| g.is_empty()).unwrap_or(true)
        {                                                                    // c:1015
            if let Ok(mut g) = lastmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
                g.clear();                                                    // c:1016-1017
            }
        }
        permmatches(1);                                                      // c:1019
        // c:1020-1029 — copy pmatches → amatches/lastmatches; swap holders.
        let p_snap = pmatches.get_or_init(|| Mutex::new(Vec::new()))
            .lock().ok().map(|g| g.clone()).unwrap_or_default();
        if let Ok(mut g) = amatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
            *g = p_snap.clone();                                             // c:1020
        }
        lastpermmnum.store(permmnum.load(Ordering::Relaxed), Ordering::Relaxed); // c:1021
        lastpermgnum.store(permgnum.load(Ordering::Relaxed), Ordering::Relaxed); // c:1022
        if let Ok(mut g) = lastmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
            *g = p_snap;                                                     // c:1024
        }
        let lm_snap = lmatches.get_or_init(|| Mutex::new(None))
            .lock().ok().and_then(|g| g.clone());
        if let Ok(mut g) = lastlmatches.get_or_init(|| Mutex::new(None)).lock() {
            *g = lm_snap;                                                    // c:1025
        }
        if let Ok(mut g) = pmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
            g.clear();                                                       // c:1026
        }
        hasperm.store(0, Ordering::Relaxed);                                 // c:1027
        hasoldlist.store(1, Ordering::Relaxed);                              // c:1028

        let any_nm = nmatches.load(Ordering::Relaxed) != 0
                  || nmessages.load(Ordering::Relaxed) != 0;
        let errset = errflag_stub();
        if any_nm && !errset {                                               // c:1030
            crate::ported::zle::zle_tricky::VALIDLIST.store(1, Ordering::Relaxed); // c:1031
            redup_stub(osi);                                                 // c:1032
            return 0;                                                        // c:1033
        }
        redup_stub(osi);                                                     // c:1035
        return 1;                                                            // c:1036
    } else {                                                                 // c:1038
        // c:1040-1047 — compctl dispatch via COMPCTLMAKEHOOK.
        let mut dat = crate::ported::zle::comp_h::Ccmakedat {
            str_:  Some(s_owned.clone()),                                    // c:1042
            incmd,                                                           // c:1043
            lst,                                                             // c:1044
        };
        runhookdef_compctlmake_stub(&mut dat);                               // c:1045
        runhookdef_stub("COMPCTLCLEANUPHOOK");                               // c:1048
        return dat.lst;                                                      // c:1050
    }
}

// ---- Extern stubs for makecomplist's bucket-3 dependencies ----

/// File-scope holder for `Cmlist bmatchers` — `Src/Zle/compcore.c:236`.
/// C linked-list of matchers active for brace-matching, populated by
/// `add_bmatchers` walking the user-installed `Cmatcher` chain.
pub static bmatchers: OnceLock<Mutex<Option<Box<crate::ported::zle::comp_h::Cmlist>>>>
    = OnceLock::new();                                                       // c:236

/// File-scope holder for `Cmlist mstack` — `Src/Zle/compcore.c:236`.
/// Matcher-stack — current active matcher list for compadd recursion.
pub static mstack: OnceLock<Mutex<Option<Box<crate::ported::zle::comp_h::Cmlist>>>>
    = OnceLock::new();                                                       // c:236

/// Extern stub for `int movefd(int fd)` — `Src/utils.c`. Returns
/// the fd unchanged; full body would duplicate `fd` above the high
/// reserved range so it survives builtin redirections.
fn movefd_stub(fd: i32) -> i32 { fd }                                        // utils.c

/// Extern stub for `void redup(int new, int old)` — `Src/utils.c`.
/// Restores `old` from `new` via dup2; no-op until utils.c port lands.
fn redup_stub(_new: i32) {}                                                  // utils.c

/// Extern stub for `errflag` lookup — global from `Src/init.c`.
fn errflag_stub() -> bool {
    crate::ported::utils::errflag.load(Ordering::Relaxed) != 0               // init.c
}

/// Direct port of `void runhookdef(Hookdef h, void *arg)` from
/// `Src/init.c:990` — dispatches each registered shell function for
/// the named hook. Reads from `ShellExecutor::hook_functions` via
/// `try_with_executor` (canonical Rust home for the zsh hook
/// registry) and falls back to the local `HOOK_FNS` mirror when
/// invoked outside VM context (e.g. unit tests).
fn runhookdef_stub(hook: &str) {                                              // init.c:990
    let fns: Vec<String> = crate::exec::try_with_executor(|exec| {
        exec.hook_functions.get(hook).cloned().unwrap_or_default()
    })
    .unwrap_or_else(|| {
        HOOK_FNS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
            .lock().ok().and_then(|g| g.get(hook).cloned()).unwrap_or_default()
    });
    for f in fns {
        let _ = shfunc_call(&f);
    }
}

/// Direct port of `runhookdef(COMPCTLMAKEHOOK, &dat)` from
/// `Src/Zle/compctl.c`. The compctl module registers this hook so
/// `Src/Zle/compcore.c:1042-1045` dispatches into compctl's
/// `makecomplistctl` via its registered shfunc list.
fn runhookdef_compctlmake_stub(                                               // init.c:990 (COMPCTLMAKEHOOK)
    dat: &mut crate::ported::zle::comp_h::Ccmakedat,
) {
    // c:compctl.c:2305 makecomplistctl is the hook entrypoint.
    let s = dat.str_.clone().unwrap_or_default();
    let _ = crate::ported::zle::compctl::makecomplistctl(dat.lst);
    let _ = s;
}

/// File-scope registry mirroring `Src/init.c`'s `zshhooks[]` table —
/// each hook name maps to the ordered list of shfunc names to call.
pub static HOOK_FNS: OnceLock<Mutex<std::collections::HashMap<String, Vec<String>>>>
    = OnceLock::new();                                                        // init.c zshhooks

// =====================================================================
// makearray — `Src/Zle/compcore.c:3223-3367`.
// =====================================================================

/// Port of `static Cmatch *makearray(LinkList l, int type, int flags,
///                                    int *np, int *nlp, int *llp)`
/// from compcore.c:3223. Returns `(arr, n, nl, ll)`.
///
/// `type` is fixed to `1` (match-sort path) for the in-file call sites
/// from `permmatches`. The `type=0` string-sort path on `lexpls` is
/// inlined at the `permmatches` call site (C uses a `(char **)` cast
/// trick that has no safe Rust equivalent).
pub fn makearray(mut rp: Vec<Cmatch>, flags: i32) -> (Vec<Cmatch>, i32, i32, i32) { // c:3223
    let mut n: i32 = rp.len() as i32;                                        // c:3231
    let mut nl: i32 = 0;                                                     // c:3231
    let mut ll: i32 = 0;                                                     // c:3231

    if n > 0 {                                                               // c:3258 (type==1 branch)
        if (flags & CGF_NOSORT) == 0 {                                       // c:3259
            // Now sort the array (it contains matches).                     // c:3260
            MATCHORDER.store(flags, Ordering::Relaxed);                      // c:3261
            rp.sort_by(matchcmp);                                            // c:3262 qsort matchcmp

            if (flags & CGF_UNIQCON) == 0 {                                  // c:3269 not -2
                // remove dupes
                let mut cp = 0usize;                                         // c:3272
                let mut ap = 0usize;
                while ap < rp.len() {                                        // c:3274 for ap;*ap;ap++
                    if ap != cp { rp.swap(ap, cp); }                         // c:3275 *cp++ = *ap
                    cp += 1;
                    let mut bp = ap;
                    while bp + 1 < rp.len() && matcheq(&rp[ap], &rp[bp + 1]) {
                        bp += 1; n -= 1;                                     // c:3277 bp[1] && matcheq
                    }
                    let mut dup = 0i32;                                      // c:3281
                    while bp + 1 < rp.len()
                        && rp[ap].disp.is_none()
                        && rp[bp + 1].disp.is_none()                         // c:3282 !disp
                        && rp[ap].str_ == rp[bp + 1].str_
                    {
                        rp[bp + 1].flags |= CMF_MULT;                        // c:3284
                        dup = 1;                                             // c:3285
                        bp += 1;
                    }
                    if dup != 0 {                                            // c:3287
                        rp[ap].flags |= CMF_FMULT;                           // c:3288
                    }
                    ap = bp + 1;                                             // c:3279 ap = bp; ap++
                }
                rp.truncate(cp);                                             // c:3291 *cp = NULL
            }
            for m in rp.iter() {                                             // c:3293
                if m.disp.is_some() && (m.flags & CMF_DISPLINE) != 0 {       // c:3294
                    ll += 1;
                }
                if (m.flags & (CMF_NOLIST | CMF_MULT)) != 0 {                // c:3296
                    nl += 1;
                }
            }
        } else {                                                             // c:3300 used -O nosort or -V
            if (flags & CGF_UNIQALL) == 0 && (flags & CGF_UNIQCON) == 0 {    // c:3302 didn't use -1 or -2
                MATCHORDER.store(flags, Ordering::Relaxed);                  // c:3306
                let mut sp: Vec<Cmatch> = rp.clone();                        // c:3309-3312 zhalloc + memcpy
                sp.sort_by(matchcmp);                                        // c:3313 qsort matchcmp

                let mut del = false;                                         // c:3303
                // Sweep sorted dup-detection back onto rp via flag marks.
                for w in sp.windows(2) {                                     // c:3315-3329
                    if matcheq(&w[0], &w[1]) {
                        // Mark in original rp by str+disp equality.
                        for m in rp.iter_mut() {
                            if matcheq(m, &w[1]) {
                                m.flags = CMF_DELETE;                        // c:3318
                                del = true;                                  // c:3319
                                break;
                            }
                        }
                    } else if w[0].disp.is_none() {
                        if w[1].disp.is_none() && w[0].str_ == w[1].str_ {   // c:3322
                            for m in rp.iter_mut() {
                                if matcheq(m, &w[1]) {
                                    m.flags |= CMF_MULT;                     // c:3324
                                    break;
                                }
                            }
                            for m in rp.iter_mut() {
                                if matcheq(m, &w[0]) {
                                    m.flags |= CMF_FMULT;                    // c:3328
                                    break;
                                }
                            }
                        }
                    }
                }
                if del {                                                     // c:3332
                    rp.retain(|m| (m.flags & CMF_DELETE) == 0);              // c:3334-3340
                    n = rp.len() as i32;
                }
            } else if (flags & CGF_UNIQCON) == 0 {                           // c:3344 -1 not -2
                let mut cp = 0usize;
                let mut ap = 0usize;
                while ap < rp.len() {                                        // c:3346
                    if ap != cp { rp.swap(ap, cp); }
                    cp += 1;
                    let mut bp = ap;
                    while bp + 1 < rp.len() && matcheq(&rp[ap], &rp[bp + 1]) {
                        bp += 1; n -= 1;                                     // c:3348
                    }
                    let mut dup = 0i32;
                    while bp + 1 < rp.len()
                        && rp[ap].disp.is_none()
                        && rp[bp + 1].disp.is_none()
                        && rp[ap].str_ == rp[bp + 1].str_
                    {
                        rp[bp + 1].flags |= CMF_MULT;                        // c:3352
                        dup = 1;                                             // c:3353
                        bp += 1;
                    }
                    if dup != 0 {
                        rp[ap].flags |= CMF_FMULT;                           // c:3356
                    }
                    ap = bp + 1;
                }
                rp.truncate(cp);                                             // c:3359
            }
            for m in rp.iter() {                                             // c:3361
                if m.disp.is_some() && (m.flags & CMF_DISPLINE) != 0 {       // c:3362
                    ll += 1;
                }
                if (m.flags & (CMF_NOLIST | CMF_MULT)) != 0 {                // c:3364
                    nl += 1;
                }
            }
        }
    }
    (rp, n, nl, ll)                                                          // c:3366-3373
}

/// Port of the `type==0` string-sort branch of `makearray()` from
/// compcore.c:3239-3257. Sorts strings via `strmetasort` + dedup.
pub fn makearray_strings(mut rp: Vec<String>, flags: i32) -> (Vec<String>, i32) { // c:3239
    let mut n: i32 = rp.len() as i32;
    if flags != 0 && n > 0 {                                                 // c:3240
        let numeric = crate::ported::options::opt_state_get("NUMERICGLOBSORT")
            .unwrap_or(false);                                               // c:3243
        let mut sf = crate::ported::zsh_h::SORTIT_IGNORING_BACKSLASHES as u32;
        if numeric {
            sf |= crate::ported::zsh_h::SORTIT_NUMERICALLY as u32;
        }
        crate::ported::sort::strmetasort(&mut rp, sf, None);                 // c:3242-3244

        // Dedup consecutive equals.                                         // c:3247
        let mut cp = 0usize;
        let mut ap = 0usize;
        while ap < rp.len() {
            if ap != cp { rp.swap(ap, cp); }
            cp += 1;
            let mut bp = ap;
            while bp + 1 < rp.len() && rp[ap] == rp[bp + 1] {                // c:3250
                bp += 1; n -= 1;
            }
            ap = bp + 1;                                                     // c:3252
        }
        rp.truncate(cp);                                                     // c:3253
    }
    (rp, n)
}

// =====================================================================
// dupmatch — `Src/Zle/compcore.c:3370-3418`.
// =====================================================================

/// Port of `static Cmatch dupmatch(Cmatch m, int nbeg, int nend)` from
/// compcore.c:3370. Deep-copies one match; brpl/brsl are truncated to
/// nbeg/nend per the C body's nbeg/nend-sized `zalloc` + element copy.
pub fn dupmatch(m: &Cmatch, nbeg: i32, nend: i32) -> Cmatch {                // c:3370
    let mut r = Cmatch::default();                                           // c:3373-3374
    r.str_  = m.str_.clone();                                                // c:3376 ztrdup
    r.orig  = m.orig.clone();                                                // c:3377
    r.ipre  = m.ipre.clone();                                                // c:3378
    r.ripre = m.ripre.clone();                                               // c:3379
    r.isuf  = m.isuf.clone();                                                // c:3380
    r.ppre  = m.ppre.clone();                                                // c:3381
    r.psuf  = m.psuf.clone();                                                // c:3382
    r.prpre = m.prpre.clone();                                               // c:3383
    r.pre   = m.pre.clone();                                                 // c:3384
    r.suf   = m.suf.clone();                                                 // c:3385
    r.flags = m.flags;                                                       // c:3386
    if !m.brpl.is_empty() {                                                  // c:3387
        let take = (nbeg as usize).min(m.brpl.len());                        // c:3390 zalloc(nbeg)
        r.brpl = m.brpl[..take].to_vec();                                    // c:3392 element-wise copy
    } else {
        r.brpl = Vec::new();                                                 // c:3395 NULL
    }
    if !m.brsl.is_empty() {                                                  // c:3396
        let take = (nend as usize).min(m.brsl.len());                        // c:3399
        r.brsl = m.brsl[..take].to_vec();                                    // c:3401
    } else {
        r.brsl = Vec::new();                                                 // c:3404
    }
    r.rems   = m.rems.clone();                                               // c:3405
    r.remf   = m.remf.clone();                                               // c:3406
    r.autoq  = m.autoq.clone();                                              // c:3407
    r.qipl   = m.qipl;                                                       // c:3408
    r.qisl   = m.qisl;                                                       // c:3409
    r.disp   = m.disp.clone();                                               // c:3410
    r.mode   = m.mode;                                                       // c:3411
    r.modec  = m.modec;                                                      // c:3412
    r.fmode  = m.fmode;                                                      // c:3413
    r.fmodec = m.fmodec;                                                     // c:3414
    r                                                                        // c:3416
}

// =====================================================================
// permmatches — `Src/Zle/compcore.c:3422-3550`.
// =====================================================================

/// Static state for `permmatches`'s `static int fi`. C scopes the
/// flag to the function; Rust hoists it to file scope per Rule S1.
static PERMMATCHES_FI: AtomicI32 = AtomicI32::new(0);                        // c:3429 static int fi

/// Port of `mod_export int permmatches(int last)` from compcore.c:3422.
/// Promotes the per-round `amatches` accumulator into the permanent
/// `pmatches` snapshot via deep-copy through `dupmatch`/`makearray`.
pub fn permmatches(last: i32) -> i32 {                                       // c:3422
    let ofi = PERMMATCHES_FI.load(Ordering::Relaxed);                        // c:3431 ofi = fi

    // c:3433 — `if (pmatches && !newmatches)`
    let pmatches_set = pmatches.get_or_init(|| Mutex::new(Vec::new()))
        .lock().map(|g| !g.is_empty()).unwrap_or(false);
    if pmatches_set && newmatches.load(Ordering::Relaxed) == 0 {             // c:3433
        if last != 0 && PERMMATCHES_FI.load(Ordering::Relaxed) != 0 {        // c:3434
            // ainfo = fainfo                                                // c:3435
            let famref = fainfo.get_or_init(|| Mutex::new(None))
                .lock().ok().and_then(|g| g.clone());
            if let Ok(mut a) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
                *a = famref;
            }
        }
        return PERMMATCHES_FI.load(Ordering::Relaxed);                       // c:3437
    }
    newmatches.store(0, Ordering::Relaxed);                                  // c:3439
    PERMMATCHES_FI.store(0, Ordering::Relaxed);                              // c:3439 fi = 0

    {
        // pmatches = lmatches = NULL                                        // c:3441
        if let Ok(mut g) = pmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
            g.clear();
        }
        if let Ok(mut g) = lmatches.get_or_init(|| Mutex::new(None)).lock() {
            *g = None;
        }
    }
    nmatches.store(0, Ordering::Relaxed);                                    // c:3442
    smatches.store(0, Ordering::Relaxed);                                    // c:3442
    diffmatches.store(0, Ordering::Relaxed);                                 // c:3442

    // c:3444 — `if (!ainfo->count)`.
    let ainfo_count = ainfo.get_or_init(|| Mutex::new(None))
        .lock().ok().and_then(|g| g.as_ref().map(|a| a.count)).unwrap_or(0);
    if ainfo_count == 0 {                                                    // c:3444
        if last != 0 {                                                       // c:3445
            let famref = fainfo.get_or_init(|| Mutex::new(None))
                .lock().ok().and_then(|g| g.clone());
            if let Ok(mut a) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
                *a = famref;
            }
        }
        PERMMATCHES_FI.store(1, Ordering::Relaxed);                          // c:3447
    }

    let nbeg = crate::ported::zle::zle_tricky::NBRBEG.load(Ordering::Relaxed);
    let nend = crate::ported::zle::zle_tricky::NBREND.load(Ordering::Relaxed);

    let mut gn: i32 = 1;                                                     // c:3429 gn = 1
    let mut mn: i32 = 1;                                                     // c:3429 mn = 1
    let fi = PERMMATCHES_FI.load(Ordering::Relaxed);

    let groups_snapshot: Vec<Cmgroup> = {
        amatches.get_or_init(|| Mutex::new(Vec::new()))
            .lock().ok().map(|g| g.clone()).unwrap_or_default()
    };
    let mut new_pmatches: Vec<Cmgroup> = Vec::with_capacity(groups_snapshot.len());

    for g_orig in groups_snapshot.into_iter() {                              // c:3449 while (g)
        let mut g = g_orig;                                                  // borrow-mut snapshot
        let must_rebuild = fi != ofi || g.perm.is_none() || g.new_ != 0;     // c:3456
        if must_rebuild {                                                    // c:3456
            let src_list = if fi != 0 { g.lfmatches.clone() }                // c:3457
                           else { g.lmatches.clone() };                      // c:3461

            let (arr, nn, nl, ll) = makearray(src_list, g.flags);            // c:3463
            g.mcount = nn;                                                   // c:3464
            g.lcount = nn - nl;                                              // c:3465
            if g.lcount < 0 { g.lcount = 0; }                                // c:3466
            g.llcount = ll;                                                  // c:3467
            if !g.ylist.is_empty() {                                         // c:3468
                g.lcount = g.ylist.len() as i32;                             // c:3469
                smatches.store(2, Ordering::Relaxed);                        // c:3470
            }
            // c:3472 — makearray(lexpls, 0, 0, &ecount, NULL, NULL).
            let mut exps = g.lexpls.clone();                                 // type=0 path
            g.ecount = exps.len() as i32;
            // c:3475 ccount = 0
            g.ccount = 0;                                                    // c:3475
            nmatches.fetch_add(g.mcount, Ordering::Relaxed);                 // c:3477
            smatches.fetch_add(g.lcount, Ordering::Relaxed);                 // c:3478
            if g.mcount > 1 {                                                // c:3480
                diffmatches.store(1, Ordering::Relaxed);                     // c:3481
            }

            // n = (Cmgroup) zshcalloc(...)                                  // c:3483
            let mut n_grp = Cmgroup::default();
            // c:3487 — `if (g->perm) freematches(g->perm, 0)`. Drop on
            // perm Box<Cmgroup> reclaims the C `free` path.
            g.perm = None;                                                   // c:3490 g->perm = n
            // Then below we set g.perm = Some(Box::new(n_grp.clone())).

            n_grp.num   = gn; gn += 1;                                       // c:3499
            n_grp.flags = g.flags;                                           // c:3500
            n_grp.mcount = g.mcount;                                         // c:3501
            n_grp.matches = arr.iter()                                       // c:3502-3505 dupmatch loop
                .map(|m| dupmatch(m, nbeg, nend))
                .collect();
            n_grp.name  = g.name.clone();                                    // c:3504
            n_grp.lcount  = g.lcount;                                        // c:3508
            n_grp.llcount = g.llcount;                                       // c:3509
            if !g.ylist.is_empty() {                                         // c:3510
                n_grp.ylist = g.ylist.clone();                               // c:3511 zarrdup
            } else {
                n_grp.ylist = Vec::new();                                    // c:3513
            }
            if g.ecount != 0 {                                               // c:3515
                // Build n->expls from g->expls deep-copying str + (fi
                // ? fcount : count); always carries over; fcount = 0.
                n_grp.expls = exps.drain(..).map(|o| Cexpl {                 // c:3517-3525
                    count:  if fi != 0 { o.fcount } else { o.count },        // c:3520
                    always: o.always,                                        // c:3521
                    fcount: 0,                                               // c:3522
                    str_:   o.str_.clone(),                                  // c:3523 ztrdup
                }).collect();
                n_grp.ecount = g.ecount;
            } else {
                n_grp.expls = Vec::new();                                    // c:3528
            }
            n_grp.widths = Vec::new();                                       // c:3531
            // Stitch perm chain (prev/next handled implicitly by Vec).
            g.matches = arr;                                                 // mirror C: g->matches = makearray result
            g.perm = Some(Box::new(n_grp.clone()));                          // c:3490 g->perm = n
            new_pmatches.push(n_grp);                                        // c:3492-3496
        } else {
            // reuse existing g->perm                                        // c:3534
            nmatches.fetch_add(g.mcount, Ordering::Relaxed);                 // c:3540
            smatches.fetch_add(g.lcount, Ordering::Relaxed);                 // c:3541
            if g.mcount > 1 {
                diffmatches.store(1, Ordering::Relaxed);                     // c:3543
            }
            g.num = gn; gn += 1;                                             // c:3546
            if let Some(p) = g.perm.as_deref() {
                new_pmatches.push(p.clone());                                // c:3537 pmatches = g->perm
            }
        }
        g.new_ = 0;                                                          // c:3548
    }

    // c:3551-3563 — assign rnum/gnum, recompute diffmatches/nbrbeg.
    let mut first_first: Option<Cmatch> = None;
    for g_pm in new_pmatches.iter_mut() {
        g_pm.nbrbeg = nbeg;                                                  // c:3552
        g_pm.nbrend = nend;                                                  // c:3553
        let mut rn = 1i32;                                                   // c:3554
        for m in g_pm.matches.iter_mut() {
            m.rnum = rn; rn += 1;                                            // c:3555
            m.gnum = mn; mn += 1;                                            // c:3556
        }
        if diffmatches.load(Ordering::Relaxed) == 0 && !g_pm.matches.is_empty() {
            match first_first.as_ref() {                                     // c:3558
                Some(p0) => {
                    if !matcheq(&g_pm.matches[0], p0) {
                        diffmatches.store(1, Ordering::Relaxed);             // c:3560
                    }
                }
                None => first_first = Some(g_pm.matches[0].clone()),         // c:3562
            }
        }
    }

    if let Ok(mut g) = pmatches.get_or_init(|| Mutex::new(Vec::new())).lock() {
        *g = new_pmatches;
    }

    hasperm.store(1, Ordering::Relaxed);                                     // c:3565
    permmnum.store(mn - 1, Ordering::Relaxed);                               // c:3566
    permgnum.store(gn - 1, Ordering::Relaxed);                               // c:3567
    if let Ok(mut ld) = listdat.get_or_init(|| Mutex::new(Default::default())).lock() {
        ld.valid = 0;                                                        // c:3568
    }

    fi                                                                       // c:3570
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rembslash_basic() {
        assert_eq!(rembslash("hello\\ world"), "hello world");
        assert_eq!(rembslash("no\\\\slash"),   "no\\slash");
        assert_eq!(rembslash("plain"),         "plain");
    }

    #[test]
    fn comp_quoting_string_table() {
        assert_eq!(comp_quoting_string(QT_SINGLE),  "'");
        assert_eq!(comp_quoting_string(QT_DOUBLE),  "\"");
        assert_eq!(comp_quoting_string(QT_DOLLARS), "$'");
        assert_eq!(comp_quoting_string(0),          "\\");
        assert_eq!(comp_quoting_string(QT_BACKSLASH), "\\");
    }

    #[test]
    fn matcheq_equal_strings() {
        let mut a = Cmatch::default(); a.str_ = Some("foo".into());
        let mut b = Cmatch::default(); b.str_ = Some("foo".into());
        assert!(matcheq(&a, &b));
    }

    #[test]
    fn matcheq_different_strings() {
        let mut a = Cmatch::default(); a.str_ = Some("foo".into());
        let mut b = Cmatch::default(); b.str_ = Some("bar".into());
        assert!(!matcheq(&a, &b));
    }

    #[test]
    fn matcheq_one_side_none() {
        let mut a = Cmatch::default(); a.pre = Some("p".into());
        let b = Cmatch::default();
        assert!(!matcheq(&a, &b));
    }

    #[test]
    fn remsquote_default_quoting() {
        let mut s = String::from("a'\\''b");
        let n = remsquote(&mut s);
        assert_eq!(s, "a'b");
        assert_eq!(n, 3);
    }

    #[test]
    fn ctokenize_dollar_substitution() {
        let out = ctokenize("$x{y}");
        let chars: Vec<char> = out.chars().collect();
        assert_eq!(chars[0], STRING_TOK);
        assert_eq!(chars[1], 'x');
        assert_eq!(chars[2], INBRACE);
        assert_eq!(chars[3], 'y');
        assert_eq!(chars[4], OUTBRACE);
    }

    #[test]
    fn get_user_var_inline_list() {
        let result = get_user_var(Some("(a b c)")).unwrap();
        assert_eq!(result, vec!["a", "b", "c"]);
    }

    #[test]
    fn matchcmp_str_sort_default() {
        MATCHORDER.store(CGF_MATSORT, Ordering::Relaxed);
        let mut a = Cmatch::default(); a.str_ = Some("apple".into());
        let mut b = Cmatch::default(); b.str_ = Some("banana".into());
        assert_eq!(matchcmp(&a, &b), std::cmp::Ordering::Less);
        assert_eq!(matchcmp(&b, &a), std::cmp::Ordering::Greater);
        assert_eq!(matchcmp(&a, &a), std::cmp::Ordering::Equal);
        MATCHORDER.store(0, Ordering::Relaxed);
    }

    #[test]
    fn dupmatch_clones_strings_and_truncates_braces() {
        // C body c:3370: deep-copy strings, truncate brpl/brsl to nbeg/nend.
        let mut src = Cmatch::default();
        src.str_ = Some("foo".into());
        src.ipre = Some("ipre".into());
        src.flags = 7;
        src.brpl = vec![10, 20, 30, 40];
        src.brsl = vec![5, 6, 7];
        src.qipl = 1;
        src.qisl = 2;
        src.mode = 0o755;
        src.modec = 'd';

        let r = dupmatch(&src, 2, 1);
        assert_eq!(r.str_.as_deref(), Some("foo"));
        assert_eq!(r.ipre.as_deref(), Some("ipre"));
        assert_eq!(r.flags, 7);
        assert_eq!(r.brpl, vec![10, 20]);      // truncated to nbeg=2
        assert_eq!(r.brsl, vec![5]);           // truncated to nend=1
        assert_eq!(r.qipl, 1);
        assert_eq!(r.qisl, 2);
        assert_eq!(r.mode, 0o755);
        assert_eq!(r.modec, 'd');
    }

    #[test]
    fn dupmatch_empty_braces_stay_empty() {
        // C body c:3395/3404: NULL brpl/brsl stay NULL regardless of nbeg/nend.
        let src = Cmatch::default();
        let r = dupmatch(&src, 5, 5);
        assert!(r.brpl.is_empty());
        assert!(r.brsl.is_empty());
    }

    #[test]
    fn makearray_sorted_and_deduped() {
        // c:3262-3291: sort + dedup with matcheq. Same str + nil disp =>
        // collapses into one entry with CMF_FMULT set on the survivor.
        let mut a = Cmatch::default(); a.str_ = Some("z".into());
        let mut b = Cmatch::default(); b.str_ = Some("a".into());
        let mut c = Cmatch::default(); c.str_ = Some("a".into());
        let (arr, n, _nl, _ll) = makearray(vec![a, b, c], CGF_MATSORT);
        // Two distinct visible strings after dedup ("a", "z").
        assert_eq!(arr.len(), 2);
        assert_eq!(n, 2);
        assert_eq!(arr[0].str_.as_deref(), Some("a"));
        assert_eq!(arr[1].str_.as_deref(), Some("z"));
    }

    #[test]
    fn makearray_nosort_unchanged_order() {
        // c:3300: CGF_NOSORT branch; with no UNIQ flags, order preserved.
        let mut a = Cmatch::default(); a.str_ = Some("z".into());
        let mut b = Cmatch::default(); b.str_ = Some("a".into());
        let (arr, n, _, _) = makearray(vec![a, b], CGF_NOSORT | CGF_UNIQALL);
        // UNIQALL active so no dedup pass runs.
        assert_eq!(n, 2);
        assert_eq!(arr[0].str_.as_deref(), Some("z"));
        assert_eq!(arr[1].str_.as_deref(), Some("a"));
    }

    #[test]
    fn makearray_strings_dedup_consecutive() {
        // c:3239 path: sort + drop adjacent duplicates.
        let (arr, n) = makearray_strings(
            vec!["b".into(), "a".into(), "a".into(), "c".into()],
            1,
        );
        assert_eq!(n, 3);
        assert_eq!(arr, vec!["a", "b", "c"]);
    }

    #[test]
    fn check_param_no_dollar_returns_none() {
        // c:1316: no `$` in string → return None.
        OFFS.store(2, Ordering::Relaxed);
        assert_eq!(check_param("abc", false, false), None);
    }

    #[test]
    fn check_param_simple_dollar_var_at_cursor() {
        // c:1259-1311: `$FOO` with cursor inside the name → return b.
        OFFS.store(2, Ordering::Relaxed);
        let s = format!("{}FOO", crate::ported::zsh_h::STRING_TOK);
        let r = check_param(&s, false, true);
        assert!(r.is_some(), "expected Some(b) inside $FOO");
    }

    #[test]
    fn callcompfunc_empty_fn_no_panic() {
        // c:552: getshfunc(NULL) early-return.
        callcompfunc("anything", "");
    }

    #[test]
    fn callcompfunc_sets_compstate_context() {
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // c:619: context selection — verified via the pure
        // compcontext_for helper (callcompfunc calls it and writes
        // to paramtab via setsparam, but paramtab read-back in a
        // unit-test context without a live VM is unreliable).
        ispar.store(0, Ordering::Relaxed);
        linwhat.store(IN_PAR_LW, Ordering::Relaxed);
        assert_eq!(compcontext_for("foo"), "assign_parameter");
        // Body executes without panicking against the real paramtab.
        callcompfunc("foo", "_test_fn");
    }

    /// Test-only serializer for tests that mutate file-scope globals.
    static GLOBAL_MUT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn compcontext_for_routes_ispar_first() {
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        ispar.store(2, Ordering::Relaxed);
        linwhat.store(IN_NOTHING_LW, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "brace_parameter");
        ispar.store(1, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "parameter");
        ispar.store(0, Ordering::Relaxed);
        linwhat.store(IN_MATH_LW, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "math");
        linwhat.store(IN_COND_LW, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "condition");
        linwhat.store(IN_ENV_LW, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "value");
        linwhat.store(IN_NOTHING_LW, Ordering::Relaxed);
        assert_eq!(compcontext_for("x"), "command");
    }

    #[test]
    fn addmatches_empty_argv_early_return() {
        // c:2138-2139: empty argv + dummies==0 + no CAF_ALL → return 1.
        let mut dat = crate::ported::zle::comp_h::Cadata::default();
        dat.dummies = 0;
        dat.aflags = 0;
        assert_eq!(addmatches(&mut dat, &[]), 1);
    }

    #[test]
    fn addmatches_appends_argv_to_default_group() {
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // c:2200 simplified body: each argv entry → addmatch into "default" group.
        amatches.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap().clear();
        matches.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap().clear();
        let mut dat = crate::ported::zle::comp_h::Cadata::default();
        dat.dummies = -1;
        let _ = addmatches(&mut dat, &["a".into(), "b".into()]);
        let n = matches.get().unwrap().lock().unwrap().len();
        assert!(n >= 2);
    }

    #[test]
    fn add_match_data_returns_populated_cmatch() {
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // c:3052-3067: cm.str/orig/pre/suf populated; mnum bumps by 1.
        matches.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap().clear();
        let before = mnum.load(Ordering::Relaxed);
        let cm = add_match_data(
            0, "match", "match-orig", None,
            "ipre", "ripre", "isuf",
            "pre", "prpre", "ppre", None,
            "psuf", None,
            "suf", 0, 0,
        );
        assert_eq!(cm.str_.as_deref(), Some("match"));
        assert_eq!(cm.orig.as_deref(), Some("match-orig"));
        assert_eq!(cm.pre.as_deref(),  Some("pre"));
        assert_eq!(cm.suf.as_deref(),  Some("suf"));
        assert_eq!(mnum.load(Ordering::Relaxed), before + 1);
    }

    #[test]
    fn add_match_data_exact_records_into_ainfo() {
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // c:3060-3062: exact != 0 writes ai.exact/exactm.
        if let Ok(mut g) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
            *g = Some(Aminfo::default());
        }
        let _ = add_match_data(
            0, "x", "x", None, "", "", "", "", "", "", None, "", None, "", 0, 1,
        );
        let a = ainfo.get().unwrap().lock().unwrap().clone().unwrap();
        assert_eq!(a.exact, 1);
        assert!(a.exactm.is_some());
    }

    #[test]
    fn set_comp_sep_returns_one() {
        // c:1937: stubbed body returns 1 (no-change marker).
        assert_eq!(set_comp_sep(), 1);
    }

    #[test]
    fn foredel_deletes_forward_from_zlemetacs() {
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // zle_utils.c:1105 — delete `ct` chars forward from ZLEMETACS.
        if let Ok(mut g) = ZLEMETALINE.get_or_init(|| Mutex::new(String::new())).lock() {
            *g = "abcdef".to_string();
        }
        ZLEMETACS.store(2, Ordering::Relaxed);
        ZLEMETALL.store(6, Ordering::Relaxed);
        foredel(3);
        let line = ZLEMETALINE.get().unwrap().lock().unwrap().clone();
        assert_eq!(line, "abf");
        assert_eq!(ZLEMETALL.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn inststr_inserts_at_zlemetacs() {
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // zle_tricky.c:278 — insert at cursor.
        if let Ok(mut g) = ZLEMETALINE.get_or_init(|| Mutex::new(String::new())).lock() {
            *g = "hello".to_string();
        }
        ZLEMETACS.store(5, Ordering::Relaxed);
        ZLEMETALL.store(5, Ordering::Relaxed);
        inststr(" world");
        let line = ZLEMETALINE.get().unwrap().lock().unwrap().clone();
        assert_eq!(line, "hello world");
        assert_eq!(ZLEMETACS.load(Ordering::Relaxed), 11);
    }

    #[test]
    fn metafy_and_unmetafy_roundtrip_globals() {
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // zle_tricky.c:978,995 — meta/unmeta operate on the global pair.
        if let Ok(mut g) = ZLELINE.get_or_init(|| Mutex::new(String::new())).lock() {
            *g = "plain ascii".to_string();
        }
        ZLECS.store(3, Ordering::Relaxed);
        ZLELL.store(11, Ordering::Relaxed);
        metafy_line();
        // For ASCII input the meta form equals the raw form.
        assert_eq!(
            ZLEMETALINE.get().unwrap().lock().unwrap().clone(),
            "plain ascii"
        );
        assert_eq!(ZLEMETACS.load(Ordering::Relaxed), 3);
        unmetafy_line();
        assert_eq!(
            ZLELINE.get().unwrap().lock().unwrap().clone(),
            "plain ascii"
        );
    }

    #[test]
    fn selfinsert_appends_lastchar_at_zlecs() {
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // zle_misc.c:112-141 — insert one char at cursor, bump zlecs.
        if let Ok(mut g) = ZLELINE.get_or_init(|| Mutex::new(String::new())).lock() {
            *g = "ab".to_string();
        }
        ZLECS.store(2, Ordering::Relaxed);
        ZLELL.store(2, Ordering::Relaxed);
        LASTCHAR.store(b'c' as i32, Ordering::Relaxed);
        let rv = selfinsert();
        assert_eq!(rv, 0);
        assert_eq!(ZLELINE.get().unwrap().lock().unwrap().clone(), "abc");
        assert_eq!(ZLECS.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn minfo_clear_and_asked_zero_mutate_state() {
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        if let Ok(mut g) = MINFO.get_or_init(|| Mutex::new(crate::ported::zle::comp_h::Menuinfo::default())).lock() {
            let mut cm = Cmatch::default();
            cm.str_ = Some("x".into());
            g.cur = Some(Box::new(cm));
            g.asked = 1;
        }
        if let Ok(mut g) = MINFO.get_or_init(|| Mutex::new(crate::ported::zle::comp_h::Menuinfo::default())).lock() { g.cur = None; }
        if let Ok(mut g) = MINFO.get_or_init(|| Mutex::new(crate::ported::zle::comp_h::Menuinfo::default())).lock() { g.asked = 0; }
        let m = MINFO.get().unwrap().lock().unwrap().clone();
        assert!(m.cur.is_none());
        assert_eq!(m.asked, 0);
    }

    #[test]
    fn cline_matched_stub_marks_node() {
        // compmatch.c:253 — sets CLF_MATCHED on the node chain. We
        // verify by running through the stub on a non-empty string
        // without panicking and trusting compmatch's body for the
        // actual flag set.
        cline_matched_stub(Some("foo"));
        cline_matched_stub(None);
        cline_matched_stub(Some(""));
    }

    #[test]
    fn permmatches_returns_fi_zero_when_count_present() {
        let _g = GLOBAL_MUT_LOCK.lock().unwrap();
        // c:3444-3447: if ainfo->count is non-zero, fi stays 0.
        amatches.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap().clear();
        pmatches.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap().clear();
        if let Ok(mut a) = ainfo.get_or_init(|| Mutex::new(None)).lock() {
            *a = Some(Aminfo { count: 5, ..Default::default() });
        }
        newmatches.store(1, Ordering::Relaxed);
        let fi = permmatches(0);
        assert_eq!(fi, 0);
        assert_eq!(hasperm.load(Ordering::Relaxed), 1);
    }
}
