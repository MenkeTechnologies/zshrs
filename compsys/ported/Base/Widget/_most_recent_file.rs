//! Port of `_most_recent_file` — complete most recently modified file.
//!
//! Local shell reference: `compsys/functions/Base/Widget/_most_recent_file`
//! (system copy `/opt/homebrew/share/zsh/functions/_most_recent_file`).
//!
//! Upstream shell source (full):
//! ```text
//! 11  local file tilde etilde
//! 12  if [[ $PREFIX = \~*/* ]]; then
//! 13    tilde=${PREFIX%%/*}
//! 14    etilde=${~tilde} 2>/dev/null
//! 17    eval "file=($PREFIX*$SUFFIX(om[${NUMERIC:-1}]N))"
//! 18    file=(${file/#$etilde})
//! 19    file=($tilde${(q)^file})
//! 20  else
//! 21    eval "file=($PREFIX*$SUFFIX(om[${NUMERIC:-1}]N))"
//! 22    file=(${(q)file})
//! 23  fi
//! 24  (( $#file )) && compadd -U -i "$IPREFIX" -I "$ISUFFIX" -f -Q -- $file
//! ```
//!
//! Strict Rust port: handles `~/`/`~user/` expansion (shell:12-19),
//! sorts by mtime descending, and honors `numeric_prefix` (mirrors
//! `${NUMERIC:-1}` → `om[N]` index, 1-based: 1=newest, 2=second-
//! newest, …). The pattern arg corresponds to `$PREFIX*$SUFFIX`
//! shell expansion.

use std::fs;
use std::path::Path;

use crate::compcore::CompletionState;
use crate::completion::Completion;

use super::shared::glob_match;

/// Resolve a leading `~` / `~user/` to an absolute path. Returns
/// None if the user lookup fails.
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

/// _most_recent_file - Complete most recently modified file.
///
/// `numeric_prefix` ≥ 1 selects which entry to emit (1 = newest).
/// 0 is treated as 1 (matches shell's `${NUMERIC:-1}`).
pub fn _most_recent_file(
    state: &mut CompletionState,
    dir: &str,
    pattern: Option<&str>,
    numeric_prefix: usize,
) -> bool {
    let n = if numeric_prefix == 0 { 1 } else { numeric_prefix };
    let expanded = match expand_tilde(dir) {
        Some(e) => e,
        None => return false,
    };
    let entries = match fs::read_dir(&expanded) {
        Ok(e) => e,
        Err(_) => return false,
    };

    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            if let Some(pat) = pattern {
                glob_match(pat, &e.file_name().to_string_lossy())
            } else {
                true
            }
        })
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            let modified = meta.modified().ok()?;
            Some((e, modified))
        })
        .collect();

    // Sort newest-first.
    files.sort_by_key(|b| std::cmp::Reverse(b.1));

    if let Some((entry, _)) = files.get(n - 1) {
        let name = entry.file_name();
        // Preserve user's typed `~` form in the output.
        let display_dir = if dir.starts_with('~') {
            Path::new(dir).to_string_lossy().into_owned()
        } else {
            expanded
        };
        let full = format!("{}/{}", display_dir, name.to_string_lossy());
        state.add_match(Completion::new(&full), None);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    #[test]
    fn picks_most_recently_modified_in_pattern() {
        // Make a tmp dir with two files; touch the second one later
        // so it has the most recent mtime.
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_mrf_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let path_old = tmp.join("old.log");
        let path_new = tmp.join("new.log");
        std::fs::File::create(&path_old).unwrap().write_all(b"a").unwrap();
        std::thread::sleep(Duration::from_millis(50));
        std::fs::File::create(&path_new).unwrap().write_all(b"b").unwrap();

        let mut state = CompletionState::new();
        assert!(_most_recent_file(
            &mut state,
            tmp.to_str().unwrap(),
            Some("*.log"),
            1,
        ));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert_eq!(names.len(), 1, "exactly ONE most-recent file emitted");
        assert!(names[0].ends_with("/new.log"), "got {:?}", names);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn nonexistent_dir_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_most_recent_file(&mut state, "/no/such/dir", None, 1));
    }

    #[test]
    fn empty_dir_returns_false() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_mrf_empty_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut state = CompletionState::new();
        assert!(!_most_recent_file(&mut state, tmp.to_str().unwrap(), None, 1));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn no_pattern_matches_oldest_or_any_file() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_mrf_nopat_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("one.txt"), b"x").unwrap();
        let mut state = CompletionState::new();
        assert!(_most_recent_file(&mut state, tmp.to_str().unwrap(), None, 1));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn pattern_filter_drops_unmatching() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_mrf_pat_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("only.rs"), b"x").unwrap();
        let mut state = CompletionState::new();
        // Pattern `*.toml` doesn't match — returns false.
        assert!(!_most_recent_file(
            &mut state,
            tmp.to_str().unwrap(),
            Some("*.toml"),
            1,
        ));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn numeric_prefix_2_selects_second_newest() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_mrf_n2_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::File::create(tmp.join("a.log")).unwrap().write_all(b"x").unwrap();
        std::thread::sleep(Duration::from_millis(30));
        std::fs::File::create(tmp.join("b.log")).unwrap().write_all(b"x").unwrap();
        std::thread::sleep(Duration::from_millis(30));
        std::fs::File::create(tmp.join("c.log")).unwrap().write_all(b"x").unwrap();
        let mut state = CompletionState::new();
        assert!(_most_recent_file(
            &mut state,
            tmp.to_str().unwrap(),
            Some("*.log"),
            2,
        ));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names[0].ends_with("/b.log"), "n=2 should pick b.log (second-newest); got {:?}", names);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn numeric_prefix_beyond_file_count_returns_false() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_mrf_oob_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("only.log"), b"x").unwrap();
        let mut state = CompletionState::new();
        assert!(!_most_recent_file(
            &mut state,
            tmp.to_str().unwrap(),
            None,
            5,
        ));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn numeric_zero_treated_as_one() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_mrf_zero_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("only.log"), b"x").unwrap();
        let mut state = CompletionState::new();
        assert!(_most_recent_file(
            &mut state,
            tmp.to_str().unwrap(),
            None,
            0,
        ));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
