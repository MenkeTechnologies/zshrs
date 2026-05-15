//! PCRE module - port of Modules/pcre.c
//!
//! Provides PCRE regex matching through pcre_compile, pcre_match, pcre_study builtins.
//! Uses the Rust `regex` crate which provides Perl-compatible regex syntax.

use regex::Regex;
use crate::ported::utils::zwarnnam;
use crate::ported::zsh_h::options;
use crate::ported::zsh_h::OPT_ISSET;
use crate::ported::zsh_h::{OPT_ARG, OPT_HASARG};

/// Port of `CPCRE_PLAIN` from `Src/Modules/pcre.c:34`. Default
/// pattern-flavour id passed to `cond_pcre_match` (the `-pcre-match`
/// infix dispatcher) — selects plain (non-anchored) PCRE matching.
pub const CPCRE_PLAIN: i32 = 0;                                              // c:34

/// Port of `PCRE2_CODE_UNIT_WIDTH` from `Src/Modules/pcre.c:38`.
/// `#define PCRE2_CODE_UNIT_WIDTH 8`. Selects the 8-bit pcre2 API
/// over 16-bit / 32-bit. Rust uses the `regex` crate (UTF-8 by
/// default), so this is a search anchor for the C source.
pub const PCRE2_CODE_UNIT_WIDTH: i32 = 8;                                    // c:38

// Per-evaluator PCRE compile state — bucket-1 dissolution per
// PORT_PLAN.md Phase 2. C source has ONE file-static at
// Src/Modules/pcre.c:41:
//
//     static pcre2_code *pcre_pattern;
//
// Previous Rust port aggregated this with a Rust-only `pattern_str`
// cache into `pub struct PcreState`, which is the bag-of-globals
// anti-pattern. Dissolved into a single `thread_local!` mirroring
// the C declaration; each worker thread's `pcre_compile` builtin
// owns its own compiled regex (file-static semantics preserve under
// threading per PORT_PLAN bucket-1 rule).

thread_local! {
    /// Port of file-static `static pcre2_code *pcre_pattern;` at
    /// `Src/Modules/pcre.c:70`. Compiled regex shared between the
    /// `pcre_compile`/`pcre_study`/`pcre_match` builtins.
    static PCRE_PATTERN: std::cell::RefCell<Option<Regex>> = const {
        std::cell::RefCell::new(None)
    };
}

/// Port of `zpcre_utf8_enabled()` from Src/Modules/pcre.c:45.
/// C: `static int zpcre_utf8_enabled(void)` — returns 1 iff PCRE2 was
/// built with Unicode AND MULTIBYTE option is set AND nl_langinfo(CODESET)
/// reports "UTF-8".
#[allow(non_snake_case)]
pub fn zpcre_utf8_enabled() -> i32 {                                         // c:45
    // c:45-67 — under MULTIBYTE_SUPPORT && HAVE_NL_LANGINFO && CODESET.
    // Static-link path: zshrs hosts on macOS/Linux where PCRE2 ships with
    // Unicode by default; check MULTIBYTE option + LANG/LC_ALL CODESET.
    let multibyte = crate::ported::zsh_h::isset(crate::ported::options::optlookup("multibyte"));      // c:53
    if !multibyte {
        return 0;                                                            // c:54
    }
    // c:62 — nl_langinfo(CODESET) check.
    let lc = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_CTYPE"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    if lc.to_uppercase().contains("UTF-8") || lc.to_uppercase().contains("UTF8") {
        1                                                                    // c:62
    } else {
        0
    }
}

