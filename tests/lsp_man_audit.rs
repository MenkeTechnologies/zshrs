//! LSP per-builtin flag coverage audit against canonical `man zshall`.
//!
//! Truth source: bundled `tests/data/zsh{builtins,modules,zle}.man.txt` —
//! literal `man -P cat <page> | col -b` output. The parser scans each
//! page for `^       <builtin> [ ... ]` signature lines, extracts every
//! short-flag letter mentioned in bracketed `[ -abc ]` / `[ -X arg ]`
//! groups, then asserts `lsp::extract_builtin_flags(name)` (reached
//! via `zsh::extensions::lsp::lookup_builtin_flag_docs_override` for
//! the hand-curated entries, and via the existing scrapers for the
//! rest) surfaces every letter.
//!
//! When the test fails, the eprintln output lists every missing
//! `(builtin, -X)` pair so the next port iteration knows precisely
//! which entry to add to `BUILTIN_FLAG_DOCS_OVERRIDE`.
//!
//! Refresh the bundled fixtures with:
//!   for p in zshbuiltins zshmodules zshzle; do
//!     man -P cat "$p" | col -b > tests/data/"$p".man.txt
//!   done

use std::collections::{BTreeMap, BTreeSet};

const FIXTURE_DIR: &str = "tests/data";

/// Parse a man-text fixture into `{ builtin_name → set-of-flag-letters }`.
///
/// Pattern: signature lines start at column 7 (`^       <name>`),
/// where `<name>` is an ASCII identifier optionally followed by
/// underscores. Flag groups are `[ -<letters>[ arg] ]` blocks (either
/// compressed-letter `[ -abc ]` form or `[ -X arg ]` single-letter
/// form, separated by `|` in choice groups like `[ -a | -b ]`).
fn parse_signatures(text: &str) -> BTreeMap<String, BTreeSet<char>> {
    let mut out: BTreeMap<String, BTreeSet<char>> = BTreeMap::new();
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        // Signature lines: exactly 7 leading spaces, then identifier.
        let bytes = line.as_bytes();
        if bytes.len() < 8 || &bytes[..7] != b"       " {
            continue;
        }
        let rest = &line[7..];
        // First whitespace-delimited token is the builtin name.
        let (name, after) = match rest.split_once([' ', '\t']) {
            Some(p) => p,
            None => continue,
        };
        // Name must start with alpha / `_` and contain only ident chars.
        if !name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            continue;
        }
        if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        // Collect signature lines: this line + any continuation lines
        // that start with at least 13 spaces (man indents continuations
        // further than the signature). Stop at the first blank line OR
        // the first non-blank, non-continuation line.
        let mut sig = after.to_string();
        for cont in lines.iter().skip(i + 1) {
            if cont.trim().is_empty() {
                break;
            }
            // Continuation lines start with >=12 spaces (man indents at 13+).
            if !cont.starts_with("            ") {
                break;
            }
            sig.push(' ');
            sig.push_str(cont.trim());
            if sig.len() > 2000 {
                break;
            }
        }
        // Find every `[ ... ]` group; inside each, find `-<letters>`.
        let entry = out.entry(name.to_string()).or_default();
        for letter in extract_flag_letters(&sig) {
            entry.insert(letter);
        }
    }
    // Drop entries with zero flags — those aren't auditable.
    out.retain(|_, v| !v.is_empty());
    out
}

/// Walk `sig` for `[ ... ]` groups; inside each, extract option
/// letters from `-<letters>` runs (skipping `{+|-}` typeset-style
/// prefixes which represent any-of toggle states, and skipping
/// `[ name ... ]` style positional placeholders that don't start
/// with `-`).
fn extract_flag_letters(sig: &str) -> Vec<char> {
    let mut out = Vec::new();
    let bytes = sig.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'[' {
            i += 1;
            continue;
        }
        // Find matching `]` (bracket nesting is rare but possible).
        let mut depth = 1;
        let mut j = i + 1;
        while j < bytes.len() && depth > 0 {
            match bytes[j] {
                b'[' => depth += 1,
                b']' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                break;
            }
            j += 1;
        }
        if depth != 0 {
            break;
        }
        let inner = &sig[i + 1..j];
        for letter in scan_option_letters(inner) {
            out.push(letter);
        }
        i = j + 1;
    }
    out
}

