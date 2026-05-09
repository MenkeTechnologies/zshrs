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

use crate::ported::exec::ShellExecutor;

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
/// inline at regex.c:96-185. Rust port mirrors that writeback by
/// taking `&mut ShellExecutor` so the param-table mutation has
/// the same observable effect.
///
/// `a` is the cond-op argv: `a[0]` is the LHS string, `a[1]` is
/// the RHS pattern (matching C's `cond_str(a, 0, 0)` /
/// `cond_str(a, 1, 0)` reads at regex.c:62-63).
pub fn zcond_regex_match(exec: &mut ShellExecutor, a: &[&str], id: i32) -> i32 {  // c:54
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
    let casematch = exec.options.get("casematch").copied().unwrap_or(true);
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
    let bashre = exec.options.get("bashrematch").copied().unwrap_or(false);
    let ksharr = exec.options.get("ksharrays").copied().unwrap_or(false);

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
        exec.arrays.insert("BASH_REMATCH".to_string(), arr);             // c:116 assignaparam
        return return_value;
    }

    // c:119-121 — assignsparam("MATCH", full-match-text).
    let m0 = captures.get(0).expect("regex matched but no group 0");
    let full = m0.as_str().to_string();                                  // c:120 metafy
    exec.variables.insert("MATCH".to_string(), full);                    // c:121 assignsparam

    // c:124-135 — char-offset MBEGIN. C walks the pre-match bytes
    // counting MB_CHARLEN-stepped characters; Rust collapses to
    // chars().count() over the byte slice up to m->rm_so since
    // String::chars() handles UTF-8 boundaries natively.
    let so = m0.start();
    let eo = m0.end();
    let mbegin_chars = lhstr[..so].chars().count() as i64;               // c:128-133
    let kshoff: i64 = if ksharr { 0 } else { 1 };                        // c:134 !isset(KSHARRAYS)
    let mbegin = mbegin_chars + kshoff;                                  // c:134
    exec.variables.insert("MBEGIN".to_string(), mbegin.to_string());     // c:134 assigniparam

    // c:138-145 — MEND.
    let match_chars = lhstr[so..eo].chars().count() as i64;
    let mend_total = mbegin_chars + match_chars;
    let mend = mend_total + kshoff - 1;                                  // c:145
    exec.variables.insert("MEND".to_string(), mend.to_string());         // c:145 assigniparam

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
        exec.arrays.insert("match".to_string(),  arr);                   // c:182 setaparam
        exec.arrays.insert("mbegin".to_string(), mbegin_arr);            // c:183 setaparam
        exec.arrays.insert("mend".to_string(),   mend_arr);              // c:184 setaparam
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

    fn fresh_exec() -> ShellExecutor { ShellExecutor::new() }

    #[test]
    fn match_writes_match_and_mbegin_mend() {
        let mut e = fresh_exec();
        let r = zcond_regex_match(&mut e, &["hello world", "wor.d"], ZREGEX_EXTENDED);
        assert_eq!(r, 1);
        assert_eq!(e.variables.get("MATCH").map(|s| s.as_str()), Some("world"));
        // 1-based MBEGIN: 'w' is the 7th char.
        assert_eq!(e.variables.get("MBEGIN").map(|s| s.as_str()), Some("7"));
        assert_eq!(e.variables.get("MEND").map(|s| s.as_str()), Some("11"));
    }

    #[test]
    fn captures_populate_match_array() {
        let mut e = fresh_exec();
        let r = zcond_regex_match(&mut e, &["foo=42", "([a-z]+)=([0-9]+)"], ZREGEX_EXTENDED);
        assert_eq!(r, 1);
        let arr = e.arrays.get("match").cloned().unwrap_or_default();
        assert_eq!(arr, vec!["foo".to_string(), "42".to_string()]);
        let mbegin = e.arrays.get("mbegin").cloned().unwrap_or_default();
        let mend = e.arrays.get("mend").cloned().unwrap_or_default();
        assert_eq!(mbegin, vec!["1".to_string(), "5".to_string()]);
        assert_eq!(mend,   vec!["3".to_string(), "6".to_string()]);
    }

    #[test]
    fn no_match_returns_zero_and_writes_nothing() {
        let mut e = fresh_exec();
        let r = zcond_regex_match(&mut e, &["abc", "xyz"], ZREGEX_EXTENDED);
        assert_eq!(r, 0);
        assert!(e.variables.get("MATCH").is_none());
    }

    #[test]
    fn invalid_pattern_returns_zero() {
        let mut e = fresh_exec();
        assert_eq!(
            zcond_regex_match(&mut e, &["anything", "["], ZREGEX_EXTENDED),
            0
        );
    }

    #[test]
    fn missing_args_returns_zero() {
        let mut e = fresh_exec();
        assert_eq!(zcond_regex_match(&mut e, &[], ZREGEX_EXTENDED), 0);
        assert_eq!(zcond_regex_match(&mut e, &["only_lhs"], ZREGEX_EXTENDED), 0);
    }

    #[test]
    fn bashrematch_writes_BASH_REMATCH_array() {
        let mut e = fresh_exec();
        e.options.insert("bashrematch".to_string(), true);
        let r = zcond_regex_match(&mut e, &["foo=42", "([a-z]+)=([0-9]+)"], ZREGEX_EXTENDED);
        assert_eq!(r, 1);
        let arr = e.arrays.get("BASH_REMATCH").cloned().unwrap_or_default();
        // BASH_REMATCH includes element 0 (the full match).
        assert_eq!(arr, vec!["foo=42".to_string(), "foo".to_string(), "42".to_string()]);
    }

    #[test]
    fn ksharrays_makes_offsets_zero_based() {
        let mut e = fresh_exec();
        e.options.insert("ksharrays".to_string(), true);
        let r = zcond_regex_match(&mut e, &["hello world", "wor.d"], ZREGEX_EXTENDED);
        assert_eq!(r, 1);
        assert_eq!(e.variables.get("MBEGIN").map(|s| s.as_str()), Some("6"));
        assert_eq!(e.variables.get("MEND").map(|s| s.as_str()), Some("10"));
    }

    #[test]
    fn casematch_off_is_case_insensitive() {
        let mut e = fresh_exec();
        e.options.insert("casematch".to_string(), false);
        let r = zcond_regex_match(&mut e, &["HELLO", "hello"], ZREGEX_EXTENDED);
        assert_eq!(r, 1);
    }
}

/// Port of `setup_()` from `Src/Modules/regex.c:229`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn setup_() -> i32 {                                                 // c:229
    0                                                                    // c:232
}

/// Port of `features_()` from `Src/Modules/regex.c:236`. C body is
/// `*features = featuresarray(m, &module_features); return 0;`.
/// Static-link path: 0.
pub fn features_() -> i32 {                                              // c:236
    0                                                                    // c:240
}

/// Port of `enables_()` from `Src/Modules/regex.c:244`. C body is
/// `return handlefeatures(m, &module_features, enables);`.
/// Static-link path: 0.
pub fn enables_() -> i32 {                                               // c:244
    0                                                                    // c:247
}

/// Port of `boot_()` from `Src/Modules/regex.c:251`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn boot_() -> i32 {                                                  // c:251
    0                                                                    // c:254
}

/// Port of `cleanup_()` from `Src/Modules/regex.c:258`. C body is
/// `return setfeatureenables(m, &module_features, NULL);`.
pub fn cleanup_() -> i32 {                                               // c:258
    0                                                                    // c:261
}

/// Port of `finish_()` from `Src/Modules/regex.c:265`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn finish_() -> i32 {                                                // c:265
    0                                                                    // c:268
}
