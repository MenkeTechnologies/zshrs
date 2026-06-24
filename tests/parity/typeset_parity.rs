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

/// `Src/builtin.c:2244` — typeset_single's print arm is gated on
/// `!OPT_ISSET(ops,'g')`. Bare `typeset -g NAME` is a silent declare
/// at scope, not a listing of an existing param. Hit during zinit's
/// plugin loader which runs `typeset -g VAR` for already-exported
/// env vars on every load; without the gate zshrs spammed `VAR=value`
/// per call.
mod g_suppresses_bare_print {
    use super::*;

    /// `typeset -g NAME` (NAME exists, no `=`) — silent.
    #[test]
    fn dash_g_silent_for_existing_scalar() {
        assert_parity("TPSG_A=val; typeset -g TPSG_A; echo END");
    }

    /// `-gx` — silent (export attr, no listing).
    #[test]
    fn dash_gx_silent_for_existing_scalar() {
        assert_parity("TPSG_B=val; typeset -gx TPSG_B; echo END");
    }

    /// `-gxU NAME` — silent.
    #[test]
    fn dash_gxU_silent_for_existing_scalar() {
        assert_parity("TPSG_C=val; typeset -gxU TPSG_C; echo END");
    }

    /// `-gxU NAME=val` (assignment) — silent + sets.
    #[test]
    fn dash_gxU_assignment_silent_and_sets() {
        assert_parity("typeset -gxU TPSG_D=/from/typeset; echo $TPSG_D");
    }

    /// `-g` doesn't suppress when `-p` is set: `-p` always prints
    /// reparseable form (c:2242).
    #[test]
    fn dash_g_does_not_suppress_dash_p() {
        assert_parity("TPSG_E=val; typeset -gp TPSG_E");
    }

    /// `typeset NAME` (no `-g`, NAME exists) — prints `NAME=value`.
    /// Negative pin: the `-g` gate must NOT apply when `-g` is absent.
    #[test]
    fn bare_typeset_existing_still_prints() {
        assert_parity("TPSG_F=val; typeset TPSG_F");
    }

    /// `typeset -g NAME` (NAME does NOT exist) — silent declare.
    /// Without an existing param, the print path doesn't fire anyway,
    /// but pin the surface: no output, no error.
    #[test]
    fn dash_g_fresh_name_silent() {
        assert_parity("typeset -g TPSG_FRESH; echo END");
    }

    /// `typeset NAME` inside a function — silent (creates local
    /// shadow via the c:2469 createparam path, not the print arm).
    #[test]
    fn bare_typeset_in_function_silent() {
        assert_parity("TPSG_G=outer; f() { typeset TPSG_G; }; f; echo END");
    }

    /// `typeset -g NAME` inside a function — silent (global mark,
    /// no shadow, no print).
    #[test]
    fn dash_g_in_function_silent() {
        assert_parity("TPSG_H=outer; f() { typeset -g TPSG_H; }; f; echo END");
    }
}

/// `Src/builtin.c:3042-3098` — `-m PATTERN` / `+m PATTERN` parity.
/// Two distinct subpaths in C:
///   `+m`: direct paramtab scan, PRINT_TYPE | PRINT_NAMEONLY.
///   `-m`: per-match typeset_single (gated on `-g` via c:2244).
///
/// The earlier zshrs port collapsed both into one `printparamnode(...,
/// PRINT_INCLUDEVALUE)` call which broke `+m` (emitted `name=value`
/// instead of `name`) and ignored `-g` suppression so `typeset -gm
/// 'PAT'` spammed every match. Pipe through `sort` so the parity
/// check is independent of paramtab iteration order (zsh: hash
/// bucket, zshrs: sorted via hnamcmp).
mod m_pattern_listing {
    use super::*;

