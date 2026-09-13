//! A prefix assignment (`VAR=val cmd`) must put `VAR` back exactly as it was.
//!
//! C's `save_params` (`Src/exec.c:4465`) snapshots the WHOLE displaced `Param`
//! with `copyparam(tpm, pm, 0)` (`Src/exec.c:4491-4493` →
//! `Src/params.c:1257`) — type flags, base, width, level and the value union
//! together — and `restore_params` (`Src/exec.c:4519`) puts that node back.
//!
//! zshrs's live (fusevm) path saved a single `getsparam(name)` string instead,
//! which is enough for a scalar and destructive for everything else:
//!
//! ```text
//! typeset -A H=(a 1 b 2); f(){ :; }; H=x f; print "${H[a]-NONE} ${(t)H} ${#H}"
//!   zsh   -> 1    association 2
//!   zshrs -> NONE             0      # the association was DESTROYED
//!
//! a=(1 2 3); f(){ :; }; a=x f; print "${#a} ${(t)a}"
//!   zsh   -> 3 array
//!   zshrs -> 5 scalar                # the array became its joined string
//! ```
//!
//! Two separate failures, one cause. The scalar getter has nothing to read for
//! a PM_HASHED param — an association's pairs live outside the `Param`, in
//! `paramtab_hashed_storage` — so the frame recorded `None` and the restore
//! took the "name did not exist" arm and unset it. For an array the getter
//! returns the IFS-joined string, and since no type flags were saved either,
//! the restore installed a PM_SCALAR over the PM_ARRAY.
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

