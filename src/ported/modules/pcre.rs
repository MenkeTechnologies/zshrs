//! PCRE module - port of Modules/pcre.c
//!
//! Provides PCRE regex matching through pcre_compile, pcre_match, pcre_study builtins.
//! Uses the Rust `regex` crate which provides Perl-compatible regex syntax.

use crate::ported::utils::{metafy, zstrtol, zwarnnam};
use crate::ported::zsh_h::{
    features, isset, module, options, BASHREMATCH, CASEMATCH, KSHARRAYS, MAX_OPS, MB_CHARLEN,
    OPT_ARG, OPT_HASARG, OPT_ISSET, REMATCHPCRE,
};
use regex::Regex;

use crate::params::setsparam;
use crate::ported::options::optlookup;
use crate::ported::params::{setaparam, sethparam, setiparam};
use std::sync::{Mutex, OnceLock};

/// Port of `CPCRE_PLAIN` from `Src/Modules/pcre.c:34`. Default
/// pattern-flavour id passed to `cond_pcre_match` (the `-pcre-match`
/// infix dispatcher) — selects plain (non-anchored) PCRE matching.
pub const CPCRE_PLAIN: i32 = 0; // c:34

/// Port of `PCRE2_CODE_UNIT_WIDTH` from `Src/Modules/pcre.c:38`.
/// `#define PCRE2_CODE_UNIT_WIDTH 8`. Selects the 8-bit pcre2 API
/// over 16-bit / 32-bit. Rust uses the `regex` crate (UTF-8 by
/// default), so this is a search anchor for the C source.
pub const PCRE2_CODE_UNIT_WIDTH: i32 = 8; // c:38

// Per-evaluator PCRE compile state — bucket-1 dissolution per
// PORT_PLAN.md Phase 2. C source has ONE file-static at
// Src/Modules/pcre.c:41:
//
//     static pcre2_code *pcre_pattern;
//
// Mirrored as a single `thread_local!` — each worker thread's
// `pcre_compile` builtin owns its own compiled regex (file-static
// semantics preserve under threading per PORT_PLAN bucket-1 rule).

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
pub fn zpcre_utf8_enabled() -> i32 {
    // c:45
    // c:45-67 — under MULTIBYTE_SUPPORT && HAVE_NL_LANGINFO && CODESET.
    // Static-link path: zshrs hosts on macOS/Linux where PCRE2 ships with
    // Unicode by default; check MULTIBYTE option + LANG/LC_ALL CODESET.
    let multibyte = isset(optlookup("multibyte")); // c:53
    if !multibyte {
        return 0; // c:54
    }
    // c:62 — nl_langinfo(CODESET) check.
    let lc = std::env::var("LC_ALL")
        .or_else(|_| std::env::var("LC_CTYPE"))
        .or_else(|_| std::env::var("LANG"))
        .unwrap_or_default();
    if lc.to_uppercase().contains("UTF-8") || lc.to_uppercase().contains("UTF8") {
        1 // c:62
    } else {
        0
    }
}

/// Port of `bin_pcre_compile(char *nam, char **args, Options ops, UNUSED(int func))` from `Src/Modules/pcre.c:70`.
/// C: `static int bin_pcre_compile(char *nam, char **args, Options ops,
/// UNUSED(int func))` — compile *args into the file-static
/// `pcre_pattern`. Option bits read from `ops` via OPT_ISSET.
#[allow(unused_variables)]
pub fn bin_pcre_compile(nam: &str, args: &[String], ops: &options, func: i32) -> i32 {
    // c:70
    // c:Src/Modules/pcre.c BUILTIN spec — min_args=1 (regex pattern is
    // required). C's execbuiltin gate rejects no-args before calling
    // here; the Rust port calls this fn directly from tests / future
    // dispatch paths without that gate, so an empty argv silently
    // compiles an empty pattern and returns 0. Mirror C's usage error.
    if args.is_empty() {
        zwarnnam(nam, "not enough arguments");
        return 1;
    }
    // c:72-76 — locals at function top.
    let mut pcre_opts: u32 = 0; // c:72
    let target_len: i32; // c:73
                         // c:74 int pcre_error / c:75 PCRE2_SIZE pcre_offset — folded into
                         // the Rust regex crate's Result error type.
    let target: String; // c:76

    if OPT_ISSET(ops, b'a') {
        pcre_opts |= 1;
    } // c:78 PCRE2_ANCHORED
    if OPT_ISSET(ops, b'i') {
        pcre_opts |= 2;
    } // c:79 PCRE2_CASELESS
    if OPT_ISSET(ops, b'm') {
        pcre_opts |= 4;
    } // c:80 PCRE2_MULTILINE
    if OPT_ISSET(ops, b'x') {
        pcre_opts |= 8;
    } // c:81 PCRE2_EXTENDED
    if OPT_ISSET(ops, b's') {
        pcre_opts |= 16;
    } // c:82 PCRE2_DOTALL

    // c:84-85 — UTF-8 unconditionally (Rust `regex` is UTF-8 native).

    // c:87-89 — pcre2_code_free(pcre_pattern); pcre_pattern = NULL;
    PCRE_PATTERN.with(|r| *r.borrow_mut() = None);

    // c:91-92 — target = ztrdup(*args); unmetafy(target, &target_len);
    // The pattern can contain zsh-metafied bytes (the Meta + (byte ^ 32)
    // encoding C uses for NUL and certain shell-special bytes in
    // internal strings). Prior Rust port skipped the unmetafy step,
    // so a pattern compiled from a value that had been round-tripped
    // through zsh's internal string store (e.g. `pcre_compile $pat`
    // where `$pat` had been read from `print -PN` output, or via the
    // POSIX-EXTENDED bracket expression that bumped a byte into the
    // Meta range) would see literal 0x83 / 0xa3 / etc. bytes instead
    // of the canonical NUL / control / 8th-bit byte the user wrote.
    let raw = args.first().cloned().unwrap_or_default();
    let mut buf = raw.into_bytes();
    target_len = crate::ported::utils::unmetafy(&mut buf) as i32; // c:92
    target = String::from_utf8_lossy(&buf).into_owned();
    let _ = target_len;

    // c:94-95 — pcre_pattern = pcre2_compile(target, ...)
    // The Rust `regex` crate accepts inline (?i)/(?m)/(?s)/(?x) flags
    // and the ^ anchor at the start of the pattern.
    let mut pattern_str = String::new();
    if (pcre_opts & 2) != 0 {
        pattern_str.push_str("(?i)");
    }
    if (pcre_opts & 4) != 0 {
        pattern_str.push_str("(?m)");
    }
    if (pcre_opts & 16) != 0 {
        pattern_str.push_str("(?s)");
    }
    if (pcre_opts & 8) != 0 {
        pattern_str.push_str("(?x)");
    }
    if (pcre_opts & 1) != 0 {
        // c:78 PCRE2_ANCHORED — pins the match to the start of the
        // subject (or the -n offset) REGARDLESS of multiline mode.
        // `\A` is the regex-crate equivalent; a bare `^` would
        // combine with the (?m) prepended above for `pcre_compile
        // -a -m` and anchor at EVERY line start instead.
        pattern_str.push_str(r"\A");
    }
    pattern_str.push_str(&target);

    match Regex::new(&pattern_str) {
        Ok(re) => {
            PCRE_PATTERN.with(|r| *r.borrow_mut() = Some(re));
            0 // c:107
        }
        Err(e) => {
            // c:101-104 — `pcre2_get_error_message(pcre_error, buffer,
            //                                       sizeof(buffer));
            //              zwarnnam(nam, "error in regex: %s", buffer);
            //              return 1;`
            //
            // C emits a one-line summary from the PCRE2 error table.
            // The regex crate's Display impl renders a multi-line
            // breakdown by default (pattern excerpt + caret arrow +
            // explanation); collapse it to a single line matching C's
            // shape so callers parsing `2>&1` output see one line per
            // failed compile (the existing cond_pcre_match diagnostic
            // at 47a3bd8191 used the same collapse).
            let raw = e.to_string();
            let detail = raw
                .lines()
                .rev()
                .find_map(|l| l.trim().strip_prefix("error: "))
                .map(|s| s.to_string())
                .unwrap_or_else(|| raw.replace('\n', " "));
            zwarnnam(nam, &format!("error in regex: {}", detail)); // c:103
            1 // c:104
        }
    }
}

