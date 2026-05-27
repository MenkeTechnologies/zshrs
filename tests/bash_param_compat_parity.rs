//! Bash-style parameter expansion compat tests in zsh:
//! `${V:offset:length}`, `${!V}`, `${V^^}`, `${V,,}`, `${V^}`, `${V,}`.
//!
//! These work in zsh (some require modes; pinned to actual zsh behavior).

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}
fn zsh_path() -> &'static str {
    if Path::new("/opt/homebrew/bin/zsh").exists() {
        "/opt/homebrew/bin/zsh"
    } else if Path::new("/usr/local/bin/zsh").exists() {
        "/usr/local/bin/zsh"
    } else {
        "/bin/zsh"
    }
}
fn zsh_available() -> bool {
    Command::new(zsh_path())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
struct R {
    stdout: String,
    exit: i32,
}
fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn run_zshrs(s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn assert_parity(s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(s);
    let r = run_zshrs(s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit);
}

mod substring {
    use super::*;

    /// `${V:offset}` from offset to end.
    #[test]
    fn substring_offset_only() {
        assert_parity(r#"X=helloworld; echo "${X:5}""#);
    }

    /// `${V:offset:length}` substring.
    #[test]
    fn substring_offset_length() {
        assert_parity(r#"X=helloworld; echo "${X:0:5}""#);
    }

    #[test]
    fn substring_middle() {
        assert_parity(r#"X=helloworld; echo "${X:2:3}""#);
    }

    /// Negative offset → from end.
    /// zsh requires KSH_ARRAYS or special handling; pin behavior.
    #[test]
    fn substring_negative_offset() {
        assert_parity(r#"X=helloworld; echo "${X: -3}""#);
    }

    /// Offset beyond length → empty.
    #[test]
    fn substring_offset_past_length() {
        assert_parity(r#"X=abc; echo "[${X:10}]""#);
    }

    /// Length 0 → empty.
    #[test]
    fn substring_length_zero() {
        assert_parity(r#"X=abc; echo "[${X:0:0}]""#);
    }
}

mod indirect {
    use super::*;

    /// `${!V}` bash-style indirect.
    /// In zsh this only works in KSH_ARRAYS / sh / bash emulation.
    /// Pin actual behavior.
    #[test]
    fn indirect_basic_zsh_behavior() {
        // Without bash emul, `${!V}` triggers parameter-name-matching
        // behavior in zsh. Verify both shells behave the same.
        assert_parity(r#"X=hello; Y=X; echo "${!Y}""#);
    }

    /// `${!prefix*}` lists var names with prefix.
    #[test]
    fn indirect_prefix_star_list_names() {
        assert_parity(r#"FOO_a=1; FOO_b=2; FOO_c=3; echo "${!FOO_*}""#);
    }

    /// `${!prefix@}` lists with quoting.
    #[test]
    fn indirect_prefix_at_list_names() {
        assert_parity(r#"BAR_x=1; BAR_y=2; print -l ${(o)${!BAR_@}}"#);
    }
}

mod case_upper_lower {
    use super::*;

    /// `${V^^}` uppercase all.
    #[test]
    fn upper_all() {
        assert_parity(r#"X=hello; echo "${X:u}""#);
    }

    /// `${V,,}` lowercase all (zsh: `${V:l}`).
    #[test]
    fn lower_all() {
        assert_parity(r#"X=HELLO; echo "${X:l}""#);
    }

    /// `${V^}` uppercase first char (zsh-bash compat may differ).
    #[test]
    fn upper_first() {
        assert_parity(r#"X=hello; echo "${X^}""#);
    }

    /// Mixed case.
    #[test]
    fn upper_already_upper() {
        assert_parity(r#"X=HELLO; echo "${X:u}""#);
    }
}

mod default_op {
    use super::*;

    /// `${V:-default}` use default if unset/empty.
    #[test]
    fn default_when_unset() {
        assert_parity(r#"echo "${UNSET_VAR_XYZ:-fallback}""#);
    }

    #[test]
    fn default_when_empty() {
        assert_parity(r#"X=""; echo "${X:-fallback}""#);
    }

    #[test]
    fn default_when_set() {
        assert_parity(r#"X=actual; echo "${X:-fallback}""#);
    }

    /// `${V-default}` (no colon) — only if unset, not if empty.
    #[test]
    fn default_dash_only_unset() {
        assert_parity(r#"X=""; echo "[${X-fallback}]""#);
    }
}

mod assign_op {
    use super::*;

    /// `${V:=default}` assign default to V if unset/empty.
    #[test]
    fn assign_default() {
        assert_parity(r#"echo "${V_XYZ:=assigned}"; echo "$V_XYZ""#);
    }

    /// `${V:=default}` — V already set → no change.
    #[test]
    fn assign_default_keeps_existing() {
        assert_parity(r#"V_AAA=keep; echo "${V_AAA:=new}"; echo "$V_AAA""#);
    }
}

mod alternate_op {
    use super::*;

    /// `${V:+alt}` — if set+nonempty use alt, else empty.
    #[test]
    fn alternate_when_set() {
        assert_parity(r#"X=set; echo "[${X:+alt}]""#);
    }

    #[test]
    fn alternate_when_empty() {
        assert_parity(r#"X=""; echo "[${X:+alt}]""#);
    }

    #[test]
    fn alternate_when_unset() {
        assert_parity(r#"echo "[${UNSET_XYZ:+alt}]""#);
    }
}

mod error_op {
    use super::*;

    /// `${V:?msg}` — error if unset/empty.
    #[test]
    fn error_when_unset_in_subshell() {
        // In a subshell because the error aborts.
        assert_parity(r#"(echo "${UNSET_XYZ:?missing}" 2>/dev/null); echo exit=$?"#);
    }

    /// `${V:?msg}` when set → just returns value.
    #[test]
    fn error_when_set_returns_value() {
        assert_parity(r#"X=ok; echo "${X:?missing}""#);
    }
}

mod string_length {
    use super::*;

    /// `${#V}` string length.
    #[test]
    fn length_basic() {
        assert_parity(r#"X=hello; echo "${#X}""#);
    }

    #[test]
    fn length_empty() {
        assert_parity(r#"X=""; echo "${#X}""#);
    }

    #[test]
    fn length_unset() {
        assert_parity(r#"echo "${#UNSET_XYZ}""#);
    }

    /// `${#arr}` array length.
    #[test]
    fn length_of_array() {
        assert_parity(r#"arr=(a b c d e); echo "${#arr}""#);
    }
}

mod strip_pattern {
    use super::*;

    /// `${V#pat}` shortest prefix.
    #[test]
    fn strip_shortest_prefix() {
        assert_parity(r#"X=foofoobar; echo "${X#foo}""#);
    }

    /// `${V##pat}` longest prefix.
    #[test]
    fn strip_longest_prefix() {
        assert_parity(r#"X=foofoobar; echo "${X##foo*foo}""#);
    }

    /// `${V%pat}` shortest suffix.
    #[test]
    fn strip_shortest_suffix() {
        assert_parity(r#"X=hello.txt.bak; echo "${X%.*}""#);
    }

    /// `${V%%pat}` longest suffix.
    #[test]
    fn strip_longest_suffix() {
        assert_parity(r#"X=hello.txt.bak; echo "${X%%.*}""#);
    }
}
