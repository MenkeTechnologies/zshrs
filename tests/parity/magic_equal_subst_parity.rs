//! `MAGIC_EQUAL_SUBST` parity — pins which `~`/`=` in a `word=value`
//! argument get filename expansion, and, just as importantly, which do
//! NOT because they came out of quotes.
//!
//! C decides with the LEXER TOKENS. `filesub` (`Src/subst.c:667-698`)
//! looks for the `Equals` token, and each `:`-separated component only
//! expands when the character after the colon is the `Tilde`/`Equals`
//! TOKEN. Those tokens exist only for an UNQUOTED character, so quoting
//! any part of `:=` makes it inert — that is the whole rule.
//!
//! The regression these pin: zshrs untokenized the word and then ran
//! `shtokenize` over it again before calling `filesub`, which re-marked
//! every literal `~`/`=` as a token, quoted ones included. With
//! `setopt magic_equal_subst`, `print -r -- --a='b:=c'` then tried an
//! `=` filename expansion on the quoted `=c`, printed `c not found` and
//! DROPPED the word. Real-world damage: `fzf-tab.plugin.zsh` builds
//! `FZF_TAB_COMMAND=( … --height='${FZF_TMUX_HEIGHT:=75%}' … )`, so the
//! plugin errored `75%} not found` at load.
//!
//! Both halves matter, so both are pinned here: the EXPAND cases must
//! keep expanding (a tokens-only fix that over-corrects breaks
//! `--prefix=~/dir`), and the INERT cases must stay literal.
//!
//! Skip pattern: tests no-op silently when `zsh` isn't on PATH.

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

