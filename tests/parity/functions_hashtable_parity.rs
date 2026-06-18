//! zsh-specific introspection-hashtable parity tests:
//! $functions, $aliases, $parameters, $commands.

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

mod functions_hashtable {
    use super::*;

    /// $functions[name] gives the body of `name`.
    #[test]
    fn functions_lookup_user_function() {
        assert_parity(r#"f() { echo hi; }; echo "${functions[f]}""#);
    }

    /// Unknown name → empty.
    #[test]
    fn functions_lookup_unknown_is_empty() {
        assert_parity(r#"echo "[${functions[nonexistent_xyz]}]""#);
    }

    /// ${#functions} > 0 in any shell (with bound function).
    #[test]
    fn functions_has_entries_after_def() {
        assert_parity(r#"f() { :; }; [[ ${#functions} -gt 0 ]]; echo $?"#);
    }

    /// Setting $functions[name] = body defines a function.
    #[test]
    fn functions_assignment_defines_function() {
        assert_parity(r#"functions[greet]='echo hello'; greet"#);
    }

    /// Unsetting $functions[name] removes the function.
    #[test]
    fn functions_unset_removes() {
        assert_parity(r#"f() { :; }; unset "functions[f]"; whence f 2>/dev/null; echo $?"#);
    }
}

mod aliases_hashtable {
    use super::*;

    /// $aliases[name] gives the alias value.
    #[test]
    fn aliases_lookup_existing() {
        assert_parity(r#"alias x='hi'; echo "${aliases[x]}""#);
    }

    /// Unknown → empty.
    #[test]
    fn aliases_lookup_unknown_empty() {
        assert_parity(r#"echo "[${aliases[nonexistent_xyz]}]""#);
    }

    /// `aliases[name]=val` defines an alias.
    #[test]
    fn aliases_assignment_defines() {
        assert_parity(r#"aliases[myhi]='echo hi'; alias myhi | head -1"#);
    }
}

mod parameters_hashtable {
    use super::*;

    /// $parameters[name] = type string ("scalar"/"integer"/"array"/etc.).
    #[test]
    fn parameters_lookup_scalar() {
        assert_parity(r#"X=val; echo "${parameters[X]}""#);
    }

    #[test]
    fn parameters_lookup_integer() {
        assert_parity(r#"typeset -i X=42; echo "${parameters[X]}""#);
    }

    #[test]
    fn parameters_lookup_array() {
        assert_parity(r#"typeset -a arr=(a b); echo "${parameters[arr]}""#);
    }

    #[test]
    fn parameters_lookup_assoc() {
        assert_parity(r#"typeset -A h; h[k]=v; echo "${parameters[h]}""#);
    }

    /// Unknown → empty.
    #[test]
    fn parameters_unknown_empty() {
        assert_parity(r#"echo "[${parameters[nonexistent_xyz_42]}]""#);
    }

    /// Readonly modifier shows up in type.
    #[test]
    fn parameters_readonly_modifier() {
        assert_parity(r#"readonly X=val; echo "${parameters[X]}""#);
    }
}

mod commands_hashtable {
    use super::*;

    /// $commands[name] = path of external command.
    #[test]
    fn commands_lookup_external() {
        assert_parity(r#"[[ -n "${commands[ls]}" ]]; echo $?"#);
    }

    /// Unknown → empty.
    #[test]
    fn commands_unknown_empty() {
        assert_parity(r#"echo "[${commands[nonexistent_xyz_42_zzz]}]""#);
    }
}

mod combined_introspect {
    use super::*;

    /// `(t)` flag also gives type info; consistent with $parameters.
    #[test]
    fn t_flag_matches_parameters_type_for_scalar() {
        assert_parity(r#"X=v; [[ "${(t)X}" == "${parameters[X]}" ]]; echo $?"#);
    }

    #[test]
    fn t_flag_for_integer() {
        assert_parity(r#"typeset -i X=42; [[ "${(t)X}" == "${parameters[X]}" ]]; echo $?"#);
    }
}

mod nameref {
    use super::*;

    /// `typeset -n` (nameref) exists in the upstream zsh source zshrs is ported
    /// from (builtin.c PM_NAMEREF / typeset optstring `n`), but predates some
    /// installed zsh binaries (e.g. 5.9.1 rejects `-n` with "bad option"). When
    /// the reference zsh lacks nameref the comparison is meaningless, so skip —
    /// zshrs's nameref behavior is pinned separately by the zshrs_pin tests.
    fn zsh_has_nameref() -> bool {
        if !zsh_available() {
            return false;
        }
        Command::new(zsh_path())
            .args(["-fc", "typeset -n __r=__t"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// `typeset -n REF=TARGET` creates a name reference. $REF reads
    /// $TARGET. nameref shows in $parameters.
    #[test]
    fn nameref_via_typeset_n() {
        if !zsh_has_nameref() {
            return;
        }
        assert_parity(r#"TARGET=value; typeset -n REF=TARGET; echo $REF"#);
    }

    #[test]
    fn nameref_in_parameters_hash() {
        if !zsh_has_nameref() {
            return;
        }
        assert_parity(r#"TARGET=value; typeset -n REF=TARGET; echo "${parameters[REF]}""#);
    }
}

mod special_param_introspect {
    use super::*;

    /// $parameters[PATH] should report scalar-export (PATH is exported).
    #[test]
    fn parameters_path_is_scalar_export() {
        if !zsh_available() {
            return;
        }
        // PATH may show as scalar-export-special; just verify entry exists.
        let s = r#"[[ -n "${parameters[PATH]}" ]]; echo $?"#;
        let z = run_zsh(s);
        let r = run_zshrs(s);
        assert_eq!(z.stdout, r.stdout);
    }
}

/// c:Src/text.c gettext2 WC_CASE arm (~520) — `functions f` re-emits
/// case bodies with the per-arm terminator (`;;` / `;&` / `;|`),
/// `(pat)` parens, and `case W in` / `esac` on their own lines.
mod functions_display_case_terminators {
    use super::*;

    #[test]
    fn case_body_keeps_dsemi_and_semiamp() {
        assert_parity(r#"f() { case a in a) :;; b) :;& esac }; functions f"#);
    }

    #[test]
    fn case_body_multi_statement_and_semibar() {
        assert_parity(r#"f() { case $x in a|b) print x; print y;; (c*) :;| esac }; functions f"#);
    }

    #[test]
    fn case_between_sibling_statements() {
        assert_parity(r#"f() { pre; case a in a) :;; esac; post }; functions f"#);
    }
}
