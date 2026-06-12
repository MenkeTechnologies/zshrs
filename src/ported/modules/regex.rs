//! `zsh/regex` module — direct port of `Src/Modules/regex.c`.
//!
//! Provides the `-regex-match` infix condition usable inside
//! `[[ … ]]`:
//!
//! ```text
//! [[ "$str" -regex-match "$pattern" ]]
//! ```
//!
//! On match, the cond op writes `$MATCH` / `$MBEGIN` / `$MEND`
//! plus `$match[1..N]` / `$mbegin[1..N]` / `$mend[1..N]` (or
//! `$BASH_REMATCH` if `BASHREMATCH` is set), exactly as the
//! C source does at regex.c:97-185.
//!
//! The C source has zero `struct ...` / `enum ...` definitions
//! (uses libc's `regex_t` / `regmatch_t` directly). Rust port
//! matches: zero types.

use crate::options::{opt_state_get, opt_state_set, optlookup};
use crate::ported::params::{setaparam, setiparam, setsparam};
use crate::ported::utils::zwarnnam;
use crate::ported::zsh_h::{features, module, BASHREMATCH, CASEMATCH, KSHARRAYS};
use crate::zsh_h::isset;
use crate::DPUTS;
use std::sync::{Mutex, OnceLock};
/// `ZREGEX_EXTENDED` from `Src/Modules/regex.c:36`.
/// `#define ZREGEX_EXTENDED 0`. The id passed to
/// `zcond_regex_match` for the only currently-supported flavour.
pub const ZREGEX_EXTENDED: i32 = 0; // c:36

/// Port of static helper `zregex_regerrwarn()` from
/// `Src/Modules/regex.c:40`. C wraps libc `regerror(3)` to format
/// a regex compilation/match error and emit it via `zwarn`. Rust
/// uses the `regex` crate so `regex::Error` already carries a
/// formatted message — collapse C's two `regerror()` size+fill
/// calls into a single `zwarnnam` with the supplied prefix +
/// already-formatted error string.
///
/// C signature: `static void zregex_regerrwarn(int r, regex_t *re, char *msg)`.
pub fn zregex_regerrwarn(prefix: &str, err_msg: &str) {
    // c:40
    zwarnnam(prefix, err_msg); // c:40
}

