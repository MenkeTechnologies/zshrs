//! A prefix assignment in front of a SHELL FUNCTION marks its variable
//! exported for the duration of the call.
//!
//! `execcmd` builds `addvars`' flags from the resolved command word
//! (`Src/exec.c:4137-4147`):
//!
//! ```text
//! /* Export this if the command is a shell function,
//!  * but not if it's a builtin.
//!  */
//! int flags = 0;
//! if (is_shfunc)
//!     flags |= ADDVAR_EXPORT;
//! ```
//!
//! and `addvars` turns that bit into a temporary `allexport`
//! (`Src/exec.c:2641-2651`):
//!
//! ```text
//! if ((addflags & ADDVAR_EXPORT) && !strchr(name, '[')) {
//!     ...
//!     allexp = opts[ALLEXPORT];
//!     opts[ALLEXPORT] = 1;
//!     if (isset(KSHARRAYS))
//!         unsetparam(name);
//!     pm = assignsparam(name, val, myflags);
//!     opts[ALLEXPORT] = allexp;
//! }
//! ```
//!
//! so the assignment runs as if `setopt allexport` were in force and the
//! parameter ends up PM_EXPORTED — `createparam` ORs the bit in for a name
//! that did not exist (`Src/params.c:1170-1171`), `assignstrvalue`'s tail
//! calls `export_param` for one that did (`Src/params.c:2836-2841`), and
//! `addenv` is what actually sets the flag (`Src/params.c:5478`).
//!
//! zshrs only published the value into the process environment with
//! `zputenv`, so the child of the command saw it but the PARAMETER never
//! carried the flag:
//!
//! ```text
//! f(){ print ${(t)ZQX} }; ZQX=y f
//!   zsh   -> scalar-export
//!   zshrs -> scalar
//!
//! f(){ export -p | grep ZQX }; ZQX=y f
//!   zsh   -> export ZQX=y
//!   zshrs -> (nothing)
//! ```
//!
//! Every expected value below is what `/opt/homebrew/bin/zsh` itself prints;
//! the tests compare against a live oracle rather than a hardcoded string, so
//! they cannot drift from it.
//!
//! Skip pattern: tests no-op silently when zsh isn't available.

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

/// `-f` on both sides so no rc file can contribute, and every `PXe*` name is
/// cleared from the inherited environment so a stray export cannot manufacture
/// (or mask) a divergence — these tests read exported-ness, which is exactly
/// what an inherited value would fake.
fn run(bin: &str, script: &str) -> String {
    let mut cmd = Command::new(bin);
    for n in [
        "PXes", "PXearr", "PXeassoc", "PXeint", "PXeflt", "PXeexp", "PXeloc", "PXeb", "PXesub",
    ] {
        cmd.env_remove(n);
    }
    let o = cmd
        .args(["-f", "-c", script])
        .output()
        .unwrap_or_else(|e| panic!("invoke {bin}: {e}"));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn assert_matches_oracle(what: &str, scripts: &[&str]) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let bin = zshrs_bin();
    let bin = bin.to_str().expect("utf-8 path");
    for script in scripts {
        let want = run(zsh_path(), script);
        let got = run(bin, script);
        assert_eq!(
            got, want,
            "{what}: `{script}`\n  zsh   {want:?}\n  zshrs {got:?}"
        );
    }
}

/// The reported case: a name that did not exist before the prefix. C's
/// `createparam` ORs PM_EXPORTED in under the forced `allexport`
/// (`Src/params.c:1170-1171`), so `${(t)}`, `typeset -p` and `export -p` all
/// have to agree that it is exported, and the child still has to see it.
#[test]
fn a_prefix_assignment_to_a_shell_function_exports() {
    assert_matches_oracle(
        "`X=y shellfn` marks X exported",
        &[
            "f(){ print -r -- ${(t)PXes} }; PXes=y f",
            "f(){ typeset -p PXes }; PXes=y f",
            "f(){ export -p | grep '^export PXes' }; PXes=y f",
            // The environment side must not regress: the child of the
            // function still sees the value.
            "f(){ /usr/bin/printenv PXes }; PXes=y f",
            // Two names in one prefix, and the same name twice.
            "f(){ print -r -- \"${(t)PXes}|${(t)PXeb}\" }; PXes=y PXeb=z f",
            "f(){ print -r -- ${(t)PXes} }; PXes=y PXes=z f",
        ],
    );
}

/// The other half of the C comment at `Src/exec.c:4138-4140` — "but not if
/// it's a builtin". ADDVAR_EXPORT is never set for a builtin or for the
/// `command` / `builtin` prefixes, so the parameter stays a plain scalar.
/// A fix that exported unconditionally would break these.
#[test]
fn a_prefix_assignment_to_a_builtin_does_not_export() {
    assert_matches_oracle(
        "`X=y builtin` leaves X unexported",
        &[
            "PXes=y eval 'print -r -- ${(t)PXes}'",
            "PXes=y builtin eval 'print -r -- ${(t)PXes}'",
            "PXes=y eval 'export -p' | grep -c '^export PXes' ",
            // `command` forces the external lookup, so nothing in the
            // shell's own parameter table survives the call either way.
            "PXes=y command true; print -r -- \"after=${(t)PXes}:${PXes-UNSET}\"",
        ],
    );
}

