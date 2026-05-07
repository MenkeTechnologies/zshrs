//! PCRE module - port of Modules/pcre.c
//!
//! Provides PCRE regex matching through pcre_compile, pcre_match, pcre_study builtins.
//! Uses the Rust `regex` crate which provides Perl-compatible regex syntax.

use regex::Regex;
use std::collections::HashMap;

/// Compiled PCRE pattern state.
/// Port of the file-static `pcre_pattern` / `pcre_extra` /
/// `pcre_hints` slot Src/Modules/pcre.c keeps to share a compiled
/// regex between `bin_pcre_compile` (line 70), `bin_pcre_study`
/// (line 112), and `bin_pcre_match` (line 328). C zsh's source uses
/// PCRE2's `pcre2_code *`; the Rust `regex` crate gives us an
/// equivalent compiled handle.
#[derive(Debug)]
pub struct PcreState {
    pattern: Option<Regex>,
    pattern_str: Option<String>,
}

impl Default for PcreState {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/pcre.c`.
    fn default() -> Self {
        Self::new()
    }
}

impl PcreState {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/pcre.c`.
    pub fn new() -> Self {
        Self {
            pattern: None,
            pattern_str: None,
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/pcre.c`.
    pub fn has_pattern(&self) -> bool {
        self.pattern.is_some()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/pcre.c`.
    pub fn clear(&mut self) {
        self.pattern = None;
        self.pattern_str = None;
    }
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

/// Compile a PCRE pattern.
/// Port of the `pcre2_compile_8()` core inside `bin_pcre_compile()`
/// from Src/Modules/pcre.c:70 — translates the option flag bag
/// (`-i` caseless, `-x` extended, `-m` multiline, `-s` dotall,
/// `-a` anchored) into the `(?i)` / `(?x)` / `(?m)` / `(?s)` /
/// `^` prefixes the Rust `regex` crate accepts and stores the
/// compiled handle in `state` for later `pcre_match`/`pcre_study`.
pub fn pcre_compile(
    pattern: &str,
    options: &PcreCompileOptions,
    state: &mut PcreState,
) -> Result<(), String> {
    state.clear();

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
            state.pattern = Some(re);
            state.pattern_str = Some(pattern_str);
            Ok(())
        }
        Err(e) => Err(format!("error in regex: {}", e)),
    }
}

/// Study a compiled pattern (helper for `bin_pcre_study` —
/// Src/Modules/pcre.c:112). The C source calls
/// `pcre2_jit_compile()` to JIT-optimize the compiled pattern;
/// the Rust `regex` crate already builds an optimal NFA at
/// compile time, so this is a no-op other than the "no pattern"
/// guard the C source also returns.
pub fn pcre_study(state: &PcreState) -> Result<(), String> {
    if state.pattern.is_none() {
        return Err("no pattern has been compiled for study".to_string());
    }
    Ok(())
}

/// Match a string against the compiled pattern.
/// Port of the `pcre2_match_8()` + `zpcre_get_substrings()` core of
/// `bin_pcre_match()` from Src/Modules/pcre.c:328 — runs the match,
/// captures numbered groups (the `ovector` walk in
/// `zpcre_get_substrings()` at line 157), and surfaces named
/// captures via the same `pcre2_substring_get_byname` lookup the C
/// source performs.
pub fn pcre_match(
    text: &str,
    options: &PcreMatchOptions,
    state: &PcreState,
) -> Result<PcreMatchResult, String> {
    let re = state
        .pattern
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
}

/// `[[ s -pcre-match pat ]]` cond-test entry point.
/// Port of `cond_pcre_match()` from Src/Modules/pcre.c:422 — the
/// dispatch hook the lexer wires for the `-pcre-match` operator.
/// Compiles `rhs` on the fly (no shared compile state required) and
/// returns `(matched, result)` so the caller can decide whether to
/// install the match-vars side effects.
pub fn cond_pcre_match(lhs: &str, rhs: &str, caseless: bool) -> (bool, PcreMatchResult) {
    let options = PcreCompileOptions {                                          // c:422
        caseless,                                                               // c:422
        ..Default::default()                                                    // c:422
    };                                                                          // c:422

    let mut state = PcreState::new();                                           // c:422

    if pcre_compile(rhs, &options, &mut state).is_err() {                       // c:422
        return (false, PcreMatchResult::no_match());                            // c:422
    }                                                                           // c:422

    let match_options = PcreMatchOptions::default();                            // c:422

    match pcre_match(lhs, &match_options, &state) {                             // c:422
        Ok(result) => (result.matched, result),                                 // c:422
        Err(_) => (false, PcreMatchResult::no_match()),                         // c:422
    }                                                                           // c:422
}

/// `pcre_compile` builtin entry point.
/// Port of `bin_pcre_compile()` from Src/Modules/pcre.c:70 — wraps
/// `pcre_compile()` with the same "no args" diagnostic the C source
/// emits.
pub fn bin_pcre_compile(
    args: &[&str],
    options: &PcreCompileOptions,
    state: &mut PcreState,
) -> (i32, String) {
    if args.is_empty() {
        return (1, "pcre_compile: pattern required\n".to_string());
    }

    match pcre_compile(args[0], options, state) {
        Ok(()) => (0, String::new()),
        Err(e) => (1, format!("pcre_compile: {}\n", e)),
    }
}

/// `pcre_study` builtin entry point.
/// Port of `bin_pcre_study()` from Src/Modules/pcre.c:112 — wraps
/// `pcre_study()` with the same exit-status convention.
pub fn bin_pcre_study(state: &PcreState) -> (i32, String) {
    match pcre_study(state) {
        Ok(()) => (0, String::new()),
        Err(e) => (1, format!("pcre_study: {}\n", e)),
    }
}

/// `pcre_match` builtin entry point.
/// Port of `bin_pcre_match()` from Src/Modules/pcre.c:328 — wraps
/// `pcre_match()` with the C source's "1 on no-match, 0 on match"
/// exit-status convention.
pub fn bin_pcre_match(
    args: &[&str],
    options: &PcreMatchOptions,
    state: &PcreState,
) -> (i32, PcreMatchResult) {
    if args.is_empty() {
        return (1, PcreMatchResult::no_match());
    }

    match pcre_match(args[0], options, state) {
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
    fn test_pcre_state_new() {
        let state = PcreState::new();
        assert!(!state.has_pattern());
    }

    #[test]
    fn test_pcre_compile_simple() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();

        let result = pcre_compile("hello", &options, &mut state);
        assert!(result.is_ok());
        assert!(state.has_pattern());
    }

    #[test]
    fn test_pcre_compile_invalid() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();

        let result = pcre_compile("[invalid", &options, &mut state);
        assert!(result.is_err());
    }

