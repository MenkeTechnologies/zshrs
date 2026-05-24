//! Port of `_bash_completions` — compatibility with bash completion
//! specs. Moved from `compsys/functions.rs`.

use crate::base::{CompleterResult, MainCompleteState};

/// _bash_completions - Compatibility with bash completion specs
///
/// This function provides interoperability with bash's programmable completion.
/// It parses bash compspec strings and generates completions accordingly.
///
/// Compspec format: -F function | -C command | -G globpat | -W wordlist | -X filterpat
pub fn _bash_completions(state: &mut MainCompleteState, compspec: &str) -> CompleterResult {
    let prefix = &state.comp.params.prefix.clone();
    let mut matches = Vec::new();

    // Parse compspec arguments
    let args: Vec<&str> = compspec.split_whitespace().collect();
    let mut i = 0;

    while i < args.len() {
        match args[i] {
            "-W" if i + 1 < args.len() => {
                // Word list: -W "word1 word2 word3"
                i += 1;
                let wordlist = args[i].trim_matches('"').trim_matches('\'');
                for word in wordlist.split_whitespace() {
                    if word.to_lowercase().starts_with(&prefix.to_lowercase()) {
                        matches.push(crate::Completion::new(word));
                    }
                }
            }
            "-G" if i + 1 < args.len() => {
                // Glob pattern: -G "*.txt"
                i += 1;
                let pattern = args[i].trim_matches('"').trim_matches('\'');
                if let Ok(entries) = std::fs::read_dir(".") {
                    for entry in entries.flatten() {
                        if let Some(name) = entry.file_name().to_str() {
                            if glob_match_simple(pattern, name)
                                && name.to_lowercase().starts_with(&prefix.to_lowercase())
                            {
                                matches.push(crate::Completion::new(name));
                            }
                        }
                    }
                }
            }
            "-d" => {
                // Directories only
                if let Ok(entries) = std::fs::read_dir(".") {
                    for entry in entries.flatten() {
                        if let Ok(ft) = entry.file_type() {
                            if ft.is_dir() {
                                if let Some(name) = entry.file_name().to_str() {
                                    if name.to_lowercase().starts_with(&prefix.to_lowercase()) {
                                        let mut c = crate::Completion::new(format!("{}/", name));
                                        c.flags |= crate::CompletionFlags::FILE;
                                        matches.push(c);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "-f" => {
                // Files
                if let Ok(entries) = std::fs::read_dir(".") {
                    for entry in entries.flatten() {
                        if let Some(name) = entry.file_name().to_str() {
                            if name.to_lowercase().starts_with(&prefix.to_lowercase()) {
                                let mut c = crate::Completion::new(name);
                                c.flags |= crate::CompletionFlags::FILE;
                                matches.push(c);
                            }
                        }
                    }
                }
            }
            "-c" => {
                // Commands from PATH
                if let Ok(path) = std::env::var("PATH") {
                    for dir in path.split(':') {
                        if let Ok(entries) = std::fs::read_dir(dir) {
                            for entry in entries.flatten() {
                                if let Some(name) = entry.file_name().to_str() {
                                    if name.to_lowercase().starts_with(&prefix.to_lowercase()) {
                                        matches.push(crate::Completion::new(name));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "-u" => {
                // Usernames
                if let Ok(content) = std::fs::read_to_string("/etc/passwd") {
                    for line in content.lines() {
                        if let Some(user) = line.split(':').next() {
                            if user.to_lowercase().starts_with(&prefix.to_lowercase()) {
                                matches.push(crate::Completion::new(user));
                            }
                        }
                    }
                }
            }
            "-F" | "-C" => {
                // Function or command - would need to invoke bash, skip
                i += 1;
            }
            "-X" | "-P" | "-S" if i + 1 < args.len() => {
                // Filter/prefix/suffix - skip
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    if !matches.is_empty() {
        // Add matches to a group
        let mut group = crate::CompletionGroup::new("bash-completion");
        group.matches = matches;
        state.comp.groups.push(group);
        state.comp.nmatches += state
            .comp
            .groups
            .last()
            .map(|g| g.matches.len())
            .unwrap_or(0);
        CompleterResult::Matched
    } else {
        CompleterResult::NoMatch
    }
}

/// Simple glob matching for bash compatibility
fn glob_match_simple(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();

    fn match_impl(p: &[u8], t: &[u8]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some(b'*'), _) => match_impl(&p[1..], t) || (!t.is_empty() && match_impl(p, &t[1..])),
            (Some(b'?'), Some(_)) => match_impl(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a == b => match_impl(&p[1..], &t[1..]),
            _ => false,
        }
    }

    match_impl(pattern, text)
}
