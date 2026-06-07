//! Diagnostic dump: run a handful of `%`-escape prompt sequences
//! through `expand_prompt` and print the byte output to stderr.
//!
//! Was `ported::prompt::tests::dump_prompt_escapes` but it has no
//! assertions — it's a developer-facing trace dump for inspecting
//! what `%B`, `%F{red}`, `%{ABCD%}`, etc. produce after expansion.
//! Moved out of `cargo test` so it doesn't surface as a permanent
//! "ignored" item.
//!
//! Usage: `cargo run --example dump_prompt_escapes`

use zsh::prompt::expand_prompt;

fn main() {
    // No global_state_lock — this is a standalone example, not a
    // parallel test; `test_util` is `#[cfg(test)]` and isn't visible
    // from the `examples/` build. Stamp the same option/init state
    // that `test_util::global_state_lock` would normally provide so
    // `%`-escape dispatch fires.
    zsh::ported::options::opt_state_set("exec", true);
    zsh::ported::options::opt_state_set("promptpercent", true);
    zsh::ported::options::opt_state_set("promptbang", true);
    zsh::ported::utils::inittyptab();
    for s in &["%B", "%b", "%F{red}", "%f", "%{ABCD%}", "%{ABCD%}xyz"] {
        let out = expand_prompt(s);
        eprintln!("expand({s:?}) = {out:?}");
    }
}