    #[test]
    fn test_pcre_compile_caseless() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions {
            caseless: true,
            ..Default::default()
        };

        let result = pcre_compile("hello", &options, &mut state);
        assert!(result.is_ok());

        let match_opts = PcreMatchOptions::default();
        let result = pcre_match("HELLO WORLD", &match_opts, &state).unwrap();
        assert!(result.matched);
    }

    #[test]
    fn test_pcre_study_no_pattern() {
        let state = PcreState::new();
        let result = pcre_study(&state);
        assert!(result.is_err());
    }

    #[test]
    fn test_pcre_study_with_pattern() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();
        pcre_compile("hello", &options, &mut state).unwrap();

        let result = pcre_study(&state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_pcre_match_simple() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();
        pcre_compile("hello", &options, &mut state).unwrap();

        let match_opts = PcreMatchOptions::default();
        let result = pcre_match("hello world", &match_opts, &state).unwrap();
        assert!(result.matched);
        assert_eq!(result.full_match, Some("hello".to_string()));
    }

    #[test]
    fn test_pcre_match_no_match() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();
        pcre_compile("hello", &options, &mut state).unwrap();

        let match_opts = PcreMatchOptions::default();
        let result = pcre_match("goodbye world", &match_opts, &state).unwrap();
        assert!(!result.matched);
    }

    #[test]
    fn test_pcre_match_captures() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();
        pcre_compile(r"(\w+) (\w+)", &options, &mut state).unwrap();

        let match_opts = PcreMatchOptions::default();
        let result = pcre_match("hello world", &match_opts, &state).unwrap();
        assert!(result.matched);
        assert_eq!(result.captures.len(), 2);
        assert_eq!(result.captures[0], Some("hello".to_string()));
        assert_eq!(result.captures[1], Some("world".to_string()));
    }

    #[test]
    fn test_pcre_match_named_captures() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();
        pcre_compile(r"(?P<first>\w+) (?P<second>\w+)", &options, &mut state).unwrap();

        let match_opts = PcreMatchOptions::default();
        let result = pcre_match("hello world", &match_opts, &state).unwrap();
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
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();
        pcre_compile("world", &options, &mut state).unwrap();

        let match_opts = PcreMatchOptions {
            offset: 6,
            ..Default::default()
        };
        let result = pcre_match("hello world", &match_opts, &state).unwrap();
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
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();
        let (status, _) = bin_pcre_compile(&[], &options, &mut state);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_pcre_match_no_pattern() {
        let state = PcreState::new();
        let options = PcreMatchOptions::default();
        let (status, _) = bin_pcre_match(&["test"], &options, &state);
        assert_eq!(status, 1);
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// pcre_compile - compile a PCRE pattern
    /// `pcre_compile` builtin — delegates to canonical port at
    /// `src/ported/modules/pcre.rs:244` (`bin_pcre_compile()` from
    /// `Src/Modules/pcre.c:70`). All option parsing and pattern
    /// compilation now lives in the canonical port; this shim only
    /// builds the `&[&str]` view and threads `self.pcre_state`.
    pub(crate) fn bin_pcre_compile(&mut self, args: &[String]) -> i32 {
        use crate::pcre::PcreCompileOptions;
        let mut options = PcreCompileOptions::default();
        let mut positional: Vec<&str> = Vec::new();
        for arg in args {
            match arg.as_str() {
                "-a" => options.anchored = true,
                "-i" => options.caseless = true,
                "-m" => options.multiline = true,
                "-s" => options.dotall = true,
                "-x" => options.extended = true,
                s if !s.starts_with('-') => positional.push(s),
                _ => {}
            }
        }
        let (status, output) = crate::pcre::bin_pcre_compile(
            &positional, &options, &mut self.pcre_state,
        );
        if !output.is_empty() {
            if status == 0 { print!("{}", output); } else { eprint!("{}", output); }
        }
        status
    }
    /// `pcre_match` builtin — delegates to canonical port at
    /// `src/ported/modules/pcre.rs:273` (`bin_pcre_match()` from
    /// `Src/Modules/pcre.c:328`). The shim parses `-v`/`-a` argv
    /// flags, calls the canonical matcher, then writes the resulting
    /// `MATCH`/`match` capture data back into the executor's
    /// variable/array tables — that side-effect cannot live in the
    /// canonical port because it doesn't own those tables.
    pub(crate) fn bin_pcre_match(&mut self, args: &[String]) -> i32 {
        use crate::pcre::PcreMatchOptions;

        let mut var_name = "MATCH".to_string();
        let mut array_name = "match".to_string();
        let mut positional: Vec<&str> = Vec::new();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "-v" => { i += 1; if i < args.len() { var_name = args[i].clone(); } }
                "-a" => { i += 1; if i < args.len() { array_name = args[i].clone(); } }
                s if !s.starts_with('-') => positional.push(s),
                _ => {}
            }
            i += 1;
        }

        let options = PcreMatchOptions {
            match_var: Some(var_name.clone()),
            array_var: Some(array_name.clone()),
            ..Default::default()
        };

        let (status, result) = crate::pcre::bin_pcre_match(
            &positional, &options, &self.pcre_state,
        );
        if status == 0 {
            if let Some(m) = result.full_match {
                self.variables.insert(var_name, m);
            }
            let matches: Vec<String> = result.captures.into_iter().flatten().collect();
            self.arrays.insert(array_name, matches);
        }
        status
    }
    /// pcre_study - optimize compiled PCRE (no-op in Rust regex)
    pub(crate) fn bin_pcre_study(&mut self, _args: &[String]) -> i32 {
        use crate::pcre::pcre_study;

        match pcre_study(&self.pcre_state) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("pcre_study: {}", e);
                1
            }
        }
    }
}
// END moved-from-exec-rs
