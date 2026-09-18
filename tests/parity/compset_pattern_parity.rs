//! `compset -P` / `-S` pattern parity: what an ESCAPED BACKSLASH in the
//! pattern argument means.
//!
//! `bin_compset` hands its pattern argument to `tokenize()` +
//! `remnulargs()` before compiling it (c:Src/Zle/complete.c:1213-1214),
//! and those two are what decide whether a backslash in the argument is
//! a QUOTE of the next character or a literal backslash CHARACTER:
//!
//!   * `zshtokenize` (c:Src/glob.c:3585, flags 0) honours a backslash
//!     only before a character its `ztokens` scan recognises, and it
//!     records that by overwriting the BACKSLASH's own position with
//!     `Bnull` (c:3600-3602 and c:3642-3643), leaving the escaped
//!     character raw.
//!   * `remnulargs` (c:Src/glob.c:3658) then deletes those `Bnull`s.
//!
//! So `\(` collapses to a bare literal `(`, while `\\` leaves a REAL
//! backslash byte behind — and `patcomppiece` has no `case '\\'` at all
//! (c:Src/pattern.c:1579-1601; only `case Bnullkeep` at c:1589 is
//! special), so that byte compiles as ordinary text that only a literal
//! backslash in the word can match.
//!
//! zshrs's pattern normalizer reads a raw `\X` as a QUOTE of X instead
//! (the `\\` arm in `src/ported/pattern.rs`), so the surviving backslash
//! silently vanished and the pattern matched the UNESCAPED text. The
//! live symptom was `_git`'s `__git_format_ref`, which gates its
//! ref-field completion on
//!
//! ```text
//! compset -P '%\\\((\*|)'
//! ```
//!
//! — a pattern meaning `%` + a literal `\` + a literal `(`. Against
//! `$PREFIX='%('` (what `git for-each-ref --format=%(<TAB>` produces,
//! measured identical on both shells) zsh does NOT match and offers the
//! single candidate `%(`; zshrs matched and offered 39 ref fields.
//!
//! Driven through `zle -C`, not `compinit`: `compset` only needs
//! `incompfunc` set, which a completion widget does, and a stock-fpath
//! `compinit` would add half a minute per shell for nothing.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_verdict, sq, DRAIN, OPEN};

/// Drive one `compset -P <pattern>` against one typed word.
///
/// The completer adds exactly one of two candidates, so the candidate
/// TAB inserts IS the verdict — no Return is needed, which matters
/// because the typed words here contain an unbalanced `(` that would
/// leave the shell at a continuation prompt.
///
/// `compadd -U` is required, not cosmetic: without it `compadd` filters
/// every candidate against `$PREFIX`, and neither verdict name begins
/// with the typed `(`, so BOTH branches added nothing and the probe
/// scored `K=no` on the reference shell.
///
/// `ZZMARK` and the `${{all##*ZZMARK}}` strip are load-bearing: the
/// inner shell echoes every setup line back onto the pty, so the setup
/// that DEFINES `ZZMATCHED`/`ZZNOMATCH` puts both names in the
/// transcript before completion has run at all. A verdict reading the
/// whole transcript would score either way round as a match.
fn compset_driver(pattern: &str, typed: &str) -> String {
    let setup = format!(
        r#"_zzc() {{ if compset -P '{pattern}'; then compadd -U -Q - ZZMATCHED; else compadd -U -Q - ZZNOMATCH; fi }}; zle -C zzc complete-word _zzc; bindkey '^I' zzc"#
    );
    let setup_q = sq(&setup);
    format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w 'bindkey -e'
zpty -w w {setup_q}
zpty -w w 'print ZZMARK'
sleep 2
zpty -w -n w 'zzrun {typed}'
sleep 2
zpty -w -n w $'\\t'
sleep 3
{DRAIN}
all=\"${{all##*ZZMARK}}\"
"
    )
}

/// `compset -P '\\\('` is `\` + `(`: a literal backslash followed by a
/// literal paren. The word is a bare `(`, which has no backslash, so it
/// must NOT match.
///
/// This is the `__git_format_ref` gate reduced to one pattern. Before
/// the fix zshrs dropped the literal backslash, compiled the pattern as
/// a bare literal `(`, matched, and took the branch zsh does not.
#[test]
fn an_escaped_backslash_in_a_compset_pattern_needs_a_real_backslash() {
    let driver = format!(
        "{}
if [[ $all == *ZZNOMATCH* && $all != *ZZMATCHED* ]]; then print \"K=yes\"; else print \"K=no\"; fi
",
        compset_driver(r"\\\(", "(")
    );
    assert_same_verdict(
        &driver,
        "K",
        "compset -P '\\\\\\(' refused a word with no literal backslash",
    );
}

/// The control that keeps the case above from being "passed" by a shell
/// that simply stopped matching. One backslash is a QUOTE: `\(` is the
/// literal paren alone, which the bare `(` DOES match. Both shells
/// matched this one before the fix and must still match it after.
#[test]
fn a_single_escape_in_a_compset_pattern_still_quotes_the_paren() {
    let driver = format!(
        "{}
if [[ $all == *ZZMATCHED* && $all != *ZZNOMATCH* ]]; then print \"K=yes\"; else print \"K=no\"; fi
",
        compset_driver(r"\(", "(")
    );
    assert_same_verdict(
        &driver,
        "K",
        "compset -P '\\(' matched the quoted paren",
    );
}