    /// `-m PAT` prints `name=value` for each match.
    #[test]
    fn dash_m_prints_name_equals_value() {
        assert_parity(r#"TPSM_A=1; TPSM_B=2; typeset -m 'TPSM_*' | sort"#);
    }

    /// `+m PAT` prints names only (NAMEONLY). For typed params an
    /// attribute prefix (`integer NAME`, etc.) is included.
    #[test]
    fn plus_m_prints_names_only() {
        assert_parity(r#"TPSM_C=1; TPSM_D=2; typeset +m 'TPSM_*' | sort"#);
    }

    /// `+m PAT` with a typed match — attribute prefix appears.
    #[test]
    fn plus_m_typed_param_shows_attr_prefix() {
        assert_parity(r#"typeset -i TPSM_INT=42; typeset +m 'TPSM_INT' | sort"#);
    }

    /// `-gm PAT` — silent. The `-g` flag suppresses typeset_single's
    /// PRINT_INCLUDEVALUE emit. This was the canary bug.
    #[test]
    fn dash_gm_silent() {
        assert_parity(r#"TPSM_E=1; TPSM_F=2; typeset -gm 'TPSM_*'; echo END"#);
    }

    /// `-g +m PAT` — `+m` uses scanmatchtable directly, bypassing
    /// typeset_single's `-g` gate. Names still print.
    #[test]
    fn dash_g_plus_m_still_prints_names() {
        assert_parity(r#"TPSM_G=1; TPSM_H=2; typeset -g +m 'TPSM_*' | sort"#);
    }

    /// `-pm PAT` — reparseable form (`typeset NAME=value` lines).
    #[test]
    fn dash_pm_prints_reparseable() {
        assert_parity(r#"TPSM_I=1; typeset -pm 'TPSM_I'"#);
    }

    /// `-m` pattern with no matches — silent, exit 0.
    #[test]
    fn dash_m_no_match_silent() {
        assert_parity(r#"typeset -m 'TPSM_NO_MATCH_AT_ALL_*'; echo END"#);
    }

    /// `+m` pattern with no matches — silent, exit 0.
    #[test]
    fn plus_m_no_match_silent() {
        assert_parity(r#"typeset +m 'TPSM_NO_MATCH_AT_ALL_*'; echo END"#);
    }

    /// Quantifier `?` in pattern — single-char match works.
    #[test]
    fn dash_m_question_metachar() {
        assert_parity(r#"TPSM_X1=a; TPSM_X2=b; TPSM_LONG=c; typeset -m 'TPSM_X?' | sort"#);
    }

    /// `-m` inside a function — typeset_single's print arm runs
    /// against pattern-matched params, no shadow creation.
    #[test]
    fn dash_m_in_function_still_lists() {
        assert_parity(r#"TPSM_J=v; f() { typeset -m 'TPSM_J' | sort; }; f; echo END"#);
    }

    /// `-gm` inside a function — silent (combined with c:2244 gate).
    #[test]
    fn dash_gm_in_function_silent() {
        assert_parity(r#"TPSM_K=v; f() { typeset -gm 'TPSM_K'; }; f; echo END"#);
    }
}

/// c:Src/builtin.c:2355-2378 — type conversion of a READONLY param:
/// zsh carries readonly/exported into the new param's flags, turns
/// PM_READONLY off during the delete/recreate, and the converted
/// param comes out readonly again. Same-type readonly reassignment
/// still errors.
mod readonly_type_conversion {
    use super::*;

    #[test]
    fn readonly_array_to_assoc_allowed() {
        assert_parity(r#"typeset -r h2=(); typeset -A h2=(k v); typeset -p h2"#);
    }

    #[test]
    fn readonly_scalar_to_array_allowed() {
        assert_parity(r#"typeset -r s=x; typeset -a s=(1 2); typeset -p s"#);
    }

    #[test]
    fn readonly_assoc_to_array_allowed() {
        assert_parity(r#"typeset -rA h=(a b); typeset -a h=(1 2); typeset -p h"#);
    }

    #[test]
    fn readonly_exported_conversion_keeps_export() {
        assert_parity(r#"typeset -rx e=(); typeset -A e=(k v); typeset -p e"#);
    }

    #[test]
    fn readonly_same_type_reassign_still_errors() {
        assert_parity(r#"typeset -rA h=(a b); typeset -A h=(c d); typeset -p h"#);
    }
}

/// `name+=(array)` on an existing scalar/integer/float converts the param
/// to an array, inserting the OLD value (via getstrvalue) at the front
/// (c:params.c:3344-3354). The prior port dropped the old value for
/// numeric params (u_str was None) and mis-formatted floats (getstrvalue
/// float arm bypassed convfloat's format flags).
mod augment_scalar_to_array {
    use super::*;

    #[test]
    fn integer_append_array_keeps_old() {
        assert_parity("integer i=1; i+=(2 3); print -l $i");
    }

    #[test]
    fn integer_base_append_array_keeps_formatted_old() {
        assert_parity("typeset -i 16 x=255; x+=(2 3); print -l $x");
    }

    #[test]
    fn scalar_append_array_keeps_old() {
        assert_parity("i=hello; i+=(a b); print -l $i");
    }

    #[test]
    fn float_e_append_array_keeps_eformat_old() {
        assert_parity("float f=2.5; f+=(3.5 4.5); print -l $f");
    }

    #[test]
    fn float_E_append_array() {
        assert_parity("typeset -E f=1.5; f+=(2.5); print -l $f");
    }
}

/// getstrvalue float arm honors the format flag (PM_EFLOAT/PM_FFLOAT) and
/// precision (pm.base) per c:2366-2370 — independent of the augment path.
mod float_getstrvalue_format {
    use super::*;

    #[test]
    fn float_default_eformat() {
        assert_parity("float f=2.5; echo $f");
    }

    #[test]
    fn float_F_fixed() {
        assert_parity("typeset -F f=2.5; echo $f");
    }

    #[test]
    fn float_F_precision() {
        assert_parity("typeset -F 3 f=2.5; echo $f");
    }
}

/// `typeset -L N`/`-R N`/`-Z N` field WIDTH must persist when the param
/// is declared WITHOUT an inline value (the createparam branch,
/// c:builtin.c:2528-2533 typeset_setwidth) — not only on `name=value`.
mod valueless_field_width {
    use super::*;

    #[test]
    fn left_width_then_assign() {
        assert_parity(r#"typeset -L 10 f; f="hi"; print "[$f]""#);
    }

    #[test]
    fn left_width_then_loop() {
        assert_parity(r#"typeset -L 10 f; for f in once twice; do print "[$f]"; done"#);
    }

    #[test]
    fn right_width_then_assign() {
        assert_parity(r#"typeset -R 5 r; r=hi; print "[$r]""#);
    }

    #[test]
    fn zero_width_then_assign() {
        assert_parity(r#"typeset -Z 6 z; z=42; print "[$z]""#);
    }

    /// `-L 10 -F 3` keeps width(10) and precision(3) distinct.
    #[test]
    fn width_and_precision_distinct() {
        assert_parity(r#"typeset -L 10 -F 3 g; g=1.5; print "[$g]""#);
    }

    #[test]
    fn typeset_p_shows_width() {
        assert_parity("typeset -L 10 -F 3 g; typeset -p g");
    }
}

/// `pm.base` is shared between integer radix and float precision, so
/// switching an EXISTING numeric var BETWEEN integer and float must reset
/// it (else float precision 3 → integer base 3 = "3#10", int base 16 →
/// float 16-digit precision). E↔F and same-type re-declares keep it.
mod numeric_type_change_base_reset {
    use super::*;

    #[test]
    fn float_to_int_resets_base() {
        assert_parity("typeset -F3 f=3.14159; typeset -i f; print $f");
    }

    #[test]
    fn float_to_int_typeset_p() {
        assert_parity("typeset -F3 f=3.14159; typeset -i f; typeset -p f");
    }

    #[test]
    fn int_to_float_resets_precision() {
        assert_parity("typeset -i 16 x=255; typeset -F x; print $x");
    }

    /// Same-type re-declare KEEPS the base.
    #[test]
    fn int_to_int_keeps_base() {
        assert_parity("typeset -i16 f=255; typeset -i f; print $f");
    }

    #[test]
    fn float_to_float_keeps_precision() {
        assert_parity("typeset -F3 f=3.14159; typeset -F f; print $f");
    }
}

/// `typeset -p argv` prints the positional parameters (the `argv`/`*`/`@`
/// special array IS the positional list, stored in PPARAMS — not the
/// paramtab entry's empty u_arr). The explicit-name print path read the
/// empty u_arr and showed `( )`.
mod typeset_p_positional_array {
    use super::*;

    #[test]
    fn typeset_p_argv_global() {
        assert_parity("set -- a b c; typeset -p argv");
    }

    #[test]
    fn typeset_p_argv_in_function() {
        assert_parity("() { typeset -p argv } x y z");
    }

    #[test]
    fn typeset_p_argv_empty() {
        assert_parity("set --; typeset -p argv");
    }

    /// Regular array still prints correctly (regression guard).
    #[test]
    fn typeset_p_regular_array() {
        assert_parity("typeset -a arr=(1 2 3); typeset -p arr");
    }
}

/// `typeset -p1 NAME` prints arrays/assocs one element per line with the
/// closing paren on its own line (PRINT_LINE, c:builtin.c:2761-2765).
/// The named-arg print path ignored the `1`, printing single-line.
mod typeset_p1_multiline {
    use super::*;

    #[test]
    fn p1_array() {
        assert_parity("local -a a=(x y z); typeset -p1 a");
    }

    #[test]
    fn p1_assoc() {
        assert_parity("local -A h=(one two three four); typeset -p1 h");
    }

    #[test]
    fn p1_empty_assoc() {
        assert_parity("local -A e; typeset -p1 e");
    }

    /// quoted/special elements one-per-line.
    #[test]
    fn p1_array_quoting() {
        assert_parity(r#"local -a a=('&' sand '""' '' plugh); typeset -p1 a"#);
    }

    /// `-p` (no 1) stays single-line (regression guard).
    #[test]
    fn p_without_1_single_line() {
        assert_parity("local -a a=(x y z); typeset -p a");
    }
}

/// `typeset +m`/`-m PATTERN` reveal PM_HIDE params (V10private + magic
/// assocs). The `-m`/`+m` PATTERN path uses scanmatchtable, which (unlike
/// the bare scanhashtable list path) does NOT exclude PM_HIDE — an
/// explicit pattern reveals hidden params. Pinned as DIRECT assertions
/// (not assert_parity) because homebrew zsh 5.9.1 predates the pmtypes
/// "hide" row (c:Src/params.c:6022, commit 6b21e5c0e2, post-5.9.1), so
/// zshrs (ported from the newer tree) emits "hide" where 5.9.1 omits it.
/// These pins assert the actual bug fixes, version-skew-independent:
///   1. a `private` var APPEARS under `typeset +m` (was wrongly filtered
///      by a blanket PM_HIDE skip — builtin.rs +m scan path).
///   2. its listing carries NO spurious "readonly" (PM_RO_BY_DESIGN
///      readonly-attr expansion must be gated on !PM_REMOVABLE —
///      params.rs printparamnode; V10private.ztst:31 → `local hide x`).
///   3. IPDEF4 specials (LINENO) STILL show "readonly" (gate regression
///      guard — they are PM_READONLY_SPECIAL, not removable).
mod private_and_hidden_listing {
    use super::*;

    fn has_private_module() -> bool {
        run_zshrs("zmodload zsh/param/private; echo ok")
            .stdout
            .trim()
            == "ok"
    }

    #[test]
    fn private_var_appears_in_typeset_plus_m() {
        if !has_private_module() {
            return;
        }
        let r = run_zshrs("zmodload zsh/param/private; () { private px=1; typeset +m px }");
        // Was empty before the +m PM_HIDE-filter removal.
        assert!(
            r.stdout.contains("px"),
            "private var missing from `typeset +m`: {:?}",
            r.stdout
        );
        assert!(
            r.stdout.contains("local"),
            "private var not listed as local: {:?}",
            r.stdout
        );
    }

    #[test]
    fn private_var_listing_has_no_spurious_readonly() {
        if !has_private_module() {
            return;
        }
        let r = run_zshrs("zmodload zsh/param/private; () { private px=1; typeset +m px }");
        // PM_RO_BY_DESIGN must NOT expand to the readonly attr for a
        // PM_REMOVABLE private var (V10private.ztst:31 = `local hide px`).
        assert!(
            !r.stdout.contains("readonly"),
            "private var wrongly shows readonly: {:?}",
            r.stdout
        );
    }

    #[test]
    fn ipdef4_special_still_shows_readonly() {
        // Regression guard for the !PM_REMOVABLE gate: LINENO is
        // PM_READONLY_SPECIAL (non-removable) and keeps its readonly attr.
        let r = run_zshrs("typeset +m LINENO");
        assert!(
            r.stdout.contains("readonly"),
            "LINENO lost its readonly attr: {:?}",
            r.stdout
        );
    }

    #[test]
    fn magic_assoc_revealed_by_pattern() {
        // `typeset +m 'a*'` must surface the `aliases` magic assoc
        // (scanmatchtable reveals PM_HIDE). Was filtered before.
        let r = run_zshrs("typeset +m 'a*'");
        assert!(
            r.stdout.contains("aliases"),
            "magic assoc `aliases` not revealed by +m pattern: {:?}",
            r.stdout
        );
    }
}

/// Assignment to a declared `private` var. is_readonly_param blanket-
/// rejected privates (they carry PM_RO_BY_DESIGN), so a SAME-scope write
/// errored "read-only variable". The gate now mirrors the private GSU's
/// pps_setfn level check (c:Src/Modules/param_private.c:300-307): a write
/// in the SAME scope (`locallevel == pm->level`) or above the wrap level
/// is permitted; a deeper nested-scope write is still rejected and aborts.
mod private_assignment_scope {
    use super::*;

    fn has_private() -> bool {
        run_zshrs("zmodload zsh/param/private; echo ok")
            .stdout
            .trim()
            == "ok"
    }
    fn p(s: &str) {
        if !has_private() {
            return;
        }
        assert_parity(&format!("zmodload zsh/param/private 2>/dev/null; {s}"));
    }

    /// Same-scope write (scalar, +=, valueless-then-set) is permitted.
    #[test]
    fn same_scope_write_permitted() {
        p("() { private px=1; px=2; print $px }");
        p("() { private px=1; px=2; px+=x; print $px }");
        p("() { private px; px=set; print $px }");
    }

    /// A private shadowing a global, written in the same scope, leaves
    /// the global intact after the scope exits.
    #[test]
    fn private_shadow_write_restores_global() {
        p("typeset -g g=1; () { private g=2; g=3; print $g }; print $g");
    }

    /// Nested-scope write of an outer function's private still ERRORS and
    /// aborts the inner function (stdout + nonzero exit match zsh).
    #[test]
    fn nested_scope_write_rejected() {
        p("f(){ private x=inner; g; }; g(){ x=fromG; print \"g:$x\" }; f");
        p("f(){ private x=1; g; print \"f:$x\" }; g(){ print \"g:${x:-unset}\"; x=2 }; f");
    }

    /// Nested-scope READ of a shadowing private falls through to the
    /// shadowed global (the private is hidden), unaffected by the gate.
    #[test]
    fn nested_scope_read_falls_through() {
        p("x=outer; f(){ private x=inner; g; print $x }; g(){ print ${x:-unset} }; f");
    }

    /// Array private same-scope assignment (regression — already worked
    /// via the array store path, must stay working).
    #[test]
    fn array_private_same_scope() {
        p("() { private -a arr; arr=(a b c); print -l $arr }");
    }

    /// Non-private + real readonly unaffected (regression).
    #[test]
    fn ordinary_and_readonly_unaffected() {
        p("typeset -g y=1; y=2; print $y");
        p("readonly r=1; r=2; print after");
    }
}

/// `local -P`/`local -Pa` — zsh/param/private's setup_ (c:Src/Modules/
/// param_private.c:682-685) REPLACES the `local` builtintab node's
/// handlerfunc+optstr with bin_private's once the module is loaded, so
/// `local` accepts the -P private-scope flags. zshrs replicates the swap
/// by routing `local` through the `private` builtin when the module is
/// bound. Before the module loads, `local -P` errors "bad option: -P"
/// exactly like stock zsh (compared via assert_parity stdout + exit).
mod local_dash_p_handler_swap {
    use super::*;

    /// Before zmodload, `local -P` is an unknown option (stdout + exit).
    #[test]
    fn local_dash_p_errors_before_module_load() {
        assert_parity("() { local -P p=1; print ${p:-none} }");
        assert_parity("local -P p=1; print ${p:-none}");
    }

    /// After zmodload, `local -P`/`local -Pa` create private-scoped vars.
    #[test]
    fn local_dash_p_after_module_load() {
        assert_parity("zmodload zsh/param/private; () { local -P p=1; print $p }");
        assert_parity("zmodload zsh/param/private; () { local -Pa arr=(a b c); print -l $arr }");
        assert_parity("zmodload zsh/param/private; () { local -PU u=(a a b); print -l $u }");
    }

    /// Plain `local` (no -P) still behaves as a normal local after the
    /// swap — bin_typeset treats `local`/`private` identically.
    #[test]
    fn plain_local_unaffected_after_load() {
        assert_parity("zmodload zsh/param/private; () { local q=9; print $q }");
        assert_parity("zmodload zsh/param/private; x=g; () { local x=loc; print $x }; print $x");
        assert_parity(
            "typeset -g gg=5; zmodload zsh/param/private; () { local gg=7; gg=8; print $gg }; print $gg",
        );
    }

    /// A `local -P` private is hidden from a nested function (fall-through
    /// read of the shadowed global), same as `private -P`.
    #[test]
    fn local_private_hidden_in_nested_scope() {
        assert_parity(
            "zmodload zsh/param/private; () { local -P p=1; g() { print ${p:-unset} }; g; print $p }",
        );
    }
}