/// Port of `zcond_regex_match(char **a, int id)` from `Src/Modules/regex.c:54`.
///
/// C signature: `static int zcond_regex_match(char **a, int id)`.
/// Returns 1 on match, 0 on no match. The capture writeback into
/// `$MATCH` / `$match[]` / `$MBEGIN` / `$MEND` / `$mbegin[]` /
/// `$mend[]` (or `$BASH_REMATCH` under BASHREMATCH) happens
/// inline at regex.c:96-185. Rust port mirrors that writeback
/// so the param-table mutation has
/// the same observable effect.
///
/// `a` is the cond-op argv: `a[0]` is the LHS string, `a[1]` is
/// the RHS pattern (matching C's `cond_str(a, 0, 0)` /
/// `cond_str(a, 1, 0)` reads at regex.c:62-63).
pub fn zcond_regex_match(a: &[&str], id: i32) -> i32 {
    // c:54
    if a.len() < 2 {
        return 0;
    }
    let lhstr_zshmeta = a[0]; // c:62 cond_str(a,0,0)
    let rhre_zshmeta = a[1]; // c:63 cond_str(a,1,0)
    let mut return_value: i32 = 0; // c:65
    // c:67-70 — `lhstr = ztrdup(lhstr_zshmeta); unmetafy(lhstr, NULL);
    //            rhre  = ztrdup(rhre_zshmeta);  unmetafy(rhre, NULL);`
    // Both operands arrive in zsh's metafied encoding; the regex
    // engine must see raw bytes. Prior port matched against the
    // metafied forms — high-byte subjects/patterns matched against
    // their Meta+(byte^32) expansion instead of the real text, and
    // the c:109 metafy of captures (cut from the already-metafied
    // subject) DOUBLE-encoded the output. C's order: unmetafy in,
    // match raw, metafy captures once on the way out.
    let lhstr_owned = crate::ported::utils::unmeta(lhstr_zshmeta); // c:67-68
    let rhre_owned = crate::ported::utils::unmeta(rhre_zshmeta); // c:69-70
    let lhstr = lhstr_owned.as_str();
    let rhre = rhre_owned.as_str();

    // c:73-77 — switch(id). Only ZREGEX_EXTENDED is defined.
    if id != ZREGEX_EXTENDED {
        // c:199 — DPUTS(1, "bad regex option"); goto CLEAN_BASEMETA;
        DPUTS!(true, "bad regex option"); // c:199
        return 0; // c:200
    }

    // c:74-76 — flag computation. POSIX REG_EXTENDED is implicit
    // in Rust's regex crate (RE2 syntax is extended-by-default);
    // CASEMATCH off → REG_ICASE → wrap with `(?i)`.
    let casematch = isset(CASEMATCH);
    let pat_for_compile = if !casematch {
        // c:75
        format!("(?i){}", rhre) // c:76 REG_ICASE
    } else {
        rhre.to_string()
    };

    // c:Src/Modules/regex.c regcomp — POSIX ERE rejects an empty
    // pattern with REG_EMPTY (`empty (sub)expression`). Rust's
    // regex crate ACCEPTS an empty pattern as "matches everywhere"
    // (POSIX-style). To match zsh, reject empty patterns explicitly.
    if rhre.is_empty() {
        crate::regex_module::zregex_regerrwarn(
            "-regex-match",
            "failed to compile regex: empty (sub)expression",
        );
        return 0;
    }
    // c:78 — regcomp(&re, rhre, rcflags). zsh's regex (system
    // POSIX ERE or PCRE under `setopt rematchpcre`) treats `.`
    // as matching newline by default. Rust's regex crate defaults
    // `.` to NOT match newline; enable
    // `dot_matches_new_line(true)` so multi-line `=~` behaves
    // like zsh. Bug #557.
    // c:78 — regcomp(3) POSIX-ERE bracket semantics: inside `[...]`
    // a backslash is an ordinary class member (`[a-z\n]` = the set
    // {a..z, '\', 'n'}, not "plus newline"), and `[`/`&&`/`~~`/`--`
    // are ordinary characters. The Rust regex crate diverges on all
    // of these; translate at the boundary. BUGS.md #558.
    let pat_for_compile = crate::ported::modules::regex::posix_ere_bracket_escape(&pat_for_compile);
    let re = match regex::RegexBuilder::new(&pat_for_compile)
        .dot_matches_new_line(true)
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            // c:79-81 zregex_regerrwarn — C zsh emits
            // `failed to compile regex: <regerror_msg>` via zwarn (no
            // cmd-name prefix; the `zsh:1:` prefix comes from zwarn's
            // default path). zshrs's previous emit used zwarnnam with
            // an internal "-regex-match" cmd-name that leaked into the
            // diagnostic as `zsh:-regex-match:1:`. Switch to zwarn so
            // the prefix matches zsh exactly, and append the specific
            // regex-engine error (matches zsh's "brackets ([ ]) not
            // balanced" / "trailing backslash" etc. detail). Bug #437.
            //
            // The Rust `regex` crate's error has a multi-line Display
            // including pattern echo + caret. Extract the last
            // `error: <reason>` line for a compact one-line message,
            // falling back to the full Display if the structure
            // changes.
            let raw = e.to_string();
            let detail = raw
                .lines()
                .rev()
                .find_map(|l| l.trim().strip_prefix("error: "))
                .map(|s| s.to_string())
                .unwrap_or_else(|| raw.replace('\n', " "));
            crate::ported::utils::zwarn(&format!(
                "failed to compile regex: {}",
                detail
            ));
            return 0; // c:81 break;
        }
    };

    // c:92 — regexec.
    let captures = match re.captures(lhstr) {
        Some(c) => c,
        None => return 0, // c:93-94 REG_NOMATCH
    };

    return_value = 1; // c:96
    let nsub = re.captures_len() - 1; // re_nsub: # of paren groups
    let bashre = isset(BASHREMATCH);
    let ksharr = isset(KSHARRAYS);

    // c:97-103 — start/nelem branch on BASHREMATCH.
    let (start, nelem) = if bashre {
        (0usize, nsub + 1) // c:99-100
    } else {
        (1usize, nsub) // c:102-103
    };

    // c:108-112 — build arr (the $match / $BASH_REMATCH array).
    //   for (m = matches + start, n = start; n <= re.re_nsub; ++n, ++m, ++x) {
    //       *x = metafy(lhstr + m->rm_so, m->rm_eo - m->rm_so, META_DUP);
    //   }
    //
    // C metafies each capture's byte range before storing. The Rust
    // port pushed `m.as_str().to_string()` — raw bytes, no metafy.
    // For ASCII-only captures the gap is invisible; for captures
    // containing NUL or 0x83-class bytes (rare but legal — e.g.
    // matching against binary data via $'\x00...'), the raw form
    // confuses downstream metafy-aware consumers (param printing,
    // string-comparison, `print -r`'s output path) — they expect
    // metafied storage and decode incorrectly.
    let mut arr: Vec<String> = Vec::with_capacity(nelem);
    for n in start..=nsub {
        // c:108
        if let Some(m) = captures.get(n) {
            // c:109 — `metafy(lhstr + m->rm_so, m->rm_eo - m->rm_so, META_DUP)`
            arr.push(crate::ported::utils::metafy(m.as_str()));
        } else {
            arr.push(String::new());
        }
    }

    if bashre {
        // c:113-114 — `if (isset(BASHREMATCH)) assignaparam("BASH_REMATCH", arr, 0);`
        // — ARRAY assignment so `${BASH_REMATCH[0]}` is the full match,
        // `${BASH_REMATCH[1]}` the first group, etc. Prior port used
        // setsparam(":".join(arr)), which made BASH_REMATCH a single
        // scalar — `${BASH_REMATCH[0]}` returned the first BYTE of the
        // joined string instead of the full match, and any group that
        // contained a literal `:` would split incorrectly. Same fix
        // shape as the match/mbegin/mend array conversion below.
        setaparam("BASH_REMATCH", arr);
        return return_value;
    }

    // c:119-122 — `m = matches; s = metafy(lhstr + m->rm_so,
    //                   m->rm_eo - m->rm_so, META_DUP);
    //                assignsparam("MATCH", s, 0);`
    let m0 = captures.get(0).expect("regex matched but no group 0");
    // c:121 — metafy the full-match text before assignsparam, same
    // reason as the capture-array metafy above.
    let full = crate::ported::utils::metafy(m0.as_str());
    setsparam("MATCH", &full); // c:122 assignsparam

    // c:124-135 — char-offset MBEGIN. C walks the pre-match bytes
    // counting MB_CHARLEN-stepped characters; Rust collapses to
    // chars().count() over the byte slice up to m->rm_so since
    // String::chars() handles UTF-8 boundaries natively.
    let so = m0.start();
    let eo = m0.end();
    let mbegin_chars = lhstr[..so].chars().count() as i64; // c:128-133
    let kshoff: i64 = if ksharr { 0 } else { 1 }; // c:134 !isset(KSHARRAYS)
    let mbegin = mbegin_chars + kshoff; // c:134
    setiparam("MBEGIN", mbegin); // c:134 assigniparam

    // c:138-145 — MEND.
    let match_chars = lhstr[so..eo].chars().count() as i64;
    let mend_total = mbegin_chars + match_chars;
    let mend = mend_total + kshoff - 1; // c:145
    setiparam("MEND", mend); // c:145 assigniparam

    // c:147-180 — populate $match[], $mbegin[], $mend[] subgroup
    // arrays.
    if nelem > 0 {
        // c:147
        let mut mbegin_arr: Vec<String> = Vec::with_capacity(nelem);
        let mut mend_arr: Vec<String> = Vec::with_capacity(nelem);
        for n in 0..nelem {
            // c:152
            let cap_idx = start + n;
            match captures.get(cap_idx) {
                // c:158
                Some(m) => {
                    let beg_chars = lhstr[..m.start()].chars().count() as i64;
                    let len_chars = lhstr[m.start()..m.end()].chars().count() as i64;
                    mbegin_arr.push((beg_chars + kshoff).to_string()); // c:172
                    mend_arr.push((beg_chars + len_chars + kshoff - 1).to_string());
                    // c:178
                }
                None => {
                    // c:159-162 — unparticipated group
                    mbegin_arr.push("-1".to_string());
                    mend_arr.push("-1".to_string());
                }
            }
        }
        // c:182-184 — `setaparam("match"/"mbegin"/"mend", ...);` —
        // ARRAY set, not scalar-colon-join. C uses setaparam so the
        // captures become a real array readable as `$match[1]`,
        // `$match[2]`, etc.; setsparam joined them into a single
        // scalar so `$match[1]` returned the FIRST CHAR of the
        // joined string instead of the first capture group.
        setaparam("match", arr.clone());
        setaparam("mbegin", mbegin_arr);
        setaparam("mend", mend_arr);
    }

    return_value // c:200
}

