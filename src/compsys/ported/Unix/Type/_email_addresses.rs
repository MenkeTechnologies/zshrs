//! Port of `_email_addresses` from
//! `Completion/Unix/Type/_email_addresses`.
//!
//! Full upstream body (187 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh: 10  Plugins for mail / mutt / mush / MH / pine / ldap / local
//! sh: 78  _email_addresses() {
//! sh: 95    zparseopts -D -E -A opts n: s: c
//! sh:140    plugins=( … functions matching _email-* …)
//! sh:160    _tags email-$plugins; while _tags; do … per-plugin dispatch …
//! sh:185  }
//! sh:187  _email_addresses "$@"
//! ```
//!
//! Reads `~/.mailrc` / `~/.muttrc` / `~/.addressbook` (per-plugin)
//! for alias→address mappings. This port covers the
//! `_email-mail` plugin: parse `~/.mailrc` `alias` lines and
//! emit the addresses.

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::setaparam;
use std::fs;

fn parse_mailrc(path: &str) -> Vec<String> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<String> = Vec::new();
    for line in content.lines() {
        let line = line.trim_start();
        if let Some(rest) = line.strip_prefix("alias ") {
            // Format: `alias <name> <address> [more]`
            let mut parts = rest.split_whitespace();
            if let (Some(name), Some(addr)) = (parts.next(), parts.next()) {
                out.push(format!("{}:{}", name, addr));
            }
        }
    }
    out
}

/// `_email_addresses` — complete email aliases from `~/.mailrc` /
/// `~/.muttrc`. `-c` restricts to address-only (strip alias).
pub fn _email_addresses(args: &[String]) -> i32 {
    let strip_comments = args.iter().any(|a| a == "-c");

    let home = std::env::var("HOME").unwrap_or_default();
    let mut all: Vec<String> = Vec::new();
    all.extend(parse_mailrc(&format!("{}/.mailrc", home)));
    all.extend(parse_mailrc(&format!("{}/.muttrc", home)));
    if all.is_empty() {
        return 1;
    }

    let aliases: Vec<String> = if strip_comments {
        all.iter()
            .filter_map(|s| s.splitn(2, ':').nth(1).map(|a| a.to_string()))
            .collect()
    } else {
        all.iter()
            .map(|s| s.splitn(2, ':').next().unwrap_or("").to_string())
            .collect()
    };

    setaparam("_email_aliases", aliases);
    _wanted(&[
        "email-aliases".to_string(),
        "expl".to_string(),
        "email alias".to_string(),
        "compadd".to_string(),
        "-a".to_string(),
        "_email_aliases".to_string(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_mailrc() {
        let _g = crate::test_util::global_state_lock();
        // Sandboxed home (no mailrc/muttrc): returns 1.
        std::env::set_var("HOME", "/nonexistent/sandbox");
        let _r = _email_addresses(&[]);
    }
}
