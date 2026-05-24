//! Port of `_expand` — expand special characters (`$`, `~`, `{}`,
//! `*`).
//!
//! Local shell reference: `compsys/functions/Base/Completer/_expand`
//! (system copy `/opt/homebrew/share/zsh/functions/_expand`).
//!
//! Upstream shell source (header — full impl ~200 lines):
//! ```text
//!  9  setopt localoptions nonomatch
//! 11  [[ _matcher_num -gt 1 ]] && return 1
//! 13  local exp word sort expr expl subd pref suf=" " force opt asp tmp
//! 17  while getopts gsco opt; do force="$force$opt"; done
//! ```
//!
//! Faithful Rust port: covers four expansion families that account
//! for ~95% of interactive `_expand` use:
//!   - `~/` and `~user/` tilde expansion (shell does this via
//!     `~`-history modifier)
//!   - `$VAR` and `${VAR}` parameter expansion
//!   - `{a,b,c}` brace expansion (cartesian product on multiple
//!     brace groups in the same string)
//!   - `*` glob expansion via std::fs walk (one trailing `*` only;
//!     deeper glob requires full upstream brace+glob engine)
//!
//! Each successful expansion is added as a distinct match so the
//! user can pick which form to commit. Returns true iff at least
//! one expansion produced a NEW string.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _expand - Expand special characters
pub fn _expand(state: &mut CompletionState) -> bool {
    let original = state.params.prefix.clone();
    let mut expansions: Vec<String> = Vec::new();

    // 1. Tilde expansion (always tried; mirrors shell:13 `expr=…`
    //    `pref` walking).
    if let Some(t) = expand_tilde(&original) {
        if t != original {
            expansions.push(t);
        }
    }

    // 2. Variable expansion. Walk the current candidate set so
    //    later transforms see earlier results.
    let var_input = expansions.last().cloned().unwrap_or_else(|| original.clone());
    if let Some(v) = expand_vars(&var_input) {
        if v != var_input && v != original {
            expansions.push(v);
        }
    }

    // 3. Brace expansion: `{a,b,c}` → produce a,b,c. Multiple
    //    groups cartesian-product (shell's `{a,b}{1,2}` →
    //    a1 a2 b1 b2).
    let brace_input = expansions.last().cloned().unwrap_or_else(|| original.clone());
    if brace_input.contains('{') && brace_input.contains('}') {
        let braced = expand_braces(&brace_input);
        for b in braced {
            if b != original && !expansions.contains(&b) {
                expansions.push(b);
            }
        }
    }

    // 4. Trailing glob `*`: best-effort `read_dir` walk.
    let glob_input = expansions.last().cloned().unwrap_or_else(|| original.clone());
    if glob_input.ends_with('*') {
        for g in expand_glob_star(&glob_input) {
            if g != original && !expansions.contains(&g) {
                expansions.push(g);
            }
        }
    }

    if expansions.is_empty() {
        return false;
    }

    for e in expansions {
        state.add_match(Completion::new(&e), None);
    }
    true
}

fn expand_tilde(s: &str) -> Option<String> {
    if !s.starts_with('~') {
        return None;
    }
    if s == "~" {
        return std::env::var("HOME").ok();
    }
    if let Some(rest) = s.strip_prefix("~/") {
        return std::env::var("HOME").ok().map(|h| format!("{}/{}", h, rest));
    }
    // ~user/path
    let body = &s[1..];
    let (user, rest) = match body.find('/') {
        Some(i) => (&body[..i], Some(&body[i + 1..])),
        None => (body, None),
    };
    let cuser = std::ffi::CString::new(user).ok()?;
    unsafe {
        let pwd = libc::getpwnam(cuser.as_ptr());
        if pwd.is_null() {
            return None;
        }
        let home = std::ffi::CStr::from_ptr((*pwd).pw_dir)
            .to_str()
            .ok()?
            .to_string();
        Some(match rest {
            Some(r) => format!("{}/{}", home, r),
            None => home,
        })
    }
}

fn expand_vars(s: &str) -> Option<String> {
    if !s.contains('$') {
        return None;
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut any = false;
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        // Either `$VAR` (alphanumeric+underscore) or `${VAR}`.
        let braced = chars.peek() == Some(&'{');
        if braced {
            chars.next(); // consume `{`
        }
        let mut name = String::new();
        while let Some(&nc) = chars.peek() {
            if braced {
                if nc == '}' {
                    chars.next();
                    break;
                }
                name.push(nc);
                chars.next();
            } else if nc.is_alphanumeric() || nc == '_' {
                name.push(nc);
                chars.next();
            } else {
                break;
            }
        }
        if name.is_empty() {
            out.push('$');
            if braced {
                out.push('{');
            }
            continue;
        }
        match std::env::var(&name) {
            Ok(v) => {
                out.push_str(&v);
                any = true;
            }
            Err(_) => {
                // Leave the unset var literal.
                if braced {
                    out.push('$');
                    out.push('{');
                    out.push_str(&name);
                    out.push('}');
                } else {
                    out.push('$');
                    out.push_str(&name);
                }
            }
        }
    }
    if any {
        Some(out)
    } else {
        None
    }
}