/// A name that ALREADY existed takes the other C arm —
/// `assignstrvalue`'s `isset(ALLEXPORT)` tail calls `export_param`
/// (`Src/params.c:2836-2841`) — and it has to work for every parameter type,
/// including the ones whose displaced value does not live in `u.str`.
#[test]
fn an_existing_parameter_of_any_type_is_exported_and_restored() {
    assert_matches_oracle(
        "existing parameter exported for the call, restored after",
        &[
            "typeset PXes=old; f(){ print -r -- \"${(t)PXes}:$PXes\" }; PXes=new f; \
             print -r -- \"after=${(t)PXes}:$PXes\"",
            "integer PXeint=3; f(){ print -r -- ${(t)PXeint} }; PXeint=9 f; \
             print -r -- \"after=${(t)PXeint}:$PXeint\"",
            "float PXeflt=1.5; f(){ print -r -- ${(t)PXeflt} }; PXeflt=2.5 f; \
             print -r -- \"after=${(t)PXeflt}:$PXeflt\"",
            // The displaced array/association must come back untouched —
            // this is the matrix `45d5bb3760` pinned, re-checked with the
            // export flag now in play.
            "PXearr=(a b c); f(){ print -r -- \"${(t)PXearr}:$PXearr\" }; PXearr=x f; \
             print -r -- \"after=${(t)PXearr}:n=${#PXearr}:${PXearr[*]}\"",
            "typeset -A PXeassoc=(a 1 b 2); f(){ print -r -- ${(t)PXeassoc} }; PXeassoc=x f; \
             print -r -- \"after=${(t)PXeassoc}:n=${#PXeassoc}:${PXeassoc[a]-NONE}\"",
            // Already exported: the flag was there before and is still
            // there after, with the ORIGINAL value back in place.
            "typeset -x PXeexp=old; f(){ print -r -- \"${(t)PXeexp}:$PXeexp\" }; PXeexp=new f; \
             print -r -- \"after=${(t)PXeexp}:$PXeexp\"",
        ],
    );
}

/// The prefix lands on a `local` from an enclosing scope: the export is added
/// to THAT node (`scalar-local-export`), and the local is back to
/// `scalar-local` with its own value once the inner call returns.
#[test]
fn a_prefix_assignment_over_a_local_exports_the_local() {
    assert_matches_oracle(
        "`X=y shellfn` over an enclosing local",
        &[
            "g(){ print -r -- \"${(t)PXeloc}:$PXeloc\"; /usr/bin/printenv PXeloc }; \
             f(){ local PXeloc=loc; PXeloc=pfx g; \
             print -r -- \"after=${(t)PXeloc}:$PXeloc\" }; f; \
             print -r -- \"outer=${(t)PXeloc}:${PXeloc-UNSET}\"",
            // A `local` declared in the CALLED function shadows the prefix
            // entirely, so it is a plain `scalar-local`.
            "f(){ local PXeloc=inner; print -r -- ${(t)PXeloc} }; PXeloc=y f; \
             print -r -- \"after=${(t)PXeloc}:${PXeloc-UNSET}\"",
        ],
    );
}

/// The two guards inside C's export arm. `!strchr(name, '[')`
/// (`Src/exec.c:2641`) keeps a subscripted prefix out of it, and
/// `if (isset(KSHARRAYS)) unsetparam(name)` (`Src/exec.c:2648-2649`) forces
/// the name to be recreated as a scalar instead of written into the array
/// that is already there.
#[test]
fn the_export_arms_own_guards_hold() {
    assert_matches_oracle(
        "subscripted prefix and the KSH_ARRAYS unsetparam",
        &[
            "setopt ksharrays; PXearr=(1 2); f(){ print -r -- \"${(t)PXearr}:${PXearr[0]}\" }; \
             PXearr=x f; print -r -- \"after=${(t)PXearr}:n=${#PXearr}\"",
            "PXesub=(1 2 3); f(){ print -r -- \"${(t)PXesub}:${PXesub[2]}\" }; PXesub[2]=x f; \
             print -r -- \"after=${(t)PXesub}:${PXesub[*]}\"",
        ],
    );
}

/// `allexport` already on, and `typeset -x` inside the called function: the
/// save/restore of `opts[ALLEXPORT]` (`Src/exec.c:2646` / `c:2651`) has to
/// leave the option exactly as it found it, and the restore of the parameter
/// must not be confused by the flag the call added.
#[test]
fn the_allexport_option_itself_is_left_alone() {
    assert_matches_oracle(
        "allexport saved and restored around the assignment",
        &[
            "setopt allexport; f(){ print -r -- ${(t)PXes} }; PXes=y f; \
             print -r -- \"opt=${options[allexport]}:after=${(t)PXes}:${PXes-UNSET}\"",
            "f(){ print -r -- ${(t)PXes} }; PXes=y f; \
             print -r -- \"opt=${options[allexport]}\"; PXeb=1; print -r -- ${(t)PXeb}",
            "f(){ typeset -x PXes; print -r -- ${(t)PXes} }; PXes=y f; \
             print -r -- \"after=${(t)PXes}:${PXes-UNSET}\"",
        ],
    );
}

/// A prefix assignment on a tied special. `PATH` is already exported, so the
/// export arm changes nothing observable about it — but the tied `$path`, the
/// child's environment and the restore all still have to behave.
#[test]
fn a_tied_special_still_behaves_under_the_export_arm() {
    assert_matches_oracle(
        "`PATH=/zzz shellfn`",
        &[
            "f(){ print -r -- \"${(t)PATH}|$PATH|${path[1]}|${#path}\" }; \
             p1=$path[1]; PATH=/zzz f; print -r -- \"after=${(t)PATH}:$path[1]:$p1\"",
            "f(){ /usr/bin/printenv PATH }; PATH=/zzz f",
        ],
    );
}
