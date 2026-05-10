//! PCRE module - port of Modules/pcre.c
//!
//! Provides PCRE regex matching through pcre_compile, pcre_match, pcre_study builtins.
//! Uses the Rust `regex` crate which provides Perl-compatible regex syntax.

use regex::Regex;
use crate::ported::utils::zwarnnam;
use std::collections::HashMap;

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
    /// `Src/Modules/pcre.c:41`. Compiled regex shared between the
    /// `pcre_compile`/`pcre_study`/`pcre_match` builtins.
    static PCRE_PATTERN: std::cell::RefCell<Option<Regex>> = const {
        std::cell::RefCell::new(None)
    };
}

// WARNING: NOT IN PCRE.C — Rust-only helpers around the
// thread_local PCRE_PATTERN state. C inlines the equivalent
// pcre_pattern reads/writes directly inside `bin_pcre_compile`,
// `bin_pcre_study`, `bin_pcre_match`, and `cond_pcre_match`. The
// Rust port factors them into named fns because all four bin_*
// + cond entry points would otherwise duplicate the
// `with(|r| r.borrow_mut())` / option-flag-to-prefix translation.

/// Internal-only: compile a pattern into the thread_local
/// PCRE_PATTERN slot. Inline analog of the `pcre2_compile_8()` core
/// of `bin_pcre_compile()` (Src/Modules/pcre.c:70).
fn compile_pattern(
    pattern: &str,
    options: &PcreCompileOptions,
) -> Result<(), String> {
    let mut pattern_str = String::new();
    if options.caseless {
        pattern_str.push_str("(?i)");
    }
    if options.multiline {
        pattern_str.push_str("(?m)");
    }
    if options.dotall {
        pattern_str.push_str("(?s)");
    }
    if options.extended {
        pattern_str.push_str("(?x)");
    }
    if options.anchored {
        pattern_str.push('^');
    }
    pattern_str.push_str(pattern);
    match Regex::new(&pattern_str) {
        Ok(re) => {
            PCRE_PATTERN.with(|r| *r.borrow_mut() = Some(re));
            Ok(())
        }
        Err(e) => Err(format!("error in regex: {}", e)),
    }
}

/// Internal-only: query whether a pattern is compiled.
fn has_pattern() -> bool {
    PCRE_PATTERN.with(|r| r.borrow().is_some())
}

/// Internal-only: run the thread_local PCRE_PATTERN against `text`.
/// Inline analog of `pcre2_match_8()` + `zpcre_get_substrings()`
/// inside `bin_pcre_match()` (Src/Modules/pcre.c:328).
fn match_pattern(
    text: &str,
    options: &PcreMatchOptions,
) -> Result<PcreMatchResult, String> {
    let result = PCRE_PATTERN.with(|r| {
        let guard = r.borrow();
        let re = guard
            .as_ref()
            .ok_or_else(|| "no pattern has been compiled".to_string())?;
        let search_text = if options.offset > 0 && options.offset < text.len() {
            &text[options.offset..]
        } else if options.offset >= text.len() {
            return Ok(PcreMatchResult::no_match());
        } else {
            text
        };
        let caps = match re.captures(search_text) {
            Some(c) => c,
            None => return Ok(PcreMatchResult::no_match()),
        };
        let full_match = caps.get(0).map(|m| m.as_str().to_string());
        let match_start = caps.get(0).map(|m| m.start() + options.offset);
        let match_end = caps.get(0).map(|m| m.end() + options.offset);
        let mut captures = Vec::new();
        for i in 1..caps.len() {
            captures.push(caps.get(i).map(|m| m.as_str().to_string()));
        }
        let mut named_captures = HashMap::new();
        for name in re.capture_names().flatten() {
            if let Some(m) = caps.name(name) {
                named_captures.insert(name.to_string(), m.as_str().to_string());
            }
        }
        Ok(PcreMatchResult {
            matched: true,
            full_match,
            captures,
            named_captures,
            match_start,
            match_end,
        })
    });
    result
}

/// Options for pcre_compile
#[derive(Debug, Default, Clone)]
pub struct PcreCompileOptions {
    pub anchored: bool,
    pub caseless: bool,
    pub multiline: bool,
    pub extended: bool,
    pub dotall: bool,
}

