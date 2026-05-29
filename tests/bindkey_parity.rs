//! bindkey parity tests against the brew/system zsh.
//!
//! Compares `bindkey -M <km>` listing output byte-for-byte against
//! the upstream zsh binary. Catches regressions in the
//! `bin_bindkey` → `scankeymap` → `scanbindlist` → `bindlistout`
//! chain — listing format, range-collapsing, undefined-key skip,
//! `bindztrdup` escape, `dquotedztrdup` quoting.
#![allow(non_snake_case)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

fn zsh_path() -> &'static str {
    if Path::new("/opt/homebrew/bin/zsh").exists() {
        "/opt/homebrew/bin/zsh"
    } else if Path::new("/usr/local/bin/zsh").exists() {
        "/usr/local/bin/zsh"
    } else {
        "/bin/zsh"
    }
}

fn zsh_available() -> bool {
    Command::new(zsh_path())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct R {
    stdout: String,
    exit: i32,
}

fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .env_remove("EDITOR")
        .env_remove("VISUAL")
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

fn run_zshrs(s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .env_remove("ZSHRS_CACHE")
        .env_remove("EDITOR")
        .env_remove("VISUAL")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

fn assert_parity(s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(s);
    let r = run_zshrs(s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n  {s}\n--- zsh ---\n{}\n--- zshrs ---\n{}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit, "exit divergence on `{s}`");
}

/// Substring-containment check — for cases where zsh's version differs
/// from our forked source (`tests/data/zsh*`). The full listing may
/// drift, but specific bindings should still be present in both.
fn assert_contains_in_both(s: &str, needle: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(s);
    let r = run_zshrs(s);
    assert!(
        z.stdout.contains(needle),
        "zsh stdout missing `{needle}` for `{s}`:\n{}",
        z.stdout,
    );
    assert!(
        r.stdout.contains(needle),
        "zshrs stdout missing `{needle}` for `{s}`:\n{}",
        r.stdout,
    );
}

#[test]
fn bindkey_dash_M_emacs_byte_identical() {
    assert_parity("bindkey -M emacs");
}

#[test]
fn bindkey_dash_M_viins_byte_identical() {
    assert_parity("bindkey -M viins");
}

#[test]
fn bindkey_dash_l_lists_keymap_names_substring() {
    // `bindkey -l` lists keymap names — lex-sorted, one per line.
    // We can't byte-compare the full list because zsh 5.9 ships
    // extra keymaps (`command`, `isearch`, `viopp`, `visual`) that
    // our forked source (5.9.0.3-test) doesn't yet have via the
    // C default_bindings table. The format is what's pinned here:
    // lex-sorted names, one per line, `.safe` first.
    if !zsh_available() {
        return;
    }
    let r = run_zshrs("bindkey -l");
    assert!(r.stdout.starts_with(".safe\n"));
    for must in [".safe", "emacs", "main", "vicmd", "viins"] {
        assert!(
            r.stdout.lines().any(|l| l == must),
            "zshrs `bindkey -l` missing `{must}`:\n{}",
            r.stdout,
        );
    }
}

#[test]
fn bindkey_dash_lL_emits_create_commands_substring() {
    // `bindkey -lL` emits `bindkey -N <name>` / `bindkey -A <pri> <alias>`
    // lines per `scanlistmaps` in `zle_keymap.c:856` (BS_LIST path).
    // Skips `.safe` per c:864. We assert the format of the lines we
    // know are present — full equality breaks on the 5.9 vs forked
    // keymap-set delta documented above.
    if !zsh_available() {
        return;
    }
    let r = run_zshrs("bindkey -lL");
    assert!(
        !r.stdout.contains(".safe"),
        "scanlistmaps should skip .safe"
    );
    for must in [
        "bindkey -N emacs",
        "bindkey -N vicmd",
        "bindkey -N viins",
        "bindkey -A emacs main",
    ] {
        assert!(
            r.stdout.lines().any(|l| l == must),
            "zshrs `bindkey -lL` missing `{must}`:\n{}",
            r.stdout,
        );
    }
}

#[test]
fn bindkey_no_args_lists_main() {
    // Plain `bindkey` lists the "main" keymap. With EDITOR/VISUAL
    // unset, main → emacs in both zsh and zshrs.
    assert_parity("bindkey");
}

#[test]
fn bindkey_viins_caret_A_is_self_insert() {
    // ^A in viins (VIINSBIND[1] = z_selfinsert) — the bug this test
    // catches: `bindkey -M viins` previously returned emacs bindings
    // because the `M` opt was missing `:` arg marker, so the parser
    // consumed "viins" as a positional and fell back to "main".
    assert_contains_in_both("bindkey -M viins", r#""^A"-"^C" self-insert"#);
}

#[test]
fn bindkey_emacs_caret_A_is_beginning_of_line() {
    assert_contains_in_both("bindkey -M emacs", r#""^A" beginning-of-line"#);
}

#[test]
fn bindkey_emacs_tab_is_expand_or_complete() {
    // ^I (Tab) in emacs (EMACSBIND[9] = z_expandorcomplete).
    assert_contains_in_both("bindkey -M emacs", r#""^I" expand-or-complete"#);
}

#[test]
fn bindkey_viins_tab_is_expand_or_complete() {
    // ^I (Tab) in viins (VIINSBIND[9] = z_expandorcomplete) — same
    // default as emacs per Src/Zle/zle_bindings.c:265.
    assert_contains_in_both("bindkey -M viins", r#""^I" expand-or-complete"#);
}

#[test]
fn bindkey_unknown_keymap_errors_with_status_1() {
    if !zsh_available() {
        return;
    }
    let z = run_zsh("bindkey -M nosuch_zzz");
    let r = run_zshrs("bindkey -M nosuch_zzz");
    assert_eq!(z.exit, 1, "zsh should exit 1 on unknown keymap");
    assert_eq!(r.exit, 1, "zshrs should exit 1 on unknown keymap");
}

#[test]
fn bindkey_dash_M_emacs_dash_L_emits_bindkey_M_prefix() {
    // `bindkey -ML emacs` outputs each line as `bindkey -M emacs "<seq>" <cmd>`
    // per bindlistout c:1188-1197. Tests the BS_LIST format path.
    assert_contains_in_both(
        "bindkey -M emacs -L",
        r#"bindkey -M emacs "^A" beginning-of-line"#,
    );
}

#[test]
fn bindkey_dash_a_vicmd_dash_L_emits_dash_a_prefix() {
    // `bindkey -a -L` (= -M vicmd -L) outputs lines as `bindkey -a ...`
    // per bindlistout c:1191.
    assert_contains_in_both("bindkey -a -L", r#"bindkey -a"#);
}
