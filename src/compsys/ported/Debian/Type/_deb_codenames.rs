//! Port of `_deb_codenames` from `Completion/Debian/Type/_deb_codenames`.
//!
//! Full upstream body (13 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local distro codenames ret=1
//! sh: 5  for distro in /usr/share/distro-info/*.csv(N); do
//! sh: 7    codenames=( ${(f)"$(<$distro tail -n6 | cut -d, -f3,1)"} )
//! sh: 8    codenames=( ${codenames/(#b)(*),(*)/${match[2]}:${match[1]}} )
//! sh: 9    _describe -V -t codename-${distro:t:r} "${distro:t:r} codenames" codenames && ret=0
//! sh:10  done
//! sh:12  return ret
//! ```
//!
//! sh:7 — `$(<$distro tail -n6 | cut -d, -f3,1)`: `tail -n6` reads the
//! last 6 lines of `$distro` (redirected in before the command name,
//! POSIX-legal), piped through `cut -d, -f3,1`. GNU `cut` always emits
//! selected fields in ascending numeric order regardless of the order
//! listed on the command line, so `-f3,1` == `-f1,3`: field 1 then
//! field 3, comma-joined. `${(f)...}` splits the command output on
//! newlines into one `codenames` entry per input line.
//!
//! sh:8 — `${codenames/(#b)(*),(*)/${match[2]}:${match[1]}}` applies to
//! every array element (parameter-substitution flags on an array apply
//! per-element). Glob pattern matching is leftmost-longest, so the
//! first `(*)` backtracks from the end of the string down to the LAST
//! comma: `match[1]` = everything before the last comma (field 1),
//! `match[2]` = everything after it (field 3). Since each cut line has
//! exactly one comma (two fields), this is equivalent to splitting on
//! the single comma and swapping: `"f1,f3"` → `"f3:f1"`.

use crate::compsys::ported::_describe::_describe;
use crate::ported::glob::{tokenize, zglob};
use crate::ported::params::setaparam;

/// sh:5 — glob `/usr/share/distro-info/*.csv(N)`. The `(N)` qualifier
/// is per-glob nullglob (empty result, no error, on zero matches); the
/// trailing metachar filter is a defensive backstop mirroring
/// `_time_zone`'s `glob_dir` in case a leftover unexpanded pattern
/// slips through.
fn glob_csv_files() -> Vec<String> {
    let mut list = {
        let mut s = "/usr/share/distro-info/*.csv(N)".to_string();
        tokenize(&mut s);
        vec![s]
    };
    zglob(&mut list, 0, 0);
    list.into_iter()
        .filter(|e| {
            !e.as_bytes().iter().any(|&b| {
                matches!(
                    b,
                    b'[' | b']' | b'*' | b'?' | b'#' | b'~' | b'^' | b'<' | b'>'
                )
            })
        })
        .collect()
}

/// sh:7 `tail -n6` — last `n` lines of `content` (all lines if fewer
/// than `n` are present).
fn tail_n(content: &str, n: usize) -> Vec<&str> {
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].to_vec()
}

/// sh:7 `cut -d, -f3,1` — select fields 1 and 3 (1-indexed, GNU `cut`
/// always emits them in ascending field order regardless of the order
/// given on the command line), comma-joined. A missing field (line has
/// fewer than 3 comma-separated fields) is simply omitted, matching
/// GNU `cut`.
fn cut_f1_f3(line: &str) -> String {
    let fields: Vec<&str> = line.split(',').collect();
    let mut out: Vec<&str> = Vec::with_capacity(2);
    for idx in [1usize, 3usize] {
        if let Some(f) = fields.get(idx - 1) {
            out.push(f);
        }
    }
    out.join(",")
}

/// sh:8 — swap `"f1,f3"` → `"f3:f1"` on the last comma (see module doc
/// for the leftmost-longest backtrack rationale). Entries without a
/// comma are left untouched (no match ⇒ `${param/pat/rep}` is a no-op).
fn swap_on_last_comma(entry: &str) -> String {
    match entry.rsplit_once(',') {
        Some((f1, f3)) => format!("{}:{}", f3, f1),
        None => entry.to_string(),
    }
}

