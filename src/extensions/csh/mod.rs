//! csh/tcsh source → zsh source translator for `zshrs --csh`.
//!
//! zshrs-original — no zsh C counterpart. csh syntax is not zsh syntax
//! (`set x = 5`, `setenv`, `@ n = 1 + 2`, `if (...) then`/`endif`,
//! `foreach`/`end`, `switch`/`endsw`, `$#x`, `$x[2]`, `>&`, `|&`), so a
//! `--csh` input is rewritten here into zsh text with the same meaning and
//! run by the ordinary engine. The reference is /bin/tcsh (macOS `/bin/csh`
//! is tcsh); every rule here is checked against it.
//!
//! Pure `std`; no crate dependencies, so the tree compiles standalone with
//! `rustc --edition 2021 --test mod.rs`.
//!
//! Layering (each file owns one concern):
//!   * [`lex`]   — logical lines, quote-aware splitting (shared helpers)
//!   * [`words`] — one csh word → zsh word (`$x[2]`, `$#x`, `$?x`, `:h`, …)
//!   * [`expr`]  — csh expressions (`if`/`while`/`@`) → zsh conditions/arith
//!   * [`cmds`]  — one command line (lists, pipes, redirections, builtins)
//!   * [`ctl`]   — control structures with block nesting

pub mod cmds;
pub mod ctl;
pub mod expr;
pub mod lex;
pub mod words;

/// Translate a whole csh script (a file, a `-c` string) to zsh, with tcsh's
/// end-of-input behaviour: a block left open still runs its body, and an
/// error is raised only where tcsh has to skip over the missing closer.
/// Errors carry the csh-side message text real tcsh prints
/// (`if: Expression Syntax.`, `Too many ('s`, …).
pub fn translate(src: &str) -> Result<String, String> {
    run(src, ctl::Translator::finish_eof)
}

/// Translate input that may still be arriving (a prompt, stdin). A block
/// left open is an `Err` whose text ends in `not found.`, which the caller
/// treats as "keep reading lines".
pub fn translate_partial(src: &str) -> Result<String, String> {
    run(src, ctl::Translator::finish)
}

fn run(
    src: &str,
    finish: fn(&mut ctl::Translator, &mut String) -> Result<(), String>,
) -> Result<String, String> {
    let mut t = ctl::Translator::new();
    let mut out = String::new();
    for line in lex::logical_lines(src) {
        t.feed(&line, &mut out)?;
    }
    finish(&mut t, &mut out)?;
    Ok(out)
}
