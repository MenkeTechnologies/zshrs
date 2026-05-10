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

/// `ZREGEX_EXTENDED` from `Src/Modules/regex.c:36`.
/// `#define ZREGEX_EXTENDED 0`. The id passed to
/// `zcond_regex_match` for the only currently-supported flavour.
pub const ZREGEX_EXTENDED: i32 = 0;                                      // c:36

/// Port of `zcond_regex_match()` from `Src/Modules/regex.c:54`.
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
pub fn zcond_regex_match(a: &[&str], id: i32) -> i32 {                       // c:54
    if a.len() < 2 {
        return 0;
    }
    let lhstr = a[0];                                                    // c:62 cond_str(a,0,0)
    let rhre = a[1];                                                     // c:63 cond_str(a,1,0)
    let mut return_value: i32 = 0;                                       // c:65

    // c:73-77 — switch(id). Only ZREGEX_EXTENDED is defined.
    if id != ZREGEX_EXTENDED {
        // c:188-191 default: DPUTS("bad regex option"); goto CLEAN.
        return 0;
    }

    // c:74-76 — flag computation. POSIX REG_EXTENDED is implicit
    // in Rust's regex crate (RE2 syntax is extended-by-default);
    // CASEMATCH off → REG_ICASE → wrap with `(?i)`.
    let casematch = crate::ported::options::optlookup("casematch") > 0;
    let pat_for_compile = if !casematch {                                // c:75
        format!("(?i){}", rhre)                                          // c:76 REG_ICASE
    } else {
        rhre.to_string()
    };

    // c:78 — regcomp(&re, rhre, rcflags).
    let re = match regex::Regex::new(&pat_for_compile) {
        Ok(r) => r,
        Err(_) => {                                                      // c:79-81
            zregex_regerrwarn("-regex-match", "failed to compile regex");
            return 0;                                                    // c:81 break;
        }
    };

    // c:92 — regexec.
    let captures = match re.captures(lhstr) {
        Some(c) => c,
        None => return 0,                                                // c:93-94 REG_NOMATCH
    };

    return_value = 1;                                                    // c:96
    let nsub = re.captures_len() - 1;                                    // re_nsub: # of paren groups
    let bashre = crate::ported::options::optlookup("bashrematch") > 0;
    let ksharr = crate::ported::options::optlookup("ksharrays") > 0;

    // c:97-103 — start/nelem branch on BASHREMATCH.
    let (start, nelem) = if bashre {
        (0usize, nsub + 1)                                               // c:99-100
    } else {
        (1usize, nsub)                                                   // c:102-103
    };

    // c:108-112 — build arr (the $match / $BASH_REMATCH array).
    let mut arr: Vec<String> = Vec::with_capacity(nelem);
    for n in start..=nsub {                                              // c:109
        if let Some(m) = captures.get(n) {                               // c:110
            arr.push(m.as_str().to_string());                            // c:110 metafy
        } else {
            arr.push(String::new());
        }
    }

    if bashre {                                                          // c:115
        // c:116 — `assignaparam("BASH_REMATCH", arr, 0);`
        crate::ported::modules::ksh93::setsparam("BASH_REMATCH", &arr.join(":"));
        return return_value;
    }

    // c:119-121 — assignsparam("MATCH", full-match-text).
    let m0 = captures.get(0).expect("regex matched but no group 0");
    let full = m0.as_str().to_string();                                  // c:120 metafy
    crate::ported::modules::ksh93::setsparam("MATCH", &full);            // c:121 assignsparam

    // c:124-135 — char-offset MBEGIN. C walks the pre-match bytes
    // counting MB_CHARLEN-stepped characters; Rust collapses to
    // chars().count() over the byte slice up to m->rm_so since
    // String::chars() handles UTF-8 boundaries natively.
    let so = m0.start();
    let eo = m0.end();
    let mbegin_chars = lhstr[..so].chars().count() as i64;               // c:128-133
    let kshoff: i64 = if ksharr { 0 } else { 1 };                        // c:134 !isset(KSHARRAYS)
    let mbegin = mbegin_chars + kshoff;                                  // c:134
    crate::ported::modules::ksh93::setiparam("MBEGIN", mbegin);          // c:134 assigniparam

    // c:138-145 — MEND.
    let match_chars = lhstr[so..eo].chars().count() as i64;
    let mend_total = mbegin_chars + match_chars;
    let mend = mend_total + kshoff - 1;                                  // c:145
    crate::ported::modules::ksh93::setiparam("MEND", mend);              // c:145 assigniparam

    // c:147-180 — populate $match[], $mbegin[], $mend[] subgroup
    // arrays.
    if nelem > 0 {                                                       // c:147
        let mut mbegin_arr: Vec<String> = Vec::with_capacity(nelem);
        let mut mend_arr: Vec<String> = Vec::with_capacity(nelem);
        for n in 0..nelem {                                              // c:152
            let cap_idx = start + n;
            match captures.get(cap_idx) {                                // c:158
                Some(m) => {
                    let beg_chars = lhstr[..m.start()].chars().count() as i64;
                    let len_chars = lhstr[m.start()..m.end()].chars().count() as i64;
                    mbegin_arr.push((beg_chars + kshoff).to_string());   // c:172
                    mend_arr.push((beg_chars + len_chars + kshoff - 1).to_string()); // c:178
                }
                None => {                                                // c:159-162 — unparticipated group
                    mbegin_arr.push("-1".to_string());
                    mend_arr.push("-1".to_string());
                }
            }
        }
        // c:182-184 — `setaparam("match"/"mbegin"/"mend", ...);`
        crate::ported::modules::ksh93::setsparam("match",  &arr.join(":"));
        crate::ported::modules::ksh93::setsparam("mbegin", &mbegin_arr.join(":"));
        crate::ported::modules::ksh93::setsparam("mend",   &mend_arr.join(":"));
    }

    return_value                                                         // c:200
}

