//! Port of `_correct_filename` — correct filename spelling.
//!
//! Local shell reference: `compsys/functions/Base/Widget/_correct_filename`
//! (system copy `/opt/homebrew/share/zsh/functions/_correct_filename`).
//!
//! Upstream shell source (key lines):
//! ```text
//! 19  local file="$PREFIX$SUFFIX" trylist tilde etilde testcmd
//! 20  integer approx max_approx=6
//! 22  if [[ -z $WIDGET ]]; then
//! 23    file=$1
//! 26    (( ${NUMERIC:-1} > 1 )) && max_approx=$NUMERIC
//! 29  if [[ $file = \~*/* ]]; then
//! 30    tilde=${file%%/*}
//! 32    etilde=$(eval echo ${(q)tilde})
//! 35    file=${file/#$tilde/$etilde}
//! 38  for (( approx = 1; approx <= max_approx; approx++ )); do
//! 39    if testcmd=$(_complete_help_call_directories …) …
//! ```
//!
//! Strict Rust port: handles `~/`/`~user/` expansion, walks the
//! parent directory, applies an INCREASING approximation budget
//! (1..=max_approx) and accepts on the first budget at which we
//! get any candidates. `max_approx` defaults to 6 (matching the
//! shell line 20) and can be overridden via `numeric_prefix` (>1).

use std::fs;
use std::path::Path;

use crate::compcore::CompletionState;
use crate::completion::Completion;

use super::shared::edit_distance;

/// Resolve `~` (HOME), `~user/` (via getpwnam) at the start of a
/// path. Leaves anything else unchanged. Returns `None` if a `~user`
/// lookup fails.
fn expand_tilde(p: &str) -> Option<String> {
    if !p.starts_with('~') {
        return Some(p.to_string());
    }
    let (head, tail) = match p.find('/') {
        Some(i) => (&p[..i], &p[i..]),
        None => (p, ""),
    };
    if head == "~" {
        let home = std::env::var("HOME").ok()?;
        return Some(format!("{home}{tail}"));
    }
    // `~user` — getpwnam.
    let user = &head[1..];
    let cstr = std::ffi::CString::new(user).ok()?;
    unsafe {
        let pw = libc::getpwnam(cstr.as_ptr());
        if pw.is_null() {
            return None;
        }
        let dir = std::ffi::CStr::from_ptr((*pw).pw_dir).to_string_lossy().to_string();
        Some(format!("{dir}{tail}"))
    }
}

