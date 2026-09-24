//! `unset` builtin deep parity:
//! scalar, array element, assoc key, function-local, readonly, special.

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

mod basic_scalar {
    use super::*;

    #[test]
    fn unset_scalar_var() {
        assert_parity(r#"X=value; unset X; echo "[$X]""#);
    }

    /// `[[ -v X ]]` checks if set.
    #[test]
    fn unset_then_v_test_false() {
        assert_parity(r#"X=value; unset X; [[ -v X ]]; echo $?"#);
    }

    /// `[[ -v X ]]` true when set.
    #[test]
    fn set_then_v_test_true() {
        assert_parity(r#"X=value; [[ -v X ]]; echo $?"#);
    }

    /// Unset of never-set var = no-op.
    #[test]
    fn unset_unknown_var_no_error() {
        assert_parity(r#"unset NEVER_SET_XYZ; echo $?"#);
    }
}

mod multi_var_unset {
    use super::*;

    /// `unset A B C` removes multiple.
    #[test]
    fn unset_multiple_scalars() {
        assert_parity(r#"A=1; B=2; C=3; unset A B C; echo "[$A][$B][$C]""#);
    }

    /// Mix of set + unset args.
    #[test]
    fn unset_mix_set_unset() {
        assert_parity(r#"A=1; unset A NONEXIST B; echo "[$A]""#);
    }
}

mod array_element {
    use super::*;

    /// `unset 'arr[2]'` removes one elem.
    #[test]
    fn unset_array_element() {
        assert_parity(r#"arr=(a b c d); unset 'arr[2]'; print -l "${(@)arr}""#);
    }

    /// Whole array.
    #[test]
    fn unset_whole_array() {
        assert_parity(r#"arr=(a b c); unset arr; echo "${#arr}""#);
    }
}

mod assoc_key {
    use super::*;

    /// `unset 'H[key]'` removes one key.
    #[test]
    fn unset_assoc_key() {
        assert_parity(r#"typeset -A H=(a 1 b 2 c 3); unset 'H[b]'; echo "${#H}""#);
    }

    /// Removed key returns empty.
    #[test]
    fn removed_key_lookup_empty() {
        assert_parity(r#"typeset -A H; H[k]=v; unset 'H[k]'; echo "[${H[k]}]""#);
    }
}

mod scoping {
    use super::*;

    /// `unset` inside function removes outer.
    #[test]
    fn unset_in_function_removes_outer() {
        assert_parity(
            r#"
OUTER=val
f() { unset OUTER; }
f
echo "[$OUTER]"
"#,
        );
    }

    /// `local X; unset X` removes local — outer should be visible after fn.
    #[test]
    fn unset_local_in_function_uncovers_outer() {
        assert_parity(
            r#"
OUTER=outer-val
f() {
  local OUTER=inner
  unset OUTER
  echo "inside:[$OUTER]"
}
f
echo "outside:[$OUTER]"
"#,
        );
    }
}

mod readonly {
    use super::*;

    /// Unset on readonly errors.
    #[test]
    fn unset_readonly_errors() {
        assert_parity(r#"readonly R=val; unset R 2>/dev/null; echo "exit=$? val=[$R]""#);
    }
}

mod special_params {
    use super::*;

    /// Unsetting $PWD — generally an error or special handling.
    #[test]
    fn unset_pwd_handling() {
        assert_parity(r#"unset PWD 2>/dev/null; echo exit=$?"#);
    }

    /// Unsetting RANDOM removes the magic.
    /// Not seed-dependent, despite what the `#[ignore]` here used to claim:
    /// `RANDOM=foo` arithmetic-evaluates to 0, so both shells `srand(0)` and
    /// the first draw is fixed. Measured 2026-08-28 — 20034 from both shells
    /// on 5 consecutive runs each, and the test passed 5/5 under `--ignored`.
    /// The ignore was hiding a passing test, so it guarded nothing.
    #[test]
    fn unset_random_then_set_literal() {
        assert_parity(r#"unset RANDOM; RANDOM=foo; echo "$RANDOM""#);
    }
}

mod unset_dash_f {
    use super::*;

    /// `unset -f f` removes function.
    #[test]
    fn unset_dash_f_function() {
        assert_parity(
            r#"
f() { echo hi; }
unset -f f
type f 2>/dev/null
echo "exit=$?"
"#,
        );
    }

    /// Var with same name as function is separate.
    #[test]
    fn unset_dash_f_doesnt_touch_var() {
        assert_parity(
            r#"
X=value
X() { echo fn; }
unset -f X
echo "[$X]"
"#,
        );
    }
}

mod unset_dash_v {
    use super::*;

    /// `unset -v X` explicitly removes variable.
    #[test]
    fn unset_dash_v_var() {
        assert_parity(r#"X=val; unset -v X; echo "[$X]""#);
    }

    /// `unset -v` doesn't touch same-named function.
    #[test]
    fn unset_dash_v_doesnt_touch_function() {
        assert_parity(
            r#"
Y=val
Y() { echo fn; }
unset -v Y
Y
"#,
        );
    }
}

mod unset_export {
    use super::*;

    /// Unset of exported var.
    #[test]
    fn unset_exported_removes_from_env() {
        assert_parity(r#"export MYV=val; unset MYV; printenv MYV 2>/dev/null; echo "exit=$?""#);
    }
}

/// `unset` of a function-local shadow of a zsh/parameter row the script
/// never read. No `zmodload` on purpose: the row is still the PM_AUTOLOAD
/// stub, and bin_unset's stub removal (c:Src/params.c:3874) must apply only
/// to that global stub, not to the local node `getnode2` returns
/// (c:Src/builtin.c:3884-3886), whose `pm->old` holds the special.
mod unset_local_shadow_of_magic_row {
    use super::*;

    /// zsh `1 1`; zshrs dropped the local with its `old` chain: `0 0`.
    #[test]
    fn assoc_shadow_unset_keeps_commands() {
        assert_parity(
            r#"f(){ local -A commands; unset commands }; f; print $+commands $(( ${#commands} > 0 ))"#,
        );
    }

    /// Same on the alias row completion functions shadow.
    #[test]
    fn assoc_shadow_unset_keeps_aliases() {
        assert_parity(
            r#"alias a1=b; f(){ local -A aliases; unset aliases }; f; print $+aliases ${(k)aliases}"#,
        );
    }

    /// Array-shaped row (PARTAB_ARRAY).
    #[test]
    fn array_shadow_unset_keeps_dirstack() {
        assert_parity(r#"f(){ local -a dirstack; unset dirstack }; f; print $+dirstack"#);
    }

    /// Pattern unset walks the same stub arm.
    #[test]
    fn pattern_unset_of_shadow_keeps_commands() {
        assert_parity(r#"f(){ local -A commands; unset -m 'command?' }; f; print $+commands"#);
    }

    /// Unset from a nested function still hits the caller's local.
    #[test]
    fn unset_from_callee_keeps_commands() {
        assert_parity(r#"g(){ unset commands }; f(){ local -A commands; g }; f; print $+commands"#);
    }

    /// Element unset acts on the local's own table: zsh `z w`, zshrs
    /// warned "assignment to invalid subscript range" and kept `x`.
    #[test]
    fn element_unset_on_assoc_shadow() {
        assert_parity(r#"f(){ local -A commands=(x y z w); unset "commands[x]"; print ${(kv)commands} }; f"#);
    }

    /// The shapes completion functions use must not touch aliastab.
    #[test]
    fn array_shadow_of_aliases_leaves_aliastab() {
        assert_parity(
            r#"alias a1=b; f(){ local -a aliases; aliases=(a b c); print ${#aliases} }; f; g(){ local -a aliases=(a b c) }; g; print ${(k)aliases}; alias"#,
        );
    }
}

mod unset_pattern {
    use super::*;

    /// `unset -m pattern` — pattern-match unset.
    #[test]
    fn unset_dash_m_pattern() {
        assert_parity(
            r#"
FOO_A=1; FOO_B=2; BAR=3
unset -m 'FOO_*'
echo "[$FOO_A][$FOO_B][$BAR]"
"#,
        );
    }
}

/// c:Src/subst.c:2805 sets vunset for a PM_UNSET node, and c:2813 emits a
/// `(t)` tag only when `(flags & PM_DECLARED) || !(flags & PM_UNSET)`.
/// `unset h` on a function's `local -a h` keeps the node, because the local
/// scope still owns it, but marks it PM_UNSET without PM_DECLARED, so `$+h`
/// is 0, `${(t)h}` is empty and `${h+word}` takes the unset branch. zshrs's array-existence probe
/// answered on the node's TYPE alone and reported a live `array-local`
/// (`1 array-local`), while an unset `local -A` already read as unset.
mod unset_array_local_reads_unset {
    use super::*;

    /// The reported repro and the default-operator family.
    #[test]
    fn set_probes_see_the_unset_local() {
        assert_parity(r#"f(){ local -a h=(a b); unset h; print -r -- $+h "[${(t)h}]" }; f"#);
        assert_parity(
            r#"f(){ local -a h=(a b); unset h; print -r -- "[${h+set}]" "[${h-unset}]" "[${h:-dflt}]" "[${h:+alt}]" }; f"#,
        );
        assert_parity(r#"f(){ local -a path_copy=(a); unset path_copy; print -r -- $+path_copy }; f; print -r -- $+path"#);
    }

    /// Reads of the unset local stay empty, and assigning revives it.
    #[test]
    fn reads_and_reassignment() {
        assert_parity(r#"f(){ local -a h=(a b); unset h; print -r -- "[${#h}]" "[${h[1]}]" "[${(@)h}]" }; f"#);
        assert_parity(r#"f(){ local -a h=(a b); unset h; h=(z); print -r -- $+h "[${(t)h}]" "[$h]" }; f"#);
    }

    /// Shapes that already agreed: an unset assoc local, an unset global
    /// array, a declared local that was never given a value, and `-g`.
    #[test]
    fn controls_that_already_agreed() {
        assert_parity(r#"f(){ local -A h=(a b); unset h; print -r -- $+h "[${(t)h}]" "[${h+set}]" }; f"#);
        assert_parity(r#"h=(a b); unset h; print -r -- $+h "[${(t)h}]" "[${h+set}]""#);
        assert_parity(r#"f(){ local -a h; print -r -- $+h "[${(t)h}]" "[${h+set}]" }; f"#);
        assert_parity(r#"f(){ typeset -ga G=(a); unset G; print -r -- $+G "[${(t)G}]" }; f"#);
    }
}

/// `unset "name[sub]"` on a SCALAR splices the subscripted character range
/// out (c:Src/builtin.c:3902-3915: `getindex(&ss, &vbuf, SCANPM_ASSIGNING)`
/// then `setstrvalue(&vbuf, ztrdup(""))`); on a numeric type it is
/// `zerrnam(name, "%s: invalid element for unset")` (c:3919-3921). The
/// scalar arm was missing, so every scalar element unset was a silent no-op.
mod scalar_element {
    use super::*;

    #[test]
    fn single_char_and_ranges() {
        for sub in ["1", "3", "-1", "2,2", "2,4", "-3,-2", "1,-1", "9"] {
            assert_parity(&format!(r#"var=value; unset 'var[{sub}]'; echo "[$var]" $?"#));
        }
    }

    #[test]
    fn zero_subscript_is_an_invalid_range() {
        assert_parity(r#"var=value; unset 'var[0]'; echo "[$var]" $?"#);
    }

    #[test]
    fn local_multibyte_and_exported() {
        assert_parity(r#"f(){ local l=abcd; unset 'l[2,3]'; echo $l }; f"#);
        assert_parity(r#"var=héllo; unset 'var[2]'; echo $var"#);
        assert_parity(r#"typeset -x e=xyz; unset 'e[1]'; echo $e; env | grep '^e='"#);
    }

    #[test]
    fn readonly_scalar_is_rejected() {
        assert_parity(r#"readonly r=abc; unset 'r[2]' 2>/dev/null; echo rc=$? $r"#);
    }

    #[test]
    fn integer_element_aborts() {
        assert_parity(r#"integer i=3; unset 'i[1]' 2>/dev/null; echo notreached"#);
    }
}

/// Nameref arms of `unset`. The release zsh binary the parity helpers use
/// has no `typeset -n`, so these are zshrs pins with the expected output of
/// the zsh dev tree (which agrees with the vendored src/zsh tree here).
mod nameref_zshrs_pin {
    use super::*;

    fn out(s: &str) -> String {
        run_zshrs(s).stdout
    }

    /// c:3897-3901 — the element unset resolves the ref and splices the
    /// referent scalar.
    #[test]
    fn subscripted_unset_through_a_ref_edits_the_referent() {
        assert_eq!(
            out(r#"typeset var=value; typeset -n p=var; unset 'p[2,3]'; typeset -p var"#),
            "typeset var=vue\n"
        );
    }

    /// A placeholder ref resolves to itself (PM_NAMEREF), which the c:3919
    /// type check rejects.
    #[test]
    fn subscripted_unset_of_a_placeholder_ref_is_invalid() {
        let r = run_zshrs(r#"typeset -n q; unset 'q[1]' 2>/dev/null; echo notreached"#);
        assert_eq!(r.stdout, "");
        assert_eq!(r.exit, 1);
    }

    /// c:3843-3846 — `unset -m` applies the literal-name nameref rule:
    /// without `-n` the referent goes, with `-n` the ref itself.
    #[test]
    fn unset_m_resolves_refs() {
        let base = r#"typeset var0=foo; typeset -n ref1=var0 ref2=ref1"#;
        assert_eq!(
            out(&format!("f() {{ {base}; unset -m ref1; typeset -p var0 ref1 ref2 2>/dev/null }}; f")),
            "typeset -n ref1=var0\ntypeset -n ref2=ref1\n"
        );
        assert_eq!(
            out(&format!("f() {{ {base}; unset -m ref2; typeset -p var0 ref1 ref2 2>/dev/null }}; f")),
            "typeset -n ref1=var0\ntypeset -n ref2=ref1\n"
        );
        assert_eq!(
            out(&format!("f() {{ {base}; unset -n -m ref1; typeset -p var0 ref1 ref2 2>/dev/null }}; f")),
            "typeset var0=foo\ntypeset -n ref2=ref1\n"
        );
    }
}

/// c:Src/builtin.c:3873-3878 — `if ((ss && !subscript) || !isident(s))
/// { zerrnam(...); }`. isident (c:Src/params.c:1309) accepts an all-digit
/// name, so `unset 1` is a quiet no-op; a bad name is zerrnam, which sets
/// errflag and abandons the rest of the command list.
mod unset_name_check_is_isident_and_zerrnam {
    use super::*;

    #[test]
    fn digit_names_are_identifiers() {
        assert_parity("set -- a b; unset 1; echo $? \"[$1]\" $#; unset 5 0; echo $?");
    }

    #[test]
    fn a_bad_name_abandons_the_list() {
        assert_parity("x=1; unset -v x 1x y 2>/dev/null; echo $? ${x-gone}");
        assert_parity("f() { unset 'a[' 2>/dev/null; echo in }; f; echo after");
        assert_parity("unset '#' 2>&1 | cut -d: -f3-");
    }
}