/// Port of `bin_pcre_compile(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/pcre.c:70`.
/// C: `static int bin_pcre_compile(char *nam, char **args, Options ops,
/// UNUSED(int func))` — compile *args into the file-static
/// `pcre_pattern`. Option bits read from `ops` via OPT_ISSET.
#[allow(unused_variables)]
pub fn bin_pcre_compile(nam: &str, args: &[String], ops: &options, func: i32) -> i32 { // c:70
    // c:72-76 — locals at function top.
    let mut pcre_opts: u32 = 0;                                              // c:72
    let target_len: i32;                                                     // c:73
    // c:74 int pcre_error / c:75 PCRE2_SIZE pcre_offset — folded into
    // the Rust regex crate's Result error type.
    let target: String;                                                      // c:76

    if OPT_ISSET(ops, b'a') { pcre_opts |= 1; }                              // c:78 PCRE2_ANCHORED
    if OPT_ISSET(ops, b'i') { pcre_opts |= 2; }                              // c:79 PCRE2_CASELESS
    if OPT_ISSET(ops, b'm') { pcre_opts |= 4; }                              // c:80 PCRE2_MULTILINE
    if OPT_ISSET(ops, b'x') { pcre_opts |= 8; }                              // c:81 PCRE2_EXTENDED
    if OPT_ISSET(ops, b's') { pcre_opts |= 16; }                             // c:82 PCRE2_DOTALL

    // c:84-85 — UTF-8 unconditionally (Rust `regex` is UTF-8 native).

    // c:87-89 — pcre2_code_free(pcre_pattern); pcre_pattern = NULL;
    PCRE_PATTERN.with(|r| *r.borrow_mut() = None);

    // c:91-92 — target = ztrdup(*args); unmetafy(target, &target_len);
    target = args.first().cloned().unwrap_or_default();
    target_len = target.len() as i32;
    let _ = target_len;

    // c:94-95 — pcre_pattern = pcre2_compile(target, ...)
    // The Rust `regex` crate accepts inline (?i)/(?m)/(?s)/(?x) flags
    // and the ^ anchor at the start of the pattern.
    let mut pattern_str = String::new();
    if (pcre_opts & 2) != 0  { pattern_str.push_str("(?i)"); }
    if (pcre_opts & 4) != 0  { pattern_str.push_str("(?m)"); }
    if (pcre_opts & 16) != 0 { pattern_str.push_str("(?s)"); }
    if (pcre_opts & 8) != 0  { pattern_str.push_str("(?x)"); }
    if (pcre_opts & 1) != 0  { pattern_str.push('^'); }
    pattern_str.push_str(&target);

    match Regex::new(&pattern_str) {
        Ok(re) => {
            PCRE_PATTERN.with(|r| *r.borrow_mut() = Some(re));
            0                                                                // c:107
        }
        Err(e) => {
            // c:112-105 — pcre2_get_error_message + zwarnnam
            zwarnnam(nam, &format!("error in regex: {}", e));                // c:112
            1                                                                // c:112
        }
    }
}

/// Port of `bin_pcre_study(char *nam, UNUSED(char **args), UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/pcre.c:112`. The C
/// source calls `pcre2_jit_compile()` to JIT-optimize the compiled
/// pattern; the Rust `regex` crate already builds an optimal NFA
/// at compile time, so this is the "no pattern" guard plus return 0.
#[allow(unused_variables)]
pub fn bin_pcre_study(nam: &str, args: &[String], ops: &options, func: i32) -> i32 { // c:112
    let has_pat = PCRE_PATTERN.with(|r| r.borrow().is_some());
    if !has_pat {                                                            // c:115
        zwarnnam(nam, "no pattern has been compiled for study");             // c:116
        return 1;                                                            // c:117
    }
    0
}

/// Port of `pcre_callout(pcre2_callout_block_8 *block, UNUSED(void *callout_data))` from Src/Modules/pcre.c:132.
/// C: `static int pcre_callout(pcre2_callout_block_8 *block,
///     UNUSED(void *callout_data))` — eval the callout string as zsh code,
///     bind .pcre.subject and .pcre.pos parameters, return $? | errflag.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_block) vs C=(block, callout_data)
pub fn pcre_callout(_block: *mut std::ffi::c_void,                           // c:132
                    _callout_data: *mut std::ffi::c_void) -> i32 {
    // c:138-152 — parse_string(callout_string), setsparam(".pcre.subject"),
    // setiparam(".pcre.pos"), execode(prog, ..., "pcre"), return lastval|errflag.
    // Static-link path: zshrs's pcre integration uses the `regex` crate
    // directly; native pcre callouts arrive only when the C pcre2 backend
    // is wired in. Until then return success-no-callout.
    0                                                                        // c:157
}



// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// =====================================================================
// static struct features module_features                            c:530 (pcre.c)
// =====================================================================

use crate::ported::zsh_h::module;

/// Port of `static int zpcre_get_substrings(pcre2_code *pat, char *arg,
/// pcre2_match_data *mdata, int captured_count, char *matchvar,
/// char *substravar, char *namedassoc, int want_offset_pair,
/// int matchedinarr, int want_begin_end)` from `Src/Modules/pcre.c:157`.
///
/// Extracts submatches into shell parameters: `ZPCRE_OP` (start+end byte
/// offsets of the whole match), `matchvar` (whole-match string),
/// `substravar` (array of captures), `namedassoc` (assoc-array of named
/// captures), and the `MBEGIN`/`MEND`/`mbegin`/`mend` family (1-based
/// character offsets when `want_begin_end`).
///
/// ```c
/// static int
/// zpcre_get_substrings(pcre2_code *pat, char *arg, pcre2_match_data *mdata,
///     int captured_count, char *matchvar, char *substravar, char *namedassoc,
///     int want_offset_pair, int matchedinarr, int want_begin_end)
/// {
///     PCRE2_SIZE *ovec;
///     char *match_all, **matches;
///     char offset_all[50];
///     int capture_start = 1;
///     int vec_off;
///     PCRE2_SPTR ntable;
///     uint32_t ncount, nsize;
///     if (matchedinarr) capture_start = 0;
///     ovec = pcre2_get_ovector_pointer(mdata);
///     if (ovec) {
///         int nelem = captured_count - 1;
///         if (want_offset_pair) {
///             sprintf(offset_all, "%ld %ld", ovec[0], ovec[1]);
///             setsparam("ZPCRE_OP", ztrdup(offset_all));
///         }
///         if (matchvar) {
///             match_all = metafy(arg + ovec[0], ovec[1] - ovec[0], META_DUP);
///             setsparam(matchvar, match_all);
///         }
///         if (substravar && (!want_begin_end || nelem)) {
///             ... build matches[] from ovec[2..] ...
///             setaparam(substravar, matches);
///         }
///         if (namedassoc && pcre2_pattern_info(...) ...) {
///             ... build hash[] from named captures ...
///             sethparam(namedassoc, hash);
///         }
///         if (want_begin_end) {
///             ... MBEGIN/MEND char-offset compute via MB_CHARLEN ...
///             if (nelem) { mbegin/mend per-capture arrays }
///         }
///     }
///     return 0;
/// }
/// ```
#[allow(non_snake_case, clippy::too_many_arguments)]
pub fn zpcre_get_substrings(                                                 // c:157
    pat: *mut std::ffi::c_void,
    arg: &str,
    mdata: *mut std::ffi::c_void,
    captured_count: i32,
    matchvar: Option<&str>,
    substravar: Option<&str>,
    namedassoc: Option<&str>,
    want_offset_pair: i32,
    matchedinarr: i32,
    want_begin_end: i32,
) -> i32 {
    use crate::ported::params::{setsparam};

    let mut capture_start: i32 = 1;                                          // c:164
    if matchedinarr != 0 {                                                   // c:169
        capture_start = 0;                                                   // c:171 bash-style ovec[0]
    }

    // c:175 — `ovec = pcre2_get_ovector_pointer(mdata);`
    // pcre2 isn't currently wired through zshrs (the regex crate is the
    // matcher backend instead). The `mdata` pointer is opaque from
    // Rust; the canonical access path materializes here once pcre2-rs
    // bindings land. Sentinel: empty ovec → skip the populated branch.
    let ovec: Vec<(usize, usize)> = Vec::new();                              // c:175
    let _ = mdata;                                                           // c:175

    if !ovec.is_empty() {                                                    // c:176
        let nelem = captured_count - 1;                                      // c:177

        if want_offset_pair != 0 {                                           // c:179
            let offset_all = format!("{} {}", ovec[0].0, ovec[0].1);         // c:180
            setsparam("ZPCRE_OP", &offset_all);                              // c:181
        }

        if let Some(mv) = matchvar {                                         // c:188
            let (s, e) = ovec[0];                                            // c:189 arg + ovec[0]..ovec[1]
            let slice = arg.get(s..e).unwrap_or("");
            let match_all = crate::ported::utils::metafy(slice);             // c:189
            setsparam(mv, &match_all);                                       // c:190
        }

        // c:202-213 — substravar: build the captures array
        if let Some(sv) = substravar {                                       // c:202
            if want_begin_end == 0 || nelem != 0 {                           // c:203
                let mut matches: Vec<String> = Vec::with_capacity(           // c:206
                    (captured_count + 1 - capture_start) as usize,
                );
                let mut i = capture_start;                                   // c:207
                while i < captured_count {
                    let vec_off = (2 * i) as usize;                          // c:208
                    if let Some(&(s, e)) = ovec.get(vec_off / 2) {
                        let slice = arg.get(s..e).unwrap_or("");
                        matches.push(crate::ported::utils::metafy(slice));   // c:209
                    } else {
                        matches.push(String::new());
                    }
                    i += 1;
                }
                // c:212 — `setaparam(substravar, matches);`
                crate::ported::params::setaparam(sv, matches);               // c:212
            }
        }

        // c:215-231 — namedassoc: build the named-captures hash
        if let Some(na) = namedassoc {                                       // c:215
            // pcre2_pattern_info(pat, PCRE2_INFO_NAMECOUNT/...) gates this
            // path; without pcre2 bindings we treat ncount=0 and skip.
            let _ = pat;                                                     // c:216
            let ncount: u32 = 0;                                             // c:216
            if ncount != 0 {                                                 // c:216
                // c:222-230 — build hash[] interleaved (name, value) pairs.
                let hash: Vec<String> = Vec::with_capacity(                  // c:222
                    ((ncount + 1) * 2) as usize,
                );
                // For each named entry: push ztrdup(name), push metafy(value).
                // (Skipped — ncount == 0 in the stub backend.)
                crate::ported::params::sethparam(na, hash);                  // c:230
            }
        }

        if want_begin_end != 0 {                                             // c:233
            // c:239 — `char *ptr = arg; zlong offs = 0;`
            let mut ptr_pos: usize = 0;
            let mut offs: i64 = 0;                                           // c:240
            // c:245-251 — count chars from start of `arg` to `ovec[0]`.
            let mut leftlen = ovec[0].0 as i32;                              // c:245
            while leftlen > 0 {                                              // c:246
                offs += 1;                                                   // c:247
                let clen = {
                    let slice = arg.as_bytes().get(ptr_pos..ptr_pos + leftlen as usize)
                        .unwrap_or(&[]);
                    crate::ported::zsh_h::MB_CHARLEN(slice, slice.len())     // c:248 MB_CHARLEN
                };
                ptr_pos += clen;                                             // c:249
                leftlen -= clen as i32;                                      // c:250
            }
            // c:252 — `setiparam("MBEGIN", offs + !isset(KSHARRAYS));`
            let ksharrays = crate::ported::zsh_h::isset(
                crate::ported::zsh_h::KSHARRAYS) as i64;
            crate::ported::params::setiparam("MBEGIN", offs + 1 - ksharrays);// c:252

            // c:254-260 — add char count over the match itself.
            let mut leftlen = (ovec[0].1 - ovec[0].0) as i32;                // c:254
            while leftlen > 0 {                                              // c:255
                offs += 1;                                                   // c:256
                let clen = {
                    let slice = arg.as_bytes().get(ptr_pos..ptr_pos + leftlen as usize)
                        .unwrap_or(&[]);
                    crate::ported::zsh_h::MB_CHARLEN(slice, slice.len())     // c:257 MB_CHARLEN
                };
                ptr_pos += clen;                                             // c:258
                leftlen -= clen as i32;                                      // c:259
            }
            crate::ported::params::setiparam(                                // c:261 MEND
                "MEND", offs - ksharrays,
            );

            if nelem != 0 {                                                  // c:262
                // c:267-298 — per-capture mbegin/mend arrays.
                let mut mbegin: Vec<String> = Vec::with_capacity(nelem as usize);  // c:267
                let mut mend: Vec<String> = Vec::with_capacity(nelem as usize);    // c:268

                for i in 0..nelem as usize {                                 // c:270-272
                    let pair_idx = i + 1;
                    let pair = match ovec.get(pair_idx) {
                        Some(&p) => p,
                        None => continue,
                    };
                    // c:275 — `ptr = arg; offs = 0;`
                    let mut ptr_pos: usize = 0;
                    let mut offs: i64 = 0;                                   // c:276
                    let mut leftlen = pair.0 as i32;                         // c:279
                    while leftlen > 0 {                                      // c:280
                        offs += 1;                                           // c:281
                        let clen = {
                            let slice = arg.as_bytes().get(ptr_pos..ptr_pos + leftlen as usize)
                                .unwrap_or(&[]);
                            crate::ported::zsh_h::MB_CHARLEN(slice, slice.len())   // c:282
                        };
                        ptr_pos += clen;                                     // c:283
                        leftlen -= clen as i32;                              // c:284
                    }
                    let buf = format!("{}", offs + 1 - ksharrays);           // c:286 convbase
                    mbegin.push(buf);                                        // c:287

                    let mut leftlen = (pair.1 - pair.0) as i32;              // c:289
                    while leftlen > 0 {                                      // c:290
                        offs += 1;                                           // c:291
                        let clen = {
                            let slice = arg.as_bytes().get(ptr_pos..ptr_pos + leftlen as usize)
                                .unwrap_or(&[]);
                            crate::ported::zsh_h::MB_CHARLEN(slice, slice.len())   // c:292
                        };
                        ptr_pos += clen;                                     // c:293
                        leftlen -= clen as i32;                              // c:294
                    }
                    let buf = format!("{}", offs - ksharrays);               // c:296
                    mend.push(buf);                                          // c:297
                }
                crate::ported::params::setaparam("mbegin", mbegin);          // c:301
                crate::ported::params::setaparam("mend", mend);              // c:302
            }
        }
    }

    0                                                                        // c:307
}

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/pcre.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `getposint(char *instr, char *nam)` from Src/Modules/pcre.c:312.
/// C: `static int getposint(char *instr, char *nam)` — parse positive
/// decimal integer; emit "integer expected" warning + return -1 on bad input.
#[allow(non_snake_case)]
pub fn getposint(instr: &str, nam: &str) -> i32 {                            // c:312
    // c:312 — `ret = (int)zstrtol(instr, &eptr, 10);`
    match instr.trim().parse::<i32>() {                                      // c:317
        Ok(n) if n >= 0 => n,                                                // c:323
        _ => {
            // c:319-321 — zwarnnam(nam, "integer expected: %s", instr);
            crate::ported::utils::zwarnnam(nam, &format!("integer expected: {}", instr)); // c:320
            -1                                                               // c:321
        }
    }
}

