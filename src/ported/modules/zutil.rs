//! Zsh utility builtins - port of Modules/zutil.c
//!
//! Provides zstyle, zformat, zparseopts builtins.

use regex::Regex;
use crate::ported::utils::zwarnnam;
use std::collections::HashMap;
use indexmap::IndexMap;
// ZStyle is defined below (moved from exec.rs).

/// Save/restore for the per-pattern-match magic vars `$match`,
/// `$mbegin`, `$mend`. Direct port of `MatchData` and the
/// `savematch`/`restorematch`/`freematch` trio in
/// src/zsh/Src/Modules/zutil.c:33-80.
///
/// zstyle's `-e` (eval pattern on retrieve) and zregexparse's
/// inner pattern matches both want to evaluate patterns without
/// clobbering the caller's `$match[]`, `$mbegin[]`, `$mend[]`
/// variables. The C version keeps a heap-duplicated copy in a
/// `MatchData` struct, runs the inner match, then either
/// restores or frees. The Rust port stores `Option<Vec<String>>`
/// — `None` means the var was unset.
pub struct MatchData {
    pub r#match: Option<Vec<String>>,
    pub mbegin: Option<Vec<String>>,
    pub mend: Option<Vec<String>>,
}

impl MatchData {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zutil.c`.
    /// Snapshot the current match arrays from the supplied accessor.
    /// Mirrors zutil.c:39-52 savematch.
    ///
    /// `get_arr(name)` should return `Some(arr.clone())` if the
    /// array variable exists, else `None` — same semantic as
    /// zsh's `getaparam(name)` returning NULL.
    pub fn save<F: Fn(&str) -> Option<Vec<String>>>(get_arr: F) -> Self {
        Self {
            r#match: get_arr("match"),
            mbegin: get_arr("mbegin"),
            mend: get_arr("mend"),
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zutil.c`.
    /// Restore the match arrays via the supplied set/unset callbacks.
    /// Mirrors zutil.c:54-69 restorematch — set if Some(arr),
    /// otherwise unset.
    pub fn restore<S, U>(self, mut set_arr: S, mut unset_arr: U)
    where
        S: FnMut(&str, Vec<String>),
        U: FnMut(&str),
    {
        match self.r#match {
            Some(a) => set_arr("match", a),
            None => unset_arr("match"),
        }
        match self.mbegin {
            Some(a) => set_arr("mbegin", a),
            None => unset_arr("mbegin"),
        }
        match self.mend {
            Some(a) => set_arr("mend", a),
            None => unset_arr("mend"),
        }
    }
}

/// One pattern→values entry for a `zstyle` style.
/// Port of `struct stypat` from Src/Modules/zutil.c — `setstypat()`
/// (line 295) creates entries, `addstyle()` (line 403) inserts them
/// into the style table, `lookupstyle()` (line 443) walks them in
/// weight order. Same `weight` formula as the C source.
#[derive(Debug, Clone)]
pub struct StylePattern {
    pub pattern: String,
    pub weight: u64,
    pub values: Vec<String>,
    pub eval: bool,
}