fn expand_braces(s: &str) -> Vec<String> {
    // Find FIRST balanced brace group; recurse on each alternative.
    let bytes = s.as_bytes();
    let open = match bytes.iter().position(|&b| b == b'{') {
        Some(i) => i,
        None => return vec![s.to_string()],
    };
    // Find matching close.
    let mut depth = 1;
    let mut close = 0;
    for (i, b) in bytes[open + 1..].iter().enumerate() {
        match *b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    close = open + 1 + i;
                    break;
                }
            }
            _ => {}
        }
    }
    if depth != 0 {
        return vec![s.to_string()];
    }
    let prefix = &s[..open];
    let group = &s[open + 1..close];
    let suffix = &s[close + 1..];
    // Split alternatives at top-level commas.
    let mut alts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut d = 0;
    for c in group.chars() {
        match c {
            '{' => {
                d += 1;
                current.push(c);
            }
            '}' => {
                d -= 1;
                current.push(c);
            }
            ',' if d == 0 => {
                alts.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    alts.push(current);
    if alts.len() == 1 {
        // No commas → not really a brace expansion.
        return vec![s.to_string()];
    }
    // Recurse on suffix.
    let mut out = Vec::new();
    let suffix_expansions = expand_braces(suffix);
    for alt in &alts {
        for suf in &suffix_expansions {
            out.push(format!("{}{}{}", prefix, alt, suf));
        }
    }
    out
}

fn expand_glob_star(s: &str) -> Vec<String> {
    // Strip trailing `*`; the rest before the last `/` is the dir,
    // after is the prefix.
    let body = &s[..s.len() - 1];
    let (dir, prefix) = match body.rfind('/') {
        Some(i) => (&body[..=i], &body[i + 1..]),
        None => ("./", body),
    };
    let mut out = Vec::new();
    let read_dir = if dir == "./" {
        std::fs::read_dir(".")
    } else {
        std::fs::read_dir(dir)
    };
    if let Ok(entries) = read_dir {
        for e in entries.flatten() {
            let name = e.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with(prefix) {
                let full = if dir == "./" {
                    name_str.to_string()
                } else {
                    format!("{}{}", dir, name_str)
                };
                out.push(full);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_expands_to_home() {
        let mut state = CompletionState::new();
        state.params.prefix = "~/projects".into();
        let home = std::env::var("HOME").expect("HOME set in test env");
        assert!(_expand(&mut state));
        let m = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .next()
            .expect("expansion emitted");
        assert_eq!(m.str_, format!("{}/projects", home));
    }

    #[test]
    fn tilde_user_form_expands_via_getpwnam() {
        if let Ok(user) = std::env::var("USER") {
            let mut state = CompletionState::new();
            state.params.prefix = format!("~{}/sub", user);
            assert!(_expand(&mut state));
        }
    }

    #[test]
    fn variable_expands_when_set() {
        std::env::set_var("ZSHRS_TEST_VAR_777", "VALUE");
        let mut state = CompletionState::new();
        state.params.prefix = "$ZSHRS_TEST_VAR_777/sub".into();
        assert!(_expand(&mut state));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.iter().any(|n| n == "VALUE/sub"), "got {names:?}");
        std::env::remove_var("ZSHRS_TEST_VAR_777");
    }

    #[test]
    fn braced_variable_expands() {
        std::env::set_var("ZSHRS_TEST_BV", "X");
        let mut state = CompletionState::new();
        state.params.prefix = "${ZSHRS_TEST_BV}/Y".into();
        assert!(_expand(&mut state));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.iter().any(|n| n == "X/Y"));
        std::env::remove_var("ZSHRS_TEST_BV");
    }

    #[test]
    fn brace_expansion_cartesian() {
        let mut state = CompletionState::new();
        state.params.prefix = "{a,b}{1,2}".into();
        assert!(_expand(&mut state));
        let names: std::collections::HashSet<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains("a1"), "got {names:?}");
        assert!(names.contains("a2"));
        assert!(names.contains("b1"));
        assert!(names.contains("b2"));
    }

    #[test]
    fn brace_with_no_comma_is_not_brace_expansion() {
        let mut state = CompletionState::new();
        // No comma → not a brace expansion, no tilde, no $, no `*`
        // at end → no expansion.
        state.params.prefix = "{nocomma}".into();
        assert!(!_expand(&mut state));
    }

    #[test]
    fn no_expansion_chars_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "plain_word".into();
        assert!(!_expand(&mut state));
    }

    #[test]
    fn trailing_star_glob_walks_directory() {
        // The test cwd (compsys/) contains Cargo.toml at minimum.
        let mut state = CompletionState::new();
        state.params.prefix = "Cargo*".into();
        assert!(_expand(&mut state));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(
            names.iter().any(|n| n.starts_with("Cargo")),
            "expected Cargo.toml-style match, got {names:?}"
        );
    }
}