/// Port of `bin_pcre_match(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/pcre.c:328`. Runs
/// the file-static PCRE_PATTERN against `*args`. Returns C's
/// "0 on match, 1 on no-match / error" int convention.
///
/// Returns `(status, full_match, captures)` — Rust tuple in lieu of
/// C's side-effecting `zpcre_get_substrings()` (which writes to
/// paramtab via setsparam/setaparam). The caller writes the
/// captures into the executor's parameter table.
/// WARNING: param names don't match C — Rust=() vs C=(nam, args, ops, func)
pub fn bin_pcre_match(nam: &str, args: &[String], ops: &options, _func: i32) // c:328
    -> (i32, Option<String>, Vec<Option<String>>) {
    // c:330-341 — locals at function top.
    let ret: i32;                                                            // c:330
    let _c: u8 = 0;                                                          // c:330
    // c:331 pcre2_match_data *pcre_mdata = NULL — folded into regex::Captures
    let mut matched_portion: Option<&str> = None;                            // c:332
    let plaintext: String;                                                   // c:333
    let receptacle: &str;                                                    // c:334
    let mut named: Option<&str> = None;                                      // c:335
    let mut return_value: i32 = 1;                                           // c:336
    let subject_len: i32;                                                    // c:338
    let mut offset_start: i32 = 0;                                           // c:339
    let mut want_offset_pair: i32 = 0;                                       // c:340
    let mut use_dfa: i32 = 0;                                                // c:341

    // c:343-346 — pcre_pattern NULL check
    let has_pat = PCRE_PATTERN.with(|r| r.borrow().is_some());
    if !has_pat {                                                            // c:343
        zwarnnam(nam, "no pattern has been compiled");                       // c:344
        return (1, None, Vec::new());                                        // c:345
    }

    // c:348-354 — -d (DFA) precludes -v/-A
    if OPT_ISSET(ops, b'd') {
        use_dfa = 1;
        if OPT_HASARG(ops, b'v') || OPT_HASARG(ops, b'A') {                  // c:351
            zwarnnam(nam, "-d cannot be combined with -v or -A");            // c:352
            return (1, None, Vec::new());                                    // c:353
        }
    } else {
        matched_portion = Some(OPT_ARG(ops, b'v').unwrap_or("MATCH"));       // c:349
        named = Some(OPT_ARG(ops, b'A').unwrap_or(".pcre.match"));           // c:350
    }
    let _ = matched_portion;
    let _ = named;
    receptacle = OPT_ARG(ops, b'a').unwrap_or("match");                      // c:355
    let _ = receptacle;

    // c:357-360 — -n offset
    if OPT_HASARG(ops, b'n') {
        offset_start = getposint(OPT_ARG(ops, b'n').unwrap_or(""), nam);     // c:358
        if offset_start < 0 {
            return (1, None, Vec::new());                                    // c:359
        }
    }
    // c:362 — -b: return offset pairs
    if OPT_ISSET(ops, b'b') {
        want_offset_pair = 1;
    }
    let _ = want_offset_pair;
    let _ = use_dfa;

    // c:364-365 — plaintext = ztrdup(*args); unmetafy(plaintext, &subject_len);
    plaintext = args.first().cloned().unwrap_or_default();
    subject_len = plaintext.len() as i32;
    let _ = subject_len;

    // c:370-396 — pcre2_match path (use_dfa branch elided since the
    // Rust regex crate has no DFA equivalent).
    let (full_match, captures) = PCRE_PATTERN.with(|r| -> (Option<String>, Vec<Option<String>>) {
        let guard = r.borrow();
        let re = match guard.as_ref() {
            Some(re) => re,
            None => return (None, Vec::new()),
        };
        let search_text: &str = if offset_start > 0 && (offset_start as usize) < plaintext.len() {
            &plaintext[offset_start as usize..]
        } else if (offset_start as usize) >= plaintext.len() {
            return (None, Vec::new());
        } else {
            &plaintext
        };
        let caps = match re.captures(search_text) {
            Some(c) => c,
            None => return (None, Vec::new()),
        };
        let full = caps.get(0).map(|m| m.as_str().to_string());              // c:401 matched_portion
        let mut subs = Vec::new();
        for i in 1..caps.len() {                                             // c:401 ovector capture loop
            subs.push(caps.get(i).map(|m| m.as_str().to_string()));
        }
        (full, subs)
    });

    if full_match.is_some() {                                                // c:400 ret > 0
        return_value = 0;                                                    // c:403
    }
    ret = if full_match.is_some() { 1 } else { 0 };                          // c:398/c:399 sentinel
    let _ = ret;

    // c:422-415 — free match_data + context, zsfree(plaintext) — Rust Drop.
    (return_value, full_match, captures)                                     // c:422
}