/// Options for pcre_match
#[derive(Debug, Default, Clone)]
pub struct PcreMatchOptions {
    pub match_var: Option<String>,
    pub array_var: Option<String>,
    pub assoc_var: Option<String>,
    pub offset: usize,
    pub return_offsets: bool,
    pub use_dfa: bool,
}

/// Result of a PCRE match
#[derive(Debug, Clone)]
pub struct PcreMatchResult {
    pub matched: bool,
    pub full_match: Option<String>,
    pub captures: Vec<Option<String>>,
    pub named_captures: HashMap<String, String>,
    pub match_start: Option<usize>,
    pub match_end: Option<usize>,
}

impl PcreMatchResult {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/pcre.c`.
    pub fn no_match() -> Self {
        Self {
            matched: false,
            full_match: None,
            captures: Vec::new(),
            named_captures: HashMap::new(),
            match_start: None,
            match_end: None,
        }
    }
}

/// Port of `cond_pcre_match()` from `Src/Modules/pcre.c:422`. The
/// `-pcre-match` operator dispatch hook the lexer wires for `[[ s
/// -pcre-match pat ]]`. Compiles `rhs` on the fly (overwriting the
/// thread_local PCRE_PATTERN) and returns `(matched, result)` so
/// the caller can install match-var side effects.
pub fn cond_pcre_match(lhs: &str, rhs: &str, caseless: bool) -> (bool, PcreMatchResult) {
    let options = PcreCompileOptions {                                          // c:422
        caseless,                                                               // c:422
        ..Default::default()                                                    // c:422
    };
    if compile_pattern(rhs, &options).is_err() {                                // c:422
        return (false, PcreMatchResult::no_match());
    }
    let match_options = PcreMatchOptions::default();
    match match_pattern(lhs, &match_options) {                                  // c:422
        Ok(result) => (result.matched, result),
        Err(_) => (false, PcreMatchResult::no_match()),
    }
}

/// Port of `bin_pcre_compile()` from `Src/Modules/pcre.c:70`.
///
/// Mirrors C — reads positional args, validates non-empty, compiles
/// into the file-static (thread_local in zshrs) PCRE_PATTERN. C
/// signature has no state arg; the file-static is the implicit
/// state. Returns (status, message).
pub fn bin_pcre_compile(
    args: &[&str],
    options: &PcreCompileOptions,
) -> (i32, String) {
    if args.is_empty() {
        return (1, "pcre_compile: pattern required\n".to_string());
    }
    match compile_pattern(args[0], options) {
        Ok(()) => (0, String::new()),
        Err(e) => (1, format!("pcre_compile: {}\n", e)),
    }
}

/// Port of `bin_pcre_study()` from `Src/Modules/pcre.c:112`. The C
/// source calls `pcre2_jit_compile()` to JIT-optimize the compiled
/// pattern; the Rust `regex` crate already builds an optimal NFA
/// at compile time, so this is the "no pattern" guard the C source
/// also returns and nothing else.
pub fn bin_pcre_study() -> (i32, String) {
    if !has_pattern() {
        return (
            1,
            "pcre_study: no pattern has been compiled for study\n".to_string(),
        );
    }
    (0, String::new())
}

/// Port of `bin_pcre_match()` from `Src/Modules/pcre.c:328`. Runs
/// the file-static (thread_local in zshrs) PCRE_PATTERN against
/// `args[0]`. C's "1 on no-match, 0 on match" exit-status convention
/// preserved.
pub fn bin_pcre_match(
    args: &[&str],
    options: &PcreMatchOptions,
) -> (i32, PcreMatchResult) {
    if args.is_empty() {
        return (1, PcreMatchResult::no_match());
    }
    match match_pattern(args[0], options) {
        Ok(result) => {
            if result.matched {
                (0, result)
            } else {
                (1, result)
            }
        }
        Err(_) => (1, PcreMatchResult::no_match()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pcre_initial_no_pattern() {
        assert!(!has_pattern());
    }

    #[test]
    fn test_pcre_compile_simple() {
        let options = PcreCompileOptions::default();

        let result = compile_pattern("hello", &options);
        assert!(result.is_ok());
        assert!(has_pattern());
    }

    #[test]
    fn test_pcre_compile_invalid() {
        let options = PcreCompileOptions::default();

        let result = compile_pattern("[invalid", &options);
        assert!(result.is_err());
    }

    #[test]
    fn test_pcre_compile_caseless() {
        let options = PcreCompileOptions {
            caseless: true,
            ..Default::default()
        };

        let result = compile_pattern("hello", &options);
        assert!(result.is_ok());

        let match_opts = PcreMatchOptions::default();
        let result = match_pattern("HELLO WORLD", &match_opts).unwrap();
        assert!(result.matched);
    }

    #[test]
    fn test_pcre_study_no_pattern() {
        let (status, _) = bin_pcre_study();
        let result: Result<(), &str> = if status == 0 { Ok(()) } else { Err("err") };
        assert!(result.is_err());
    }

    #[test]
    fn test_pcre_study_with_pattern() {
        let options = PcreCompileOptions::default();
        compile_pattern("hello", &options).unwrap();

        let (status, _) = bin_pcre_study();
        let result: Result<(), &str> = if status == 0 { Ok(()) } else { Err("err") };
        assert!(result.is_ok());
    }

    #[test]
    fn test_pcre_match_simple() {
        let options = PcreCompileOptions::default();
        compile_pattern("hello", &options).unwrap();

        let match_opts = PcreMatchOptions::default();
        let result = match_pattern("hello world", &match_opts).unwrap();
        assert!(result.matched);
        assert_eq!(result.full_match, Some("hello".to_string()));
    }

    #[test]
    fn test_pcre_match_no_match() {
        let options = PcreCompileOptions::default();
        compile_pattern("hello", &options).unwrap();

        let match_opts = PcreMatchOptions::default();
        let result = match_pattern("goodbye world", &match_opts).unwrap();
        assert!(!result.matched);
    }

    #[test]
    fn test_pcre_match_captures() {
        let options = PcreCompileOptions::default();
        compile_pattern(r"(\w+) (\w+)", &options).unwrap();

        let match_opts = PcreMatchOptions::default();
        let result = match_pattern("hello world", &match_opts).unwrap();
        assert!(result.matched);
        assert_eq!(result.captures.len(), 2);
        assert_eq!(result.captures[0], Some("hello".to_string()));
        assert_eq!(result.captures[1], Some("world".to_string()));
    }

    #[test]
    fn test_pcre_match_named_captures() {
        let options = PcreCompileOptions::default();
        compile_pattern(r"(?P<first>\w+) (?P<second>\w+)", &options).unwrap();

        let match_opts = PcreMatchOptions::default();
        let result = match_pattern("hello world", &match_opts).unwrap();
        assert!(result.matched);
        assert_eq!(
            result.named_captures.get("first"),
            Some(&"hello".to_string())
        );
        assert_eq!(
            result.named_captures.get("second"),
            Some(&"world".to_string())
        );
    }

    #[test]
    fn test_pcre_match_with_offset() {
        let options = PcreCompileOptions::default();
        compile_pattern("world", &options).unwrap();

        let match_opts = PcreMatchOptions {
            offset: 6,
            ..Default::default()
        };
        let result = match_pattern("hello world", &match_opts).unwrap();
        assert!(result.matched);
        assert_eq!(result.match_start, Some(6));
    }

    #[test]
    fn test_cond_pcre_match() {
        let (matched, _) = cond_pcre_match("hello world", "hello", false);
        assert!(matched);

        let (matched, _) = cond_pcre_match("hello world", "HELLO", true);
        assert!(matched);

        let (matched, _) = cond_pcre_match("hello world", "HELLO", false);
        assert!(!matched);
    }

    #[test]
    fn test_builtin_pcre_compile_no_args() {
        let options = PcreCompileOptions::default();
        let (status, _) = bin_pcre_compile(&[], &options);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_pcre_match_no_pattern() {
        let options = PcreMatchOptions::default();
        let (status, _) = bin_pcre_match(&["test"], &options);
        assert_eq!(status, 1);
    }
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

use std::sync::{Mutex, OnceLock};
use crate::ported::zsh_h::{features as features_t, module};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None, bn_size: 3,                                       // bintab[3]: pcre_compile, pcre_match, pcre_study
        cd_list: None, cd_size: 1,                                       // cotab[1]: pcre-match
        mf_list: None, mf_size: 0,
        pd_list: None, pd_size: 0,
        n_abstract: 0,
    }))
}

/// Port of `setup_()` from `Src/Modules/pcre.c:542`.
pub fn setup_(_m: *const module) -> i32 { 0 }

/// Port of `features_()` from `Src/Modules/pcre.c:549`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_()` from `Src/Modules/pcre.c:557`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_()` from `Src/Modules/pcre.c:564`.
pub fn boot_(_m: *const module) -> i32 { 0 }

/// Port of `cleanup_()` from `Src/Modules/pcre.c:571`.
pub fn cleanup_(m: *const module) -> i32 {
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_()` from `Src/Modules/pcre.c:578`.
pub fn finish_(_m: *const module) -> i32 { 0 }

fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["b:pcre_compile".to_string(), "b:pcre_match".to_string(),
         "b:pcre_study".to_string(), "C:pcre-match".to_string()]
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

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/pcre.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `getposint()` from Src/Modules/pcre.c:312.
/// C: `static int getposint(char *instr, char *nam)` — parse positive
/// decimal integer; emit "integer expected" warning + return -1 on bad input.
#[allow(non_snake_case)]
pub fn getposint(instr: &str, nam: &str) -> i32 {                            // c:312
    // c:317 — `ret = (int)zstrtol(instr, &eptr, 10);`
    match instr.trim().parse::<i32>() {                                      // c:317
        Ok(n) if n >= 0 => n,                                                // c:323
        _ => {
            // c:319-321 — zwarnnam(nam, "integer expected: %s", instr);
            crate::ported::utils::zwarnnam(nam, &format!("integer expected: {}", instr)); // c:320
            -1                                                               // c:321
        }
    }
}

/// Port of `pcre_callout()` from Src/Modules/pcre.c:132.
/// C: `static int pcre_callout(pcre2_callout_block_8 *block,
///     UNUSED(void *callout_data))` — eval the callout string as zsh code,
///     bind .pcre.subject and .pcre.pos parameters, return $? | errflag.
#[allow(non_snake_case)]
pub fn pcre_callout(_block: *mut std::ffi::c_void,                           // c:132
                    _callout_data: *mut std::ffi::c_void) -> i32 {
    // c:138-152 — parse_string(callout_string), setsparam(".pcre.subject"),
    // setiparam(".pcre.pos"), execode(prog, ..., "pcre"), return lastval|errflag.
    // Static-link path: zshrs's pcre integration uses the `regex` crate
    // directly; native pcre callouts arrive only when the C pcre2 backend
    // is wired in. Until then return success-no-callout.
    0                                                                        // c:155
}

/// Port of `zpcre_get_substrings()` from Src/Modules/pcre.c:157.
/// C: `static int zpcre_get_substrings(pcre2_code *pat, char *arg,
///     pcre2_match_data *mdata, int captured_count, char *matchvar,
///     char *substravar, char *namedassoc, int want_offset_pair,
///     int matchedinarr, int want_begin_end)` — extract submatches
///     into shell parameters.
#[allow(non_snake_case)]
pub fn zpcre_get_substrings(_pat: *mut std::ffi::c_void, _arg: &str,         // c:157
                            _mdata: *mut std::ffi::c_void,
                            _captured_count: i32,
                            _matchvar: Option<&str>, _substravar: Option<&str>,
                            _namedassoc: Option<&str>,
                            _want_offset_pair: i32, _matchedinarr: i32,
                            _want_begin_end: i32) -> i32 {
    // c:170-310 — pcre2_get_ovector_pointer + setsparam("ZPCRE_OP"/match/etc).
    // Static-link path: implementation lives in the regex-backed bin_pcre_match
    // dispatcher; this stub is reserved for the future native backend.
    0
}

/// Port of `zpcre_utf8_enabled()` from Src/Modules/pcre.c:45.
/// C: `static int zpcre_utf8_enabled(void)` — returns 1 iff PCRE2 was
/// built with Unicode AND MULTIBYTE option is set AND nl_langinfo(CODESET)
/// reports "UTF-8".
#[allow(non_snake_case)]
pub fn zpcre_utf8_enabled() -> i32 {                                         // c:45
    // c:48-67 — under MULTIBYTE_SUPPORT && HAVE_NL_LANGINFO && CODESET.
    // Static-link path: zshrs hosts on macOS/Linux where PCRE2 ships with
    // Unicode by default; check MULTIBYTE option + LANG/LC_ALL CODESET.
    let multibyte = crate::ported::options::optlookup("multibyte") > 0;      // c:53
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