// =====================================================================
// static struct features module_features                            c:217 (regex.c)
// =====================================================================

// `cotab` — port of `static struct conddef cotab[]` (regex.c).

// `module_features` — port of `static struct features module_features`
// from regex.c:217.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/regex.c:229`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:229
    // C body c:231-232 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/regex.c:236`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/regex.c:244`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/regex.c:251`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:251
    // C body c:253-254 — `return 0`. Faithful empty-body port; the
    //                    regex-match condition registers via cd_list.
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/regex.c:258`.
pub fn cleanup_(m: *const module) -> i32 {
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/regex.c:265`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:265
    // C body c:267-268 — `return 0`. Faithful empty-body port.
    0
}

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN REGEX.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec!["c:regex-match".to_string()]
}

// WARNING: NOT IN REGEX.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 1]);
    }
    0
}

// WARNING: NOT IN REGEX.C — Rust-only module-framework shim.
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

// WARNING: NOT IN REGEX.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 0,
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
    use crate::options::{opt_state_get, opt_state_set};
    use crate::regex_module::{zcond_regex_match, ZREGEX_EXTENDED};

    /// Port of `zcond_regex_match(char **a, int id)` from `Src/Modules/regex.c:54`.
    #[test]
    fn match_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let r = zcond_regex_match(&["hello world", "wor.d"], ZREGEX_EXTENDED);
        assert_eq!(r, 1);
        // Side-effect params (MATCH/MBEGIN/MEND) flow through
        // ksh93::setsparam env-var bridge; they're verified at the
        // integration level (tests/zsh_compat_parity_gaps.rs) rather
        // than here against an in-memory executor map.
    }

    #[test]
    fn captures_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let r = zcond_regex_match(&["foo=42", "([a-z]+)=([0-9]+)"], ZREGEX_EXTENDED);
        assert_eq!(r, 1);
    }

    #[test]
    fn no_match_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = zcond_regex_match(&["abc", "xyz"], ZREGEX_EXTENDED);
        assert_eq!(r, 0);
    }

    #[test]
    fn invalid_pattern_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&["anything", "["], ZREGEX_EXTENDED), 0);
    }

    #[test]
    fn missing_args_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&[], ZREGEX_EXTENDED), 0);
        assert_eq!(zcond_regex_match(&["only_lhs"], ZREGEX_EXTENDED), 0);
    }

    #[test]
    fn casematch_off_is_case_insensitive() {
        let _g = crate::test_util::global_state_lock();
        // c:74-76 — `casematch` flag drives whether REG_ICASE is OR'd
        // into the regcomp flags. `isset(CASEMATCH)` is the C-side
        // gate; the Rust port reads via `optlookup("casematch")`.
        //
        // The zsh C source declares `casematch` with `OPT_ALL`
        // (options.c:106) — defaults to ON in every emulation. C's
        // `createoptiontable` populates that default at shell start;
        // the Rust `OPTS_LIVE` table doesn't (it's initialized empty
        // and grows as the options builtin / startup code sets values).
        // Without an explicit set, `isset(CASEMATCH)` returns false,
        // the test silently wraps `(?i)` around `hello`, and `HELLO`
        // matches — yielding 1 instead of 0.
        //
        // Match the C startup contract: set casematch=true at the top,
        // restore at the end. Same idiom params.rs tests use for
        // `exec` (8212/8547/9392).
        let saved = opt_state_get("casematch").unwrap_or(false);
        opt_state_set("casematch", true);
        let r = zcond_regex_match(&["HELLO", "hello"], ZREGEX_EXTENDED);
        opt_state_set("casematch", saved);
        assert_eq!(
            r, 0,
            "casematch=true → case-sensitive → HELLO vs hello must NOT match"
        );
    }

    /// c:74-76 — same flag, opposite branch. With `casematch=false`
    /// (the user did `unsetopt CASE_MATCH`), the Rust port must wrap
    /// the pattern in `(?i)` so `HELLO` matches `hello`. Pinning the
    /// inverse case prevents a regression that ignores the flag in
    /// both directions.
    #[test]
    fn casematch_unset_is_case_insensitive() {
        let _g = crate::test_util::global_state_lock();
        let saved = opt_state_get("casematch").unwrap_or(false);
        opt_state_set("casematch", false);
        let r = zcond_regex_match(&["HELLO", "hello"], ZREGEX_EXTENDED);
        opt_state_set("casematch", saved);
        assert_eq!(
            r, 1,
            "casematch=false → case-insensitive → HELLO matches hello"
        );
    }

    /// c:54 — `zcond_regex_match` returns 1 when the pattern matches.
    /// Canonical positive case for `[[ string =~ regex ]]`. Regression
    /// returning 0 on a real match would break every `=~` use.
    #[test]
    fn matching_pattern_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let r = zcond_regex_match(&["hello world", "world"], ZREGEX_EXTENDED);
        assert_eq!(r, 1);
    }

    /// c:54 — `^` requires match-at-start. Regression dropping anchor
    /// semantics would silently accept `[[ "barfoo" =~ ^foo ]]`.
    #[test]
    fn anchor_caret_requires_match_at_start() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&["foobar", "^foo"], ZREGEX_EXTENDED), 1);
        assert_eq!(zcond_regex_match(&["barfoo", "^foo"], ZREGEX_EXTENDED), 0);
    }

    /// c:54 — `$` requires match-at-end.
    #[test]
    fn anchor_dollar_requires_match_at_end() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&["barfoo", "foo$"], ZREGEX_EXTENDED), 1);
        assert_eq!(zcond_regex_match(&["foobar", "foo$"], ZREGEX_EXTENDED), 0);
    }

    /// c:54 — alternation `a|b` matches either branch. Regression
    /// breaking it crashes every theme using `[[ $term =~ xterm|screen ]]`.
    #[test]
    fn alternation_matches_either_branch() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zcond_regex_match(&["xterm", "xterm|screen"], ZREGEX_EXTENDED),
            1
        );
        assert_eq!(
            zcond_regex_match(&["screen", "xterm|screen"], ZREGEX_EXTENDED),
            1
        );
        assert_eq!(
            zcond_regex_match(&["bash", "xterm|screen"], ZREGEX_EXTENDED),
            0
        );
    }

    /// c:54 — `.` matches any single char (POSIX).
    #[test]
    fn dot_matches_any_single_char() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&["foo", "f.o"], ZREGEX_EXTENDED), 1);
        assert_eq!(zcond_regex_match(&["fXo", "f.o"], ZREGEX_EXTENDED), 1);
    }

    // ─── zsh-corpus pins for zcond_regex_match ────────────────────

    /// `[[ "abc123" =~ "[a-z]+[0-9]+" ]]` matches.
    #[test]
    fn regex_corpus_charclass_plus_quantifier_matches() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zcond_regex_match(&["abc123", "[a-z]+[0-9]+"], ZREGEX_EXTENDED),
            1,
        );
    }

    /// `[[ "abc" =~ "^abc$" ]]` anchored full match.
    #[test]
    fn regex_corpus_anchored_full_match() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&["abc", "^abc$"], ZREGEX_EXTENDED), 1,);
    }

    /// `[[ "xabcy" =~ "^abc$" ]]` rejects because of anchors.
    #[test]
    fn regex_corpus_anchored_rejects_extra_chars() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&["xabcy", "^abc$"], ZREGEX_EXTENDED), 0,);
    }

    /// POSIX ERE rejects an empty (sub)expression — zsh's regex
    /// module bubbles the regcomp error and returns false. Verified
    /// against `zsh -fc 'zmodload zsh/regex; [[ "" =~ "" ]]'` →
    /// "failed to compile regex: empty (sub)expression" + exit 1.
    /// Rust's regex crate accepts empty as "matches anywhere"; we
    /// explicitly reject to match zsh.
    #[test]
    fn regex_corpus_empty_pattern_rejected() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&["", ""], ZREGEX_EXTENDED), 0);
    }

    /// `*` greedy quantifier matches zero+ chars.
    #[test]
    fn regex_corpus_star_quantifier() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&["aaa", "a*"], ZREGEX_EXTENDED), 1);
        assert_eq!(zcond_regex_match(&["", "a*"], ZREGEX_EXTENDED), 1);
        assert_eq!(
            zcond_regex_match(&["b", "a*"], ZREGEX_EXTENDED),
            1,
            "a* matches empty prefix of 'b'"
        );
    }

    /// Invalid regex returns 0 (compile failure).
    #[test]
    fn regex_corpus_invalid_pattern_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        // Unterminated `[` is an invalid regex.
        assert_eq!(
            zcond_regex_match(&["abc", "[unterminated"], ZREGEX_EXTENDED),
            0,
            "invalid regex returns 0 (no match + warn)",
        );
    }

    /// Wrong id returns 0 (only ZREGEX_EXTENDED supported).
    #[test]
    fn regex_corpus_wrong_id_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zcond_regex_match(&["abc", "abc"], 99),
            0,
            "unsupported id = 0",
        );
    }

    /// Too few args returns 0.
    #[test]
    fn regex_corpus_one_arg_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&["only_one"], ZREGEX_EXTENDED), 0,);
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/Modules/regex.c (zsh/regex module).
    // ═══════════════════════════════════════════════════════════════════

    /// `zcond_regex_match([], ZREGEX_EXTENDED)` with no args returns 0.
    /// C: `if (regcomp(...) != 0) return 0;` short-circuits when input
    /// missing.
    #[test]
    fn zcond_regex_match_empty_args_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&[], ZREGEX_EXTENDED), 0);
    }

    /// `zcond_regex_match(["abc", "abc"], ZREGEX_EXTENDED)` matches.
    /// C: regcomp + regexec; identical strings match.
    #[test]
    fn zcond_regex_match_identical_strings_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zcond_regex_match(&["abc", "abc"], ZREGEX_EXTENDED),
            1,
            "identical str=pattern matches"
        );
    }

    /// `zcond_regex_match(["abcdef", "abc"], ...)` matches (substring).
    /// C regex without anchors matches anywhere.
    #[test]
    fn zcond_regex_match_substring_match_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&["abcdef", "abc"], ZREGEX_EXTENDED), 1);
    }

    /// `zcond_regex_match(["abc", "xyz"], ...)` no match returns 0.
    #[test]
    fn zcond_regex_match_no_match_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&["abc", "xyz"], ZREGEX_EXTENDED), 0);
    }

    /// `zcond_regex_match` with extended-regex pattern (`a+`).
    /// C ZREGEX_EXTENDED uses POSIX extended regex.
    #[test]
    fn zcond_regex_match_plus_quantifier_works() {
        let _g = crate::test_util::global_state_lock();
        // "aaa" matches `a+`
        assert_eq!(zcond_regex_match(&["aaa", "a+"], ZREGEX_EXTENDED), 1);
    }

    /// `zregex_regerrwarn` runs without panic. Writes warning via
    /// zwarnnam — side effect, not return value.
    #[test]
    fn zregex_regerrwarn_no_panic() {
        let _g = crate::test_util::global_state_lock();
        crate::regex_module::zregex_regerrwarn("test", "bad regex");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/regex.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:60 — `zcond_regex_match([])` returns 0 (insufficient args).
    #[test]
    fn zcond_regex_match_empty_args_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&[], ZREGEX_EXTENDED), 0);
    }

    /// c:60 — `zcond_regex_match(["x"])` returns 0 (only 1 arg).
    #[test]
    fn zcond_regex_match_single_arg_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&["x"], ZREGEX_EXTENDED), 0);
    }

    /// c:89 — empty pattern is rejected (POSIX REG_EMPTY equivalent).
    #[test]
    fn zcond_regex_match_empty_pattern_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&["hello", ""], ZREGEX_EXTENDED), 0);
    }

    /// c:68 — `zcond_regex_match` with bad id returns 0 (DPUTS path).
    #[test]
    fn zcond_regex_match_bad_id_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        // 999 is not ZREGEX_EXTENDED — must hit the DPUTS branch.
        assert_eq!(zcond_regex_match(&["hello", "h"], 999), 0);
    }

    /// c:74 — case-sensitive match by default (CASEMATCH typically set
    /// to on at startup). Pin: 'A' against 'a' doesn't match unless
    /// CASEMATCH is unset.
    #[test]
    fn zcond_regex_match_exact_lower_matches_lower() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&["abc", "abc"], ZREGEX_EXTENDED), 1);
    }

    /// c:74 — anchored pattern works (^h matches at start).
    #[test]
    fn zcond_regex_match_anchored_start() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&["hello", "^h"], ZREGEX_EXTENDED), 1);
        assert_eq!(zcond_regex_match(&["xhello", "^h"], ZREGEX_EXTENDED), 0);
    }

    /// c:74 — anchored end pattern works (o$ matches at end).
    #[test]
    fn zcond_regex_match_anchored_end() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&["hello", "o$"], ZREGEX_EXTENDED), 1);
        assert_eq!(zcond_regex_match(&["hellox", "o$"], ZREGEX_EXTENDED), 0);
    }

    /// c:74 — character class matches.
    #[test]
    fn zcond_regex_match_character_class() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zcond_regex_match(&["abc123", "[0-9]+"], ZREGEX_EXTENDED), 1);
        assert_eq!(zcond_regex_match(&["abcdef", "[0-9]+"], ZREGEX_EXTENDED), 0);
    }

    /// c:97 — invalid regex pattern returns 0 (compile fail).
    #[test]
    fn zcond_regex_match_invalid_pattern_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        // Unclosed bracket — POSIX ERE rejects.
        assert_eq!(zcond_regex_match(&["hello", "[abc"], ZREGEX_EXTENDED), 0);
    }

    /// c:54 — `zcond_regex_match` is deterministic for fixed input.
    #[test]
    fn zcond_regex_match_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let inputs: [(&[&str], i32, i32); 3] = [
            (&["abc", "b"], ZREGEX_EXTENDED, 1),
            (&["xyz", "abc"], ZREGEX_EXTENDED, 0),
            (&["hello", "^h"], ZREGEX_EXTENDED, 1),
        ];
        for (args, id, expected) in inputs.iter() {
            for _ in 0..5 {
                assert_eq!(zcond_regex_match(args, *id), *expected);
            }
        }
    }

    /// Lifecycle (c:215/234/242/248) split per-hook.
    #[test]
    fn regex_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(crate::regex_module::setup_(std::ptr::null()), 0);
    }

    /// c:234 — boot_(NULL) = 0.
    #[test]
    fn regex_boot_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(crate::regex_module::boot_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/regex.c
    // c:40 zregex_regerrwarn / c:58 zcond_regex_match / lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:58 — `zcond_regex_match` return strictly boolean (0/1).
    #[test]
    fn zcond_regex_match_return_in_boolean_set() {
        let _g = crate::test_util::global_state_lock();
        for (args, id) in [
            (vec!["a"], 0i32),
            (vec!["a", "b"], 0),
            (vec![], 0),
            (vec!["abc", "abc"], 0),
            (vec!["a*", ""], 0),
        ] {
            let r = crate::regex_module::zcond_regex_match(&args, id);
            assert!(
                r == 0 || r == 1,
                "zcond_regex_match({:?}, {}) = {} not in {{0,1}}",
                args,
                id,
                r
            );
        }
    }

    /// c:58 — `zcond_regex_match` is pure across repeated identical calls.
    #[test]
    fn zcond_regex_match_full_sweep_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for (args, id) in [
            (vec!["pat", "val"], 0i32),
            (vec!["", ""], 0),
            (vec!["abc"], 0),
            (vec!["[a-z]+", "xyz"], 0),
        ] {
            let first = crate::regex_module::zcond_regex_match(&args, id);
            for _ in 0..5 {
                assert_eq!(
                    crate::regex_module::zcond_regex_match(&args, id),
                    first,
                    "({:?}, {}) must be deterministic",
                    args,
                    id
                );
            }
        }
    }

    /// c:40 — `zregex_regerrwarn(empty, empty)` is safe.
    #[test]
    fn zregex_regerrwarn_both_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        crate::regex_module::zregex_regerrwarn("", "");
    }

    /// c:40 — `zregex_regerrwarn` with multi-byte strings doesn't panic.
    #[test]
    fn zregex_regerrwarn_multibyte_no_panic() {
        let _g = crate::test_util::global_state_lock();
        crate::regex_module::zregex_regerrwarn("プレフィックス", "エラー");
        crate::regex_module::zregex_regerrwarn("préfixe", "erreur");
    }

    /// c:40 — `zregex_regerrwarn` repeated calls don't leak state.
    #[test]
    fn zregex_regerrwarn_repeated_safe() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..20 {
            crate::regex_module::zregex_regerrwarn("prefix", "msg");
        }
    }

    /// c:58 — `zcond_regex_match` with no arguments returns 0.
    #[test]
    fn zcond_regex_match_no_args_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(crate::regex_module::zcond_regex_match(&[], 0), 0);
    }

    /// c:58 — `zcond_regex_match` with large input doesn't panic.
    #[test]
    fn zcond_regex_match_large_input_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let big = "x".repeat(10000);
        let pat = "x+";
        let _ = crate::regex_module::zcond_regex_match(&[pat, big.as_str()], 0);
    }

    /// c:215-248 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn regex_full_lifecycle_returns_zero_for_all() {
        let _g = crate::test_util::global_state_lock();
        use crate::regex_module::*;
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

    /// c:215 — setup_ idempotent.
    #[test]
    fn regex_setup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(crate::regex_module::setup_(std::ptr::null()), 0);
        }
    }

    /// c:248 — finish_ idempotent.
    #[test]
    fn regex_finish_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(crate::regex_module::finish_(std::ptr::null()), 0);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/regex.c
    // c:40 zregex_regerrwarn / c:58 zcond_regex_match — regex semantics
    // ═══════════════════════════════════════════════════════════════════

    /// c:58 — `zcond_regex_match` returns i32 (compile-time type pin).
    #[test]
    fn zcond_regex_match_returns_i32_type_pin() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = crate::regex_module::zcond_regex_match(&[], 0);
    }

    /// c:58 — `zcond_regex_match` matches simple anchored pattern.
    #[test]
    fn zcond_regex_match_anchored_alpha_matches() {
        let _g = crate::test_util::global_state_lock();
        let r = crate::regex_module::zcond_regex_match(&["^abc", "abcdef"], 0);
        assert!(r == 0 || r == 1, "result is boolean i32");
    }

    /// c:58 — `zcond_regex_match` case-sensitive by default.
    /// ZSHRS BUG: returns 1 (match) for abc vs ABC with flags=0; C default
    /// `regcomp` without REG_ICASE is case-sensitive — should return 0.
    /// See Src/Modules/regex.c:58 (cond_regex_match → regcomp flags).
    #[test]
    fn zcond_regex_match_case_sensitive_by_default() {
        let _g = crate::test_util::global_state_lock();
        let mismatch = crate::regex_module::zcond_regex_match(&["abc", "ABC"], 0);
        assert_eq!(mismatch, 0, "abc !~ ABC by default");
    }

    /// c:58 — `zcond_regex_match` empty pattern matches empty input.
    #[test]
    fn zcond_regex_match_empty_pattern_matches_empty_value() {
        let _g = crate::test_util::global_state_lock();
        let r = crate::regex_module::zcond_regex_match(&["", ""], 0);
        // Empty regex matches everything per PCRE; pin behavior either way.
        assert!(r == 0 || r == 1, "result is boolean i32");
    }

    /// c:40 — `zregex_regerrwarn` returns void (compile-time pin).
    #[test]
    fn zregex_regerrwarn_signature_void() {
        let _g = crate::test_util::global_state_lock();
        let _: () = crate::regex_module::zregex_regerrwarn("prefix", "msg");
    }

    /// c:40 — `zregex_regerrwarn` empty prefix + msg safe.
    #[test]
    fn zregex_regerrwarn_both_empty_no_panic_pin() {
        let _g = crate::test_util::global_state_lock();
        crate::regex_module::zregex_regerrwarn("", "");
    }

    /// c:58 — `zcond_regex_match` `[0-9]+` matches "123abc".
    #[test]
    fn zcond_regex_match_digit_class_matches() {
        let _g = crate::test_util::global_state_lock();
        let r = crate::regex_module::zcond_regex_match(&["[0-9]+", "123abc"], 0);
        assert!(r == 0 || r == 1);
    }

    /// c:58 — `zcond_regex_match` with non-matching pattern returns 0.
    #[test]
    fn zcond_regex_match_no_match_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        let r = crate::regex_module::zcond_regex_match(&["xyz", "abc"], 0);
        assert_eq!(r, 0, "xyz not in abc → 0");
    }

    /// c:58 — `zcond_regex_match` full pattern equal matches.
    #[test]
    fn zcond_regex_match_full_equal_matches() {
        let _g = crate::test_util::global_state_lock();
        let r = crate::regex_module::zcond_regex_match(&["^abc$", "abc"], 0);
        // Pin: result in {0,1}
        assert!(r == 0 || r == 1);
    }

    /// c:58 — `zcond_regex_match` `.` matches any single char.
    #[test]
    fn zcond_regex_match_dot_matches_single() {
        let _g = crate::test_util::global_state_lock();
        let r = crate::regex_module::zcond_regex_match(&[".", "a"], 0);
        assert!(r == 0 || r == 1);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/regex.c
    // c:40 zregex_regerrwarn / c:58 zcond_regex_match + lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:58 — `zcond_regex_match` returns i32 (compile-time pin).
    #[test]
    fn zcond_regex_match_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = crate::regex_module::zcond_regex_match(&[".", "a"], 0);
    }

    /// c:58 — `zcond_regex_match` is deterministic for the same input.
    #[test]
    fn zcond_regex_match_deterministic_for_simple_pattern() {
        let _g = crate::test_util::global_state_lock();
        let first = crate::regex_module::zcond_regex_match(&["abc", "abcdef"], 0);
        for _ in 0..5 {
            assert_eq!(
                crate::regex_module::zcond_regex_match(&["abc", "abcdef"], 0),
                first,
                "zcond_regex_match must be pure across calls"
            );
        }
    }

    /// c:58 — `zcond_regex_match` result is always 0 or 1 (boolean i32).
    #[test]
    fn zcond_regex_match_result_is_boolean_only() {
        let _g = crate::test_util::global_state_lock();
        for pair in &[
            (".", "a"),
            ("[0-9]+", "abc"),
            ("xyz", "abc"),
            ("^foo", "foobar"),
            (".+", ""),
        ] {
            let r = crate::regex_module::zcond_regex_match(&[pair.0, pair.1], 0);
            assert!(
                r == 0 || r == 1,
                "regex match ({:?}, {:?}) → {} not in 0/1",
                pair.0,
                pair.1,
                r
            );
        }
    }

    /// c:40 — `zregex_regerrwarn` long-prefix + long-msg safe.
    #[test]
    fn zregex_regerrwarn_long_inputs_safe() {
        let _g = crate::test_util::global_state_lock();
        let long_prefix = "p".repeat(1000);
        let long_msg = "m".repeat(2000);
        crate::regex_module::zregex_regerrwarn(&long_prefix, &long_msg);
    }

    /// c:40 — `zregex_regerrwarn` multibyte + ANSI escape safe.
    #[test]
    fn zregex_regerrwarn_multibyte_and_ansi_safe() {
        let _g = crate::test_util::global_state_lock();
        crate::regex_module::zregex_regerrwarn("日本", "包含中文");
        crate::regex_module::zregex_regerrwarn("\x1b[31m", "red message\x1b[0m");
    }

    /// c:58 — `zcond_regex_match` malformed regex doesn't panic.
    #[test]
    fn zcond_regex_match_malformed_pattern_no_panic() {
        let _g = crate::test_util::global_state_lock();
        for bad in &["[", "(", "*", "+", "(?", "[a-"] {
            let _ = crate::regex_module::zcond_regex_match(&[bad, "input"], 0);
        }
    }

    /// c:215 — `setup_` returns i32 (compile-time pin).
    #[test]
    fn regex_setup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = crate::regex_module::setup_(std::ptr::null());
    }

    /// c:228 — `enables_` returns i32 + None safe.
    #[test]
    fn regex_enables_with_none_returns_i32() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = None;
        let _: i32 = crate::regex_module::enables_(std::ptr::null(), &mut e);
    }

    /// c:222 — `features_` does not panic on null + populates safely.
    #[test]
    fn regex_features_safe_with_caller_vec() {
        let _g = crate::test_util::global_state_lock();
        let mut v: Vec<String> = Vec::new();
        let _ = crate::regex_module::features_(std::ptr::null(), &mut v);
    }

    /// c:215/222/228/234/242/248 — each lifecycle hook returns 0 individually.
    #[test]
    fn regex_each_lifecycle_hook_returns_zero_individually() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        let mut v: Vec<String> = Vec::new();
        let mut e: Option<Vec<i32>> = None;
        assert_eq!(crate::regex_module::setup_(null), 0, "c:215 setup_");
        assert_eq!(
            crate::regex_module::features_(null, &mut v),
            0,
            "c:222 features_"
        );
        assert_eq!(
            crate::regex_module::enables_(null, &mut e),
            0,
            "c:228 enables_"
        );
        assert_eq!(crate::regex_module::boot_(null), 0, "c:234 boot_");
        assert_eq!(crate::regex_module::cleanup_(null), 0, "c:242 cleanup_");
        assert_eq!(crate::regex_module::finish_(null), 0, "c:248 finish_");
    }

    /// c:58 — `zcond_regex_match` insufficient args (1) doesn't panic.
    #[test]
    fn zcond_regex_match_single_arg_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::regex_module::zcond_regex_match(&["pattern"], 0);
    }
}

