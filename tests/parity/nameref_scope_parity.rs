//! Named references across scope changes, unset through references and
//! type changes through references.
//!
//! Homebrew zsh 5.9.2 predates `typeset -n`, so the nameref cases pin the
//! output of Test/K01nameref.ztst (the upstream acceptance spec, confirmed
//! against a zsh built from current source). Cases that do not involve a
//! nameref compare live against the installed zsh.

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

fn zsh_path() -> Option<&'static str> {
    ["/opt/homebrew/bin/zsh", "/usr/local/bin/zsh", "/bin/zsh", "/usr/bin/zsh"]
        .into_iter()
        .find(|p| Path::new(p).exists())
}

/// (status, stdout, stderr)
fn run(bin: &Path, zsh_mode: bool, code: &str) -> (i32, String, String) {
    let mut cmd = Command::new(bin);
    if zsh_mode {
        cmd.arg("--zsh").env_remove("ZSHRS_CACHE");
    }
    let o = cmd.args(["-f", "-c", code]).output().expect("spawn shell");
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

fn zshrs(code: &str) -> (i32, String, String) {
    run(&zshrs_bin(), true, code)
}

fn assert_live_parity(code: &str) {
    let Some(zsh) = zsh_path() else { return };
    let want = run(Path::new(zsh), false, code);
    let got = zshrs(code);
    assert_eq!((got.0, &got.1), (want.0, &want.1), "code: {code}\nzshrs stderr: {}", got.2);
}

// c:Src/params.c:5886-5893 endparamscope → setscope: a reference bound to a
// local of the scope being left rebinds to the enclosing binding of the
// same name.
#[test]
fn nameref_rebinds_to_enclosing_variable_on_scope_exit() {
    let (_, out, _) = zshrs(
        "typeset -n ref; typeset var=g
         () { typeset var=l1
              () { typeset var=l2; ref=var; echo A $ref }
              echo B $ref }
         echo C $ref",
    );
    assert_eq!(out, "A l2\nB l1\nC g\n");
}

// K01 "hidden reference refers to a nested variable": ref3 -> ref2, where
// the global placeholder ref2 is hidden by a local integer; assigning
// through ref3 binds the HIDDEN ref2, and the binding follows scope exits.
#[test]
fn assignment_through_hidden_placeholder_binds_the_hidden_reference() {
    let (_, out, _) = zshrs(
        "typeset -n ref1; typeset -n ref2; typeset -n ref3=ref2; typeset var=aaa
         () {
           typeset -i ref2=123; typeset var=bbb
           () { typeset var=ccc; ref1=var; ref3=var
                echo A:ref1=$ref1 ref2=$ref2 ref3=$ref3 }
           echo B:ref1=$ref1 ref2=$ref2 ref3=$ref3
         }
         echo E:ref1=$ref1 ref2=$ref2 ref3=$ref3",
    );
    assert_eq!(
        out,
        "A:ref1=ccc ref2=123 ref3=ccc\nB:ref1=bbb ref2=123 ref3=bbb\nE:ref1=aaa ref2=aaa ref3=aaa\n"
    );
}

// c:Src/builtin.c:3949-3951 — a chain ending at a placeholder resolves to a
// PM_NAMEREF, so `unset` leaves it alone.
#[test]
fn unset_of_placeholder_reference_has_no_effect() {
    let (_, out, _) = zshrs(
        "f() { typeset -n ref1 ref2=ref1; unset $1; typeset -p ref1 ref2 }
         f ref1; f ref2",
    );
    assert_eq!(
        out,
        "typeset -n ref1=''\ntypeset -n ref2=ref1\ntypeset -n ref1=''\ntypeset -n ref2=ref1\n"
    );
}

// c:Src/params.c:3851-3874 — unset through a reference: a visible global is
// removed, an enclosing-scope local is kept unset (so `typeset -p` is
// silent for it).
#[test]
fn unset_through_reference_to_enclosing_scope() {
    let (_, out, _) = zshrs(
        "f() { typeset -n refg=g refl=l
               () { typeset -g g=glb; typeset l=lcl
                    () { unset refg refl }
                    typeset -p g l 2>&1 } }
         f",
    );
    assert_eq!(out, "(anon):typeset:2: no such variable: g\n");
}

