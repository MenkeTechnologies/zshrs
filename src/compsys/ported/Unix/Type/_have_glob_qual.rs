//! Port of `_have_glob_qual` from `Completion/Unix/Type/_have_glob_qual`.
//!
//! Full upstream body (24 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh:    # Test if $1 has glob qualifiers … sets match/mbegin/mend.
//! sh:    # $2 == "complete" tests the qualifiers reach the ")".
//! sh:13  local complete
//! sh:15  [[ $2 = complete ]] && complete=")"
//! sh:17  [[ -z $compstate[quote] &&
//! sh:18    ( $_comp_caller_options[bareglobqual] == on &&
//! sh:19        $1 = (#b)(((*[^\\\$]|)(\\\\)#)\()([^\)\|\~]#)$complete &&
//! sh:20        ${#match[1]} -gt 1 ||
//! sh:21      $_comp_caller_options[extendedglob] == on &&
//! sh:22        $1 = (#b)(((*[^\\\$]|)(\\\\)#)"(#q")([^\)]#)$complete
//! sh:23    ) ]]
//! ```
//!
//! sh:19/22 approx — the two `(#b)` backreference patterns are matched
//! with a hand scanner (the pattern engine has no backref capture that
//! sets `$match`). Semantics preserved: pattern A locates the rightmost
//! UNESCAPED `(` opening bare glob qualifiers (even backslash run
//! before it, preceding char not `$`); pattern B locates `(#q`. In both
//! `match[1]` is the text up to and including the opener, `match[2]` the
//! text before it, and `match[5]` the qualifier body (`[^)|~]*` for A,
//! `[^)]*` for B). Return status is the `[[ … ]]` result (0 = has quals).

use crate::ported::params::{getaparam, gethkparam, gethparam, setaparam};

/// `$name[key]` for an associative parameter.
///
/// `_comp_caller_options` is a real PM_HASHED param (`_main_complete`
/// publishes it with `sethparam`), and `getaparam` returns `None` for
/// anything that is not PM_ARRAY — so the hash MUST be read through
/// `gethkparam`/`gethparam` (c:params.c:3117/3131). The flat
/// key/value-array fallback is kept for callers that stage the assoc
/// with `setaparam` (the unit tests below, and ports that build a
/// scratch array).
fn assoc_get(name: &str, key: &str) -> Option<String> {
    let keys = gethkparam(name).unwrap_or_default();
    if !keys.is_empty() {
        let vals = gethparam(name).unwrap_or_default();
        return keys
            .iter()
            .position(|k| k == key)
            .and_then(|i| vals.get(i).cloned());
    }
    getaparam(name)?
        .chunks(2)
        .find(|kv| kv.first().map(|k| k == key).unwrap_or(false))
        .and_then(|kv| kv.get(1).cloned())
}

/// True when the `(` at `chars[i]` is unescaped: an even run of `\`
/// immediately precedes it and the char before that run is not `$`
/// (the `(*[^\\\$]|)(\\\\)#` prefix constraint).
fn unescaped_open(chars: &[char], i: usize) -> bool {
    let mut j = i;
    let mut k = 0usize;
    while j > 0 && chars[j - 1] == '\\' {
        k += 1;
        j -= 1;
    }
    if k % 2 != 0 {
        return false;
    }
    !(j > 0 && chars[j - 1] == '$')
}

/// Publish `match` (1-indexed groups 1..5) and matching `mbegin`/`mend`
/// (1-based char offsets; 0 for empty groups).
fn set_match(m1: &str, m2: &str, m5: &str, open_len: usize) {
    // group1 spans [1, len(m1)]; group2 [1, len(m2)]; group5 starts just
    // after the opener. groups 3/4 (inner prefix / backslash pairs) are
    // not reconstructed — left empty, as no caller in-tree reads them.
    let l1 = m1.chars().count();
    let l2 = m2.chars().count();
    let l5 = m5.chars().count();
    setaparam(
        "match",
        vec![
            m1.to_string(),
            m2.to_string(),
            String::new(),
            String::new(),
            m5.to_string(),
        ],
    );
    let g5_begin = if l5 == 0 { 0 } else { l2 + open_len + 1 };
    let g5_end = if l5 == 0 { 0 } else { l2 + open_len + l5 };
    setaparam(
        "mbegin",
        ["1", "1", "0", "0", &g5_begin.to_string()]
            .iter()
            .map(|s| s.to_string())
            .collect(),
    );
    setaparam(
        "mend",
        [
            l1.to_string(),
            l2.to_string(),
            "0".to_string(),
            "0".to_string(),
            g5_end.to_string(),
        ]
        .to_vec(),
    );
}