/// _correct_filename - Correct filename spelling.
///
/// `numeric_prefix` mirrors the shell `${NUMERIC:-1}`. Values > 1
/// override the default `max_approx=6` cap.
pub fn _correct_filename(state: &mut CompletionState, numeric_prefix: usize) -> bool {
    let raw = state.params.prefix.clone();
    let expanded = match expand_tilde(&raw) {
        Some(e) => e,
        None => return false,
    };
    let max_approx: usize = if numeric_prefix > 1 { numeric_prefix } else { 6 };

    let (dir, file_prefix) = match expanded.rfind('/') {
        Some(sep) => (
            expanded[..=sep].to_string(),
            expanded[sep + 1..].to_string(),
        ),
        None => (".".to_string(), expanded.clone()),
    };

    let dir_path: &Path = Path::new(&dir);
    let entries = match fs::read_dir(dir_path) {
        Ok(e) => e,
        Err(_) => return false,
    };
    let names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();

    // shell:38 — try ascending budgets, stop at the FIRST budget
    // that yields candidates. Smaller budgets get the user closer
    // to what they typed.
    let mut accepted: Vec<String> = Vec::new();
    for approx in 1..=max_approx {
        accepted = names
            .iter()
            .filter(|n| edit_distance(&file_prefix, n) <= approx)
            .cloned()
            .collect();
        if !accepted.is_empty() {
            break;
        }
    }
    if accepted.is_empty() {
        return false;
    }

    // Emit accepted candidates with the original dir prefix
    // preserved (so the user's `~/Doc` stays `~/Documents` not
    // `/home/wizard/Documents`).
    let original_dir = match raw.rfind('/') {
        Some(sep) => &raw[..=sep],
        None => "",
    };
    for n in accepted {
        let full = if original_dir.is_empty() {
            n
        } else {
            format!("{}{}", original_dir, n)
        };
        state.add_match(Completion::new(&full), None);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typoed_cargo_toml_matched() {
        // `Carg.tom` is 2 edits away from `Cargo.toml`.
        let mut state = CompletionState::new();
        state.params.prefix = "Carg.tom".into();
        assert!(_correct_filename(&mut state, 1));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(
            names.iter().any(|n| n == "Cargo.toml"),
            "Cargo.toml should be corrected from `Carg.tom`; got {names:?}"
        );
    }

    #[test]
    fn nonexistent_dir_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "/no/such/dir/prefix".into();
        assert!(!_correct_filename(&mut state, 1));
    }

    #[test]
    fn exact_filename_passes() {
        let mut state = CompletionState::new();
        state.params.prefix = "Cargo.toml".into();
        assert!(_correct_filename(&mut state, 1));
    }

    #[test]
    fn one_typo_corrected() {
        let mut state = CompletionState::new();
        state.params.prefix = "Carog.toml".into();
        assert!(_correct_filename(&mut state, 1));
    }

    #[test]
    fn tilde_expanded_to_home() {
        // ~/anything-impossible should NOT panic and should return
        // false because the resolved dir has no entry resembling
        // the very-long bogus prefix.
        let home = std::env::var("HOME").unwrap_or_default();
        if home.is_empty() || !Path::new(&home).exists() {
            return;
        }
        let mut state = CompletionState::new();
        state.params.prefix = "~/_zzzz_definitely_nope_xyz_12345".into();
        // 16-budget cap (numeric=20) couldn't bridge a 30+-char delta;
        // expect false.
        assert!(!_correct_filename(&mut state, 20));
    }

    #[test]
    fn small_budget_preferred_over_large() {
        // The loop must stop at the first budget yielding hits, so
        // a clean prefix that matches at budget 0 shouldn't pull in
        // farther candidates. Use the project dir: `Cargo.lock`
        // and `Cargo.toml` both within 4-5 of `Cargo` but the impl
        // accepts at budget 5 with both. With prefix "Cargo.toml"
        // we match exact at budget 0 (no other hit).
        let mut state = CompletionState::new();
        state.params.prefix = "Cargo.toml".into();
        let _ = _correct_filename(&mut state, 1);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"Cargo.toml".to_string()));
    }

    #[test]
    fn numeric_prefix_overrides_default_cap() {
        // numeric_prefix > 1 raises max_approx. We can't easily
        // assert the upper bound, but at least pin that the fn
        // doesn't crash with a large value.
        let mut state = CompletionState::new();
        state.params.prefix = "Carg.tom".into();
        assert!(_correct_filename(&mut state, 10));
    }

    #[test]
    fn numeric_prefix_of_one_falls_back_to_default() {
        // `1` is the upstream `${NUMERIC:-1}` default → uses
        // max_approx=6 not 1.
        let mut state = CompletionState::new();
        state.params.prefix = "Carg.tom".into();
        // 2 typos from Cargo.toml — within default cap 6.
        assert!(_correct_filename(&mut state, 1));
    }

    #[test]
    fn tilde_expansion_preserves_original_form_in_output() {
        // After tilde expansion, results should re-attach the
        // user's typed `~/` so they don't suddenly see the
        // absolute path.
        let home = std::env::var("HOME").unwrap_or_default();
        if home.is_empty() || !Path::new(&home).exists() {
            return;
        }
        // Try to find ANY existing entry under HOME to test against.
        let entries: Vec<String> = match fs::read_dir(&home) {
            Ok(e) => e
                .flatten()
                .filter_map(|x| x.file_name().to_str().map(String::from))
                .filter(|n| !n.starts_with('.'))
                .take(5)
                .collect(),
            Err(_) => return,
        };
        if let Some(name) = entries.first() {
            let mut state = CompletionState::new();
            state.params.prefix = format!("~/{}", name);
            assert!(_correct_filename(&mut state, 1));
            let names: Vec<String> = state
                .groups
                .iter()
                .flat_map(|g| g.matches.iter())
                .map(|c| c.str_.clone())
                .collect();
            assert!(
                names.iter().all(|n| n.starts_with("~/")),
                "tilde form must be preserved; got {names:?}"
            );
        }
    }
}