/// `-f` on both sides so no rc file can contribute, and the `PZ*` names are
/// cleared from the inherited environment so a stray export cannot manufacture
/// (or mask) a divergence.
fn run(bin: &str, script: &str) -> String {
    let mut cmd = Command::new(bin);
    for n in [
        "PZs", "PZarr", "PZassoc", "PZint", "PZflt", "PZexp", "PZg", "PZloc", "PZmulti",
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

/// The association half. Everything observable about the restored parameter is
/// checked — not merely that the name still exists: the type string, the
/// element count, the keys, the values, and a subscripted read.
#[test]
fn an_association_survives_a_prefix_assignment() {
    assert_matches_oracle(
        "association restored after `H=x cmd`",
        &[
            // A shell function, the case in the bug report.
            "typeset -A PZassoc=(a 1 b 2); f(){ :; }; PZassoc=x f; \
             print -r -- \"t=${(t)PZassoc}|n=${#PZassoc}|a=${PZassoc[a]-NONE}\
             |k=${(k)PZassoc}|v=${(v)PZassoc}\"",
            // A builtin, and the null builtin.
            "typeset -A PZassoc=(a 1 b 2); PZassoc=x true; \
             print -r -- \"t=${(t)PZassoc}|n=${#PZassoc}|a=${PZassoc[a]-NONE}\"",
            "typeset -A PZassoc=(a 1 b 2); PZassoc=x :; \
             print -r -- \"t=${(t)PZassoc}|n=${#PZassoc}|b=${PZassoc[b]-NONE}\"",
            // An external command.
            "typeset -A PZassoc=(a 1); PZassoc=x /usr/bin/true; \
             print -r -- \"t=${(t)PZassoc}|a=${PZassoc[a]-NONE}\"",
            // The command fails, and the command does not exist: the restore
            // is not conditional on the exit status.
            "typeset -A PZassoc=(a 1); f(){ return 3; }; PZassoc=x f; \
             print -r -- \"rc=$?|t=${(t)PZassoc}|a=${PZassoc[a]-NONE}\"",
            "typeset -A PZassoc=(a 1); PZassoc=x /nonexistent/zz 2>/dev/null; \
             print -r -- \"rc=$?|t=${(t)PZassoc}|a=${PZassoc[a]-NONE}\"",
            // A `local -A` in an enclosing scope: the restore must land on the
            // LOCAL, and the global must still be intact after the return.
            "typeset -A PZassoc=(g 1); \
             g(){ local -A PZassoc=(l 1 m 2); f(){ :; }; PZassoc=x f; \
             print -r -- \"in=${(t)PZassoc}|${#PZassoc}|l=${PZassoc[l]-NONE}\"; }; g; \
             print -r -- \"out=${(t)PZassoc}|${#PZassoc}|g=${PZassoc[g]-NONE}\"",
        ],
    );
}

/// The array half — a separate failure with the same cause. The saved snapshot
/// carried no type flags, so the joined string came back as a PM_SCALAR.
#[test]
fn an_array_survives_a_prefix_assignment() {
    assert_matches_oracle(
        "array restored after `a=x cmd`",
        &[
            "PZarr=(1 2 3); f(){ :; }; PZarr=x f; \
             print -r -- \"t=${(t)PZarr}|n=${#PZarr}|v=${PZarr[*]}|2=${PZarr[2]}\"",
            "PZarr=(1 2 3); PZarr=x true; \
             print -r -- \"t=${(t)PZarr}|n=${#PZarr}|v=${PZarr[*]}\"",
            "PZarr=(1 2 3); PZarr=x :; print -r -- \"t=${(t)PZarr}|n=${#PZarr}\"",
            "PZarr=(1 2 3); PZarr=x /usr/bin/true; \
             print -r -- \"t=${(t)PZarr}|n=${#PZarr}\"",
            // An element containing whitespace: a join/split round trip would
            // change the count even if the type were repaired.
            "PZarr=('a b' c); f(){ :; }; PZarr=x f; \
             print -r -- \"t=${(t)PZarr}|n=${#PZarr}|1=${PZarr[1]}\"",
            // The RHS reads the parameter being displaced — C snapshots before
            // `addvars` runs precisely so this works.
            "PZarr=(1 2 3); f(){ print -r -- \"inner=[$PZarr]\"; }; PZarr=$PZarr f; \
             print -r -- \"t=${(t)PZarr}|n=${#PZarr}\"",
            // `local -a` in an enclosing scope, and a `typeset -g` inside the
            // called function (which must not be reverted with the prefix).
            "PZarr=(g1 g2); \
             g(){ local -a PZarr=(l1 l2 l3); f(){ :; }; PZarr=x f; \
             print -r -- \"in=${(t)PZarr}|${#PZarr}|${PZarr[*]}\"; }; g; \
             print -r -- \"out=${(t)PZarr}|${#PZarr}|${PZarr[*]}\"",
            "f(){ typeset -g PZg=inner; }; PZarr=(1 2 3); PZarr=x f; \
             print -r -- \"t=${(t)PZarr}|n=${#PZarr}|g=$PZg\"",
        ],
    );
}

/// The types that already worked must keep working — the fix replaced the
/// restore wholesale, so scalars, integers, floats, exports, readonly
/// rejection, an unset name and the tied `PATH`/`path` pair are all pinned.
#[test]
fn the_other_parameter_types_are_unchanged() {
    assert_matches_oracle(
        "scalar / integer / float / export / readonly / unset / tied",
        &[
            "PZs=keep; f(){ :; }; PZs=x f; print -r -- \"[$PZs]|t=${(t)PZs}\"",
            "typeset -i PZint=7; f(){ :; }; PZint=9 f; \
             print -r -- \"[$PZint]|t=${(t)PZint}\"",
            "typeset -i 16 PZint=255; f(){ :; }; PZint=3 f; \
             print -r -- \"[$PZint]|t=${(t)PZint}\"",
            "typeset -F PZflt=2.5; f(){ :; }; PZflt=9 f; \
             print -r -- \"[$PZflt]|t=${(t)PZflt}\"",
            // Exported: both the parameter and the `environ` entry go back,
            // and the command itself sees the override.
            "export PZexp=keep; f(){ printenv PZexp; }; PZexp=x f; \
             print -r -- \"[$PZexp]|t=${(t)PZexp}\"; printenv PZexp",
            // A name that did not exist must not be left behind.
            "f(){ :; }; PZs=x f; print -r -- \"[${PZs-UNSET}]|t=${(t)PZs}\"",
            // PM_SPECIAL: the restore has to fire the special's own setfn, or
            // `$path` keeps the prefix value after `$PATH` is put back.
            "f(){ :; }; oldp=$PATH; oldn=${#path}; PATH=/zzz f; \
             print -r -- \"same=$([[ $PATH == $oldp ]] && print yes || print no)\
             |n=$([[ ${#path} == $oldn ]] && print same || print ${#path})|t=${(t)path}\"",
            // Several prefix assignments of mixed type in one command.
            "PZarr=(1 2); typeset -A PZassoc=(k v); PZs=s; f(){ :; }; \
             PZarr=x PZassoc=y PZs=z f; \
             print -r -- \"a=${#PZarr}/${(t)PZarr}|h=${PZassoc[k]-NONE}/${(t)PZassoc}\
             |s=$PZs/${(t)PZs}\"",
        ],
    );
}

/// A user `typeset -T` pair. `restore_params` removes the temporary scalar with
/// `unsetparam_pm(pm, 0, 0)` (c:Src/exec.c:4529), and that call cascades to the
/// tied partner through `pm->ename` (c:Src/params.c:3793-3836) before the
/// scalar is put back as a plain copy — so in zsh the ARRAY half is gone
/// afterwards. The port only cascaded from the `unsetparam` wrapper, so the
/// array survived.
#[test]
fn a_tied_pair_loses_its_partner_like_zsh() {
    assert_matches_oracle(
        "typeset -T pair under `SCALAR=x cmd`",
        &[
            "typeset -T PZs PZarr=(x y); f(){ print -r -- \"in=$PZs|${(t)PZarr}\"; }; \
             PZs=q:r f; \
             print -r -- \"s=$PZs|ts=${(t)PZs}|ta=${(t)PZarr}|n=${#PZarr}\"",
            "typeset -T PZs PZarr=(x y); PZs=q:r true; \
             print -r -- \"s=$PZs|ts=${(t)PZs}|ta=${(t)PZarr}|n=${#PZarr}\"",
            // Plain `unset` of either half.
            "typeset -T PZs PZarr=(x y); unset PZs; \
             print -r -- \"[${(t)PZs}][${(t)PZarr}]${#PZarr}\"",
            "typeset -T PZs PZarr=(x y); unset PZarr; \
             print -r -- \"[${(t)PZs}][${(t)PZarr}]\"",
        ],
    );
}