/// `_have_glob_qual` — predicate: does `$1` carry glob qualifiers?
/// Returns 0 when it does (and sets `$match`), 1 otherwise.
pub fn _have_glob_qual(args: &[String]) -> i32 {
    let s = args.first().cloned().unwrap_or_default();
    let complete = args.get(1).map(|x| x == "complete").unwrap_or(false);

    // sh:17 — inside a quote, never treat as glob qualifiers.
    if let Some(q) = assoc_get("compstate", "quote") {
        if !q.is_empty() {
            return 1;
        }
    }
    let bareglob = assoc_get("_comp_caller_options", "bareglobqual").as_deref() == Some("on");
    let extglob = assoc_get("_comp_caller_options", "extendedglob").as_deref() == Some("on");
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();

    // helper: split the tail after an opener into (body, ok) honouring
    // the anchored `$complete` (`)` or empty).
    let tail_body = |tail: &[char]| -> (Vec<char>, bool) {
        if complete {
            if tail.last() == Some(&')') {
                (tail[..tail.len() - 1].to_vec(), true)
            } else {
                (Vec::new(), false)
            }
        } else {
            (tail.to_vec(), true)
        }
    };

    // sh:18-20 — pattern A (bare glob qualifiers), rightmost unescaped `(`.
    if bareglob {
        for i in (1..n).rev() {
            if chars[i] != '(' || !unescaped_open(&chars, i) {
                continue;
            }
            let (body, ok) = tail_body(&chars[i + 1..]);
            if !ok || !body.iter().all(|c| *c != ')' && *c != '|' && *c != '~') {
                continue;
            }
            // ${#match[1]} > 1 — a prefix before `(` must exist (i >= 1).
            let m1: String = chars[..=i].iter().collect();
            if m1.chars().count() > 1 {
                let m2: String = chars[..i].iter().collect();
                let m5: String = body.iter().collect();
                set_match(&m1, &m2, &m5, 1);
                return 0;
            }
        }
    }

    // sh:21-22 — pattern B (`(#q` extended glob), rightmost unescaped `(`.
    if extglob {
        for i in (0..n).rev() {
            if chars[i] != '('
                || i + 2 >= n
                || chars[i + 1] != '#'
                || chars[i + 2] != 'q'
                || !unescaped_open(&chars, i)
            {
                continue;
            }
            let (body, ok) = tail_body(&chars[i + 3..]);
            if !ok || !body.iter().all(|c| *c != ')') {
                continue;
            }
            let m1: String = chars[..i + 3].iter().collect();
            let m2: String = chars[..i].iter().collect();
            let m5: String = body.iter().collect();
            set_match(&m1, &m2, &m5, 3);
            return 0;
        }
    }

    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::{getaparam, setaparam};

    fn set_caller_opt(bare: bool, ext: bool) {
        let mut v = Vec::new();
        v.push("bareglobqual".to_string());
        v.push(if bare { "on" } else { "off" }.to_string());
        v.push("extendedglob".to_string());
        v.push(if ext { "on" } else { "off" }.to_string());
        setaparam("_comp_caller_options", v);
        setaparam("compstate", vec!["quote".to_string(), String::new()]);
    }

    #[test]
    fn bare_qualifier_detected_and_match_set() {
        let _g = crate::test_util::global_state_lock();
        set_caller_opt(true, false);
        // `*(-/` — opener at index 1, body "-/".
        assert_eq!(_have_glob_qual(&["*(-/".to_string()]), 0);
        let m = getaparam("match").unwrap();
        assert_eq!(m[0], "*(");
        assert_eq!(m[1], "*");
        assert_eq!(m[4], "-/");
    }

    #[test]
    fn escaped_paren_is_not_a_qualifier() {
        let _g = crate::test_util::global_state_lock();
        set_caller_opt(true, false);
        // `a\(b` — the `(` is escaped (odd backslash run) → no qualifiers.
        assert_eq!(_have_glob_qual(&["a\\(b".to_string()]), 1);
    }

    #[test]
    fn extendedglob_hashq_detected() {
        let _g = crate::test_util::global_state_lock();
        set_caller_opt(false, true);
        assert_eq!(_have_glob_qual(&["*(#q-.".to_string()]), 0);
        let m = getaparam("match").unwrap();
        assert_eq!(m[0], "*(#q");
        assert_eq!(m[4], "-.");
    }

    #[test]
    fn no_options_returns_one() {
        let _g = crate::test_util::global_state_lock();
        set_caller_opt(false, false);
        assert_eq!(_have_glob_qual(&["*(-/".to_string()]), 1);
    }

    /// sh:18-19 — `$_comp_caller_options[bareglobqual]` must be read from
    /// the REAL associative parameter. `_main_complete` publishes it with
    /// `sethparam` (PM_HASHED), and `getaparam` only ever yields PM_ARRAY
    /// values, so a flat-array-only lookup silently saw `off` for every
    /// option: `_have_glob_qual` returned 1 for every word and `ls *(<TAB>`
    /// offered zero glob qualifiers instead of zsh's 47.
    #[test]
    fn caller_options_read_from_a_real_hash() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::unsetparam("_comp_caller_options");
        let _ = crate::ported::params::sethparam(
            "_comp_caller_options",
            vec![
                "bareglobqual".to_string(),
                "on".to_string(),
                "extendedglob".to_string(),
                "off".to_string(),
            ],
        );
        setaparam("compstate", vec!["quote".to_string(), String::new()]);
        assert_eq!(
            _have_glob_qual(&["*(-/".to_string()]),
            0,
            "bareglobqual=on in a PM_HASHED _comp_caller_options must be seen"
        );
        let m = getaparam("match").unwrap();
        assert_eq!(m[4], "-/");
        let _ = crate::ported::params::unsetparam("_comp_caller_options");
    }
}
