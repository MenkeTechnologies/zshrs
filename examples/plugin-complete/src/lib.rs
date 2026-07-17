//! Example native zshrs **completion** plugin.
//!
//! Registers a `greet` command and a native (Rust) completion for it. The
//! completion logic — filtering candidates by the current prefix, and
//! offering subcommand-aware options — runs entirely in Rust; zsh's
//! completion system (compsys) invokes it via `compdef`, which the
//! `completions:` section of `declare_plugin!` wires up automatically.
//!
//! Build, then inside zshrs (after `compinit`):
//! ```text
//! zmodload -R .../libgreet.dylib
//! greet <TAB>          # → alice  bob  carol  dave  erin
//! greet --lang <TAB>   # → rust  ruby  python  perl  go
//! ```

use std::os::raw::c_int;
use zshrs_plugin::{declare_plugin, Args, Host};

/// People `greet` knows how to greet. The completion filters these by the
/// current prefix.
const NAMES: &[&str] = &["alice", "bob", "carol", "dave", "erin"];

/// Languages offered after `greet --lang`.
const LANGS: &[&str] = &["rust", "ruby", "python", "perl", "go"];

/// `greet [--lang L] NAME` — print a greeting.
fn greet(host: &Host, args: &Args) -> c_int {
    let rest: Vec<&str> = args.rest().iter().map(String::as_str).collect();
    match rest.as_slice() {
        ["--lang", lang, name] => {
            host.print(&format!("Hello {name}! ({lang} is a fine choice)\n"));
            0
        }
        [name] => {
            host.print(&format!("Hello {name}!\n"));
            0
        }
        _ => {
            host.print("usage: greet [--lang LANG] NAME\n");
            2
        }
    }
}

/// Completion generator for `greet`. Invoked by compsys as
/// `__zshrs_complete_greet $CURRENT $words...`, so:
///   argv[1]        = CURRENT (1-based index of the word being completed)
///   argv[2..]      = the words on the line (words[1] == "greet")
/// It filters the candidate set by the current word's prefix and emits
/// each match with `host.add_match`.
fn greet_complete(host: &Host, args: &Args) -> c_int {
    let a = args.rest();
    // Parse CURRENT (1-based). Bail out gracefully on anything unexpected.
    let current: usize = match a.first().and_then(|s| s.parse().ok()) {
        Some(n) => n,
        None => return 1,
    };
    let words = &a[1..]; // words[0] == "greet"
    // The word being completed (1-based index → 0-based into `words`).
    let cur = current.checked_sub(1).and_then(|i| words.get(i));
    let prefix = cur.map(String::as_str).unwrap_or("");

    // Context: after `--lang`, complete languages; otherwise names.
    let prev = current
        .checked_sub(2)
        .and_then(|i| words.get(i))
        .map(String::as_str);
    let pool: &[&str] = if prev == Some("--lang") { LANGS } else { NAMES };

    for &cand in pool {
        if cand.starts_with(prefix) {
            host.add_match(cand);
        }
    }
    // Offer the --lang flag too when starting a fresh word.
    if prev != Some("--lang") && "--lang".starts_with(prefix) && !prefix.is_empty() {
        host.add_match("--lang");
    }
    0
}

declare_plugin! {
    name: "greet",
    version: "0.1.0",
    builtins:    { "greet" => greet },
    completions: { "greet" => greet_complete },
}