/// Run the body under `setopt magic_equal_subst` in both shells and
/// require identical stdout AND stderr — the divergence this file
/// guards showed up on stderr (`c not found`) with the word silently
/// missing from stdout, so checking only one stream would miss it.
fn assert_magic_parity(body: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let script = format!("setopt magic_equal_subst\n{body}");
    let z = Command::new(zsh_path())
        .args(["-f", "-c", &script])
        .output()
        .expect("invoke zsh");
    let r = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", &script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    let zo = String::from_utf8_lossy(&z.stdout).into_owned();
    let ro = String::from_utf8_lossy(&r.stdout).into_owned();
    let ze = String::from_utf8_lossy(&z.stderr).into_owned();
    let re = String::from_utf8_lossy(&r.stderr).into_owned();
    assert_eq!(
        zo, ro,
        "stdout divergence on:\n{script}\n--- zsh ---\n{zo:?}\n--- zshrs ---\n{ro:?}"
    );
    assert_eq!(
        ze, re,
        "stderr divergence on:\n{script}\n--- zsh ---\n{ze:?}\n--- zshrs ---\n{re:?}"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Unquoted — MUST still expand (guards against over-correcting the fix)
// ═══════════════════════════════════════════════════════════════════════

mod expands {
    use super::*;

    /// c:Src/subst.c:678-683 — the `Equals` token arm: `sub[1] == Tilde`.
    #[test]
    fn tilde_right_after_the_equals() {
        assert_magic_parity("print -r -- --a=~");
    }

    /// c:Src/subst.c:688-699 — the `:`-component loop.
    #[test]
    fn tilde_after_a_colon_component() {
        assert_magic_parity("print -r -- --a=b:~");
    }

    /// The `PATH=dir:dir` shape every rc file uses.
    #[test]
    fn colon_list_of_tildes() {
        assert_magic_parity("print -r -- X=~/bin:~/sbin");
    }

    /// Same shape as a real assignment, then read back.
    #[test]
    fn assignment_then_read_back() {
        assert_magic_parity("X=~/bin:~/sbin; print -r -- $X");
    }

    /// `=cmd` equals-expansion inside a `:`-component.
    #[test]
    fn equals_command_after_a_colon() {
        assert_magic_parity("print -r -- --a=b:=ls");
    }

    /// A `=cmd` that does not resolve: zsh reports `<cmd> not found` and
    /// drops the word. The DIAGNOSTIC is the parity point here.
    #[test]
    fn equals_command_not_found_is_reported() {
        assert_magic_parity("print -r -- --a=b:=zzz_no_such_command_zzz");
    }

    /// c:Src/subst.c:678 — `strchr(*namptr + 1, Equals)` has no
    /// identifier test, so a non-identifier head still counts.
    #[test]
    fn non_identifier_head_still_expands() {
        assert_magic_parity("print -r -- x:y=~/z");
    }

    /// The common `--prefix=~/dir` configure-style shape.
    #[test]
    fn double_dash_option_value() {
        assert_magic_parity("print -r -- --prefix=~/dir");
    }

    /// Colon in the head, before the `=`.
    #[test]
    fn colon_in_the_head() {
        assert_magic_parity("print -r -- ME:a=~/x");
    }

    /// The `=` is unquoted here even though the head is quoted, so this
    /// one DOES expand — the quote must not be read as "inert word".
    #[test]
    fn quoted_head_with_bare_equals_still_expands() {
        assert_magic_parity("print -r -- --a='b:'=c");
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Quoted — MUST stay literal (the regression)
// ═══════════════════════════════════════════════════════════════════════

mod inert_when_quoted {
    use super::*;

    /// Single-quoted value: zsh prints `--a=b:=c`; the broken port
    /// printed `c not found` and dropped the word.
    #[test]
    fn single_quoted_value() {
        assert_magic_parity("print -r -- --a='b:=c'");
    }

    /// Double quotes suppress it identically.
    #[test]
    fn double_quoted_value() {
        assert_magic_parity(r#"print -r -- --a="b:=c""#);
    }

    /// Quoting only the `:=` is enough — the token is what matters, not
    /// the whole word.
    #[test]
    fn only_the_colon_equals_quoted() {
        assert_magic_parity("print -r -- --a=b':='c");
    }

    /// Quoting only the `=` of the `:=` pair.
    #[test]
    fn only_the_equals_quoted() {
        assert_magic_parity("print -r -- --a=b:'='c");
    }

    /// A quoted `~` after a colon must not become `$HOME`.
    #[test]
    fn quoted_tilde_after_a_colon() {
        assert_magic_parity("print -r -- --a='b:~'");
    }

    /// The array literal from `fzf-tab.plugin.zsh:117-128`, reduced.
    /// The single-quoted `${FZF_TMUX_HEIGHT:=75%}` is a LITERAL string
    /// the plugin re-expands later with `${(e)…}`; expanding it here
    /// killed the assignment and took the plugin's load with it.
    #[test]
    fn fzf_tab_command_array_literal() {
        assert_magic_parity(
            r#"A=( --layout=reverse --height='${FZF_TMUX_HEIGHT:=75%}' )
printf '[%s]\n' "${A[@]}""#,
        );
    }

    /// Same element reached through `cmd || NAME=( … )`, which is the
    /// exact shape the plugin uses.
    #[test]
    fn array_literal_on_the_rhs_of_or() {
        assert_magic_parity(
            r#"false || A=( --height='${X:=75%}' )
printf '[%s]\n' "${A[@]}""#,
        );
    }

    /// The literal must survive to the point where `${(e)…}` expands it,
    /// and only THEN become `75%`.
    #[test]
    fn deferred_e_flag_expansion_still_works() {
        assert_magic_parity(
            r#"A=( --height='${FZF_TMUX_HEIGHT:=75%}' )
printf '[%s]\n' "${(e)A[@]}"
printf 'var=[%s]\n' "$FZF_TMUX_HEIGHT""#,
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Option off — nothing in a plain argument may be touched
// ═══════════════════════════════════════════════════════════════════════

mod option_off {
    use super::*;

    fn assert_no_magic(body: &str) {
        if !zsh_available() {
            eprintln!("skip: zsh not found");
            return;
        }
        let script = format!("unsetopt magic_equal_subst\n{body}");
        let z = Command::new(zsh_path())
            .args(["-f", "-c", &script])
            .output()
            .expect("invoke zsh");
        let r = Command::new(zshrs_bin())
            .args(["--zsh", "-f", "-c", &script])
            .env_remove("ZSHRS_CACHE")
            .output()
            .expect("invoke zshrs");
        assert_eq!(
            String::from_utf8_lossy(&z.stdout),
            String::from_utf8_lossy(&r.stdout),
            "stdout divergence on:\n{script}"
        );
        assert_eq!(
            String::from_utf8_lossy(&z.stderr),
            String::from_utf8_lossy(&r.stderr),
            "stderr divergence on:\n{script}"
        );
    }

    #[test]
    fn tilde_in_an_argument_stays_literal() {
        assert_no_magic("print -r -- --a=~/x");
    }

    #[test]
    fn colon_equals_in_an_argument_stays_literal() {
        assert_no_magic("print -r -- --a=b:=c");
    }

    /// A real assignment expands its `~` regardless of the option
    /// (`PREFORK_ASSIGN`, not `PREFORK_TYPESET`).
    #[test]
    fn real_assignment_is_unaffected_by_the_option() {
        assert_no_magic("X=~/bin:~/sbin; print -r -- $X");
    }
}