// c:Src/params.c:3859-3869 + c:5896-5902 — a hidden global unset through a
// reference stays hidden until its scope is left, then is deleted; a hidden
// local is marked unset. The visible locals are untouched.
#[test]
fn unset_through_reference_to_hidden_parameters() {
    let (_, out, _) = zshrs(
        "f() { typeset -g g=glb; typeset l=lcl; typeset -n refg=g refl=l
               () { typeset g=hide-g; typeset l=hide-l; unset refg refl
                    typeset -p g l }
               typeset -p g l 2>&1 }
         f",
    );
    assert_eq!(
        out,
        "typeset g=hide-g\ntypeset l=hide-l\nf:typeset:3: no such variable: g\n"
    );
}

// c:Src/params.c:6379-6382 setloopvar — a for-loop reference variable
// rejects an invalid referent and the loop stops there.
#[test]
fn for_loop_reference_rejects_invalid_name() {
    let (st, out, err) =
        zshrs("typeset -n ref=var; for ref in valid1 inv@lid valid2; do typeset -p ref; done");
    assert_eq!(out, "typeset -n ref=valid1\n");
    assert!(err.contains("invalid variable name: inv@lid"), "stderr: {err}");
    assert_eq!(st, 1);
}

// c:Src/params.c:3740-3746 resetparam — a type change through a reference
// to a variable hidden by a local is refused.
#[test]
fn type_change_of_hidden_variable_through_reference_is_refused() {
    let (_, out, err) = zshrs(
        "typeset -A -g ass0=(aa AA); typeset -a -g arr0=(aa AA); typeset -g str0=foo
         () { typeset -n Ass0=ass0 Arr0=arr0 Str0=str0
              typeset ass0 arr0 str0
              { Ass0=foo } always { TRY_BLOCK_ERROR=0 }; echo $?
              { Arr0=foo } always { TRY_BLOCK_ERROR=0 }; echo $?
              { Str0=(x) } always { TRY_BLOCK_ERROR=0 }; echo $? }
         typeset -p ass0 arr0 str0",
    );
    assert_eq!(
        out,
        "1\n1\n1\ntypeset -A ass0=( [aa]=AA )\ntypeset -a arr0=( aa AA )\ntypeset str0=foo\n"
    );
    for n in ["ass0", "arr0", "str0"] {
        assert!(
            err.contains(&format!("can't change type of hidden variable: {n}")),
            "stderr: {err}"
        );
    }
}

// c:Src/exec.c:2654-2663 — an assignment through an upper reference whose
// target is not in an enclosing scope fails with status 1.
#[test]
fn assignment_through_out_of_scope_upper_reference_fails() {
    let (_, out, _) = zshrs(
        "() { () { local var; typeset -nu ptr1=var; ptr1=outer; echo rc=$?; typeset -p var } }",
    );
    assert_eq!(out, "rc=1\ntypeset var=''\n");
}

// c:Src/builtin.c:2115-2126 + c:2355-2378 — removing the array or numeric
// type with `typeset +a` / `+i` converts the parameter, directly and
// through a reference.
#[test]
fn typeset_type_removal_converts_parameter() {
    assert_live_parity("typeset -a arr=(a A); typeset +a arr=barfu; typeset -p arr");
    assert_live_parity("typeset -A h=(a A); typeset +A h=barfu; typeset -p h");
    assert_live_parity("typeset -i i=1; typeset +i i=x; typeset -p i");
    assert_live_parity("f() { local -a l=(a b); typeset +a l=s; typeset -p l; }; f");
    assert_live_parity("typeset -ax ex=(a); typeset +a ex=s; typeset -p ex");
    let (_, out, _) = zshrs(
        "typeset var; typeset -n ref=var
         unset var; typeset -a var=(a A); typeset +a ref=barfu; typeset -p var
         unset var; typeset -i var=12345; typeset +i ref=barfu; typeset -p var",
    );
    assert_eq!(out, "typeset var=barfu\ntypeset var=barfu\n");
}

// c:Src/builtin.c:3103-3104 — `typeset -p NAME` prints with the accumulated
// printflags, which include PRINT_WITH_NAMESPACE, so a `.ns.name` prints.
#[test]
fn typeset_p_prints_namespaced_parameter() {
    let (_, out, _) = zshrs(
        "typeset -gr .K01.readonly=RO; typeset .a=1 b=2
         typeset -p .K01.readonly .a; typeset | grep -c '^\\.a='",
    );
    assert_eq!(out, "typeset -r .K01.readonly=RO\ntypeset .a=1\n0\n");
}