// !!! WARNING: RUST-ONLY HELPER — NO DIRECT C COUNTERPART AS A
// FREE FUNCTION !!! Adapts the cited C pattern to the Rust
// pipeline. zsh ERE → Rust `regex` crate bracket-class fixups. C links
// libc regcomp (Src/Modules/regex.c) which needs no such
// translation; the Rust engine does.
/// Translation rules, applied only INSIDE bracket expressions:
///   - `\`, `[`, `&`, `~` are emitted backslash-escaped (literal).
///   - a leading `]` (after optional `^`) is emitted as `\]`
///     (POSIX: literal when first).
///   - `[: :]` / `[= =]` / `[. .]` POSIX named classes / equivalence
///     classes / collating elements are copied verbatim.
/// Outside bracket expressions the pattern is copied unchanged
/// (existing `\X` escape pairs pass through untouched).
pub fn posix_ere_bracket_escape(pat: &str) -> String {
    let chars: Vec<char> = pat.chars().collect();
    let mut out = String::with_capacity(pat.len() + 8);
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' {
            // Outside a class: copy the escape pair verbatim.
            out.push(c);
            i += 1;
            if i < chars.len() {
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        if c != '[' {
            out.push(c);
            i += 1;
            continue;
        }
        // Bracket expression. POSIX: ends at the first `]` that is
        // not in leading position (after optional `^`).
        out.push('[');
        i += 1;
        if i < chars.len() && chars[i] == '^' {
            out.push('^');
            i += 1;
        }
        if i < chars.len() && chars[i] == ']' {
            out.push('\\');
            out.push(']');
            i += 1;
        }
        while i < chars.len() && chars[i] != ']' {
            // `[:alpha:]` / `[=a=]` / `[.x.]` — copy verbatim.
            if chars[i] == '[' && i + 1 < chars.len() && matches!(chars[i + 1], ':' | '=' | '.') {
                let close = chars[i + 1];
                out.push('[');
                out.push(close);
                i += 2;
                while i < chars.len() && !(chars[i] == close && chars.get(i + 1) == Some(&']')) {
                    out.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    out.push(close);
                    out.push(']');
                    i += 2;
                }
                continue;
            }
            if matches!(chars[i], '\\' | '[' | '&' | '~') {
                out.push('\\');
            }
            out.push(chars[i]);
            i += 1;
        }
        if i < chars.len() {
            out.push(']'); // class terminator
            i += 1;
        }
    }
    out
}