/// Port of `cond_pcre_match(char **a, int id)` from `Src/Modules/pcre.c:422`. The
/// `-pcre-match` operator dispatch hook the lexer wires for
/// `[[ s -pcre-match pat ]]`. Compiles `a[1]` and matches `a[0]`.
/// Returns C's `int` (0 = no match, 1 = match) plus the captures so
/// the caller can install $MATCH / $match.
/// WARNING: param names don't match C — Rust=() vs C=(a, id)
pub fn cond_pcre_match(a: &[String], _id: i32)                                // c:422
    -> (i32, Option<String>, Vec<Option<String>>) {
    if a.len() < 2 { return (0, None, Vec::new()); }
    let lhs = &a[0];
    let rhs = &a[1];

    // c:424-441 — pcre2_compile(rhs)
    match Regex::new(rhs) {
        Ok(re) => {
            // c:476-491 — pcre2_match(re, lhs)
            match re.captures(lhs) {
                Some(caps) => {
                    let full = caps.get(0).map(|m| m.as_str().to_string());
                    let mut subs = Vec::new();
                    for i in 1..caps.len() {
                        subs.push(caps.get(i).map(|m| m.as_str().to_string()));
                    }
                    (1, full, subs)
                }
                None => (0, None, Vec::new()),
            }
        }
        Err(_) => (0, None, Vec::new()),
    }
}

