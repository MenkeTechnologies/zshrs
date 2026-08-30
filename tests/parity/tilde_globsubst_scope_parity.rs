//! `${~spec}` inside a PATTERN operand must not glob the ENCLOSING expansion.
//!
//! C declares the GLOB_SUBST switch as a paramsubst LOCAL:
//!
//! ```c
//! /* Src/subst.c:1671 */
//! int globsubst = isset(GLOBSUBST);
//! ...
//! /* Src/subst.c:2596-2602 */
//! } else if (c == '~' || c == Tilde) {
//!     /* GLOB_SUBST (forced) on or off (doubled) */
//!     if ((c = *++s) == '~' || c == Tilde) {
//!         globsubst = 0;
//!         s++;
//!     } else
//!         globsubst = 2;
//! }
//! ```
//!
//! The pattern operand of `#`/`##`/`%`/`%%`/`/`/`//`/`:/`/`:#` is expanded by
//! `singsub(&s)` (c:Src/subst.c:3412), which re-enters `paramsubst`. Because
//! `globsubst` is that call's own local, a `~` in the pattern tokenizes only the
//! value the NESTED call splices into the pattern; the enclosing call's
//! `globsubst` — the one `strcatsub(…, globsubst, …)` (c:4352/4397/4436/4475)
//! consults to decide whether its RESULT is offered to filename generation — is
//! untouched.
//!
//! zshrs carries the switch through the GLOBAL option table (a documented
//! deviation, so the compile-emitted glob ops later in the same word pipeline can
//! see it) and unwound it only at the command-dispatch boundary. A nested
//! `${~…}` therefore left GLOB_SUBST on for the rest of the word and the OUTER
//! expansion's own value was tokenized and filename-globbed:
//!
//! ```text
//! cm=X; v='a|b'; print -r -- ${v%%${~cm}*}
//!   zsh   -> a|b
//!   zshrs -> zsh:1: no matches found: a|b
//! ```
//!
//! Named victim: zsh's own `Test/X04zlehighlight.ztst`. Its `zpty_line` helper
//! ends every captured line with
//!
//! ```text
//! print -r -- ${${REPLY%%${~cm}*}##[[:space:]]##}
//! ```
//!
//! and the file's `zle_highlight=( fg_start_code:"CDE|3" … )` puts a `|` in every
//! captured line, so all 20 assertions aborted inside the helper.
//!
//! The rows below come in pairs on purpose:
//!   * "leak" rows pin that the outer value is NOT globbed,
//!   * "still active" rows pin that the nested `~` STILL makes its own spliced
//!     value a live pattern (a fix that merely turns GLOB_SUBST off around the
//!     whole operand passes the first set and breaks the second),
//!   * "genuinely on" rows pin that a real `setopt globsubst` / an OUTER `${~v}`
//!     still globs the result.
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

