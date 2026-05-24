//! Coverage audit: every canonical name in the LSP/registry sets
//! must produce a non-placeholder hover body. Run with
//! `cargo test --test doc_coverage_audit -- --nocapture` for the
//! itemized report; the assert at the end pins the count to 0.

use zsh::extensions::lsp::lookup_doc;

/// A doc body is a "placeholder" when it has no meaningful content
/// beyond the bold-heading line — i.e. nothing after the blank-line
/// separator, or just the `_see man …_` stub from lookup_doc's
/// last-ditch fallback.
fn is_placeholder(s: &str) -> bool {
    if s.is_empty() { return true; }
    match s.split_once("\n\n") {
        Some((_, body)) => body.is_empty() || body.starts_with("_see "),
        None => true,
    }
}

// ── Canonical registries ────────────────────────────────────────────
//
// These match what the IntelliJ tool window's reflection panel lists.
// Audit script source: src/extensions/lsp.rs::dump_reflection_json.

const KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi",
    "for", "foreach", "while", "until", "do", "done",
    "case", "esac", "select", "repeat",
    "function", "local", "typeset", "declare", "export",
    "readonly", "integer", "float", "private",
    "break", "continue", "return", "exit",
    "in", "time", "coproc",
    "always", "nocorrect", "noglob",
];

const SPECIAL_VARS: &[&str] = &[
    "$0", "$?", "$!", "$$", "$#", "$*", "$@", "$-", "$_",
    "$PATH", "$HOME", "$USER", "$PWD", "$OLDPWD",
    "$ZSH_VERSION", "$RANDOM", "$LINENO", "$SECONDS",
    "$EPOCHSECONDS", "$EPOCHREALTIME",
    "$fpath", "$path", "$argv", "$pipestatus",
    "$IFS", "$PS1", "$PS2", "$PS3", "$PS4", "$RPROMPT",
    "$HISTSIZE", "$SAVEHIST", "$HISTFILE",
    "$LANG", "$LC_ALL", "$LC_COLLATE", "$LC_CTYPE",
    "$TERM", "$SHELL", "$EDITOR", "$VISUAL",
];

fn audit(label: &str, names: &[&'static str]) -> Vec<&'static str> {
    let mut missing = Vec::new();
    for &n in names {
        if is_placeholder(&lookup_doc(n)) {
            missing.push(n);
        }
    }
    eprintln!(
        "{:>12}: {}/{} covered, {} placeholder",
        label,
        names.len() - missing.len(),
        names.len(),
        missing.len(),
    );
    for n in &missing { eprintln!("           - {}", n); }
    missing
}

#[test]
fn every_keyword_has_real_doc() {
    let m = audit("keywords", KEYWORDS);
    assert!(m.is_empty(), "{} keywords have placeholder docs: {:?}", m.len(), m);
}

#[test]
fn every_special_var_has_real_doc() {
    let m = audit("specials", SPECIAL_VARS);
    assert!(m.is_empty(), "{} special vars have placeholder docs: {:?}", m.len(), m);
}

#[test]
fn every_canonical_option_has_real_doc() {
    use zsh::ported::options::ZSH_OPTIONS_SET;
    let mut names: Vec<&str> = ZSH_OPTIONS_SET.iter().copied().collect();
    names.sort();
    let m = audit("options", &names);
    assert!(m.is_empty(), "{} options have placeholder docs: {:?}", m.len(), m);
}

#[test]
fn every_canonical_builtin_has_real_doc() {
    use zsh::ported::builtin::BUILTINS;
    let mut names: Vec<String> = BUILTINS.iter().map(|b| b.node.nam.clone()).collect();
    names.sort();
    names.dedup();
    let mut missing = Vec::new();
    for n in &names {
        if is_placeholder(&lookup_doc(n)) {
            missing.push(n.clone());
        }
    }
    eprintln!(
        "{:>12}: {}/{} covered, {} placeholder",
        "builtins",
        names.len() - missing.len(),
        names.len(),
        missing.len(),
    );
    for n in &missing { eprintln!("           - {}", n); }
    assert!(missing.is_empty(), "{} builtins have placeholder docs: {:?}", missing.len(), missing);
}
