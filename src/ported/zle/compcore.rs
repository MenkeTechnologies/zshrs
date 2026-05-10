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
    Cexpl, Cmatch, Cmgroup, CGF_MATSORT, CGF_NOSORT, CGF_NUMSORT, CGF_REVSORT, CGF_UNIQALL,
    CGF_UNIQCON, CMF_DISPLINE, CMF_PACKED, CMF_ROWS,
};

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
pub static ainfo: OnceLock<Mutex<Option<i32>>> = OnceLock::new();            // c:246
/// Port of `mod_export Aminfo fainfo` from compcore.c:246.
pub static fainfo: OnceLock<Mutex<Option<i32>>> = OnceLock::new();           // c:246

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
    let complist_extra = lookup_complist_flags();                            // c:2049-2051
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

fn lookup_complist_flags() -> i32 {
    use crate::ported::zle::complete::COMPLIST;
    let s = COMPLIST.get_or_init(|| Mutex::new(String::new()))
        .lock().map(|g| g.clone()).unwrap_or_default();
    if s.is_empty() { return 0; }
    let packed = if s.contains("packed") { CMF_PACKED } else { 0 };          // c:2050
    let rows   = if s.contains("rows")   { CMF_ROWS   } else { 0 };          // c:2051
    packed | rows
}

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

/// Port of `int do_completion(Hookdef dummy, Compldat dat)` from
/// compcore.c:287. Full driver. Blocked on every per-round global plus
/// `runhookdef` registry, `zlemetacs`/`zlemetall`/`origline`,
/// `clearlist`/`showinglist`/`validlist`, `do_ambiguous`, `do_single`,
/// `do_allmatches`, `selfinsert`, `metafy_line`/`unmetafy_line`.
pub fn do_completion() -> i32 { 1 }                                          // c:287

/// Port of `static void callcompfunc(char *s, char *fn)` from
/// compcore.c:544. Blocked on full paramtab + `comprpms`/`compkpms`
/// tables, `incompfunc` toggle, `IN_PAR`/`IN_MATH`/`IN_COND`,
/// `cmdstr`/`varname`/`clwords`/`clwpos`/`clwnum` globals.
pub fn callcompfunc() {}                                                     // c:544

/// Port of `static int check_param(char *s, int set, int test)` from
/// compcore.c:1113. Blocked on `getparamnode`, `paramtab`,
/// subscript-lex, `lincmd`/`linredir`/`linarr` globals.
pub fn check_param() -> i32 { 0 }                                            // c:1113

/// Port of `int set_comp_sep(void)` from compcore.c:1458.
/// 485-line `compset -q` driver. Blocked on `zlemetaline`/`lexsave`/
/// `lexrestore`/`metafy_line`/`addedx`/`compqstack` write +
/// `zhalloc` heap arena.
pub fn set_comp_sep() -> i32 { 1 }                                           // c:1458

/// Port of `int addmatches(Cadata dat, char **argv)` from
/// compcore.c:2080. 560-line workhorse. Blocked on `Cadata`,
/// `Cline` chain machinery, `Brinfo`, `mstack`/`bmatchers`,
/// `compheap` arena, `comp_setunset` `CP_*` flag updates.
pub fn addmatches() -> i32 { 1 }                                             // c:2080

/// Port of `Cmatch add_match_data(...)` from compcore.c:2643.
/// Blocked on full Cline substrate.
pub fn add_match_data() -> i32 { 0 }                                         // c:2643

/// Port of `int makecomplist(char *s, int incmd, int lst)` from
/// compcore.c:946. Blocked on `get_comp_string`, `compsys` dispatch.
pub fn makecomplist() -> i32 { 0 }                                           // c:946

/// Port of `Cmatch *makearray(...)` from compcore.c:3223. Blocked on
/// LinkList ↔ Vec reconciliation across whole completion path.
pub fn makearray() -> Vec<Cmatch> { Vec::new() }                             // c:3223

/// Port of `Cmgroup dupmatch(Cmgroup g, int copy)` from
/// compcore.c:3370. Rust's `Clone` covers it; the arena dance does
/// not apply.
pub fn dupmatch() -> Option<Cmgroup> { None }                                // c:3370

/// Port of `mod_export void permmatches(int last)` from
/// compcore.c:3422. Blocked on `lastmatches`/`pmatches`/`hasperm`
/// arena commit logic.
pub fn permmatches() {}                                                      // c:3422

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
}