impl StylePattern {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zutil.c`.
    pub fn new(pattern: &str, values: Vec<String>, eval: bool) -> Self {
        let weight = Self::calculate_weight(pattern);
        Self {
            pattern: pattern.to_string(),
            weight,
            values,
            eval,
        }
    }

    /// Port of `setstypat()` from `Src/Modules/zutil.c:295`.
    fn calculate_weight(pattern: &str) -> u64 {
        let mut weight: u64 = 0;
        let mut tmp = 2u64;
        let mut first = true;

        for ch in pattern.chars() {
            if first && ch == '*' {
                tmp = 0;
                continue;
            }
            first = false;

            if ch == '('
                || ch == '|'
                || ch == '*'
                || ch == '['
                || ch == '<'
                || ch == '?'
                || ch == '#'
                || ch == '^'
            {
                tmp = 1;
            }

            if ch == ':' {
                weight += 1 << 32;
                first = true;
                weight += tmp;
                tmp = 2;
            }
        }
        weight + tmp
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zutil.c`.
    pub fn matches(&self, context: &str) -> bool {
        if self.pattern == "*" {
            return true;
        }

        let regex_pattern = setstypat(&self.pattern);
        if let Ok(re) = Regex::new(&regex_pattern) {
            re.is_match(context)
        } else {
            self.pattern == context
        }
    }
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/zutil.c`.
fn setstypat(pattern: &str) -> String {
    let mut result = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '*' => result.push_str(".*"),
            '?' => result.push('.'),
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                result.push('\\');
                result.push(ch);
            }
            _ => result.push(ch),
        }
    }
    result.push('$');
    result
}

/// `zstyle` storage table.
/// Port of the `zstyletab` HashTable Src/Modules/zutil.c builds —
/// `newzstyletable()` (line 270) creates it, `bin_zstyle()`
/// (line 487) drives every mutation. Same per-style insertion
/// semantics: weight-sorted so the most specific pattern wins.
#[derive(Debug, Default)]
pub struct StyleTable {
    styles: HashMap<String, Vec<StylePattern>>,
}

impl StyleTable {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zutil.c`.
    pub fn new() -> Self {
        Self::default()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zutil.c`.
    pub fn set(&mut self, pattern: &str, style: &str, values: Vec<String>, eval: bool) {
        let style_patterns = self.styles.entry(style.to_string()).or_default();

        if let Some(existing) = style_patterns.iter_mut().find(|p| p.pattern == pattern) {
            existing.values = values;
            existing.eval = eval;
        } else {
            let sp = StylePattern::new(pattern, values, eval);
            let weight = sp.weight;
            let pos = style_patterns
                .iter()
                .position(|p| p.weight < weight)
                .unwrap_or(style_patterns.len());
            style_patterns.insert(pos, sp);
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zutil.c`.
    pub fn get(&self, context: &str, style: &str) -> Option<&[String]> {
        self.styles.get(style).and_then(|patterns| {
            patterns
                .iter()
                .find(|p| p.matches(context))
                .map(|p| p.values.as_slice())
        })
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zutil.c`.
    pub fn delete(&mut self, pattern: Option<&str>, style: Option<&str>) {
        match (pattern, style) {
            (None, None) => self.styles.clear(),
            (Some(pat), None) => {
                for patterns in self.styles.values_mut() {
                    patterns.retain(|p| p.pattern != pat);
                }
                self.styles.retain(|_, v| !v.is_empty());
            }
            (Some(pat), Some(sty)) => {
                if let Some(patterns) = self.styles.get_mut(sty) {
                    patterns.retain(|p| p.pattern != pat);
                    if patterns.is_empty() {
                        self.styles.remove(sty);
                    }
                }
            }
            (None, Some(sty)) => {
                self.styles.remove(sty);
            }
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zutil.c`.
    /// Returns `(pattern, style, values)` triples — the order matches
    /// how zsh prints `zstyle -L` lines (`zstyle <pattern> <style> ...`).
    pub fn list(&self, context: Option<&str>) -> Vec<(String, String, Vec<String>)> {
        let mut result = Vec::new();
        for (style, patterns) in &self.styles {
            for pat in patterns {
                if let Some(ctx) = context {
                    if !pat.matches(ctx) {
                        continue;
                    }
                }
                result.push((pat.pattern.clone(), style.clone(), pat.values.clone()));
            }
        }
        result
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zutil.c`.
    pub fn list_styles(&self) -> Vec<&str> {
        self.styles.keys().map(|s| s.as_str()).collect()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zutil.c`.
    pub fn list_patterns(&self) -> Vec<&str> {
        let mut patterns = Vec::new();
        for pats in self.styles.values() {
            for pat in pats {
                if !patterns.contains(&pat.pattern.as_str()) {
                    patterns.push(pat.pattern.as_str());
                }
            }
        }
        patterns
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zutil.c`.
    pub fn test(&self, context: &str, style: &str, values: Option<&[&str]>) -> bool {
        if let Some(found) = self.get(context, style) {
            if let Some(test_vals) = values {
                test_vals.iter().any(|v| found.contains(&v.to_string()))
            } else {
                matches!(
                    found.first().map(|s| s.as_str()),
                    Some("true" | "yes" | "on" | "1")
                )
            }
        } else {
            false
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zutil.c`.
    pub fn test_bool(&self, context: &str, style: &str) -> Option<bool> {
        self.get(context, style).and_then(|vals| {
            if vals.len() == 1 {
                match vals[0].as_str() {
                    "yes" | "true" | "on" | "1" => Some(true),
                    "no" | "false" | "off" | "0" => Some(false),
                    _ => None,
                }
            } else {
                None
            }
        })
    }
}

/// Format a string with specifications
/// `zformat` builtin entry point.
/// Helper extracted from `bin_zformat()` (Src/Modules/zutil.c:955)
/// — same `%X:value` substitution + width / left/right-align /
/// repeat flag handling the C source's `zformat_substring()`
/// (line 814) implements.
pub fn zformat_substring(format: &str, specs: &HashMap<char, String>, presence: bool) -> String {
    // Direct port of src/zsh/Src/Modules/zutil.c:814-952
    // zformat_substring. Recursive walker that handles:
    //   - Plain `%X` substitutions
    //   - Optional `-` for right-align
    //   - Optional `N` for min width
    //   - Optional `.M` for max width
    //   - Ternary `%(SPECTEST.true-text.false-text)` — conditional
    //     substitution based on whether the spec exists / matches a
    //     numeric test value. With presence=true (zformat -F) the
    //     test compares the spec's existence/length; with
    //     presence=false (zformat -f) the test compares against an
    //     integer math eval of the spec value.
    //
    // The original C uses an output-buffer with growable backing;
    // we use a Rust String with push_* helpers. The recursive
    // descent + (skip || actval) pattern is the same.
    // Per zsh/Src/Modules/zutil.c::bin_zformat lines 975-976:
    // `specs['%']` and `specs[')']` are pre-populated to literal "%" and ")"
    // BEFORE the recursive walk, which is why `%%` produces `%` and
    // `%)` produces `)` even though no caller registers them. Rebuild
    // a private copy of the specs map with those defaults injected,
    // unless the caller explicitly overrode them.
    let mut effective: HashMap<char, String> = specs.clone();
    effective.entry('%').or_insert_with(|| "%".to_string());
    effective.entry(')').or_insert_with(|| ")".to_string());

    let bytes: Vec<char> = format.chars().collect();
    let mut out = String::with_capacity(bytes.len() + 16);
    let mut idx = 0;
    let _ = zformat_recurse(&bytes, &mut idx, &mut out, '\0', &effective, presence, false);
    out
}

/// Recursive walker for zformat. Returns the index of the
/// terminator (`endchar`). idx is mutated in place.
/// Direct port of zformat_substring (zutil.c:814-952).
fn zformat_recurse(
    bytes: &[char],
    idx: &mut usize,
    out: &mut String,
    endchar: char,
    specs: &HashMap<char, String>,
    presence: bool,
    skip: bool,
) -> Option<()> {
    while *idx < bytes.len() {
        let c = bytes[*idx];
        // Stop at endchar (zutil.c:820 `*s != endchar`).
        if endchar != '\0' && c == endchar {
            return Some(());
        }
        if c != '%' {
            // Plain text — emit unless skipping (zutil.c:937-948).
            if !skip {
                out.push(c);
            }
            *idx += 1;
            continue;
        }
        // `%` — parse the spec.
        let start = *idx;
        *idx += 1;
        // Optional `-` for right-align (zutil.c:825-826).
        let mut right = false;
        if *idx < bytes.len() && bytes[*idx] == '-' {
            right = true;
            *idx += 1;
        }
        // Optional digit run for min (zutil.c:828-831).
        let mut min: Option<i64> = None;
        if *idx < bytes.len() && bytes[*idx].is_ascii_digit() {
            let mut n: i64 = 0;
            while *idx < bytes.len() && bytes[*idx].is_ascii_digit() {
                n = n * 10 + bytes[*idx].to_digit(10).unwrap() as i64;
                *idx += 1;
            }
            min = Some(n);
        }
        // Ternary detection: `(` at this position (zutil.c:834-840).
        let testit = *idx < bytes.len() && bytes[*idx] == '(';
        // `%(-...` allows leading `-` after the paren (zutil.c:835-840).
        if testit && *idx + 1 < bytes.len() && bytes[*idx + 1] == '-' {
            right = true;
            *idx += 1;
        }
        // Optional `.MAX` or just `.` after (zutil.c:841-845).
        let mut max: Option<i64> = None;
        if *idx < bytes.len()
            && (bytes[*idx] == '.' || testit)
            && *idx + 1 < bytes.len()
            && bytes[*idx + 1].is_ascii_digit()
        {
            *idx += 1; // skip `.` or `(`
            let mut n: i64 = 0;
            while *idx < bytes.len() && bytes[*idx].is_ascii_digit() {
                n = n * 10 + bytes[*idx].to_digit(10).unwrap() as i64;
                *idx += 1;
            }
            max = Some(n);
        } else if *idx < bytes.len() && (bytes[*idx] == '.' || testit) {
            *idx += 1;
        }

        if testit && *idx < bytes.len() {
            // Ternary expression — zutil.c:847-887.
            let testval: i64 = min.or(max).unwrap_or(0);
            let spec_char = bytes[*idx];
            let actval: bool;
            let spec_val = specs.get(&spec_char);
            if let Some(sv) = spec_val.filter(|s| !s.is_empty()) {
                if presence {
                    let cmp_val: i64 = if testval != 0 {
                        sv.chars().count() as i64
                    } else {
                        1
                    };
                    actval = if right {
                        testval < cmp_val
                    } else {
                        testval >= cmp_val
                    };
                } else {
                    let signed_test = if right { -testval } else { testval };
                    let n: i64 = sv.parse().unwrap_or(0);
                    actval = (n - signed_test) != 0;
                }
            } else {
                actval = if presence { !right } else { testval != 0 };
            }
            // Skip past the spec char to find the delimiter
            // (zutil.c:874-876 endcharl = *++s).
            *idx += 1;
            if *idx >= bytes.len() {
                return None;
            }
            let endcharl = bytes[*idx];
            *idx += 1;
            // First branch (true-text) — emit only if actval is true,
            // i.e. skip = skip || !actval. Wait, C says
            // `skip || actval` for the FIRST sub-call meaning: if
            // actval is true SKIP the first branch?
            // Re-reading zutil.c:880-884 — comment says "Either skip
            // true text and output false text, or vice versa". The
            // pattern `skip || actval` for the first call means: if
            // actval, skip the first text. So the FIRST text
            // (between `(` and the delim) is the FALSE branch, the
            // SECOND text (between delim and `)`) is the TRUE.
            zformat_recurse(bytes, idx, out, endcharl, specs, presence, skip || actval)?;
            // Skip the delimiter
            if *idx < bytes.len() && bytes[*idx] == endcharl {
                *idx += 1;
            }
            zformat_recurse(bytes, idx, out, ')', specs, presence, skip || !actval)?;
            // Skip the closing `)`
            if *idx < bytes.len() && bytes[*idx] == ')' {
                *idx += 1;
            }
            continue;
        }

        if skip {
            // In skip mode — advance past spec char and continue.
            if *idx < bytes.len() {
                *idx += 1;
            }
            continue;
        }

        // Plain `%X` spec (zutil.c:890-922).
        if *idx < bytes.len() {
            let spec_char = bytes[*idx];
            *idx += 1;
            if let Some(spec_val) = specs.get(&spec_char) {
                let mut val_chars: Vec<char> = spec_val.chars().collect();
                let len = val_chars.len() as i64;
                let len = match max {
                    Some(m) if m >= 0 && len > m => {
                        val_chars.truncate(m as usize);
                        m
                    }
                    _ => len,
                };
                let outl = match min {
                    Some(m) if m >= 0 && m > len => m,
                    _ => len,
                };
                if len >= outl {
                    for &c in val_chars.iter().take(outl as usize) {
                        out.push(c);
                    }
                } else {
                    let diff = (outl - len) as usize;
                    if right {
                        for _ in 0..diff {
                            out.push(' ');
                        }
                        for &c in val_chars.iter() {
                            out.push(c);
                        }
                    } else {
                        for &c in val_chars.iter() {
                            out.push(c);
                        }
                        for _ in 0..diff {
                            out.push(' ');
                        }
                    }
                }
            } else {
                // Unknown spec — emit raw segment back
                // (zutil.c:923-936).
                for &c in &bytes[start..*idx] {
                    out.push(c);
                }
            }
        }
    }
    Some(())
}

/// Option description for zparseopts
#[derive(Debug, Clone)]
/// `zparseopts` option descriptor.
/// Port of the per-option entries `bin_zparseopts()` from
/// Src/Modules/zutil.c builds while parsing the `-D`/`-K`/`-E`/
/// `-M` argument set — the C source uses inline locals; we wrap
/// them in a struct.
pub struct OptDesc {
    pub name: String,
    pub takes_arg: bool,
    pub optional_arg: bool,
    pub multiple: bool,
    pub array_name: Option<String>,
}

impl OptDesc {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/zutil.c`.
    pub fn parse(spec: &str) -> Option<Self> {
        if spec.is_empty() {
            return None;
        }

        let mut name = String::new();
        let mut takes_arg = false;
        let mut optional_arg = false;
        let mut multiple = false;
        let mut array_name = None;
        let mut chars = spec.chars().peekable();

        while let Some(&ch) = chars.peek() {
            if ch == '+' {
                multiple = true;
                chars.next();
                break;
            } else if ch == ':' || ch == '=' {
                break;
            } else if ch == '\\' {
                chars.next();
                if let Some(c) = chars.next() {
                    name.push(c);
                }
            } else {
                name.push(ch);
                chars.next();
            }
        }

        if name.is_empty() {
            return None;
        }

        if chars.peek() == Some(&':') {
            takes_arg = true;
            chars.next();
            if chars.peek() == Some(&':') {
                optional_arg = true;
                chars.next();
            }
        }

        if chars.peek() == Some(&'=') {
            chars.next();
            array_name = Some(chars.collect());
        }

        Some(Self {
            name,
            takes_arg,
            optional_arg,
            multiple,
            array_name,
        })
    }
}

/// Parse options from arguments
#[allow(clippy::type_complexity)]
/// `zparseopts` builtin entry point.
/// Helper extracted from `bin_zparseopts()` (Src/Modules/zutil.c) —
/// the option-parser the C source ships for completion-system
/// use (`-D` consume, `-K` keep, `-E` non-strict, `-M` aliasing).
pub fn zparseopts(
    args: &[String],
    specs: &[OptDesc],
    delete: bool,
    extract: bool,
) -> Result<(HashMap<String, Vec<String>>, Vec<String>), String> {
    let mut results: HashMap<String, Vec<String>> = HashMap::new();
    let mut remaining = Vec::new();
    let mut i = 0;

    let short_opts: HashMap<char, &OptDesc> = specs
        .iter()
        .filter(|s| s.name.len() == 1)
        .map(|s| (s.name.chars().next().unwrap(), s))
        .collect();

    let long_opts: HashMap<&str, &OptDesc> = specs
        .iter()
        .filter(|s| s.name.len() > 1)
        .map(|s| (s.name.as_str(), s))
        .collect();

    while i < args.len() {
        let arg = &args[i];

        if !arg.starts_with('-') || arg == "-" {
            if extract {
                if !delete {
                    remaining.push(arg.clone());
                }
                i += 1;
                continue;
            } else {
                remaining.extend(args[i..].iter().cloned());
                break;
            }
        }

        if arg == "--" {
            i += 1;
            remaining.extend(args[i..].iter().cloned());
            break;
        }

        let opt_str = &arg[1..];

        if let Some(desc) = long_opts.get(opt_str) {
            let key = format!("-{}", desc.name);
            let entry = results.entry(key).or_default();

            if desc.takes_arg {
                if i + 1 < args.len() && !desc.optional_arg {
                    i += 1;
                    entry.push(args[i].clone());
                } else if desc.optional_arg {
                    entry.push(String::new());
                } else {
                    return Err(format!("missing argument for option: -{}", desc.name));
                }
            } else {
                entry.push(String::new());
            }
        } else if let Some(long_name) = opt_str.strip_prefix('-') {
            if let Some((name, value)) = long_name.split_once('=') {
                if let Some(desc) = long_opts.get(name) {
                    let key = format!("-{}", desc.name);
                    results.entry(key).or_default().push(value.to_string());
                } else {
                    if !extract {
                        remaining.extend(args[i..].iter().cloned());
                        break;
                    }
                    remaining.push(arg.clone());
                }
            } else if let Some(desc) = long_opts.get(long_name) {
                let key = format!("-{}", desc.name);
                let entry = results.entry(key).or_default();

                if desc.takes_arg {
                    if i + 1 < args.len() && !desc.optional_arg {
                        i += 1;
                        entry.push(args[i].clone());
                    } else if desc.optional_arg {
                        entry.push(String::new());
                    } else {
                        return Err(format!("missing argument for option: --{}", desc.name));
                    }
                } else {
                    entry.push(String::new());
                }
            } else {
                if !extract {
                    remaining.extend(args[i..].iter().cloned());
                    break;
                }
                remaining.push(arg.clone());
            }
        } else {
            let mut j = 0;
            let chars: Vec<char> = opt_str.chars().collect();

            while j < chars.len() {
                let ch = chars[j];
                if let Some(desc) = short_opts.get(&ch) {
                    let key = format!("-{}", desc.name);
                    let entry = results.entry(key).or_default();

                    if desc.takes_arg {
                        if j + 1 < chars.len() {
                            entry.push(chars[j + 1..].iter().collect());
                            break;
                        } else if i + 1 < args.len() && !desc.optional_arg {
                            i += 1;
                            entry.push(args[i].clone());
                        } else if desc.optional_arg {
                            entry.push(String::new());
                        } else {
                            return Err(format!("missing argument for option: -{}", desc.name));
                        }
                    } else {
                        entry.push(String::new());
                    }
                } else {
                    if !extract {
                        remaining.push(arg.clone());
                        remaining.extend(args[i + 1..].iter().cloned());
                        return Ok((results, remaining));
                    }
                    break;
                }
                j += 1;
            }
        }
        i += 1;
    }

    if !delete && !extract {
        remaining = args[i..].to_vec();
    }

    Ok((results, remaining))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_style_pattern_weight() {
        let p1 = StylePattern::new("*", vec![], false);
        let p2 = StylePattern::new(":completion:*", vec![], false);
        let p3 = StylePattern::new(":completion:zsh:*", vec![], false);

        assert!(p3.weight > p2.weight);
        assert!(p2.weight > p1.weight);
    }

    #[test]
    fn test_style_pattern_matches() {
        let p = StylePattern::new(":completion:*", vec![], false);
        assert!(p.matches(":completion:zsh:complete"));
        assert!(!p.matches(":other:zsh"));

        let p2 = StylePattern::new("*", vec![], false);
        assert!(p2.matches("anything"));
    }

    #[test]
    fn test_style_table_set_get() {
        let mut table = StyleTable::new();
        table.set(":completion:*", "verbose", vec!["yes".to_string()], false);

        let result = table.get(":completion:zsh", "verbose");
        assert_eq!(result, Some(&["yes".to_string()][..]));

        let result = table.get(":other", "verbose");
        assert!(result.is_none());
    }

    #[test]
    fn test_style_table_priority() {
        let mut table = StyleTable::new();
        table.set("*", "menu", vec!["no".to_string()], false);
        table.set(":completion:*", "menu", vec!["yes".to_string()], false);

        let result = table.get(":completion:zsh", "menu");
        assert_eq!(result, Some(&["yes".to_string()][..]));
    }

    #[test]
    fn test_style_table_delete() {
        let mut table = StyleTable::new();
        table.set("*", "style1", vec!["val".to_string()], false);
        table.set("*", "style2", vec!["val".to_string()], false);

        table.delete(None, Some("style1"));
        assert!(table.get("test", "style1").is_none());
        assert!(table.get("test", "style2").is_some());
    }

    #[test]
    fn test_style_test_bool() {
        let mut table = StyleTable::new();
        table.set("*", "enabled", vec!["yes".to_string()], false);
        table.set("*", "disabled", vec!["no".to_string()], false);
        table.set(
            "*",
            "multiple",
            vec!["a".to_string(), "b".to_string()],
            false,
        );

        assert_eq!(table.test_bool("ctx", "enabled"), Some(true));
        assert_eq!(table.test_bool("ctx", "disabled"), Some(false));
        assert_eq!(table.test_bool("ctx", "multiple"), None);
    }

    #[test]
    fn test_zformat_basic() {
        let mut specs = HashMap::new();
        specs.insert('n', "test".to_string());
        specs.insert('v', "42".to_string());

        let result = zformat_substring("Name: %n, Value: %v", &specs, false);
        assert_eq!(result, "Name: test, Value: 42");
    }

    #[test]
    fn test_zformat_padding() {
        let mut specs = HashMap::new();
        specs.insert('n', "hi".to_string());

        let result = zformat_substring("[%10n]", &specs, false);
        assert_eq!(result, "[hi        ]");

        let result = zformat_substring("[%-10n]", &specs, false);
        assert_eq!(result, "[        hi]");
    }

    #[test]
    fn test_zformat_truncate() {
        let mut specs = HashMap::new();
        specs.insert('n', "hello world".to_string());

        let result = zformat_substring("[%.5n]", &specs, false);
        assert_eq!(result, "[hello]");
    }

    #[test]
    fn test_zformat_escape() {
        let specs = HashMap::new();
        let result = zformat_substring("100%%", &specs, false);
        assert_eq!(result, "100%");
    }

    #[test]
    fn test_opt_desc_parse() {
        let desc = OptDesc::parse("v").unwrap();
        assert_eq!(desc.name, "v");
        assert!(!desc.takes_arg);

        let desc = OptDesc::parse("o:").unwrap();
        assert_eq!(desc.name, "o");
        assert!(desc.takes_arg);
        assert!(!desc.optional_arg);

        let desc = OptDesc::parse("o::").unwrap();
        assert!(desc.optional_arg);

        let desc = OptDesc::parse("v+").unwrap();
        assert!(desc.multiple);

        let desc = OptDesc::parse("a:=myarray").unwrap();
        assert_eq!(desc.array_name, Some("myarray".to_string()));
    }

    #[test]
    fn test_zparseopts_basic() {
        let specs = vec![OptDesc::parse("v").unwrap(), OptDesc::parse("o:").unwrap()];

        let args: Vec<String> = vec!["-v", "-o", "value", "rest"]
            .into_iter()
            .map(String::from)
            .collect();

        let (opts, remaining) = zparseopts(&args, &specs, false, false).unwrap();

        assert!(opts.contains_key("-v"));
        assert_eq!(opts.get("-o"), Some(&vec!["value".to_string()]));
        assert_eq!(remaining, vec!["rest"]);
    }

    #[test]
    fn test_zparseopts_combined() {
        let specs = vec![
            OptDesc::parse("a").unwrap(),
            OptDesc::parse("b").unwrap(),
            OptDesc::parse("c:").unwrap(),
        ];

        let args: Vec<String> = vec!["-abc", "val"].into_iter().map(String::from).collect();

        let (opts, _) = zparseopts(&args, &specs, false, false).unwrap();

        assert!(opts.contains_key("-a"));
        assert!(opts.contains_key("-b"));
        assert_eq!(opts.get("-c"), Some(&vec!["val".to_string()]));
    }

    #[test]
    fn test_zparseopts_long() {
        let specs = vec![
            OptDesc::parse("verbose").unwrap(),
            OptDesc::parse("output:").unwrap(),
        ];

        let args: Vec<String> = vec!["--verbose", "--output", "file.txt"]
            .into_iter()
            .map(String::from)
            .collect();

        let (opts, _) = zparseopts(&args, &specs, false, false).unwrap();

        assert!(opts.contains_key("-verbose"));
        assert_eq!(opts.get("-output"), Some(&vec!["file.txt".to_string()]));
    }

}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in exec.rs are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// zsh zstyle - configure styles for completion
    pub(crate) fn bin_zstyle(&mut self, args: &[String]) -> i32 {
        // PFA-SMR aspect: emit one `zstyle` event per setter call. The
        // pattern is arg[0] (or arg[1] when arg[0] is a flag like `-e`),
        // the style+values are the rest joined.
        #[cfg(feature = "recorder")]
        if crate::recorder::is_enabled() {
            let positional: Vec<&str> = args
                .iter()
                .filter(|a| !a.starts_with('-'))
                .map(String::as_str)
                .collect();
            if positional.len() >= 2 {
                let ctx = self.recorder_ctx();
                let pattern = positional[0];
                let rest = positional[1..].join(" ");
                crate::recorder::emit_zstyle(pattern, &rest, ctx);
            }
        }
        // zsh: a single non-flag positional like `zstyle X` -> `zstyle:1:
        // not enough arguments` (need at least pattern+style or
        // flag-form). zshrs's catch-all set-style path required
        // args.len() >= 2 silently.
        if args.len() == 1 && !args[0].starts_with('-') {
            zwarnnam("zstyle", "not enough arguments");
            return 1;
        }
        if args.is_empty() {
            // Bare `zstyle` lists styles grouped by name:
            //   STYLE
            //           pattern  val1 val2 ...
            //           pattern  val1 ...
            let mut grouped: std::collections::BTreeMap<String, Vec<(String, Vec<String>)>> =
                std::collections::BTreeMap::new();
            for (pattern, style, values) in self.style_table.list(None) {
                grouped.entry(style).or_default().push((pattern, values));
            }
            for (style, rows) in &grouped {
                println!("{}", style);
                for (pat, vals) in rows {
                    println!("        {} {}", pat, vals.join(" "));
                }
            }
            return 0;
        }

        // Handle options
        if args[0].starts_with('-') {
            match args[0].as_str() {
                "-d" => {
                    // Delete style
                    let pattern = args.get(1).map(|s| s.as_str());
                    let style = args.get(2).map(|s| s.as_str());
                    self.style_table.delete(pattern, style);
                    return 0;
                }
                "-g" => {
                    // Get style into array. zsh: too few args ->
                    // `zstyle:1: not enough arguments` exit 1.
                    if args.len() < 4 {
                        zwarnnam("zstyle", "not enough arguments");
                        return 1;
                    }
                    let array_name = &args[1];
                    let context = &args[2];
                    let style = &args[3];
                    if let Some(values) = self.style_table.get(context, style) {
                        self.arrays.insert(array_name.clone(), values.to_vec());
                        return 0;
                    }
                    return 1;
                }
                "-s" => {
                    // `zstyle -s CONTEXT STYLE NAME [SEP]` — get
                    // style as scalar into NAME. Direct port of
                    // Src/Modules/zutil.c:643-655 — `lookupstyle
                    // (args[1], args[2])` then `setsparam(args[3],
                    // sepjoin(...))`. The previous zshrs code had
                    // the args mis-permuted (treated args[1] as
                    // NAME), so `zstyle -s :ctx style val` left
                    // `$val` empty.
                    if args.len() < 4 {
                        zwarnnam("zstyle", "not enough arguments");
                        return 1;
                    }
                    let context = &args[1];
                    let style = &args[2];
                    let var_name = &args[3];
                    let sep = args.get(4).map(|s| s.as_str()).unwrap_or(" ");
                    if let Some(values) = self.style_table.get(context, style) {
                        self.variables.insert(var_name.clone(), values.join(sep));
                        return 0;
                    }
                    return 1;
                }
                "-t" => {
                    // Test style (check if true/yes)
                    if args.len() < 3 {
                        zwarnnam("zstyle", "not enough arguments");
                        return 1;
                    }
                    let context = &args[1];
                    let style = &args[2];
                    return if self.style_table.test_bool(context, style).unwrap_or(false) {
                        0
                    } else {
                        1
                    };
                }
                "-T" => {
                    // Test style (like -t but defaults to TRUE for
                    // unset). zsh: `zstyle -T :foo style` returns 0
                    // when the style is set OR not set; only returns
                    // non-zero when explicitly set to false. zshrs's
                    // unknown-flag fallback rejected -T as invalid.
                    if args.len() < 3 {
                        zwarnnam("zstyle", "not enough arguments");
                        return 1;
                    }
                    let context = &args[1];
                    let style = &args[2];
                    return if self.style_table.test_bool(context, style).unwrap_or(true) {
                        0
                    } else {
                        1
                    };
                }
                "-b" => {
                    // zstyle -b context style param: store boolean
                    // ("yes"/"no") in scalar `param`. Per zutil.c
                    // bin_zstyle case 'b': test_bool returning Some
                    // means we have a value, format as yes/no.
                    if args.len() < 4 {
                        zwarnnam("zstyle", "not enough arguments");
                        return 1;
                    }
                    let context = &args[1];
                    let style = &args[2];
                    let var_name = &args[3];
                    let val = if self.style_table.test_bool(context, style).unwrap_or(false) {
                        "yes"
                    } else {
                        "no"
                    };
                    self.variables.insert(var_name.clone(), val.to_string());
                    return if self.style_table.get(context, style).is_some() {
                        0
                    } else {
                        1
                    };
                }
                "-a" => {
                    // zstyle -a context style array_param: copy all
                    // style values into the named array (zutil.c
                    // bin_zstyle case 'a').
                    if args.len() < 4 {
                        zwarnnam("zstyle", "not enough arguments");
                        return 1;
                    }
                    let context = &args[1];
                    let style = &args[2];
                    let array_name = &args[3];
                    if let Some(values) = self.style_table.get(context, style) {
                        self.arrays.insert(array_name.clone(), values.to_vec());
                        return 0;
                    }
                    return 1;
                }
                "-m" => {
                    // zstyle -m context style pattern: test if any
                    // style value matches the glob pattern (zutil.c
                    // bin_zstyle case 'm'). Returns 0 on match.
                    if args.len() < 4 {
                        zwarnnam("zstyle", "not enough arguments");
                        return 1;
                    }
                    let context = &args[1];
                    let style = &args[2];
                    let pattern = &args[3];
                    if let Some(values) = self.style_table.get(context, style) {
                        for v in values {
                            if Self::glob_match_static(v, pattern) {
                                return 0;
                            }
                        }
                    }
                    return 1;
                }
                "-e" => {
                    // zstyle -e context style value...: stores values
                    // marked as expressions. The expansion happens at
                    // -g/-s lookup time. Mark the entry as eval form;
                    // we don't yet evaluate the expression on lookup,
                    // but record the values so re-listing works.
                    if args.len() < 4 {
                        zwarnnam("zstyle", "not enough arguments");
                        return 1;
                    }
                    let pattern = &args[1];
                    let style = &args[2];
                    let values: Vec<String> = args[3..].to_vec();
                    self.style_table.set(pattern, style, values, true);
                    return 0;
                }
                "-L" => {
                    // List in re-usable format. zsh's exact form is:
                    //   zstyle <pattern> <style> <val1> <val2>...
                    // with patterns/styles/values as bare words (only
                    // quoted when they contain whitespace or specials).
                    for (pattern, style, values) in self.style_table.list(None) {
                        let pat = if pattern.contains(' ') || pattern.is_empty() {
                            format!("'{}'", pattern)
                        } else {
                            pattern.clone()
                        };
                        let sty = if style.contains(' ') || style.is_empty() {
                            format!("'{}'", style)
                        } else {
                            style.clone()
                        };
                        let mut line = format!("zstyle {} {}", pat, sty);
                        for v in &values {
                            line.push(' ');
                            if v.contains(' ') || v.is_empty() {
                                line.push_str(&format!("'{}'", v.replace('\'', "'\\''")));
                            } else {
                                line.push_str(v);
                            }
                        }
                        println!("{}", line);
                    }
                    return 0;
                }
                // zsh: unknown zstyle flag errors `zstyle:1: invalid
                // option: -X` exit 1. zshrs's `_ => {}` silent
                // fallback let any unknown flag drop through to the
                // set-style path with `pattern=-X`.
                "-" => {
                    // Bare `-` is treated as "not enough arguments"
                    // by zsh — it's a degenerate flag-only invocation
                    // without a recognized option letter.
                    zwarnnam("zstyle", "not enough arguments");
                    return 1;
                }
                other => {
                    zwarnnam("zstyle", &format!("invalid option: {}", other));
                    return 1;
                }
            }
        }

        // Set style: zstyle pattern style values...
        if args.len() >= 2 {
            let pattern = &args[0];
            let style = &args[1];
            let values: Vec<String> = args[2..].to_vec();
            self.style_table.set(pattern, style, values.clone(), false);

            // Write to SQLite cache for completion lookups
            if let Some(cache) = &self.compsys_cache {
                let _ = cache.set_zstyle(pattern, style, &values, false);
            }

            // Also update legacy zstyles for backward compat
            let existing = self
                .zstyles
                .iter_mut()
                .find(|s| s.pattern == *pattern && s.style == *style);
            if let Some(s) = existing {
                s.values = args[2..].to_vec();
            } else {
                self.zstyles.push(ZStyle {
                    pattern: pattern.clone(),
                    style: style.clone(),
                    values: args[2..].to_vec(),
                });
            }
        }
        0
    }
    /// zparseopts - parse options from positional parameters
    pub(crate) fn bin_zparseopts(&mut self, args: &[String]) -> i32 {
        // zsh: bare `zparseopts` -> `zparseopts:1: not enough
        // arguments` exit 1. zshrs silently returned 0.
        if args.is_empty() {
            zwarnnam("zparseopts", "not enough arguments");
            return 1;
        }
        let mut remove_parsed = false; // -D
        let mut keep_going = false; // -E
        let mut fail_on_error = false; // -F
        let mut keep_values = false; // -K
        let mut map_names = false; // -M
        let mut array_name: Option<String> = None; // -a
        let mut assoc_name: Option<String> = None; // -A
        let mut specs: Vec<String> = Vec::new();

        let mut iter = args.iter().peekable();

        // Parse zparseopts options
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-D" => remove_parsed = true,
                "-E" => keep_going = true,
                "-F" => fail_on_error = true,
                "-K" => keep_values = true,
                "-M" => map_names = true,
                "-a" => {
                    if let Some(name) = iter.next() {
                        array_name = Some(name.clone());
                    }
                }
                "-A" => {
                    if let Some(name) = iter.next() {
                        assoc_name = Some(name.clone());
                    }
                }
                "-" | "--" => break,
                s if !s.starts_with('-') || s.contains('=') || s.contains(':') => {
                    specs.push(s.to_string());
                }
                _ => specs.push(arg.clone()),
            }
        }

        // Collect remaining specs
        for arg in iter {
            specs.push(arg.clone());
        }

        // Parse the specs to understand what options we're looking for
        #[derive(Clone)]
        struct OptSpec {
            name: String,
            takes_arg: bool,
            optional_arg: bool,
            #[allow(dead_code)]
            append: bool,
            target_array: Option<String>,
        }

        let mut opt_specs: Vec<OptSpec> = Vec::new();
        for spec in &specs {
            let mut s = spec.as_str();
            let mut target = None;

            // Check for =array at end
            if let Some(eq_pos) = s.rfind('=') {
                if !s[eq_pos + 1..].contains(':') {
                    target = Some(s[eq_pos + 1..].to_string());
                    s = &s[..eq_pos];
                }
            }

            let append = s.ends_with('+') || s.contains("+:");
            let s = s.trim_end_matches('+');

            let (name, takes_arg, optional_arg) = if s.ends_with("::") {
                (s.trim_end_matches(':').trim_end_matches(':'), true, true)
            } else if s.ends_with(':') {
                (s.trim_end_matches(':'), true, false)
            } else {
                (s, false, false)
            };

            opt_specs.push(OptSpec {
                name: name.to_string(),
                takes_arg,
                optional_arg,
                append,
                target_array: target,
            });
        }

        // Get positional parameters to parse — pull from
        // `self.positional_params` (the canonical source). Falling back to
        // `$1..$99` via get_variable misses gaps and stops on empties.
        let positionals: Vec<String> = self.positional_params.clone();
        // Track which indices got consumed so -D can splice them out
        // properly even when -E/keep_going skips over non-options.
        let mut consumed_indices: Vec<usize> = Vec::new();

        // Results: (display_name, value, canonical_spec_name)
        // - display_name is the actual arg as seen (`--foo` for alias)
        // - canonical_spec_name routes the result to the right per-spec array
        let mut results: Vec<(String, Option<String>, String)> = Vec::new();
        let mut i = 0;
        let mut parsed_count = 0;

        while i < positionals.len() {
            let arg = &positionals[i];

            if arg == "-" || arg == "--" {
                consumed_indices.push(i);
                parsed_count = i + 1;
                break;
            }

            if !arg.starts_with('-') {
                if !keep_going {
                    break;
                }
                i += 1;
                continue;
            }

            // Try to match against specs. zparseopts treats a spec
            // name beginning with `-` as a long option (matched as
            // `--<rest>` in the input). A spec with no leading dash is a
            // short option matched as `-<name>`. Strip exactly one
            // leading `-` from the arg, then compare.
            let after_one_dash = &arg[1..]; // safe: caller ensured leading `-`
            let mut matched = false;

            for spec in &opt_specs {
                let matches_eq = after_one_dash == spec.name;
                let matches_eq_arg = after_one_dash.starts_with(&format!("{}=", spec.name));
                if !matches_eq && !matches_eq_arg {
                    continue;
                }
                matched = true;
                consumed_indices.push(i);

                // With -M, if this spec's target is itself a spec name
                // (an alias), redirect to the canonical spec — record
                // under the canonical name and use its arg-handling.
                let mut effective_spec = spec.clone();
                let mut record_as_name = arg.clone();
                if map_names {
                    if let Some(tgt) = &spec.target_array {
                        if let Some(canon) = opt_specs.iter().find(|s| &s.name == tgt) {
                            effective_spec = canon.clone();
                            // Keep the actual arg as the recorded value
                            // (zsh stores `--foo`, not `-f`).
                            record_as_name = arg.clone();
                        }
                    }
                }

                if effective_spec.takes_arg {
                    let arg_value = if after_one_dash.contains('=') {
                        // Direct port of Src/Modules/zutil.c:bin_zparseopts
                        // value-collection path: when the option arg was
                        // glued to the option name with `=` (`--name=foo`),
                        // zsh stores the value WITH the leading `=` so the
                        // user's array contains `["--name", "=foo"]`.
                        // Without preserving the `=`, callers using
                        // `${arr[2]}` to detect "was a value supplied" lose
                        // information about the separator form.
                        let raw = after_one_dash
                            .split_once('=')
                            .map(|x| x.1)
                            .unwrap_or("");
                        Some(format!("={}", raw))
                    } else if i + 1 < positionals.len()
                        && (!positionals[i + 1].starts_with('-') || effective_spec.optional_arg)
                    {
                        i += 1;
                        consumed_indices.push(i);
                        Some(positionals[i].clone())
                    } else if effective_spec.optional_arg {
                        None
                    } else if fail_on_error {
                        zwarnnam("zparseopts", &format!("missing argument for option: {}", effective_spec.name));
                        return 1;
                    } else {
                        None
                    };
                    results.push((record_as_name, arg_value, effective_spec.name.clone()));
                } else {
                    results.push((record_as_name, None, effective_spec.name.clone()));
                }
                break;
            }

            if !matched && !keep_going {
                break;
            }

            i += 1;
            parsed_count = i;
        }

        // Store results in array
        if let Some(arr_name) = &array_name {
            let mut arr_values: Vec<String> = Vec::new();
            for (opt, val, _) in &results {
                arr_values.push(opt.clone());
                if let Some(v) = val {
                    arr_values.push(v.clone());
                }
            }
            self.arrays.insert(arr_name.clone(), arr_values);
        }

        // Store in associative array
        if let Some(assoc) = &assoc_name {
            let mut map: IndexMap<String, String> = IndexMap::new();
            for (opt, val, _) in &results {
                map.insert(opt.clone(), val.clone().unwrap_or_default());
            }
            self.assoc_arrays.insert(assoc.clone(), map);
        }

        // Store in per-option arrays — route by canonical spec name so
        // -M aliases land in the right bucket.
        for spec in &opt_specs {
            if let Some(target) = &spec.target_array {
                if map_names && opt_specs.iter().any(|s| Some(&s.name) == Some(target)) {
                    // This spec is itself an alias — skip; results land in
                    // the canonical spec's target.
                    continue;
                }
                let values: Vec<String> = results
                    .iter()
                    .filter(|(_, _, canon)| canon == &spec.name)
                    .flat_map(|(opt, val, _)| {
                        let mut v = vec![opt.clone()];
                        if let Some(arg) = val {
                            v.push(arg.clone());
                        }
                        v
                    })
                    .collect();
                if !values.is_empty() || !keep_values {
                    self.arrays.insert(target.clone(), values);
                }
            }
        }

        // Remove parsed arguments if -D — splice out only the consumed
        // indices (preserves intervening positionals when -E was used).
        if remove_parsed && !consumed_indices.is_empty() {
            let consumed: std::collections::HashSet<usize> =
                consumed_indices.iter().copied().collect();
            let kept: Vec<String> = positionals
                .iter()
                .enumerate()
                .filter_map(|(idx, v)| {
                    if consumed.contains(&idx) {
                        None
                    } else {
                        Some(v.clone())
                    }
                })
                .collect();
            // Wipe all old positional bindings, then reseed.
            for k in 1..=positionals.len() {
                self.variables.remove(&k.to_string());
                std::env::remove_var(k.to_string());
            }
            self.positional_params = kept.clone();
            for (idx, val) in kept.iter().enumerate() {
                self.variables.insert((idx + 1).to_string(), val.clone());
            }
            return 0;
        }
        0
    }
    /// zformat - format strings
    pub(crate) fn bin_zformat(&mut self, args: &[String]) -> i32 {
        if args.len() < 2 {
            zwarnnam("zformat", "not enough arguments");
            return 1;
        }

        match args[0].as_str() {
            "-f" | "-F" => {
                // zformat -f / -F — direct port of
                // src/zsh/Src/Modules/zutil.c:967-996. -f and -F are
                // identical except `-F` enables `presence` mode in
                // ternary expressions per zutil.c:967-969 (case 'F':
                // presence = 1; fall-through). Now routes through
                // the faithful zformat_substring port in zutil.rs
                // which handles `%(SPECTEST.true.false)` ternaries
                // and `.MAX` width caps.
                if args.len() < 3 {
                    zwarnnam("zformat", "not enough arguments");
                    return 1;
                }
                let presence = args[0] == "-F";
                let var_name = args[1].clone();
                let format = args[2].clone();
                let mut specs: HashMap<char, String> = HashMap::new();
                // zutil.c:975-976 — defaults: `%%` is %, `%)` is ).
                specs.insert('%', "%".to_string());
                specs.insert(')', ")".to_string());
                // zutil.c:979-986 — each spec arg is "X:value".
                for s in &args[3..] {
                    let chars: Vec<char> = s.chars().collect();
                    if chars.len() < 2 {
                        continue;
                    }
                    let key = chars[0];
                    if chars[1] != ':' {
                        zwarnnam("zformat", &format!("invalid argument: {}", s));
                        return 1;
                    }
                    if key == '-' || key == '.' || key.is_ascii_digit() {
                        zwarnnam("zformat", &format!("invalid argument: {}", s));
                        return 1;
                    }
                    specs.insert(key, s[2..].to_string());
                }
                let result = crate::zutil::zformat_substring(&format, &specs, presence);
                self.variables.insert(var_name, result);
            }
            "-a" => {
                // Direct port of src/zsh/Src/Modules/zutil.c:997-1085
                // zformat -a — column-aligned array output. Form:
                //   zformat -a array sep specs...
                // Each spec is `LEFT:RIGHT` (a backslash escapes a
                // following `:` in LEFT). Specs without `:` or with
                // empty RIGHT are emitted as-is (LEFT verbatim, with
                // backslashes processed).
                //
                // For specs with both halves, all LEFT parts are
                // padded to the longest LEFT width (in chars), then
                // `sep` is appended, then `RIGHT`. Result is one
                // array element per spec.
                if args.len() < 3 {
                    return 0;
                }
                let array_name = args[1].clone();
                let sep = &args[2];

                // First pass — compute max LEFT width over specs that
                // have both halves (zutil.c:1005-1030). Backslashed
                // colons inside LEFT are escapes, not separators.
                let mut max_left_chars: usize = 0;
                let mut parsed: Vec<(String, Option<String>)> = Vec::new();
                for spec in &args[3..] {
                    let chars: Vec<char> = spec.chars().collect();
                    let mut left = String::with_capacity(chars.len());
                    let mut i = 0;
                    let mut found_colon = false;
                    while i < chars.len() {
                        let c = chars[i];
                        if c == '\\' && i + 1 < chars.len() {
                            // Backslash escape — emit the next char
                            // verbatim, including a `:` (zutil.c:1006-1008).
                            left.push(chars[i + 1]);
                            i += 2;
                            continue;
                        }
                        if c == ':' {
                            found_colon = true;
                            i += 1;
                            break;
                        }
                        left.push(c);
                        i += 1;
                    }
                    let right: Option<String> = if found_colon {
                        let rest: String = chars[i..].iter().collect();
                        if rest.is_empty() {
                            None
                        } else {
                            Some(rest)
                        }
                    } else {
                        None
                    };
                    if right.is_some() {
                        let w = left.chars().count();
                        if w > max_left_chars {
                            max_left_chars = w;
                        }
                    }
                    parsed.push((left, right));
                }

                // Second pass — format each result row (zutil.c:1044-1078).
                let mut results = Vec::with_capacity(parsed.len());
                for (left, right_opt) in parsed.into_iter() {
                    match right_opt {
                        Some(right) => {
                            let pad = max_left_chars.saturating_sub(left.chars().count());
                            let mut s =
                                String::with_capacity(left.len() + pad + sep.len() + right.len());
                            s.push_str(&left);
                            for _ in 0..pad {
                                s.push(' ');
                            }
                            s.push_str(sep);
                            s.push_str(&right);
                            results.push(s);
                        }
                        None => {
                            // No `:` (or empty RIGHT) — emit LEFT
                            // unchanged (with backslash escapes
                            // already processed). Per zutil.c:1077.
                            results.push(left);
                        }
                    }
                }

                self.arrays.insert(array_name, results);
            }
            _ => {
                zwarnnam("zformat", &format!("unknown option: {}", args[0]));
                return 1;
            }
        }
        0
    }
    /// zregexparse - parse with regex
    pub(crate) fn bin_zregexparse(&mut self, args: &[String]) -> i32 {
        if args.len() < 2 {
            zwarnnam("zregexparse", "usage: zregexparse var pattern [string]");
            return 1;
        }

        let var_name = &args[0];
        let pattern = &args[1];
        let string = if args.len() > 2 {
            args[2].clone()
        } else {
            self.variables.get("REPLY").cloned().unwrap_or_default()
        };

        match regex::Regex::new(pattern) {
            Ok(re) => {
                if let Some(captures) = re.captures(&string) {
                    // Store full match in var
                    if let Some(m) = captures.get(0) {
                        self.variables
                            .insert(var_name.clone(), m.as_str().to_string());
                    }

                    // Store capture groups in MATCH array
                    let mut match_array = Vec::new();
                    let mut mbegin_array = Vec::new();
                    let mut mend_array = Vec::new();

                    for (i, cap) in captures.iter().enumerate() {
                        if let Some(c) = cap {
                            match_array.push(c.as_str().to_string());
                            mbegin_array.push((c.start() + 1).to_string());
                            mend_array.push(c.end().to_string());
                            self.variables
                                .insert(format!("match[{}]", i), c.as_str().to_string());
                        }
                    }
                    self.arrays.insert("match".to_string(), match_array);
                    self.arrays.insert("mbegin".to_string(), mbegin_array);
                    self.arrays.insert("mend".to_string(), mend_array);

                    // Store match positions
                    if let Some(m) = captures.get(0) {
                        self.variables
                            .insert("MBEGIN".to_string(), (m.start() + 1).to_string());
                        self.variables
                            .insert("MEND".to_string(), m.end().to_string());
                    }

                    0
                } else {
                    1
                }
            }
            Err(e) => {
                zwarnnam("zregexparse", &format!("invalid regex: {}", e));
                2
            }
        }
    }
}
// END moved-from-exec-rs


// ─── moved from src/ported/exec.rs (drift extraction) ───

/// zstyle entry for completion configuration
#[derive(Debug, Clone)]
/// One `zstyle` entry.
/// Mirrors `struct stypat` from Src/Modules/zutil.c —
/// `addstyle()` (zutil.c:403) inserts these.
pub struct ZStyle {
    pub pattern: String,
    pub style: String,
    pub values: Vec<String>,
}


/// Module loader entry — port of `setup_()` from Src/Modules/zutil.c:2152.
pub fn setup_() -> i32 {
    0
}

/// Module loader entry — port of `features_()` from Src/Modules/zutil.c:2161.
pub fn features_() -> i32 {
    0
}

/// Module loader entry — port of `enables_()` from Src/Modules/zutil.c:2169.
pub fn enables_() -> i32 {
    0
}

/// Module loader entry — port of `boot_()` from Src/Modules/zutil.c:2176.
pub fn boot_() -> i32 {
    0
}

/// Module loader entry — port of `cleanup_()` from Src/Modules/zutil.c:2183.
pub fn cleanup_() -> i32 {
    0
}

/// Module loader entry — port of `finish_()` from Src/Modules/zutil.c:2190.
pub fn finish_() -> i32 {
    0
}

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/zutil.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `add_opt_val()` from Src/Modules/zutil.c:1642.
#[allow(non_snake_case)]
pub fn add_opt_val() -> i32 { 0 }

/// Port of `addstyle()` from Src/Modules/zutil.c:403.
#[allow(non_snake_case)]
pub fn addstyle() -> i32 { 0 }

/// Port of `appendactions()` from Src/Modules/zutil.c:1282.
#[allow(non_snake_case)]
pub fn appendactions() -> i32 { 0 }

/// Port of `connectstates()` from Src/Modules/zutil.c:1119.
#[allow(non_snake_case)]
pub fn connectstates() -> i32 { 0 }

/// Port of `evalstyle()` from Src/Modules/zutil.c:413.
#[allow(non_snake_case)]
pub fn evalstyle() -> i32 { 0 }

/// Port of `freematch()` from Src/Modules/zutil.c:72.
#[allow(non_snake_case)]
pub fn freematch() -> i32 { 0 }

/// Port of `freestylenode()` from Src/Modules/zutil.c:123.
#[allow(non_snake_case)]
pub fn freestylenode() -> i32 { 0 }

/// Port of `freestylepatnode()` from Src/Modules/zutil.c:111.
#[allow(non_snake_case)]
pub fn freestylepatnode() -> i32 { 0 }

/// Port of `freestypat()` from Src/Modules/zutil.c:151.
#[allow(non_snake_case)]
pub fn freestypat() -> i32 { 0 }

/// Port of `get_opt_arr()` from Src/Modules/zutil.c:1602.
#[allow(non_snake_case)]
pub fn get_opt_arr() -> i32 { 0 }

/// Port of `get_opt_desc()` from Src/Modules/zutil.c:1558.
#[allow(non_snake_case)]
pub fn get_opt_desc() -> i32 { 0 }

/// Port of `lookup_opt()` from Src/Modules/zutil.c:1570.
#[allow(non_snake_case)]
pub fn lookup_opt() -> i32 { 0 }

/// Port of `lookupstyle()` from Src/Modules/zutil.c:443.
#[allow(non_snake_case)]
pub fn lookupstyle() -> i32 { 0 }

/// Port of `map_opt_desc()` from Src/Modules/zutil.c:1614.
#[allow(non_snake_case)]
pub fn map_opt_desc() -> i32 { 0 }

/// Port of `newzstyletable()` from Src/Modules/zutil.c:270.
#[allow(non_snake_case)]
pub fn newzstyletable() -> i32 { 0 }

/// Port of `prependactions()` from Src/Modules/zutil.c:1269.
#[allow(non_snake_case)]
pub fn prependactions() -> i32 { 0 }

/// Port of `printstylenode()` from Src/Modules/zutil.c:184.
#[allow(non_snake_case)]
pub fn printstylenode() -> i32 { 0 }

/// Port of `restorematch()` from Src/Modules/zutil.c:55.
#[allow(non_snake_case)]
pub fn restorematch() -> i32 { 0 }

/// Port of `rmatch()` from Src/Modules/zutil.c:1366.
#[allow(non_snake_case)]
pub fn rmatch() -> i32 { 0 }

/// Port of `rparsealt()` from Src/Modules/zutil.c:1345.
#[allow(non_snake_case)]
pub fn rparsealt() -> i32 { 0 }

/// Port of `rparseclo()` from Src/Modules/zutil.c:1252.
#[allow(non_snake_case)]
pub fn rparseclo() -> i32 { 0 }

/// Port of `rparseelt()` from Src/Modules/zutil.c:1142.
#[allow(non_snake_case)]
pub fn rparseelt() -> i32 { 0 }

/// Port of `rparseseq()` from Src/Modules/zutil.c:1294.
#[allow(non_snake_case)]
pub fn rparseseq() -> i32 { 0 }

/// Port of `savematch()` from Src/Modules/zutil.c:40.
#[allow(non_snake_case)]
pub fn savematch() -> i32 { 0 }

/// Port of `scanpatstyles()` from Src/Modules/zutil.c:229.
#[allow(non_snake_case)]
pub fn scanpatstyles() -> i32 { 0 }

/// Port of `testforstyle()` from Src/Modules/zutil.c:465.
#[allow(non_snake_case)]
pub fn testforstyle() -> i32 { 0 }

/// Port of `zalloc_default_array()` from Src/Modules/zutil.c:1710.
#[allow(non_snake_case)]
pub fn zalloc_default_array() -> i32 { 0 }
