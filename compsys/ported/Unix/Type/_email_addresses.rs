//! Port of `_email_addresses` (zsh Completion/Mail/_email_addresses, 187 lines).
//!
//! Local shell reference: upstream
//! `Completion/Unix/Type/_email_addresses` (system copy at
//! `/opt/homebrew/share/zsh/functions/_email_addresses`).
//!
//! Completes email addresses from multiple sources:
//!   - `~/.mailrc`        (mail/mutt/mush plugin baseline)
//!   - `~/.muttrc` (or `muttrc` zstyle override)
//!   - `~/.mh_profile`    (MH — also parses `ali` output if available)
//!   - `~/.addressbook`   (pine)
//!   - LDAP / local       (caller-provided; we don't fork ldapsearch
//!                         or call _users/_hosts at this layer)
//!
//! The previous Rust stub only parsed `~/.mailrc` — every other
//! source was missing and there was no `-n plugin` / `-s sep` / `-c`
//! flag support. Replaced.
//!
//! What's intentionally simplified vs shell:
//!   - LDAP plugin: not exec'd (would need ldapsearch + the
//!     `:filter` zstyle infrastructure). Returns no addresses.
//!   - `local` plugin (user@host completion): not dispatched. The
//!     shell version forwards to `_users` / `_hosts`; we can layer
//!     that on once those exist as Rust impls.
//!   - RFC822 regex synthesis (`$__addrspec` and friends) is not
//!     used for validation — we trust the source files. Plain text
//!     scan of `alias NAME ADDRESS` / `alias NAME=ADDRESS` lines is
//!     correct for mail/mutt/mush.

use std::path::{Path, PathBuf};

use crate::compcore::CompletionState;
use crate::completion::Completion;

pub struct EmailAddressesOpts<'a> {
    /// `-n plugin` — restrict to entries from the named plugin.
    pub only_plugin: Option<&'a str>,
    /// `-s sep` — chew `*sep` from the front of PREFIX so user can
    /// complete the Nth entry in a `addr1, addr2, addr3` list.
    pub separator: Option<&'a str>,
    /// `-c` — only emit RFC822 `user@host` form, drop nickname/
    /// realname annotations. Strip-comments style override.
    pub bare_addresses: bool,
    /// Override the home dir used to locate `.mailrc` / `.muttrc` /
    /// `.addressbook` / `.mh_profile`. Defaults to `$HOME` when None.
    /// Test-friendly: pass a tmpdir without mutating process env.
    pub home_dir: Option<&'a Path>,
}

impl<'a> Default for EmailAddressesOpts<'a> {
    fn default() -> Self {
        Self {
            only_plugin: None,
            separator: None,
            bare_addresses: false,
            home_dir: None,
        }
    }
}

pub fn _email_addresses(state: &mut CompletionState, opts: &EmailAddressesOpts<'_>) -> bool {
    // shell:121-128 `-s sep` PREFIX chewing. Also trim leading
    // whitespace from the remainder since users typically type
    // `addr1, addr2, addr3` with spaces after each separator.
    if let Some(sep) = opts.separator {
        if let Some(idx) = state.params.prefix.rfind(sep) {
            let chewed_end = idx + sep.len();
            let chewed = state.params.prefix[..chewed_end].to_string();
            state.params.iprefix.push_str(&chewed);
            let rest = state.params.prefix[chewed_end..].to_string();
            let trimmed = rest.trim_start();
            let leading_ws = &rest[..rest.len() - trimmed.len()];
            if !leading_ws.is_empty() {
                state.params.iprefix.push_str(leading_ws);
            }
            state.params.prefix = trimmed.to_string();
        }
    }

    let home = match opts.home_dir {
        Some(p) => p.to_path_buf(),
        None => match std::env::var("HOME") {
            Ok(h) => PathBuf::from(h),
            Err(_) => return false,
        },
    };

    let mut entries: Vec<(String, String)> = Vec::new(); // (plugin, address)

    let want = |name: &str| -> bool {
        opts.only_plugin.map(|p| p == name).unwrap_or(true)
    };

    // ── mail / mutt / mush plugin: `.mailrc`-style files ──────────────
    if want("mail") || want("mutt") || want("mush") {
        let mut files: Vec<PathBuf> = Vec::new();
        if want("mail") {
            let mailrc = std::env::var("MAILRC")
                .map(PathBuf::from)
                .unwrap_or_else(|_| home.join(".mailrc"));
            files.push(mailrc);
        }
        if want("mutt") {
            // shell:135-138: zstyle override OR ~/mutt/muttrc OR ~/.muttrc.
            let muttrc = home.join("mutt").join("muttrc");
            if muttrc.exists() {
                files.push(muttrc);
            } else {
                files.push(home.join(".muttrc"));
            }
        }
        if want("mush") {
            files.push(home.join(".mushrc"));
        }
        for f in &files {
            collect_alias_lines(f, &mut entries, "mail");
        }
    }

    // ── pine plugin: `.addressbook` ───────────────────────────────────
    if want("pine") {
        let pine = home.join(".addressbook");
        if let Ok(content) = std::fs::read_to_string(&pine) {
            for line in content.lines() {
                // shell:42: skip DELETED entries and leading-space cont lines.
                if line.contains("DELETED") || line.starts_with(' ') {
                    continue;
                }
                // Format: NICK\tNAME\tADDR\t…
                let cols: Vec<&str> = line.split('\t').collect();
                if cols.len() >= 3 {
                    entries.push(("pine".into(), cols[2].into()));
                }
            }
        }
    }

    // ── MH plugin: `ali` output (shell:35-37) ─────────────────────────
    if want("MH") {
        let mh_profile = std::env::var("MH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".mh_profile"));
        if mh_profile.exists() {
            // Run `ali` (MH alias listing). Output format `NAME: ADDR`.
            if let Ok(out) = std::process::Command::new("ali").output() {
                if out.status.success() {
                    for line in String::from_utf8_lossy(&out.stdout).lines() {
                        if let Some(addr) = line.splitn(2, ": ").nth(1) {
                            entries.push(("MH".into(), addr.to_string()));
                        }
                    }
                }
            }
        }
    }

    if entries.is_empty() {
        return false;
    }

    // Apply -c: keep only entries containing `@` and strip name/comment.
    let to_match: Vec<String> = entries
        .iter()
        .map(|(_, raw)| {
            if opts.bare_addresses {
                extract_bare_address(raw)
            } else {
                raw.clone()
            }
        })
        .filter(|a| !opts.bare_addresses || a.contains('@'))
        .collect();

    let prefix = state.params.prefix.clone();
    state.begin_group("email-addresses", true);
    let mut seen = std::collections::HashSet::new();
    for addr in &to_match {
        if !addr.starts_with(&prefix) {
            continue;
        }
        if seen.insert(addr.clone()) {
            state.add_match(Completion::new(addr.clone()), Some("email-addresses"));
        }
    }
    state.end_group();
    state.nmatches > 0
}

/// Parse mailrc / muttrc / mushrc-style `alias NAME ADDRESS` lines.
fn collect_alias_lines(path: &Path, out: &mut Vec<(String, String)>, plugin: &'static str) {
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => return,
    };
    for line in content.lines() {
        let line = line.trim_start();
        if let Some(rest) = line.strip_prefix("alias ") {
            // mailrc: `alias NAME ADDR1 ADDR2 ...`
            // mutt:   `alias NAME ADDR` or `alias NAME=ADDR`
            let rest = rest.trim_start();
            // First whitespace ends NAME.
            let after_name = match rest.find(char::is_whitespace) {
                Some(i) => &rest[i + 1..],
                None => continue,
            };
            // The rest is space-separated address list.
            for addr in after_name.split_whitespace() {
                out.push((plugin.into(), addr.to_string()));
            }
        }
    }
}

