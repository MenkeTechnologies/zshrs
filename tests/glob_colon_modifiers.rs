//! Glob colon-modifier regression — verifies `:t :h :r :e :s/X/Y/`
//! transform matched paths before they're returned. Pre-fix bug:
//! qualifiers were parsed into `QualifierSet.colon_mods` but the
//! field was never read, so `*.toml(:t)` returned `./Cargo.toml`
//! instead of `Cargo.toml`. Direct port of zsh/Src/subst.c:4531
//! `modify()` and the rem* helpers in zsh/Src/hist.c:2056-2186.
//!
//! Reference behavior captured from upstream zsh:
//!   $ zsh -c 'print -l *.toml(:t)' → Cargo.toml
//!   $ zsh -c 'print -l *.toml(:e)' → toml
//!   $ zsh -c 'print -l *.toml(:r)' → Cargo

use std::fs;
use tempfile::TempDir;

fn set_opts() {
    use zsh::ported::options::opt_state_set;
    opt_state_set("nullglob", true);
    opt_state_set("dotglob", false);
    opt_state_set("globdots", false);
    opt_state_set("extendedglob", true);
    opt_state_set("caseglob", true);
    opt_state_set("nocaseglob", false);
    opt_state_set("bareglobqual", true);
}

fn setup() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::write(root.join("Cargo.toml"), b"").unwrap();
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("docs/AI_PRIMITIVES.md"), b"").unwrap();
    tmp
}

#[test]
fn colon_t_returns_basename() {
    let tmp = setup();
    let pattern = format!("{}/*.toml(:t)", tmp.path().display());
    let got = {
        set_opts();
        zsh::glob::glob_path(&pattern)
    };
    assert_eq!(got, vec!["Cargo.toml".to_string()], "pattern={}", pattern);
}

#[test]
fn colon_e_returns_extension() {
    let tmp = setup();
    let pattern = format!("{}/*.toml(:e)", tmp.path().display());
    let got = {
        set_opts();
        zsh::glob::glob_path(&pattern)
    };
    assert_eq!(got, vec!["toml".to_string()], "pattern={}", pattern);
}

#[test]
fn colon_r_strips_extension() {
    let tmp = setup();
    let pattern = format!("{}/*.toml(:r)", tmp.path().display());
    let got = {
        set_opts();
        zsh::glob::glob_path(&pattern)
    };
    let want = format!("{}/Cargo", tmp.path().display());
    assert_eq!(got, vec![want], "pattern={}", pattern);
}

#[test]
fn colon_h_returns_dirname() {
    let tmp = setup();
    let pattern = format!("{}/docs/AI*.md(:h)", tmp.path().display());
    let got = {
        set_opts();
        zsh::glob::glob_path(&pattern)
    };
    let want = format!("{}/docs", tmp.path().display());
    assert_eq!(got, vec![want], "pattern={}", pattern);
}

#[test]
fn colon_s_substitutes_first_match() {
    let tmp = setup();
    let pattern = format!("{}/*.toml(:s/.toml/.zzz/)", tmp.path().display());
    let got = {
        set_opts();
        zsh::glob::glob_path(&pattern)
    };
    let want = format!("{}/Cargo.zzz", tmp.path().display());
    assert_eq!(got, vec![want], "pattern={}", pattern);
}

#[test]
fn chained_modifiers_apply_left_to_right() {
    // `:r:t` strips extension THEN takes basename.
    let tmp = setup();
    let pattern = format!("{}/*.toml(:r:t)", tmp.path().display());
    let got = {
        set_opts();
        zsh::glob::glob_path(&pattern)
    };
    assert_eq!(got, vec!["Cargo".to_string()], "pattern={}", pattern);
}