/// stdout + stderr + exit-status parity against the reference `zsh`.
///
/// stderr is compared too: the whole bug surfaced as a `no matches found:` /
/// `bad pattern:` diagnostic on stderr with an EMPTY stdout, so a stdout-only
/// assertion would have gone green the moment the message changed wording.
fn assert_parity(script: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let z = Command::new(zsh_path())
        .args(["-fc", script])
        .output()
        .expect("invoke zsh");
    let r = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");

    let z_out = String::from_utf8_lossy(&z.stdout).into_owned();
    let r_out = String::from_utf8_lossy(&r.stdout).into_owned();
    assert_eq!(
        z_out, r_out,
        "stdout divergence on script:\n{script}\n--- zsh ---\n{z_out:?}\n--- zshrs ---\n{r_out:?}"
    );
    // Both shells prefix diagnostics with their own argv[0]-derived name and a
    // line number; compare only whether EITHER produced a diagnostic at all.
    let z_err = !z.stderr.is_empty();
    let r_err = !r.stderr.is_empty();
    assert_eq!(
        z_err,
        r_err,
        "stderr divergence on script:\n{script}\n--- zsh ---\n{}\n--- zshrs ---\n{}",
        String::from_utf8_lossy(&z.stderr),
        String::from_utf8_lossy(&r.stderr)
    );
    assert_eq!(
        z.status.code().unwrap_or(-1),
        r.status.code().unwrap_or(-1),
        "exit divergence on script:\n{script}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// 1. The leak: a nested `${~…}` must not glob the enclosing result.
// ═══════════════════════════════════════════════════════════════════════════

mod nested_tilde_does_not_glob_the_outer_value {
    use super::assert_parity;

    /// The exact minimisation of X04zlehighlight's `zpty_line`.
    #[test]
    fn longest_suffix_strip() {
        assert_parity(r#"cm=X; v='a|b'; print -r -- ${v%%${~cm}*}"#);
    }

    #[test]
    fn shortest_suffix_strip() {
        assert_parity(r#"cm=X; v='a|b'; print -r -- ${v%${~cm}*}"#);
    }

    #[test]
    fn shortest_prefix_strip() {
        assert_parity(r#"cm=X; v='a|b'; print -r -- ${v#${~cm}}"#);
    }

    #[test]
    fn longest_prefix_strip() {
        assert_parity(r#"cm=X; v='a|b'; print -r -- ${v##${~cm}*}"#);
    }

    #[test]
    fn single_replace() {
        assert_parity(r#"cm=X; v='a|b'; print -r -- ${v/${~cm}/z}"#);
    }

    #[test]
    fn global_replace() {
        assert_parity(r#"cm=X; v='a|b'; print -r -- ${v//${~cm}/z}"#);
    }

    #[test]
    fn whole_element_replace() {
        assert_parity(r#"cm=X; v='a|b'; print -r -- ${v:/${~cm}/z}"#);
    }

    #[test]
    fn element_filter() {
        assert_parity(r#"cm=X; v='a|b'; print -r -- ${v:#${~cm}}"#);
    }

    /// `*` rather than `|`: the leak globbed on any metacharacter the value
    /// happened to carry, not just alternation.
    #[test]
    fn star_in_the_outer_value() {
        assert_parity(r#"cm=X; v='a*b'; print -r -- ${v%%${~cm}*}"#);
    }

    /// X04's real shape — the stripped value feeds a SECOND expansion.
    #[test]
    fn nested_outer_expansion_as_in_X04() {
        assert_parity(
            r#"setopt extendedglob; cm=X; v='a|b'; print -r -- ${${v%%${~cm}*}##[[:space:]]##}"#,
        );
    }

    /// The leak also reached text CONCATENATED after the expansion, because the
    /// option stayed on for the rest of the word.
    #[test]
    fn leak_reached_the_rest_of_the_word() {
        assert_parity(r#"cm=X; v='a|b'; print -r -- ${v%%${~cm}*}${v}"#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 2. The nested `~` must still do its job inside the pattern.
// ═══════════════════════════════════════════════════════════════════════════

mod nested_tilde_still_activates_its_own_splice {
    use super::assert_parity;

    /// c:Src/subst.c:1669 — GLOB_SUBST shtokenizes the spliced value, so the
    /// class matches a digit and the strip runs.
    #[test]
    fn character_class_from_the_splice_matches() {
        assert_parity(r#"cm='[0-9]'; v=a1b; print -r -- ${v#*${~cm}}"#);
    }

    /// Same splice WITHOUT the `~`: literal `[0-9]`, no strip. Pins that the
    /// scope restore did not simply turn the feature off.
    #[test]
    fn without_tilde_the_splice_stays_literal() {
        assert_parity(r#"cm='[0-9]'; v=a1b; print -r -- ${v#*$cm}"#);
    }

    #[test]
    fn alternation_from_the_splice_matches() {
        assert_parity(r#"cm='a|b'; v=bcd; print -r -- ${v#(${~cm})}"#);
    }

    #[test]
    fn replace_pattern_from_the_splice_matches() {
        assert_parity(r#"cm='[0-9]'; v=a1b; print -r -- ${v/${~cm}/Z}"#);
    }

    #[test]
    fn without_tilde_the_replace_pattern_stays_literal() {
        assert_parity(r#"cm='[0-9]'; v=a1b; print -r -- ${v/$cm/Z}"#);
    }

    /// `${~~…}` — the doubled form forces GLOB_SUBST OFF for the nested value
    /// (c:2598 `globsubst = 0`), and that must not leak either.
    #[test]
    fn doubled_tilde_forces_it_off_without_leaking() {
        assert_parity(r#"setopt globsubst; cm='[0-9]'; v=a1b; print -r -- ${v#*${~~cm}}"#);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 3. A genuinely-on GLOB_SUBST must still glob the enclosing result.
// ═══════════════════════════════════════════════════════════════════════════

mod globsubst_that_is_really_on_still_globs {
    use super::assert_parity;

    /// `setopt globsubst` is the user's own option, not a carrier flip — the
    /// result IS a pattern and filename generation fails on it.
    #[test]
    fn option_set_by_the_user() {
        assert_parity(r#"setopt globsubst; v='a|b'; print -r -- ${v%%X*}"#);
    }

    /// The OUTER expansion's own `~` still marks the outer result.
    #[test]
    fn outer_tilde_marks_the_outer_result() {
        assert_parity(r#"cm=X; v='a|b'; print -r -- ${~v%%${~cm}*}"#);
    }

    /// Outer `~` and inner `~` together: the restore must put back the OUTER's
    /// value, not the user's option.
    #[test]
    fn outer_and_inner_tilde_together_with_a_matching_pattern() {
        assert_parity(r#"cm='[0-9]'; v='a1|b'; print -r -- ${~v#*${~cm}}"#);
    }
}