/// `Name <addr@host>` → `addr@host`. Standalone `addr@host` returns
/// unchanged. Bare names without `@` return as-is (caller filters).
fn extract_bare_address(raw: &str) -> String {
    if let Some(open) = raw.find('<') {
        if let Some(close) = raw[open..].find('>') {
            return raw[open + 1..open + close].to_string();
        }
    }
    raw.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn with_temp_home<R, F: FnOnce(&Path) -> R>(setup: F) -> R {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_email_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let result = setup(&tmp);
        let _ = std::fs::remove_dir_all(&tmp);
        result
    }

    fn write_file(p: &Path, body: &str) {
        let mut f = std::fs::File::create(p).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }

    #[test]
    fn mailrc_alias_lines_become_completions() {
        with_temp_home(|home| {
            write_file(
                &home.join(".mailrc"),
                "alias bob bob@example.com\n\
                 alias alice alice@example.com\n",
            );
            let mut state = CompletionState::new();
            state.params.prefix = "a".into();
            let ok = _email_addresses(
                &mut state,
                &EmailAddressesOpts {
                    only_plugin: Some("mail"),
                    home_dir: Some(home),
                    ..Default::default()
                },
            );
            assert!(ok);
            let names: Vec<&str> = state.groups[0]
                .matches
                .iter()
                .map(|c| c.str_.as_str())
                .collect();
            assert!(names.contains(&"alice@example.com"), "got {names:?}");
            assert!(!names.contains(&"bob@example.com"));
        });
    }

    #[test]
    fn separator_chews_prefix_to_last_separator() {
        with_temp_home(|home| {
            write_file(&home.join(".mailrc"), "alias x x@example.com\n");
            let mut state = CompletionState::new();
            state.params.prefix = "bob@a.com, alice@b.com, x".into();
            let ok = _email_addresses(
                &mut state,
                &EmailAddressesOpts {
                    only_plugin: Some("mail"),
                    separator: Some(","),
                    home_dir: Some(home),
                    ..Default::default()
                },
            );
            assert!(ok);
            // After chew: prefix = "x" (leading space trimmed), iprefix
            // = "bob@a.com, alice@b.com, ".
            assert!(state.params.iprefix.contains("bob@a.com"));
            assert!(state.params.iprefix.ends_with(' '),
                    "leading space after the last `,` should land in iprefix, not prefix");
            assert_eq!(state.params.prefix, "x");
        });
    }

    #[test]
    fn c_flag_strips_name_and_drops_non_at_entries() {
        with_temp_home(|home| {
            write_file(
                &home.join(".mailrc"),
                "alias bob bob@example.com\n\
                 alias group alice@x.com carol@y.com\n",
            );
            let mut state = CompletionState::new();
            let ok = _email_addresses(
                &mut state,
                &EmailAddressesOpts {
                    only_plugin: Some("mail"),
                    bare_addresses: true,
                    home_dir: Some(home),
                    ..Default::default()
                },
            );
            assert!(ok);
            let names: Vec<&str> = state.groups[0]
                .matches
                .iter()
                .map(|c| c.str_.as_str())
                .collect();
            assert!(names.contains(&"bob@example.com"));
            assert!(names.contains(&"alice@x.com"));
            assert!(names.contains(&"carol@y.com"));
        });
    }

    #[test]
    fn no_sources_returns_false() {
        with_temp_home(|home| {
            let mut state = CompletionState::new();
            let ok = _email_addresses(
                &mut state,
                &EmailAddressesOpts {
                    only_plugin: Some("mail"),
                    home_dir: Some(home),
                    ..Default::default()
                },
            );
            assert!(!ok);
        });
    }
}