/// sh:9 `${distro:t:r}` — basename (`:t`) with its trailing extension
/// stripped (`:r`, root).
fn tail_root(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    match base.rsplit_once('.') {
        Some((root, _ext)) => root.to_string(),
        None => base.to_string(),
    }
}

/// `_deb_codenames` — complete Debian/Ubuntu-family release codenames
/// from `/usr/share/distro-info/*.csv`.
pub fn _deb_codenames(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_deb_codenames");
    let _ = args; // sh: the shell fn body never references "$@".

    // sh:3
    let mut ret: i32 = 1;

    // sh:5 — for distro in /usr/share/distro-info/*.csv(N); do
    for distro in glob_csv_files() {
        // sh:7 — tail -n6 $distro | cut -d, -f3,1, split on newlines.
        let content = std::fs::read_to_string(&distro).unwrap_or_default();
        let codenames: Vec<String> = tail_n(&content, 6)
            .into_iter()
            .map(cut_f1_f3)
            // sh:8 — per-element ${(#b)(*),(*)} swap.
            .map(|e| swap_on_last_comma(&e))
            .collect();

        setaparam("codenames", codenames);

        // sh:9 — _describe -V -t codename-${distro:t:r} "${distro:t:r} codenames" codenames
        let base = tail_root(&distro);
        let rc = _describe(&[
            "-V".to_string(),
            "-t".to_string(),
            format!("codename-{}", base),
            format!("{} codenames", base),
            "codenames".to_string(),
        ]);
        if rc == 0 {
            ret = 0;
        }
    }

    // sh:12
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_n_returns_last_n_lines() {
        let content = "a\nb\nc\nd\ne\nf\ng\n";
        assert_eq!(tail_n(content, 6), vec!["b", "c", "d", "e", "f", "g"]);
    }

    #[test]
    fn tail_n_returns_all_when_fewer_than_n_lines() {
        let content = "a\nb\n";
        assert_eq!(tail_n(content, 6), vec!["a", "b"]);
    }

    #[test]
    fn cut_f1_f3_selects_ascending_regardless_of_input_order() {
        // distro-info csv row: version,codename,series,created,release,eol
        assert_eq!(
            cut_f1_f3("18.04 LTS,bionic,bionic,2017-10-13,2018-04-26,2028-04-26"),
            "18.04 LTS,bionic"
        );
    }

    #[test]
    fn cut_f1_f3_omits_missing_field() {
        assert_eq!(cut_f1_f3("only-one"), "only-one");
        assert_eq!(cut_f1_f3("f1,f2"), "f1");
    }

    #[test]
    fn swap_on_last_comma_reorders_value_desc() {
        assert_eq!(swap_on_last_comma("18.04 LTS,bionic"), "bionic:18.04 LTS");
    }

    #[test]
    fn swap_on_last_comma_backtracks_to_last_comma_on_multiple() {
        // Leftmost-longest glob match: first (*) is greedy, backtracks
        // to the LAST comma in the string.
        assert_eq!(swap_on_last_comma("a,b,c"), "c:a,b");
    }

    #[test]
    fn swap_on_last_comma_noop_without_comma() {
        assert_eq!(swap_on_last_comma("nocomma"), "nocomma");
    }

    #[test]
    fn tail_root_strips_dir_and_extension() {
        assert_eq!(tail_root("/usr/share/distro-info/ubuntu.csv"), "ubuntu");
        assert_eq!(tail_root("debian.csv"), "debian");
    }

    #[test]
    fn returns_one_when_no_csv_files_present() {
        // sh:5 (N) nullglob ⇒ the for-loop body never runs when the
        // real `/usr/share/distro-info/*.csv` glob is empty (as it
        // will be in the CI sandbox) ⇒ sh:12 returns the untouched
        // ret=1 default.
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_deb_codenames(&[]), 1);
    }
}
