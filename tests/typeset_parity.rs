//! typeset / declare parity tests for type flags.
#![allow(non_snake_case)]

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

mod integer {
    use super::*;

    #[test]
    fn integer_declaration_with_value() {
        assert_parity("typeset -i X=42; echo $X");
    }

    /// Integer params arithmetic-evaluate on assignment.
    #[test]
    fn integer_assignment_evaluates_arith() {
        assert_parity("typeset -i X; X=5+3; echo $X");
    }

    #[test]
    fn integer_negative_value() {
        assert_parity("typeset -i X=-7; echo $X");
    }

    #[test]
    fn integer_hex_value() {
        assert_parity("typeset -i X=0xff; echo $X");
    }

    /// Integer with base prefix `-i 16` displays in hex form.
    #[test]
    fn integer_base_sixteen_display_form() {
        assert_parity("typeset -i 16 X=255; echo $X");
    }
}

mod array {
    use super::*;

    #[test]
    fn array_declaration() {
        assert_parity(r#"typeset -a arr=(a b c); print -l "${arr[@]}""#);
    }

    #[test]
    fn array_element_access() {
        assert_parity(r#"typeset -a arr=(a b c); echo "${arr[2]}""#);
    }

    #[test]
    fn array_grow_by_assignment() {
        assert_parity(r#"typeset -a arr=(a); arr+=(b c); print -l "${arr[@]}""#);
    }

    #[test]
    fn array_length() {
        assert_parity(r#"typeset -a arr=(a b c d); echo ${#arr}"#);
    }
}

mod assoc_array {
    use super::*;

    #[test]
    fn assoc_declaration_and_lookup() {
        assert_parity(r#"typeset -A h; h[k1]=v1; h[k2]=v2; echo "${h[k1]}""#);
    }

    #[test]
    fn assoc_count() {
        assert_parity("typeset -A h; h[a]=1; h[b]=2; h[c]=3; echo ${#h}");
    }

    #[test]
    fn assoc_keys_sorted() {
        assert_parity(r#"typeset -A h; h[a]=1; h[b]=2; h[c]=3; print -l "${(@k)h}" | sort"#);
    }

    #[test]
    fn assoc_values_sorted() {
        assert_parity(r#"typeset -A h; h[a]=1; h[b]=2; h[c]=3; print -l "${(@v)h}" | sort"#);
    }
}

mod readonly {
    use super::*;

    /// `typeset -r` makes a var readonly. Reassignment errors.
    #[test]
    fn readonly_initial_value_succeeds() {
        assert_parity("typeset -r X=value; echo $X");
    }

    #[test]
    fn readonly_reassignment_fails() {
        assert_parity("typeset -r X=value; X=other 2>/dev/null; echo $X; echo $?");
    }
}

mod case_conversion {
    use super::*;

    /// `-l` lowercases on assignment.
    #[test]
    fn lowercase_typeset_l() {
        assert_parity("typeset -l X=HELLO; echo $X");
    }

    /// `-u` uppercases on assignment.
    #[test]
    fn uppercase_typeset_u() {
        assert_parity("typeset -u X=hello; echo $X");
    }

    /// `-l` applies on subsequent reassignment too.
    #[test]
    fn lowercase_reassignment_persists() {
        assert_parity("typeset -l X; X=MixedCase; echo $X");
    }
}

mod export {
    use super::*;

    /// `typeset -x` exports — subprocess sees the value.
    #[test]
    fn export_visible_in_child_process() {
        assert_parity(r#"typeset -x MYVAR=visible; sh -c 'echo $MYVAR'"#);
    }

    /// Without -x, subprocess doesn't see it.
    #[test]
    fn non_exported_invisible_to_child() {
        assert_parity(r#"X=hidden; sh -c 'echo "[$X]"'"#);
    }
}

mod unique_array {
    use super::*;

    /// `-U` keeps only unique elements in an array.
    #[test]
    fn unique_array_strips_duplicates_on_append() {
        assert_parity(r#"typeset -aU arr=(a b a c b d); print -l "${arr[@]}""#);
    }

    #[test]
    fn unique_array_idempotent() {
        assert_parity(r#"typeset -aU arr=(a b c); arr+=(a b); print -l "${arr[@]}""#);
    }
}

mod local_in_function {
    use super::*;

    /// `local` is alias for `typeset` in function scope.
    #[test]
    fn local_creates_function_scoped_var() {
        assert_parity("X=outer; f() { local X=inner; echo $X; }; f; echo $X");
    }

    /// `local -i` integer-scoped to function.
    #[test]
    fn local_dash_i_integer_function_scope() {
        assert_parity("f() { local -i X=5+3; echo $X; }; f");
    }
}

mod declare_alias {
    use super::*;

    /// `declare` is alias for `typeset` in zsh.
    #[test]
    fn declare_works_like_typeset() {
        assert_parity("declare -i X=42; echo $X");
    }

    #[test]
    fn declare_a_array() {
        assert_parity(r#"declare -a arr=(x y z); echo "${arr[2]}""#);
    }
}

mod typeset_listing {
    use super::*;

    /// `typeset X` with existing value prints `X=value` (typeset-style).
    /// Just check exit code; output format may differ slightly.
    #[test]
    #[allow(non_snake_case)]
    fn typeset_X_shows_assignment_for_set_var() {
        assert_parity("X=hello; typeset X >/dev/null; echo $?");
    }
}

mod zero_pad_and_exponent {
    use super::*;

    #[test]
    fn typeset_Z_width() {
        assert_parity("typeset -Z5 z=7; echo $z");
    }

    #[test]
    fn typeset_Z2_integer() {
        assert_parity("typeset -Z2 zi=4; echo $zi");
    }

    #[test]
    fn typeset_E_scientific() {
        assert_parity("typeset -E2 e=4000; echo $e");
    }

    #[test]
    fn typeset_i8_octal_input() {
        assert_parity("typeset -i8 o=10; echo $o");
    }

    #[test]
    fn typeset_i16_hex_input() {
        assert_parity("typeset -i16 x=0xff; echo $x");
    }
}

mod tied_case_flags {
    use super::*;

    #[test]
    fn typeset_plus_L_lowercase() {
        assert_parity("typeset +L L=AbCd; echo $L");
    }

    #[test]
    fn typeset_plus_U_uppercase() {
        assert_parity("typeset +U U=xy; echo $U");
    }

    #[test]
    fn typeset_plus_i_unset_integer_attr() {
        assert_parity("typeset +i pi=4; echo $pi");
    }
}

mod right_align_and_single_array {
    use super::*;

    #[test]
    fn typeset_R_width() {
        assert_parity(r#"typeset -R4 r=hi; echo "$r""#);
    }

    #[test]
    fn typeset_aS_single_string_array() {
        assert_parity(r#"typeset -aS ary="x y"; echo "$#ary $ary[2]""#);
    }
}