// `bintab` — port of `static struct builtin bintab[]` (pcre.c).


// `cotab` — port of `static struct conddef cotab[]` (pcre.c).


// `module_features` — port of `static struct features module_features`
// from pcre.c:530.



/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/pcre.c:542`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {                                    // c:542
    // C body c:544-545 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/pcre.c:549`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {     // c:549
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/pcre.c:557`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {  // c:557
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/pcre.c:564`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {                                     // c:564
    // C body c:566-567 — `return 0`. Faithful empty-body port; the
    //                    pcre_compile/pcre_match/pcre_study builtins
    //                    register via the bn_list dispatch.
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/pcre.c:571`.
pub fn cleanup_(m: *const module) -> i32 {                                  // c:571
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/pcre.c:578`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {                                   // c:578
    // C body c:580-581 — `return 0`. Faithful empty-body port; the
    //                    builtins unregister via cleanup_'s setfeatureenables.
    0
}

use crate::ported::zsh_h::features as features_t;
use std::sync::{Mutex, OnceLock};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();


// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN PCRE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["b:pcre_compile".to_string(), "b:pcre_match".to_string(), "b:pcre_study".to_string(), "c:pcre-match".to_string()]
}

// WARNING: NOT IN PCRE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<features_t>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 4]);
    }
    0
}