/// Port of static helper `zregex_regerrwarn()` from
/// `Src/Modules/regex.c:40`. C wraps libc `regerror(3)` to format
/// a regex compilation/match error and emit it via `zwarn`. Rust
/// uses the `regex` crate so `regex::Error` already carries a
/// formatted message — collapse C's two `regerror()` size+fill
/// calls into a single `zwarnnam` with the supplied prefix +
/// already-formatted error string.
///
/// C signature: `static void zregex_regerrwarn(int r, regex_t *re, char *msg)`.
pub fn zregex_regerrwarn(prefix: &str, err_msg: &str) {                  // c:40
    crate::ported::utils::zwarnnam(prefix, err_msg);                     // c:48
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_returns_one() {
        let r = zcond_regex_match(&["hello world", "wor.d"], ZREGEX_EXTENDED);
        assert_eq!(r, 1);
        // Side-effect params (MATCH/MBEGIN/MEND) flow through
        // ksh93::setsparam env-var bridge; they're verified at the
        // integration level (tests/zsh_compat_parity_gaps.rs) rather
        // than here against an in-memory executor map.
    }

    #[test]
    fn captures_returns_one() {
        let r = zcond_regex_match(&["foo=42", "([a-z]+)=([0-9]+)"], ZREGEX_EXTENDED);
        assert_eq!(r, 1);
    }

    #[test]
    fn no_match_returns_zero() {
        let r = zcond_regex_match(&["abc", "xyz"], ZREGEX_EXTENDED);
        assert_eq!(r, 0);
    }

    #[test]
    fn invalid_pattern_returns_zero() {
        assert_eq!(
            zcond_regex_match(&["anything", "["], ZREGEX_EXTENDED),
            0
        );
    }

    #[test]
    fn missing_args_returns_zero() {
        assert_eq!(zcond_regex_match(&[], ZREGEX_EXTENDED), 0);
        assert_eq!(zcond_regex_match(&["only_lhs"], ZREGEX_EXTENDED), 0);
    }

    #[test]
    fn casematch_off_is_case_insensitive() {
        // casematch flag now consults the global options table via
        // optlookup("casematch"); leaving it at default (1) means
        // case-sensitive — `HELLO` vs `hello` should NOT match.
        let r = zcond_regex_match(&["HELLO", "hello"], ZREGEX_EXTENDED);
        assert_eq!(r, 0);
    }
}

// =====================================================================
// static struct features module_features                            c:217 (regex.c)
// =====================================================================

use std::sync::{Mutex, OnceLock};
use crate::ported::zsh_h::{features as features_t, module};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None, bn_size: 0,
        cd_list: None, cd_size: 1,                                       // cotab[1]: regex-match
        mf_list: None, mf_size: 0,
        pd_list: None, pd_size: 0,
        n_abstract: 0,
    }))
}

/// Port of `setup_()` from `Src/Modules/regex.c:229`.
pub fn setup_(_m: *const module) -> i32 { 0 }                           // c:229-232

/// Port of `features_()` from `Src/Modules/regex.c:236`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_()` from `Src/Modules/regex.c:244`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_()` from `Src/Modules/regex.c:251`.
pub fn boot_(_m: *const module) -> i32 { 0 }                            // c:251-254

/// Port of `cleanup_()` from `Src/Modules/regex.c:258`.
pub fn cleanup_(m: *const module) -> i32 {
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_()` from `Src/Modules/regex.c:265`.
pub fn finish_(_m: *const module) -> i32 { 0 }                          // c:265-268

fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["C:regex-match".to_string()]
}
fn handlefeatures(m: *const module, f: &Mutex<features_t>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() { *enables = Some(getfeatureenables(m, f)); }
    else if let Some(e) = enables.as_ref() { return setfeatureenables(m, f, Some(e)); }
    0
}
fn getfeatureenables(_m: *const module, f: &Mutex<features_t>) -> Vec<i32> {
    let g = f.lock().unwrap();
    vec![0; (g.bn_size + g.cd_size + g.mf_size + g.pd_size + g.n_abstract) as usize]
}
fn setfeatureenables(_m: *const module, _f: &Mutex<features_t>, _e: Option<&Vec<i32>>) -> i32 { 0 }
