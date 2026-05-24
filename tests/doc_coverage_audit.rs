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

// Canonical reserved-word inventory — same source as `dump_reflection_json`
// / `dump_reference_html`'s Keywords tab. All 31 entries from the
// ported `Src/hashtable.c:1076-1108` reswds[] table, matching the
// `man zshmisc` "Reserved Words" section (`Doc/Zsh/grammar.yo:501-504`).
fn canonical_keywords() -> Vec<&'static str> {
    zsh::ported::hashtable::RESWDS
        .iter()
        .map(|(n, _)| *n)
        .collect()
}

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
    let kws = canonical_keywords();
    let m = audit("keywords", &kws);
    assert!(m.is_empty(), "{} keywords have placeholder docs: {:?}", m.len(), m);
}

#[test]
fn keywords_inventory_matches_man_zshmisc_reserved_words() {
    // Pin: the keyword inventory must mirror `man zshmisc` "Reserved
    // Words" (`Doc/Zsh/grammar.yo:501-504`) verbatim. All 31 entries
    // including the declarers (`declare` / `export` / `float` /
    // `integer` / `local` / `readonly` / `typeset`) — those are
    // reserved AND also builtins; both classifications apply.
    let kws: std::collections::BTreeSet<&str> = canonical_keywords().into_iter().collect();
    let upstream: std::collections::BTreeSet<&str> = [
        "!", "[[", "{", "}",
        "case", "coproc", "declare", "do", "done", "elif", "else",
        "end", "esac", "export", "fi", "float", "for", "foreach",
        "function", "if", "integer", "local", "nocorrect", "readonly",
        "repeat", "select", "then", "time", "typeset", "until", "while",
    ]
    .into_iter()
    .collect();
    let only_in_zshrs: Vec<_> = kws.difference(&upstream).collect();
    let only_in_upstream: Vec<_> = upstream.difference(&kws).collect();
    assert!(
        only_in_zshrs.is_empty() && only_in_upstream.is_empty(),
        "keyword inventory drift from man zshmisc:\n\
         only in zshrs: {:?}\n\
         only in upstream: {:?}",
        only_in_zshrs,
        only_in_upstream,
    );
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

#[test]
fn every_canonical_extension_has_real_doc() {
    // Union of in-process ext builtins and daemon-backed `z*` builtins
    // — same source the IntelliJ Extensions tab inventories.
    let mut names: Vec<&'static str> = zsh::ext_builtins::EXT_BUILTIN_NAMES
        .iter()
        .copied()
        .chain(zsh::daemon::builtins::ZSHRS_BUILTIN_NAMES.iter().copied())
        .collect();
    names.sort();
    names.dedup();
    let m = audit("extensions", &names);
    assert!(m.is_empty(), "{} extension builtins have placeholder docs: {:?}", m.len(), m);
}

#[test]
fn every_operator_has_real_doc() {
    // The operator inventory IS the doc table — they're the same const,
    // so this can't fail by construction. Test exists as a coverage
    // gate: future additions to OPERATOR_DOCS run through `is_placeholder`
    // and catch any one-liner that forgets the body.
    let names: Vec<&'static str> = zsh::lsp::all_canonical_names()
        .into_iter()
        .filter(|n| matches!(zsh::lsp::lookup_doc(n).split_once("\n\n"), Some((h, _)) if h.contains("operator")))
        .collect::<Vec<_>>()
        // leak each name to get a 'static reference for `audit`
        .into_iter()
        .map(|s| Box::leak(s.into_boxed_str()) as &'static str)
        .collect();
    let m = audit("operators", &names);
    assert!(m.is_empty(), "{} operators have placeholder docs: {:?}", m.len(), m);
}

#[test]
fn every_compsys_fn_has_real_doc() {
    // Rust-native compsys functions (`compsys::COMPSYS_FN_NAMES`).
    // Most resolve via the yodl-derived BUILTIN_DOCS table (compsys.yo
    // / compwid.yo) — anything missing needs a hand fallback.
    let mut names: Vec<&'static str> = compsys::COMPSYS_FN_NAMES.iter().copied().collect();
    names.sort();
    let m = audit("compsys", &names);
    assert!(m.is_empty(), "{} compsys fns have placeholder docs: {:?}", m.len(), m);
}