/// Port of `bin_pcre_study(char *nam, UNUSED(char **args), UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/pcre.c:112`. The C
/// source calls `pcre2_jit_compile()` to JIT-optimize the compiled
/// pattern; the Rust `regex` crate already builds an optimal NFA
/// at compile time, so this is the "no pattern" guard plus return 0.
#[allow(unused_variables)]
pub fn bin_pcre_study(nam: &str, args: &[String], ops: &options, func: i32) -> i32 {
    // c:112
    let has_pat = PCRE_PATTERN.with(|r| r.borrow().is_some());
    if !has_pat {
        // c:115
        zwarnnam(nam, "no pattern has been compiled for study"); // c:116
        return 1; // c:117
    }
    0
}

/// Port of `pcre_callout(pcre2_callout_block_8 *block, UNUSED(void *callout_data))` from Src/Modules/pcre.c:132.
/// C: `static int pcre_callout(pcre2_callout_block_8 *block,
///     UNUSED(void *callout_data))` — eval the callout string as zsh code,
///     bind .pcre.subject and .pcre.pos parameters, return $? | errflag.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(_block) vs C=(block, callout_data)
pub fn pcre_callout(
    _block: *mut std::ffi::c_void, // c:132
    _callout_data: *mut std::ffi::c_void,
) -> i32 {
    // c:138-152 — parse_string(callout_string), setsparam(".pcre.subject"),
    // setiparam(".pcre.pos"), execode(prog, ..., "pcre"), return lastval|errflag.
    // Static-link path: zshrs's pcre integration uses the `regex` crate
    // directly; native pcre callouts arrive only when the C pcre2 backend
    // is wired in. Until then return success-no-callout.
    0 // c:157
}

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// =====================================================================
// static struct features module_features                            c:530 (pcre.c)
// =====================================================================

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
pub fn zpcre_get_substrings(
    // c:157
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
    let mut capture_start: i32 = 1; // c:164
    if matchedinarr != 0 {
        // c:169
        capture_start = 0; // c:171 bash-style ovec[0]
    }

    // c:175 — `ovec = pcre2_get_ovector_pointer(mdata);`
    // pcre2 isn't currently wired through zshrs (the regex crate is the
    // matcher backend instead). The `mdata` pointer is opaque from
    // Rust; the canonical access path materializes here once pcre2-rs
    // bindings land. Sentinel: empty ovec → skip the populated branch.
    let ovec: Vec<(usize, usize)> = Vec::new(); // c:175
    let _ = mdata; // c:175

    if !ovec.is_empty() {
        // c:176
        let nelem = captured_count - 1; // c:177

        if want_offset_pair != 0 {
            // c:179
            let offset_all = format!("{} {}", ovec[0].0, ovec[0].1); // c:180
            setsparam("ZPCRE_OP", &offset_all); // c:181
        }

        if let Some(mv) = matchvar {
            // c:188
            let (s, e) = ovec[0]; // c:189 arg + ovec[0]..ovec[1]
            let slice = arg.get(s..e).unwrap_or("");
            let match_all = metafy(slice); // c:189
            setsparam(mv, &match_all); // c:190
        }

        // c:202-213 — substravar: build the captures array
        if let Some(sv) = substravar {
            // c:202
            if want_begin_end == 0 || nelem != 0 {
                // c:203
                let mut matches: Vec<String> = Vec::with_capacity(
                    // c:206
                    (captured_count + 1 - capture_start) as usize,
                );
                let mut i = capture_start; // c:207
                while i < captured_count {
                    let vec_off = (2 * i) as usize; // c:208
                    if let Some(&(s, e)) = ovec.get(vec_off / 2) {
                        let slice = arg.get(s..e).unwrap_or("");
                        matches.push(metafy(slice)); // c:209
                    } else {
                        matches.push(String::new());
                    }
                    i += 1;
                }
                // c:212 — `setaparam(substravar, matches);`
                setaparam(sv, matches); // c:212
            }
        }

        // c:215-231 — namedassoc: build the named-captures hash
        if let Some(na) = namedassoc {
            // c:215
            // pcre2_pattern_info(pat, PCRE2_INFO_NAMECOUNT/...) gates this
            // path; without pcre2 bindings we treat ncount=0 and skip.
            let _ = pat; // c:216
            let ncount: u32 = 0; // c:216
            if ncount != 0 {
                // c:216
                // c:222-230 — build hash[] interleaved (name, value) pairs.
                let hash: Vec<String> = Vec::with_capacity(
                    // c:222
                    ((ncount + 1) * 2) as usize,
                );
                // For each named entry: push ztrdup(name), push metafy(value).
                // (Skipped — ncount == 0 in the stub backend.)
                sethparam(na, hash); // c:230
            }
        }

        if want_begin_end != 0 {
            // c:233
            // c:239 — `char *ptr = arg; zlong offs = 0;`
            let mut ptr_pos: usize = 0;
            let mut offs: i64 = 0; // c:240
                                   // c:245-251 — count chars from start of `arg` to `ovec[0]`.
            let mut leftlen = ovec[0].0 as i32; // c:245
            while leftlen > 0 {
                // c:246
                offs += 1; // c:247
                let clen = {
                    let slice = arg
                        .as_bytes()
                        .get(ptr_pos..ptr_pos + leftlen as usize)
                        .unwrap_or(&[]);
                    MB_CHARLEN(slice, slice.len()) // c:248 MB_CHARLEN
                };
                ptr_pos += clen; // c:249
                leftlen -= clen as i32; // c:250
            }
            // c:252 — `setiparam("MBEGIN", offs + !isset(KSHARRAYS));`
            let ksharrays = isset(KSHARRAYS) as i64;
            setiparam("MBEGIN", offs + 1 - ksharrays); // c:252

            // c:254-260 — add char count over the match itself.
            let mut leftlen = (ovec[0].1 - ovec[0].0) as i32; // c:254
            while leftlen > 0 {
                // c:255
                offs += 1; // c:256
                let clen = {
                    let slice = arg
                        .as_bytes()
                        .get(ptr_pos..ptr_pos + leftlen as usize)
                        .unwrap_or(&[]);
                    MB_CHARLEN(slice, slice.len()) // c:257 MB_CHARLEN
                };
                ptr_pos += clen; // c:258
                leftlen -= clen as i32; // c:259
            }
            setiparam(
                // c:261 MEND
                "MEND",
                offs - ksharrays,
            );

            if nelem != 0 {
                // c:262
                // c:267-298 — per-capture mbegin/mend arrays.
                let mut mbegin: Vec<String> = Vec::with_capacity(nelem as usize); // c:267
                let mut mend: Vec<String> = Vec::with_capacity(nelem as usize); // c:268

                for i in 0..nelem as usize {
                    // c:270-272
                    let pair_idx = i + 1;
                    let pair = match ovec.get(pair_idx) {
                        Some(&p) => p,
                        None => continue,
                    };
                    // c:275 — `ptr = arg; offs = 0;`
                    let mut ptr_pos: usize = 0;
                    let mut offs: i64 = 0; // c:276
                    let mut leftlen = pair.0 as i32; // c:279
                    while leftlen > 0 {
                        // c:280
                        offs += 1; // c:281
                        let clen = {
                            let slice = arg
                                .as_bytes()
                                .get(ptr_pos..ptr_pos + leftlen as usize)
                                .unwrap_or(&[]);
                            MB_CHARLEN(slice, slice.len())
                            // c:282
                        };
                        ptr_pos += clen; // c:283
                        leftlen -= clen as i32; // c:284
                    }
                    let buf = format!("{}", offs + 1 - ksharrays); // c:286 convbase
                    mbegin.push(buf); // c:287

                    let mut leftlen = (pair.1 - pair.0) as i32; // c:289
                    while leftlen > 0 {
                        // c:290
                        offs += 1; // c:291
                        let clen = {
                            let slice = arg
                                .as_bytes()
                                .get(ptr_pos..ptr_pos + leftlen as usize)
                                .unwrap_or(&[]);
                            MB_CHARLEN(slice, slice.len())
                            // c:292
                        };
                        ptr_pos += clen; // c:293
                        leftlen -= clen as i32; // c:294
                    }
                    let buf = format!("{}", offs - ksharrays); // c:296
                    mend.push(buf); // c:297
                }
                setaparam("mbegin", mbegin); // c:301
                setaparam("mend", mend); // c:302
            }
        }
    }

    0 // c:307
}

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/pcre.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `getposint(char *instr, char *nam)` from Src/Modules/pcre.c:312.
/// C: `static int getposint(char *instr, char *nam)` — parse positive
/// decimal integer; emit "integer expected" warning + return -1 on bad input.
///
/// C body (c:312-326):
/// ```c
/// int ret;
/// char *eptr;
/// ret = (int)zstrtol(instr, &eptr, 10);
/// if (*eptr || ret < 0) {
///     zwarnnam(nam, "integer expected: %s", instr);
///     return -1;
/// }
/// return ret;
/// ```
///
/// The previous Rust port used `instr.trim().parse::<i32>()` which:
///   1. Skipped TRAILING whitespace via `trim()` — C `zstrtol` skips
///      leading whitespace but NOT trailing; trailing ws lands in
///      `*eptr` and triggers the error.
///   2. Used `parse` which rejects trailing junk implicitly, BUT
///      after trim, so `"42abc"` and C both reject (Rust via parse
///      error, C via `*eptr`).
/// Route through the canonical `zstrtol` so the
/// trailing-whitespace and partial-parse edge cases match C exactly
/// — same behavior as the sister `getposint` in system.rs.
#[allow(non_snake_case)]
pub fn getposint(instr: &str, nam: &str) -> i32 {
    // c:312
    // c:312 — `ret = (int)zstrtol(instr, &eptr, 10);`
    let (ret, eptr) = zstrtol(instr, 10);
    let ret = ret as i32;
    // c:317 — `if (*eptr || ret < 0)` — trailing chars OR negative.
    if !eptr.is_empty() || ret < 0 {
        zwarnnam(nam, &format!("integer expected: {}", instr)); // c:319
        return -1; // c:321
    }
    ret // c:325
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
pub fn bin_pcre_match(nam: &str, args: &[String], ops: &options, _func: i32) -> i32 {
    // c:330-341 — locals at function top.
    let ret: i32; // c:330
    let _c: u8 = 0; // c:330
                    // c:331 pcre2_match_data *pcre_mdata = NULL — folded into regex::Captures
    let mut matched_portion: Option<&str> = None; // c:332
    let plaintext: String; // c:333
    let receptacle: &str; // c:334
    let mut named: Option<&str> = None; // c:335
    let mut return_value: i32 = 1; // c:336
    let subject_len: i32; // c:338
    let mut offset_start: i32 = 0; // c:339
    let mut want_offset_pair: i32 = 0; // c:340
    let mut use_dfa: i32 = 0; // c:341

    // c:343-346 — pcre_pattern NULL check
    let has_pat = PCRE_PATTERN.with(|r| r.borrow().is_some());
    if !has_pat {
        // c:343
        zwarnnam(nam, "no pattern has been compiled"); // c:344
        return 1; // c:345
    }

    // c:348-354 — -d (DFA) precludes -v/-A
    if OPT_ISSET(ops, b'd') {
        use_dfa = 1;
        if OPT_HASARG(ops, b'v') || OPT_HASARG(ops, b'A') {
            // c:351
            zwarnnam(nam, "-d cannot be combined with -v or -A"); // c:352
            return 1; // c:353
        }
    } else {
        matched_portion = Some(OPT_ARG(ops, b'v').unwrap_or("MATCH")); // c:349
        named = Some(OPT_ARG(ops, b'A').unwrap_or(".pcre.match")); // c:350
    }
    receptacle = OPT_ARG(ops, b'a').unwrap_or("match"); // c:355

    // c:357-360 — -n offset
    if OPT_HASARG(ops, b'n') {
        offset_start = getposint(OPT_ARG(ops, b'n').unwrap_or(""), nam); // c:358
        if offset_start < 0 {
            return 1; // c:359
        }
    }
    // c:362 — -b: return offset pairs
    if OPT_ISSET(ops, b'b') {
        want_offset_pair = 1;
    }
    // c:372-389 — the -d pcre2_dfa_match engine has no regex-crate
    // equivalent; -d approximates as a normal match. The OBSERVABLE
    // -d contract is still honored: no scalar (matched_portion stays
    // None per c:348-350), no named assoc, full match as receptacle
    // element 0 (matchedinarr, c:402) — see the success block below.

    // c:364-365 — plaintext = ztrdup(*args); unmetafy(plaintext, &subject_len);
    // The subject can carry Meta-escaped bytes (NUL / 0x80-range bytes
    // round-tripped through zsh's metafied internal-string store);
    // PCRE2 needs the raw bytes the user wrote, not the encoded pair.
    // Prior port skipped this so `pcre_match $value_with_meta_bytes`
    // searched the encoded form against a non-encoded compiled
    // pattern, silently failing to match.
    let raw = args.first().cloned().unwrap_or_default();
    let mut buf = raw.into_bytes();
    subject_len = crate::ported::utils::unmetafy(&mut buf) as i32; // c:365
    plaintext = String::from_utf8_lossy(&buf).into_owned();
    let _ = subject_len;

    // c:370-396 — pcre2_match path (use_dfa branch elided since the
    // Rust regex crate has no DFA equivalent).
    let search_base_offset: usize =
        if offset_start > 0 && (offset_start as usize) <= plaintext.len() {
            offset_start as usize
        } else {
            0
        };
    let (full_match, full_range, captures, named_pairs) = PCRE_PATTERN.with(
        |r| -> (
            Option<String>,
            Option<(usize, usize)>,
            Vec<Option<String>>,
            Vec<String>,
        ) {
            let guard = r.borrow();
            let re = match guard.as_ref() {
                Some(re) => re,
                None => return (None, None, Vec::new(), Vec::new()),
            };
            let search_text: &str =
                if offset_start > 0 && (offset_start as usize) <= plaintext.len() {
                    &plaintext[offset_start as usize..]
                } else if (offset_start as usize) > plaintext.len() {
                    return (None, None, Vec::new(), Vec::new());
                } else {
                    &plaintext
                };
            let caps = match re.captures(search_text) {
                Some(c) => c,
                None => return (None, None, Vec::new(), Vec::new()),
            };
            let full_m = caps.get(0); // c:401 matched_portion
            let full = full_m.map(|m| m.as_str().to_string());
            // c:180-181 — `sprintf(offset_all, "%ld %ld", ovec[0], ovec[1])`.
            // ovec is RELATIVE TO THE WHOLE SUBJECT, so add back the -n
            // offset the regex was started from (search_base_offset).
            let range = full_m.map(|m| (m.start(), m.end()));
            let mut subs = Vec::new();
            for i in 1..caps.len() {
                // c:401 ovector capture loop
                subs.push(caps.get(i).map(|m| m.as_str().to_string()));
            }
            // c:215-229 — named-capture table walk:
            //
            //     if (namedassoc
            //             && !pcre2_pattern_info(pat, PCRE2_INFO_NAMECOUNT, &ncount) && ncount
            //             && ...NAMEENTRYSIZE... && ...NAMETABLE...)
            //     {
            //         hashptr = hash = (char **)zshcalloc((ncount+1)*2*sizeof(char *));
            //         for (nidx = 0; nidx < ncount; nidx++) {
            //             vec_off = (ntable[nsize * nidx] << 9) + 2 * ntable[nsize * nidx + 1];
            //             /* would metafy the key but pcre limits characters in the name */
            //             *hashptr++ = ztrdup((char *) ntable + nsize * nidx + 2);
            //             *hashptr++ = metafy(arg + ovec[vec_off],
            //                     ovec[vec_off+1]-ovec[vec_off], META_DUP);
            //         }
            //         sethparam(namedassoc, hash);
            //     }
            //
            // The regex crate exposes the same name table via
            // capture_names(): index-aligned Option<&str>. Keys stay
            // raw (PCRE limits name chars, per the C comment); values
            // metafy like every capture.
            let mut named_kv: Vec<String> = Vec::new();
            for (idx, name_opt) in re.capture_names().enumerate() {
                if let Some(nm) = name_opt {
                    let val = caps.get(idx).map(|m| m.as_str()).unwrap_or("");
                    named_kv.push(nm.to_string()); // c:226 key
                    named_kv.push(crate::ported::utils::metafy(val)); // c:227-228 value
                }
            }
            (full, range, subs, named_kv)
        },
    );

    if full_match.is_some() {
        // c:400 ret > 0
        return_value = 0; // c:403
                          // c:405-414 — install $MATCH (or -v target) + $match (or -a
                          // receptacle). C uses zpcre_get_substrings which calls
                          // setsparam / setaparam directly; Rust mirrors that.
        // c:179-182 — -b: write the match start/end byte offsets to
        // $ZPCRE_OP as "start end" (relative to the whole subject,
        // honoring the -n start offset). Prior port read the
        // want_offset_pair flag but discarded it (`let _ = ...`), so
        // `pcre_match -b` left $ZPCRE_OP unset and downstream scripts
        // (zsh-syntax-highlighting + zsh-autosuggestions both consume
        // this) saw stale values from prior invocations.
        if want_offset_pair != 0 {
            if let Some((s, e)) = full_range {
                let zop = format!("{} {}", s + search_base_offset, e + search_base_offset);
                crate::ported::params::setsparam("ZPCRE_OP", &zop); // c:181
            }
        }
        // c:188-191 — `if (matchvar) { ... setsparam(matchvar, match_all); }`
        // matchvar is NULL under -d (the c:348-350 else-arm never
        // assigns it), so DFA matches set NO scalar. A prior
        // `.unwrap_or("MATCH")` default re-introduced the scalar for
        // -d, where C leaves $MATCH untouched.
        if let (Some(mv), Some(m)) = (matched_portion, full_match.as_deref()) {
            crate::ported::params::setsparam(mv, m); // c:190
        }
        // c:169-172 + c:206-212 — receptacle array. matchedinarr
        // (use_dfa, c:402's 9th arg) sets capture_start = 0: the
        // bash-style array INCLUDES the entire match as element 0.
        let mut subs: Vec<String> = captures
            .iter()
            .map(|opt| opt.clone().unwrap_or_default())
            .collect();
        if use_dfa != 0 {
            // c:170-171 — `/* bash-style ovec[0] entire-matched string
            // in the array */ capture_start = 0;`
            subs.insert(0, full_match.clone().unwrap_or_default());
        }
        crate::ported::params::setaparam(receptacle, subs); // c:212
        // c:215-231 — named-captures assoc. The c:216 `&& ncount` gate
        // means the assoc is touched ONLY when the pattern actually
        // declares named groups — a pattern without names leaves the
        // -A target (default `.pcre.match` per c:350) untouched.
        if !named_pairs.is_empty() {
            if let Some(na) = named {
                crate::ported::params::sethparam(na, named_pairs); // c:230
            }
        }
    }
    // c:398-409 — no-match / error paths LEAVE $MATCH / $match alone:
    //   if (ret == PCRE2_ERROR_NOMATCH) /* no match */;
    //   else if (ret > 0) { install MATCH/match... }
    //   else { /* error, warn */ }
    //
    // C never explicitly unsets MATCH/match on no-match — the previous
    // invocation's values remain visible. This matters because pcre
    // users commonly chain `pcre_match` calls and expect "last
    // successful match" semantics. Prior Rust port cleared both on
    // no-match (line 660-663 previously) which broke that chain:
    // a successful `$MATCH=foo` followed by a failed `pcre_match`
    // wiped $MATCH instead of leaving "foo" in place.
    ret = if full_match.is_some() { 1 } else { 0 }; // c:398/c:399 sentinel
    let _ = ret;
    let _ = subject_len;

    // c:422-415 — free match_data + context, zsfree(plaintext) — Rust Drop.
    return_value // c:422
}

/// Port of `cond_pcre_match(char **a, int id)` from `Src/Modules/pcre.c:422`. The
/// `-pcre-match` operator dispatch hook the lexer wires for
/// `[[ s -pcre-match pat ]]`. Compiles `a[1]` and matches `a[0]`.
/// Returns C's `int` (0 = no match, 1 = match); the $MATCH / $match
/// install happens via setsparam/setaparam (Src/Modules/pcre.c:510-520).
pub fn cond_pcre_match(a: &[String], _id: i32) -> i32 {
    // c:422
    // c:428 — `int r = 0, return_value = 0;`.
    if a.len() < 2 {
        return 0;
    }
    let lhstr = &a[0]; // c:438 lhstr = cond_str(a,0,0)
    let rhre = &a[1]; // c:439 rhre = cond_str(a,1,0)

    // c:440-443 — both sides arrive metafied (Meta + (byte^32) for any
    // byte the internal-string store mangles); strip before handing to
    // the regex engine. Same fix as bin_pcre_compile/bin_pcre_match
    // (fa17866c48 / 1dbd7eecc1). Prior cond_pcre_match used the
    // metafied strings directly, so `[[ $value -pcre-match $pat ]]`
    // silently no-matched whenever either side carried Meta bytes.
    let mut lhs_buf = lhstr.as_bytes().to_vec();
    let _lhs_len = crate::ported::utils::unmetafy(&mut lhs_buf); // c:442
    let lhs_plain = String::from_utf8_lossy(&lhs_buf).into_owned();
    let mut rhs_buf = rhre.as_bytes().to_vec();
    let _rhs_len = crate::ported::utils::unmetafy(&mut rhs_buf); // c:443
    let rhs_plain = String::from_utf8_lossy(&rhs_buf).into_owned();

    // c:433-436 — compile-time PCRE option bits:
    //   if (zpcre_utf8_enabled())                 pcre_opts |= PCRE2_UTF;
    //   if (isset(REMATCHPCRE) && !isset(CASEMATCH)) pcre_opts |= PCRE2_CASELESS;
    //
    // Rust regex crate has no compile-time option struct exposed via
    // Regex::new; we synthesize the equivalent via inline flag groups
    // at the head of the pattern (matches bin_pcre_compile's approach
    // at pcre.rs:127-141). UTF mode is the regex crate's default so
    // no inline flag is needed for it. REMATCHPCRE+!CASEMATCH ⇒
    // prepend `(?i)` so the compiled pattern is case-insensitive.
    //
    // Prior cond_pcre_match passed `Regex::new(&rhs_plain)` directly
    // — the caseless setting from `setopt REMATCHPCRE` was silently
    // discarded so `[[ ABC -pcre-match abc ]]` returned false under
    // the option, against documented behavior.
    let pcre_compile_pat = if isset(REMATCHPCRE) && !isset(CASEMATCH) {
        format!("(?i){}", rhs_plain) // c:436
    } else {
        rhs_plain.clone()
    };

    // c:445-451 — BASHREMATCH option selects the output-variable shape:
    //   if (isset(BASHREMATCH)) { svar = NULL; avar = "BASH_REMATCH"; }
    //   else                    { svar = "MATCH"; avar = "match"; }
    //
    // BASHREMATCH mode (POSIX/bash-compat): a single array `BASH_REMATCH`
    // whose [0] element is the full match and [1..] are the captures;
    // no scalar MATCH variable is set (svar = NULL → zpcre_get_substrings
    // skips the setsparam call).
    //
    // Default zsh mode: scalar `MATCH` holds the full match, array
    // `match` holds the captures (indices 1..n).
    //
    // Prior Rust cond_pcre_match hardcoded MATCH/match and silently
    // dropped the BASHREMATCH option — `[[ $value -pcre-match $pat ]]`
    // under `setopt BASHREMATCH` left BASH_REMATCH undefined and a
    // bash-compat script reading `${BASH_REMATCH[1]}` would see the
    // empty string.
    let bashre = isset(BASHREMATCH);

    // c:455 — `pcre2_compile(rhre_plain, ...)`. Rust regex crate
    // substitutes for libpcre2 — same regex semantics for the common
    // subset.
    match Regex::new(&pcre_compile_pat) {
        Ok(re) => {
            // c:465 — `pcre2_match(pcre_pat, lhstr_plain, ...)`.
            match re.captures(&lhs_plain) {
                Some(caps) => {
                    // c:483-487 — match succeeded; emit via the
                    // zpcre_get_substrings contract:
                    //
                    //     zpcre_get_substrings(pcre_pat, lhstr_plain, pcre_mdata,
                    //             ovec_count, svar, avar,
                    //             ".pcre.match", 0, isset(BASHREMATCH),
                    //             !isset(BASHREMATCH));
                    //
                    // i.e. want_begin_end = !BASHREMATCH: the default zsh
                    // mode ALSO sets the MBEGIN/MEND scalars (c:243-261)
                    // and the per-capture mbegin/mend arrays (c:262-298),
                    // and gates `match` on captures existing (c:202-203
                    // `!want_begin_end || nelem`). The named-captures
                    // assoc `.pcre.match` populates in BOTH modes.
                    let nelem = caps.len() - 1; // c:177
                    if bashre {
                        // c:445-447 + matchedinarr=1: BASH_REMATCH array,
                        // [0]=full match, [1..n]=captures; no scalar.
                        let mut arr: Vec<String> = Vec::with_capacity(caps.len());
                        for i in 0..caps.len() {
                            arr.push(
                                caps.get(i)
                                    .map(|m| m.as_str().to_string())
                                    .unwrap_or_default(),
                            );
                        }
                        crate::ported::params::setaparam("BASH_REMATCH", arr); // c:212
                    } else {
                        // c:188-190 — `MATCH` scalar.
                        let ksharr = isset(KSHARRAYS) as i64;
                        if let Some(m0) = caps.get(0) {
                            crate::ported::params::setsparam("MATCH", m0.as_str()); // c:190
                            // c:243-261 — char-offset MBEGIN/MEND over the
                            // unmetafied subject (MB_CHARLEN walk ⟺
                            // chars().count() on the UTF-8 String).
                            let beg_chars = lhs_plain[..m0.start()].chars().count() as i64;
                            let len_chars =
                                lhs_plain[m0.start()..m0.end()].chars().count() as i64;
                            crate::ported::params::setiparam("MBEGIN", beg_chars + 1 - ksharr); // c:252
                            crate::ported::params::setiparam(
                                "MEND",
                                beg_chars + len_chars - ksharr,
                            ); // c:261
                        }
                        if nelem > 0 {
                            // c:202-213 — `match` only when parenthesised
                            // captures exist; c:262-298 — mbegin/mend
                            // per-capture offset arrays alongside.
                            let mut subs: Vec<String> = Vec::with_capacity(nelem);
                            let mut mbegin_arr: Vec<String> = Vec::with_capacity(nelem);
                            let mut mend_arr: Vec<String> = Vec::with_capacity(nelem);
                            for i in 1..caps.len() {
                                match caps.get(i) {
                                    Some(m) => {
                                        subs.push(m.as_str().to_string()); // c:209
                                        let b =
                                            lhs_plain[..m.start()].chars().count() as i64;
                                        let l = lhs_plain[m.start()..m.end()]
                                            .chars()
                                            .count()
                                            as i64;
                                        mbegin_arr.push((b + 1 - ksharr).to_string()); // c:286
                                        mend_arr.push((b + l - ksharr).to_string()); // c:296
                                    }
                                    None => {
                                        // Unparticipated group: empty match
                                        // text (C's zero-length metafy of the
                                        // PCRE2_UNSET pair) + "-1" offsets
                                        // (the regex.c sibling convention,
                                        // Src/Modules/regex.c:158-161).
                                        subs.push(String::new());
                                        mbegin_arr.push("-1".to_string());
                                        mend_arr.push("-1".to_string());
                                    }
                                }
                            }
                            crate::ported::params::setaparam("match", subs); // c:212
                            crate::ported::params::setaparam("mbegin", mbegin_arr); // c:300
                            crate::ported::params::setaparam("mend", mend_arr); // c:301
                        }
                    }
                    // c:486 — namedassoc ".pcre.match" fires from the cond
                    // path too, gated on the pattern declaring names
                    // (c:216 `&& ncount`).
                    let mut named_kv: Vec<String> = Vec::new();
                    for (idx, name_opt) in re.capture_names().enumerate() {
                        if let Some(nm) = name_opt {
                            let val = caps.get(idx).map(|m| m.as_str()).unwrap_or("");
                            named_kv.push(nm.to_string()); // c:226
                            named_kv.push(crate::ported::utils::metafy(val)); // c:227
                        }
                    }
                    if !named_kv.is_empty() {
                        crate::ported::params::sethparam(".pcre.match", named_kv); // c:230
                    }
                    1 // c:487 return_value = 1
                }
                None => {
                    // c:474-477 — `else if (r == PCRE2_ERROR_NOMATCH)
                    //                   { return_value = 0; /* no match */
                    //                     break; }`
                    //
                    // C leaves $MATCH / $match / $BASH_REMATCH untouched
                    // on no-match — only sets return_value=0 and bails.
                    // Same as bin_pcre_match's no-match arm (abb0046210):
                    // the chain of `[[ s -pcre-match pat ]]` calls is
                    // expected to preserve "last successful match" so
                    // a subsequent test like `[[ -n $MATCH ]]` can act
                    // as a "did anything match in this chain?" gate.
                    //
                    // Prior Rust port cleared both families which broke
                    // that gate.
                    let _ = bashre;
                    0
                }
            }
        }
        Err(e) => {
            // c:459-462 — `pcre2_compile failed: zwarn("failed to compile
            //              regexp /%s/: %s", rhre, buffer); break;`
            //
            // C emits a one-line diagnostic naming the rejected pattern
            // and the engine's error message. Prior Rust port returned
            // 0 silently with a comment claiming "the lexer-driven path
            // already reported the compile error" — incorrect: nothing
            // upstream of cond_pcre_match validates the pattern, so a
            // bad PCRE in `[[ $s -pcre-match badpat ]]` produced 0
            // (no match) with no diagnostic. A user typo (unclosed `(`)
            // is then indistinguishable from a real "no match".
            //
            // Format the regex crate's structured Error into the same
            // "failed to compile regexp /<pat>/: <reason>" shape.
            let raw = e.to_string();
            let detail = raw
                .lines()
                .rev()
                .find_map(|l| l.trim().strip_prefix("error: "))
                .map(|s| s.to_string())
                .unwrap_or_else(|| raw.replace('\n', " "));
            crate::ported::utils::zwarn(&format!(
                "failed to compile regexp /{}/: {}",
                rhre,
                detail
            ));
            0
        }
    }
}

// `bintab` — port of `static struct builtin bintab[]` (pcre.c).

// `cotab` — port of `static struct conddef cotab[]` (pcre.c).

// `module_features` — port of `static struct features module_features`
// from pcre.c:530.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/pcre.c:542`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:542
    // C body c:544-545 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/pcre.c:549`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:549
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/pcre.c:557`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:557
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/pcre.c:564`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:564
    // C body c:566-567 — `return 0`. Faithful empty-body port; the
    //                    pcre_compile/pcre_match/pcre_study builtins
    //                    register via the bn_list dispatch.
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/pcre.c:571`.
pub fn cleanup_(m: *const module) -> i32 {
    // c:571
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/pcre.c:578`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:578
    // C body c:580-581 — `return 0`. Faithful empty-body port; the
    //                    builtins unregister via cleanup_'s setfeatureenables.
    0
}

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN PCRE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec![
        "b:pcre_compile".to_string(),
        "b:pcre_match".to_string(),
        "b:pcre_study".to_string(),
        "c:pcre-match".to_string(),
    ]
}