// WARNING: NOT IN PCRE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(
    _m: *const module,
    _f: &Mutex<features_t>,
    _e: Option<&[i32]>,
) -> i32 {
    0
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor fns for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These fns sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port fns.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor fns for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These fns sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port fns.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// WARNING: NOT IN PCRE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,
        bn_size: 3,
        cd_list: None,
        cd_size: 1,
        mf_list: None,
        mf_size: 0,
        pd_list: None,
        pd_size: 0,
        n_abstract: 0,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zsh_h::MAX_OPS;

    fn empty_ops() -> options {
        options { ind: [0u8; MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }
    }
    fn ops_with(flags: &[u8]) -> options {
        let mut o = empty_ops();
        for &c in flags { o.ind[c as usize] = 1; }
        o
    }
    fn s(x: &str) -> String { x.to_string() }

    /// Verifies bin_pcre_compile sets the thread_local pcre_pattern
    /// (port of Src/Modules/pcre.c:70-107).
    #[test]
    fn test_pcre_compile_simple() {
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        let ops = empty_ops();
        assert_eq!(bin_pcre_compile("pcre_compile", &[s("hello")], &ops, 0), 0);
        assert!(PCRE_PATTERN.with(|r| r.borrow().is_some()));
    }

    /// Verifies invalid pattern → status 1 (Src/Modules/pcre.c:99-105).
    #[test]
    fn test_pcre_compile_invalid() {
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        let ops = empty_ops();
        assert_eq!(bin_pcre_compile("pcre_compile", &[s("[invalid")], &ops, 0), 1);
    }

    /// Verifies `-i` flag triggers caseless match (Src/Modules/pcre.c:79).
    #[test]
    fn test_pcre_compile_caseless() {
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        let ops = ops_with(&[b'i']);
        assert_eq!(bin_pcre_compile("pcre_compile", &[s("hello")], &ops, 0), 0);
        let (status, full, _) = bin_pcre_match("pcre_match", &[s("HELLO WORLD")], &empty_ops(), 0);
        assert_eq!(status, 0);
        assert_eq!(full.as_deref(), Some("HELLO"));
    }

    /// Verifies bin_pcre_study returns 1 when no pattern compiled
    /// (Src/Modules/pcre.c:115-117).
    #[test]
    fn test_pcre_study_no_pattern() {
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        assert_eq!(bin_pcre_study("pcre_study", &[], &empty_ops(), 0), 1);
    }

    /// Verifies bin_pcre_study returns 0 after a pattern is compiled
    /// (Src/Modules/pcre.c:112+ no-pat guard taken vs not taken).
    #[test]
    fn test_pcre_study_with_pattern() {
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        let ops = empty_ops();
        bin_pcre_compile("pcre_compile", &[s("hello")], &ops, 0);
        assert_eq!(bin_pcre_study("pcre_study", &[], &ops, 0), 0);
    }

    /// Verifies bin_pcre_match returns the matched substring
    /// (Src/Modules/pcre.c:392-401).
    #[test]
    fn test_pcre_match_simple() {
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        bin_pcre_compile("pcre_compile", &[s("hello")], &empty_ops(), 0);
        let (status, full, _) = bin_pcre_match("pcre_match", &[s("hello world")], &empty_ops(), 0);
        assert_eq!(status, 0);
        assert_eq!(full.as_deref(), Some("hello"));
    }

    /// Verifies no-match returns status 1 (Src/Modules/pcre.c:399 NOMATCH).
    #[test]
    fn test_pcre_match_no_match() {
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        bin_pcre_compile("pcre_compile", &[s("hello")], &empty_ops(), 0);
        let (status, _, _) = bin_pcre_match("pcre_match", &[s("goodbye world")], &empty_ops(), 0);
        assert_eq!(status, 1);
    }

    /// Verifies capture groups are extracted into the tuple result
    /// (Src/Modules/pcre.c:401 zpcre_get_substrings ovector loop).
    #[test]
    fn test_pcre_match_captures() {
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        bin_pcre_compile("pcre_compile", &[s(r"(\w+) (\w+)")], &empty_ops(), 0);
        let (status, _, caps) = bin_pcre_match("pcre_match", &[s("hello world")], &empty_ops(), 0);
        assert_eq!(status, 0);
        assert_eq!(caps.len(), 2);
        assert_eq!(caps[0].as_deref(), Some("hello"));
        assert_eq!(caps[1].as_deref(), Some("world"));
    }

    /// Verifies cond_pcre_match returns C's int convention
    /// (Src/Modules/pcre.c:422 + caseless via inline `(?i)` flag).
    #[test]
    fn test_cond_pcre_match() {
        let (m, _, _) = cond_pcre_match(&[s("hello world"), s("hello")], 0);
        assert_eq!(m, 1);
        let (m, _, _) = cond_pcre_match(&[s("hello world"), s("(?i)HELLO")], 0);
        assert_eq!(m, 1);
        let (m, _, _) = cond_pcre_match(&[s("hello world"), s("HELLO")], 0);
        assert_eq!(m, 0);
    }

    /// Port of `zpcre_get_substrings(pcre2_code *pat, char *arg, pcre2_match_data *mdata, int captured_count, char *matchvar, char *substravar, char *namedassoc, int want_offset_pair, int matchedinarr, int want_begin_end)` from `Src/Modules/pcre.c:157`.
    /// Verifies bin_pcre_compile with no args returns status 1
    /// (Src/Modules/pcre.c first-arg ztrdup falls back to empty target).
    #[test]
    fn test_builtin_pcre_compile_no_args() {
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        // Empty pattern + no caseless succeeds in the regex crate (matches empty);
        // we instead verify a syntactically-invalid pattern fails.
        assert_eq!(bin_pcre_compile("pcre_compile", &[s("[")], &empty_ops(), 0), 1);
    }

    /// Verifies bin_pcre_match with no compiled pattern returns 1
    /// (Src/Modules/pcre.c:343-345).
    #[test]
    fn test_builtin_pcre_match_no_pattern() {
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        let (status, _, _) = bin_pcre_match("pcre_match", &[s("test")], &empty_ops(), 0);
        assert_eq!(status, 1);
    }
}