/// Inside a bracket group, find `-<letters>` runs.
fn scan_option_letters(s: &str) -> Vec<char> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // Look for `-` preceded by start-of-string, whitespace, `|`, or `}` (the
        // typeset `{+|-}` mode marker).
        if bytes[i] == b'-' {
            let prev = if i == 0 { b' ' } else { bytes[i - 1] };
            let is_word_start = prev == b' ' || prev == b'\t' || prev == b'|' || prev == b'}';
            if is_word_start && i + 1 < bytes.len() {
                let next = bytes[i + 1];
                // Skip `--` (separator), `-/` (path-style), `-1`..`-9`
                // letters that ARE actual flag chars.
                if next == b'-' {
                    i += 2;
                    continue;
                }
                if next.is_ascii_alphabetic() || next.is_ascii_digit() || next == b'/' {
                    // Read run of flag-letter chars.
                    let mut k = i + 1;
                    while k < bytes.len() && (bytes[k].is_ascii_alphanumeric() || bytes[k] == b'/')
                    {
                        out.push(bytes[k] as char);
                        k += 1;
                    }
                    i = k;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

fn load_fixture(name: &str) -> String {
    let path = format!("{FIXTURE_DIR}/{name}.man.txt");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("missing fixture {path}: {e}"))
}

#[test]
fn audit_lsp_builtin_flags_against_man_zshall() {
    // Combine all three man pages — covers core builtins
    // (zshbuiltins), module builtins (zshmodules), zle (zshzle).
    let texts: Vec<String> = ["zshbuiltins", "zshmodules", "zshzle"]
        .iter()
        .map(|p| load_fixture(p))
        .collect();
    let mut canonical: BTreeMap<String, BTreeSet<char>> = BTreeMap::new();
    for text in &texts {
        for (k, v) in parse_signatures(text) {
            canonical.entry(k).or_default().extend(v);
        }
    }

    // Restrict the audit to names that BOTH the man pages document
    // AND zshrs has C `optstr` for. Names that exist only in the
    // man (e.g. `case`, `function` reserved words, `redirect`) or
    // only in C (zsh-extension testing builtins) skew the report.
    let runtime_names: BTreeSet<String> = zsh::ported::builtin::BUILTINS
        .iter()
        .map(|b| b.node.nam.clone())
        .collect();
    canonical.retain(|name, _| runtime_names.contains(name));

    // Walk each remaining builtin and compare canonical letters to
    // what the LSP returns.
    let mut total_audited = 0usize;
    let mut total_letters = 0usize;
    let mut missing: Vec<(String, char)> = Vec::new();
    for (name, letters) in &canonical {
        let lsp_flags = zsh::extensions::lsp::extract_builtin_flags_for_test(name);
        let lsp_letters: BTreeSet<char> = lsp_flags
            .iter()
            .filter_map(|(f, _)| f.strip_prefix('-').and_then(|s| s.chars().next()))
            .collect();
        total_audited += 1;
        for letter in letters {
            total_letters += 1;
            if !lsp_letters.contains(letter) {
                missing.push((name.clone(), *letter));
            }
        }
    }

    let pct = 100.0 * missing.len() as f64 / total_letters.max(1) as f64;
    eprintln!(
        "man-zshall audit: {} builtins audited, {} canonical option letters, \
         {} missing from LSP ({:.1}%)",
        total_audited,
        total_letters,
        missing.len(),
        pct,
    );
    // Group by builtin for readability.
    let mut by_builtin: BTreeMap<String, Vec<char>> = BTreeMap::new();
    for (n, c) in &missing {
        by_builtin.entry(n.clone()).or_default().push(*c);
    }
    for (n, mut flags) in by_builtin {
        flags.sort();
        let joined: String = flags
            .iter()
            .map(|c| format!("-{c}"))
            .collect::<Vec<_>>()
            .join(" ");
        eprintln!("  MISSING  {n:<14} {joined}");
    }
    // Hard contract: 0% missing. Every option letter documented in
    // `man zshall` for a runtime-known builtin MUST surface through
    // the LSP `extract_builtin_flags` path (body scraper + Tier 3
    // override merge). New gaps fail this test with a precise
    // (builtin, -X) list.
    assert_eq!(
        missing.len(),
        0,
        "LSP missing {} canonical option letters vs `man zshall` — see eprintln above",
        missing.len(),
    );
}