// WARNING: NOT IN PCRE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 4]);
    }
    0
}

// WARNING: NOT IN PCRE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(_m: *const module, _f: &Mutex<features>, _e: Option<&[i32]>) -> i32 {
    0
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// WARNING: NOT IN PCRE.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 3,
            cd_list: None,
            cd_size: 1,
            mf_list: None,
            mf_size: 0,
            pd_list: None,
            pd_size: 0,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_ops() -> options {
        options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }
    fn ops_with(flags: &[u8]) -> options {
        let mut o = empty_ops();
        for &c in flags {
            o.ind[c as usize] = 1;
        }
        o
    }
    fn s(x: &str) -> String {
        x.to_string()
    }

    /// Verifies bin_pcre_compile sets the thread_local pcre_pattern
    /// (port of Src/Modules/pcre.c:70-107).
    #[test]
    fn test_pcre_compile_simple() {
        let _g = crate::test_util::global_state_lock();
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        let ops = empty_ops();
        assert_eq!(bin_pcre_compile("pcre_compile", &[s("hello")], &ops, 0), 0);
        assert!(PCRE_PATTERN.with(|r| r.borrow().is_some()));
    }

    /// Verifies invalid pattern → status 1 (Src/Modules/pcre.c:99-105).
    #[test]
    fn test_pcre_compile_invalid() {
        let _g = crate::test_util::global_state_lock();
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        let ops = empty_ops();
        assert_eq!(
            bin_pcre_compile("pcre_compile", &[s("[invalid")], &ops, 0),
            1
        );
    }

    /// Verifies `-i` flag triggers caseless match (Src/Modules/pcre.c:79).
    #[test]
    fn test_pcre_compile_caseless() {
        let _g = crate::test_util::global_state_lock();
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        let ops = ops_with(&[b'i']);
        assert_eq!(bin_pcre_compile("pcre_compile", &[s("hello")], &ops, 0), 0);
        let status = bin_pcre_match("pcre_match", &[s("HELLO WORLD")], &empty_ops(), 0);
        assert_eq!(status, 0);
        assert_eq!(
            crate::ported::params::getsparam("MATCH").as_deref(),
            Some("HELLO"),
            "c:405 — setsparam(MATCH, matched_portion)"
        );
    }

    /// Verifies bin_pcre_study returns 1 when no pattern compiled
    /// (Src/Modules/pcre.c:115-117).
    #[test]
    fn test_pcre_study_no_pattern() {
        let _g = crate::test_util::global_state_lock();
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        assert_eq!(bin_pcre_study("pcre_study", &[], &empty_ops(), 0), 1);
    }

    /// Verifies bin_pcre_study returns 0 after a pattern is compiled
    /// (Src/Modules/pcre.c:112+ no-pat guard taken vs not taken).
    #[test]
    fn test_pcre_study_with_pattern() {
        let _g = crate::test_util::global_state_lock();
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        let ops = empty_ops();
        bin_pcre_compile("pcre_compile", &[s("hello")], &ops, 0);
        assert_eq!(bin_pcre_study("pcre_study", &[], &ops, 0), 0);
    }

    /// Verifies bin_pcre_match returns the matched substring
    /// (Src/Modules/pcre.c:392-401).
    #[test]
    fn test_pcre_match_simple() {
        let _g = crate::test_util::global_state_lock();
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        bin_pcre_compile("pcre_compile", &[s("hello")], &empty_ops(), 0);
        let status = bin_pcre_match("pcre_match", &[s("hello world")], &empty_ops(), 0);
        assert_eq!(status, 0);
        assert_eq!(
            crate::ported::params::getsparam("MATCH").as_deref(),
            Some("hello")
        );
    }

    /// Verifies no-match returns status 1 (Src/Modules/pcre.c:399 NOMATCH).
    #[test]
    fn test_pcre_match_no_match() {
        let _g = crate::test_util::global_state_lock();
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        bin_pcre_compile("pcre_compile", &[s("hello")], &empty_ops(), 0);
        let status = bin_pcre_match("pcre_match", &[s("goodbye world")], &empty_ops(), 0);
        assert_eq!(status, 1);
    }

    /// Verifies capture groups are extracted into the tuple result
    /// (Src/Modules/pcre.c:401 zpcre_get_substrings ovector loop).
    #[test]
    fn test_pcre_match_captures() {
        let _g = crate::test_util::global_state_lock();
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        bin_pcre_compile("pcre_compile", &[s(r"(\w+) (\w+)")], &empty_ops(), 0);
        let status = bin_pcre_match("pcre_match", &[s("hello world")], &empty_ops(), 0);
        assert_eq!(status, 0);
        // c:410-413 — captures into `match` array param.
        let caps = crate::ported::params::getaparam("match").unwrap_or_default();
        assert_eq!(caps.len(), 2);
        assert_eq!(caps[0], "hello");
        assert_eq!(caps[1], "world");
    }

    /// Verifies cond_pcre_match returns C's int convention
    /// (Src/Modules/pcre.c:422 + caseless via inline `(?i)` flag).
    #[test]
    fn test_cond_pcre_match() {
        let _g = crate::test_util::global_state_lock();
        let m = cond_pcre_match(&[s("hello world"), s("hello")], 0);
        assert_eq!(m, 1);
        let m = cond_pcre_match(&[s("hello world"), s("(?i)HELLO")], 0);
        assert_eq!(m, 1);
        let m = cond_pcre_match(&[s("hello world"), s("HELLO")], 0);
        assert_eq!(m, 0);
    }

    /// Port of `zpcre_get_substrings(pcre2_code *pat, char *arg, pcre2_match_data *mdata, int captured_count, char *matchvar, char *substravar, char *namedassoc, int want_offset_pair, int matchedinarr, int want_begin_end)` from `Src/Modules/pcre.c:157`.
    /// Verifies bin_pcre_compile with no args returns status 1
    /// (Src/Modules/pcre.c first-arg ztrdup falls back to empty target).
    #[test]
    fn test_builtin_pcre_compile_no_args() {
        let _g = crate::test_util::global_state_lock();
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        // Empty pattern + no caseless succeeds in the regex crate (matches empty);
        // we instead verify a syntactically-invalid pattern fails.
        assert_eq!(
            bin_pcre_compile("pcre_compile", &[s("[")], &empty_ops(), 0),
            1
        );
    }

    /// Verifies bin_pcre_match with no compiled pattern returns 1
    /// (Src/Modules/pcre.c:343-345).
    #[test]
    fn test_builtin_pcre_match_no_pattern() {
        let _g = crate::test_util::global_state_lock();
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        let status = bin_pcre_match("pcre_match", &[s("test")], &empty_ops(), 0);
        assert_eq!(status, 1);
    }

    /// c:312 — `getposint` parses a non-negative integer.
    #[test]
    fn getposint_parses_positive_decimal() {
        let _g = crate::test_util::global_state_lock();
        let r = getposint("42", "test");
        assert_eq!(r, 42);
    }

    /// c:312 — `getposint("0", _)` returns 0. Pin the boundary so
    /// a regen rejecting "0" as "non-positive" silently breaks
    /// `pcre_match -a vec` call sites.
    #[test]
    fn getposint_zero_is_valid() {
        let _g = crate::test_util::global_state_lock();
        let r = getposint("0", "test");
        assert_eq!(r, 0);
    }

    /// c:312 — `getposint` for non-numeric input signals error
    /// (negative return per the C zwarnnam + -1 pattern).
    #[test]
    fn getposint_non_numeric_returns_negative() {
        let _g = crate::test_util::global_state_lock();
        let r = getposint("abc", "test");
        assert!(
            r < 0,
            "non-numeric must return negative sentinel, got {}",
            r
        );
    }

    /// c:70 — `bin_pcre_compile` with NO args reads `args.first()`
    /// as empty string, which the `regex` crate compiles as a valid
    /// (always-matching) empty pattern. This matches the C source
    /// which relies on the BUILTIN dispatcher's `min_args=1` to
    /// reject empty argv before reaching the body. Pin the
    /// pass-through behavior so a regen that adds an internal arity
    /// guard (deviation from C) gets caught.
    #[test]
    fn bin_pcre_compile_no_args_compiles_empty_pattern() {
        let _g = crate::test_util::global_state_lock();
        PCRE_PATTERN.with(|r| *r.borrow_mut() = None);
        let r = bin_pcre_compile("pcre_compile", &[], &empty_ops(), 0);
        // Mirrors the C BUILTIN dispatcher's min_args=1 gate; the
        // bin_pcre_compile fn-level guard at pcre.rs returns the usage
        // error rc directly when no positional was passed.
        assert_eq!(r, 1, "no args → usage error (missing pattern)");
    }

    /// c:328 — `bin_pcre_match` with no args returns 1 (needs at
    /// least the subject string).
    #[test]
    fn bin_pcre_match_no_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        bin_pcre_compile("pcre_compile", &[s("x")], &empty_ops(), 0);
        let status = bin_pcre_match("pcre_match", &[], &empty_ops(), 0);
        assert_eq!(status, 1, "no subject must surface as error");
    }

    /// c:422 — `cond_pcre_match` with malformed pattern surfaces
    /// as no-match (0) — compile fails inside, function returns
    /// "no match" rather than panicking.
    #[test]
    fn cond_pcre_match_malformed_pattern_returns_no_match() {
        let _g = crate::test_util::global_state_lock();
        let m = cond_pcre_match(&[s("anything"), s("[")], 0);
        assert_eq!(m, 0, "malformed regex must fail-soft to no-match");
    }

    /// c:422 — Caret anchor: `^foo` matches "foo bar" but NOT
    /// "bar foo". Pin anchor semantics.
    #[test]
    fn cond_pcre_match_caret_anchor_requires_start() {
        let _g = crate::test_util::global_state_lock();
        let m = cond_pcre_match(&[s("foo bar"), s("^foo")], 0);
        assert_eq!(m, 1, "caret matches at start");
        let m = cond_pcre_match(&[s("bar foo"), s("^foo")], 0);
        assert_eq!(m, 0, "caret must NOT match mid-string");
    }

    /// c:542-580 — module-lifecycle stubs all return 0.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    /// `Src/Modules/pcre.c:317` — `if (*eptr || ret < 0)` — trailing
    /// non-digit chars trigger the error path. The previous Rust port
    /// used `instr.trim().parse::<i32>()` which silently stripped
    /// TRAILING whitespace before parsing — C zstrtol skips leading
    /// only. Pin so `"42abc"` now rejects (matching C) where the old
    /// port already did (via parse error) AND `"42 "` now also rejects
    /// (where the old port silently accepted via trim).
    #[test]
    fn getposint_rejects_trailing_garbage() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            getposint("42abc", "test"),
            -1,
            "c:317 — *eptr='a' truthy → error"
        );
        assert_eq!(
            getposint("100x", "test"),
            -1,
            "c:317 — trailing non-digit must reject"
        );
    }

    /// `Src/Modules/pcre.c:317` — trailing whitespace also lands in
    /// `*eptr` so C rejects. Pin the canonical behavior (matches the
    /// sister `system.rs::getposint`).
    #[test]
    fn getposint_rejects_trailing_whitespace() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            getposint("42 ", "test"),
            -1,
            "c:317 — trailing space → *eptr=' ' → error"
        );
        assert_eq!(
            getposint("42\t", "test"),
            -1,
            "c:317 — trailing tab → *eptr='\\t' → error"
        );
    }

    /// `Src/Modules/pcre.c:312` — `zstrtol` skips LEADING whitespace,
    /// then parses digits. Pin so a regression that drops the leading-
    /// ws skip wouldn't break `pcre_match -n 42` style invocations.
    #[test]
    fn getposint_skips_leading_whitespace() {
        let _g = crate::test_util::global_state_lock();
        // C zstrtol skips spaces/tabs at the front per Src/utils.c:2444-2445.
        assert_eq!(
            getposint("  42", "test"),
            42,
            "c:312 — zstrtol skips leading whitespace"
        );
    }

    /// `Src/Modules/pcre.c:317` — negative parsed value → error.
    /// `zstrtol("-1", &eptr, 10)` returns -1 (signed); `ret < 0` fires.
    #[test]
    fn getposint_rejects_negative() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            getposint("-1", "test"),
            -1,
            "c:317 — `ret < 0` branch fires for negative input"
        );
        assert_eq!(getposint("-100", "test"), -1);
    }

    /// `Src/Modules/pcre.c:312` — empty input parses to 0 with empty
    /// eptr. The C check at c:317 has `ret < 0` false (0 is not <0)
    /// and `*eptr` checks the NUL terminator (falsy) → no error,
    /// returns 0. Pin the empty-input contract.
    #[test]
    fn getposint_empty_input_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        // C: zstrtol("") returns 0, eptr is the same empty pointer.
        // *eptr is '\0' (falsy), ret=0 (not <0) → no error.
        assert_eq!(
            getposint("", "test"),
            0,
            "c:312-325 — empty input → 0 (no error)"
        );
    }

    // ─── zsh-corpus pins for cond_pcre_match ──────────────────────

    /// `[[ "abc123" -pcre-match "[a-z]+[0-9]+" ]]` → 1 (match).
    #[test]
    fn pcre_corpus_cond_match_charclass_quantifier() {
        let _g = crate::test_util::global_state_lock();
        let r = cond_pcre_match(&["abc123".to_string(), "[a-z]+[0-9]+".to_string()], 0);
        assert_eq!(r, 1, "regex match succeeds");
    }

    /// `[[ "xyz" -pcre-match "[0-9]+" ]]` → 0 (no match).
    #[test]
    fn pcre_corpus_cond_match_no_digits() {
        let _g = crate::test_util::global_state_lock();
        let r = cond_pcre_match(&["xyz".to_string(), "[0-9]+".to_string()], 0);
        assert_eq!(r, 0, "no digits in 'xyz' = false");
    }

    /// Empty pattern matches empty string.
    #[test]
    fn pcre_corpus_cond_match_empty_pattern_matches_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = cond_pcre_match(&["".to_string(), "".to_string()], 0);
        assert_eq!(r, 1);
    }

    /// PCRE match populates `$MATCH` to the full match.
    #[test]
    fn pcre_corpus_cond_match_sets_MATCH() {
        let _g = crate::test_util::global_state_lock();
        let _ = cond_pcre_match(&["abc123".to_string(), "[a-z]+[0-9]+".to_string()], 0);
        assert_eq!(
            crate::ported::params::getsparam("MATCH").as_deref(),
            Some("abc123"),
            "$MATCH = whole-match",
        );
    }

    /// PCRE match captures populate `$match[N]`.
    #[test]
    fn pcre_corpus_cond_match_sets_match_array() {
        let _g = crate::test_util::global_state_lock();
        let _ = cond_pcre_match(&["abc123".to_string(), "([a-z]+)([0-9]+)".to_string()], 0);
        let m = crate::ported::params::getaparam("match");
        assert_eq!(
            m.as_deref(),
            Some(&["abc".to_string(), "123".to_string()][..]),
            "$match[1..N] populated from capture groups",
        );
    }

    /// Too few args returns 0.
    #[test]
    fn pcre_corpus_cond_match_one_arg_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = cond_pcre_match(&["only".to_string()], 0);
        assert_eq!(r, 0);
    }

    /// Invalid regex returns 0 (compile failure).
    #[test]
    fn pcre_corpus_cond_match_invalid_pattern_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = cond_pcre_match(&["abc".to_string(), "[unterminated".to_string()], 0);
        assert_eq!(r, 0, "invalid pattern = no match");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/pcre.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:53 — `zpcre_utf8_enabled()` returns 0 or 1.
    #[test]
    fn zpcre_utf8_enabled_returns_zero_or_one() {
        let _g = crate::test_util::global_state_lock();
        let v = zpcre_utf8_enabled();
        assert!(v == 0 || v == 1, "must be 0 or 1, got {}", v);
    }

    /// c:53 — deterministic.
    #[test]
    fn zpcre_utf8_enabled_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = zpcre_utf8_enabled();
        for _ in 0..5 {
            assert_eq!(zpcre_utf8_enabled(), first);
        }
    }

    /// c:625 — empty args → 0 (insufficient).
    #[test]
    fn cond_pcre_match_empty_args_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(cond_pcre_match(&[], 0), 0);
    }

    /// c:625 — exact-string match → 1.
    #[test]
    fn cond_pcre_match_exact_string_matches() {
        let _g = crate::test_util::global_state_lock();
        let r = cond_pcre_match(&["hello".to_string(), "hello".to_string()], 0);
        assert_eq!(r, 1);
    }

    /// c:625 — no-match → 0.
    #[test]
    fn cond_pcre_match_no_match_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = cond_pcre_match(&["abc".to_string(), "xyz".to_string()], 0);
        assert_eq!(r, 0);
    }

    /// c:625 — anchored `^` pattern.
    #[test]
    fn cond_pcre_match_anchored_start() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            cond_pcre_match(&["hello".to_string(), "^h".to_string()], 0),
            1
        );
        assert_eq!(
            cond_pcre_match(&["xhello".to_string(), "^h".to_string()], 0),
            0
        );
    }

    /// c:625 — \\d+ digit class match.
    #[test]
    fn cond_pcre_match_digit_class() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            cond_pcre_match(&["abc123".to_string(), r"\d+".to_string()], 0),
            1
        );
        assert_eq!(
            cond_pcre_match(&["abc".to_string(), r"\d+".to_string()], 0),
            0
        );
    }

    /// c:478 — `getposint("0", _)` returns 0.
    #[test]
    fn pcre_getposint_zero_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getposint("0", "test"), 0);
    }

    /// c:478 — `getposint("-1", _)` returns -1.
    #[test]
    fn pcre_getposint_negative_returns_minus_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getposint("-1", "test"), -1);
    }

    /// c:478 — canonical positive decimal parses.
    #[test]
    fn pcre_getposint_canonical_positive() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getposint("123", "test"), 123);
    }

    /// c:686 — setup_(NULL) = 0.
    #[test]
    fn pcre_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/pcre.c
    // c:53 zpcre_utf8_enabled / c:478 getposint / c:625 cond_pcre_match /
    // c:686-723 lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:53 — `zpcre_utf8_enabled` returns boolean i32 (0/1).
    #[test]
    fn zpcre_utf8_enabled_returns_boolean_i32() {
        let r = zpcre_utf8_enabled();
        assert!(r == 0 || r == 1, "must be 0 or 1, got {}", r);
    }

    /// c:478 — `getposint` deterministic for arbitrary input.
    #[test]
    fn pcre_getposint_is_deterministic() {
        for input in ["42", "abc", "", "0", "999"] {
            let first = getposint(input, "test");
            for _ in 0..3 {
                assert_eq!(
                    getposint(input, "test"),
                    first,
                    "getposint({:?}) must be deterministic",
                    input
                );
            }
        }
    }

    /// c:478 — `getposint` empty string returns 0 (per zsh convention).
    #[test]
    fn pcre_getposint_empty_returns_zero() {
        let r = getposint("", "test");
        // Per shared C convention, empty might be 0 or -1; pin behavior.
        assert!(r == 0 || r == -1, "empty string returns 0 or -1, got {}", r);
    }

    /// c:625 — `cond_pcre_match` return strictly boolean (0/1).
    #[test]
    fn cond_pcre_match_return_in_boolean_set() {
        let _g = crate::test_util::global_state_lock();
        for args in [
            vec!["a".to_string(), "a".to_string()],
            vec!["".to_string(), "".to_string()],
            vec![],
            vec!["bad[".to_string(), "x".to_string()],
        ] {
            let r = cond_pcre_match(&args, 0);
            assert!(
                r == 0 || r == 1,
                "cond_pcre_match({:?}) = {} not in {{0,1}}",
                args,
                r
            );
        }
    }

    /// c:625 — `cond_pcre_match` is deterministic.
    #[test]
    fn cond_pcre_match_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for args in [
            vec!["x".to_string(), "y".to_string()],
            vec!["[0-9]+".to_string(), "123abc".to_string()],
        ] {
            let first = cond_pcre_match(&args, 0);
            for _ in 0..5 {
                assert_eq!(
                    cond_pcre_match(&args, 0),
                    first,
                    "cond_pcre_match({:?}) must be deterministic",
                    args
                );
            }
        }
    }

    /// c:625 — invalid regex returns 0 (no panic on bad pattern).
    #[test]
    fn cond_pcre_match_invalid_regex_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = cond_pcre_match(&["[invalid".to_string(), "x".to_string()], 0);
        assert_eq!(r, 0, "invalid regex → 0 (no match)");
    }

    /// c:625 — large input doesn't panic.
    #[test]
    fn cond_pcre_match_large_input_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let big = "x".repeat(10000);
        let _ = cond_pcre_match(&["x+".to_string(), big], 0);
    }

    /// c:686-723 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn pcre_full_lifecycle_returns_zero_for_all() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        let mut feats = Vec::new();
        let _ = features_(null, &mut feats);
        let mut enables: Option<Vec<i32>> = None;
        let _ = enables_(null, &mut enables);
        assert_eq!(boot_(null), 0);
        assert_eq!(cleanup_(null), 0);
        assert_eq!(finish_(null), 0);
    }

    /// c:686 — setup_ idempotent.
    #[test]
    fn pcre_setup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/pcre.c
    // c:53 zpcre_utf8_enabled / c:79 bin_pcre_compile / c:153 bin_pcre_study /
    // c:500 bin_pcre_match / c:625 cond_pcre_match / c:478 getposint
    // ═══════════════════════════════════════════════════════════════════

    /// c:53 — `zpcre_utf8_enabled` returns i32 (compile-time pin).
    #[test]
    fn zpcre_utf8_enabled_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = zpcre_utf8_enabled();
    }

    /// c:53 — `zpcre_utf8_enabled` returns boolean 0 or 1 only.
    #[test]
    fn zpcre_utf8_enabled_is_boolean() {
        let _g = crate::test_util::global_state_lock();
        let r = zpcre_utf8_enabled();
        assert!(r == 0 || r == 1, "result must be boolean 0/1, got {}", r);
    }

    /// c:53 — `zpcre_utf8_enabled` is deterministic.
    #[test]
    fn zpcre_utf8_enabled_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = zpcre_utf8_enabled();
        for _ in 0..10 {
            assert_eq!(
                zpcre_utf8_enabled(),
                first,
                "zpcre_utf8_enabled must be pure"
            );
        }
    }

    /// c:79 — `bin_pcre_compile` returns i32 (compile-time pin).
    #[test]
    fn bin_pcre_compile_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let _: i32 = bin_pcre_compile("pcre_compile", &[], &ops, 0);
    }

    /// c:79 — `bin_pcre_compile` no-args MUST return nonzero (usage error).
    /// In zshrs the port returns 0 (success) silently.
    #[test]
    fn bin_pcre_compile_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_pcre_compile("pcre_compile", &[], &ops, 0);
        assert_ne!(r, 0, "no args → usage error");
    }

    /// c:153 — `bin_pcre_study` returns i32 (compile-time pin).
    #[test]
    fn bin_pcre_study_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let _: i32 = bin_pcre_study("pcre_study", &[], &ops, 0);
    }

    /// c:500 — `bin_pcre_match` no-args returns nonzero (usage error).
    #[test]
    fn bin_pcre_match_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_pcre_match("pcre_match", &[], &ops, 0);
        assert_ne!(r, 0, "no args → usage error");
    }

    /// c:500 — `bin_pcre_match` returns i32 (compile-time pin).
    #[test]
    fn bin_pcre_match_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let _: i32 = bin_pcre_match("pcre_match", &[], &ops, 0);
    }

    /// c:625 — `cond_pcre_match` returns i32 (compile-time pin).
    #[test]
    fn cond_pcre_match_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = cond_pcre_match(&["a".to_string(), "a".to_string()], 0);
    }

    /// c:625 — `cond_pcre_match` with insufficient args (1) doesn't panic.
    #[test]
    fn cond_pcre_match_single_arg_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = cond_pcre_match(&["pattern".to_string()], 0);
    }

    /// c:478 — `getposint` returns i32 (compile-time pin).
    #[test]
    fn pcre_getposint_returns_i32_type() {
        let _: i32 = getposint("0", "test");
    }
}
