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
//!
//! Whole-script behaviour handled here and in [`lex::script_lines`]:
//!   * here-document bodies and their terminators are passed through
//!     verbatim (no comment stripping, no continuation joining, blank lines
//!     kept); the terminator is the delimiter word exactly as typed, quotes
//!     included, as in tcsh;
//!   * a line the translator rejects does not discard the lines before it:
//!     [`translate`] returns the earlier lines followed by the error as a
//!     run-time `print -u2; exit 1`, because tcsh has already executed them
//!     when it reports the error;
//!   * a quote does not span lines (only `\<newline>` inside `"…"` does);
//!   * a script piped on stdin is read whole (`emulation_startup::csh_slurp`)
//!     and translated like a file, so `goto`, here-documents and the point
//!     where an error ends the script are the same as for a file; only a
//!     terminal is read line by line.

/// zsh-side helpers the translated output relies on (`cd`/`pushd`/`popd`
/// with tcsh's error text, `printenv`, the command-not-found handler).
/// Installed once, ahead of the first translated input.
pub const PREAMBLE: &str = include_str!("preamble.zsh");

/// Set for `zshrs --csh-translate --source`: the text is a sourced file, so
/// `exit` in it only ends that file (`return` in the `source` function).
static SOURCE_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_source_mode(on: bool) {
    SOURCE_MODE.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn source_mode() -> bool {
    SOURCE_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

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
    let lines = lex::script_lines(src);
    let mut t = ctl::Translator::new();
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if let Err(msg) = t.feed(line, &mut out) {
            // tcsh runs a script a line at a time: everything before the
            // faulty line has already executed when it reports the error.
            return run_lines(&lines[..i], ctl::Translator::finish_eof)
                .map(|mut text| {
                    text.push_str(&ctl::stub(&msg));
                    text.push('\n');
                    text
                })
                .or(Err(msg));
        }
    }
    t.finish_eof(&mut out)?;
    Ok(out)
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
    run_lines(&lex::script_lines(src), finish)
}

fn run_lines(
    lines: &[String],
    finish: fn(&mut ctl::Translator, &mut String) -> Result<(), String>,
) -> Result<String, String> {
    let mut t = ctl::Translator::new();
    let mut out = String::new();
    for line in lines {
        t.feed(line, &mut out)?;
    }
    finish(&mut t, &mut out)?;
    Ok(out)
}
