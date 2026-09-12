//! Unsetting an association must destroy its pairs.
//!
//! In C an association's pairs live INSIDE the `Param` (`pm->u.hash`), so
//! `stdunsetfn`'s `PM_HASHED` arm — `pm->gsu.h->setfn(pm, NULL)`
//! (`Src/params.c:3922-3925`), i.e. `hashsetfn` (`Src/params.c:4045-4050`)
//! `if (pm->u.hash && pm->u.hash != x) deleteparamtable(pm->u.hash); pm->u.hash
//! = x;` with a NULL `x` — is the whole teardown.
//!
//! zshrs keeps the pairs in `paramtab_hashed_storage`, a side map keyed by
//! NAME, and `arrhashsetfn` (the only writer of whole associations) never
//! populates `pm.u_hash`. So the port's `PM_HASHED` arm, which cleared
//! `pm.u_hash` and nothing else, was inert, and every caller that wanted the
//! pairs gone carried its own copy of the row removal instead. The paths that
//! carried no such copy silently did not unset the pairs at all:
//!
//! ```text
//! typeset -A h=(a 1 b 2); unset -m 'h*'; print ${h[a]-NONE}
//! ```
//!
//! answered `1` where zsh answers `NONE`.
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

fn run(bin: &str, script: &str) -> String {
    let o = Command::new(bin)
        .args(["-f", "-c", script])
        .output()
        .unwrap_or_else(|e| panic!("invoke {bin}: {e}"));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// Compare zshrs against the live oracle for each script.
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

/// The case the inert arm broke outright. `unset -m PAT` (`Src/builtin.c:3842`)
/// reaches `unsetparam` with no row removal of its own, so before
/// `stdunsetfn`'s `PM_HASHED` arm did C's work the pairs simply survived the
/// unset: `${h[a]}` still read `1`, `${#h}` still read `2`, and `${(t)h}` still
/// said `association`.
#[test]
fn unset_m_destroys_an_associations_pairs() {
    assert_matches_oracle(
        "unset -m on an association",
        &[
            "typeset -A h=(a 1 b 2); unset -m 'h*'; \
             print -r -- \"t=${(t)h}|n=${#h}|a=${h[a]-NONE}|k=${(k)h}\"",
            // More than one match, so the scan arm runs the removal per node.
            "typeset -A hx=(a 1); typeset -A hy=(c 3); unset -m 'h?'; \
             print -r -- \"x=${hx[a]-NONE}|y=${hy[c]-NONE}\"",
            // From inside a function, where the name is a global and the unset
            // is not a scope pop.
            "typeset -A h=(a 1); f(){ unset -m 'h*'; }; f; print -r -- \"a=${h[a]-NONE}\"",
            // A pattern that matches an association and a scalar together.
            "typeset -A hm=(a 1); hs=plain; unset -m 'h*'; \
             print -r -- \"m=${hm[a]-NONE}|s=${hs-NONE}\"",
        ],
    );
}

/// The ordinary `unset NAME` path (`Src/builtin.c:3952-3953`). This one always
/// worked, because `bin_unset` carried its own copy of the row removal; the
/// copy is gone now and `stdunsetfn` does it, so this pins that the move did
/// not change the answer — including the readonly rejection, where C takes its
/// `return 1` at `Src/params.c:3852` BEFORE reaching the unsetfn and so must
/// still drop no pairs.
#[test]
fn plain_unset_still_destroys_the_pairs() {
    assert_matches_oracle(
        "unset NAME on an association",
        &[
            "typeset -A h=(a 1 b 2); unset h; \
             print -r -- \"t=${(t)h}|n=${#h}|k=${(k)h}|a=${h[a]-NONE}\"",
            "typeset -A h=(a 1 b 2); f(){ unset h; }; f; \
             print -r -- \"t=${(t)h}|n=${#h}|a=${h[a]-NONE}\"",
            "f(){ typeset -g -A h=(a 1); unset h; }; f; \
             print -r -- \"t=${(t)h}|n=${#h}|a=${h[a]-NONE}\"",
            // Readonly: the unset is rejected, so the pairs must survive.
            "typeset -rA h=(a 1); unset h 2>/dev/null; \
             print -r -- \"rc=$?|t=${(t)h}|a=${h[a]-NONE}\"",
            // Unset then recreate: no stale pair may leak into the new bag.
            "typeset -A h=(a 1); unset h; typeset -A h=(z 9); \
             print -r -- \"k=${(k)h}|z=${h[z]}|a=${h[a]-NONE}\"",
            // A single element, which is a different C arm entirely
            // (`Src/builtin.c:3916` subscript form) and must be unaffected.
            "typeset -A h=(a 1 b 2); unset 'h[a]'; print -r -- \"k=${(k)h}|n=${#h}\"",
        ],
    );
}

/// `local -A` shadowing is the one place a name-keyed row removal could reach
/// the wrong binding. It does not come through `stdunsetfn` at all —
/// `endparamscope` unlinks the popped node itself and restores the outer
/// scope's row from `PARAMTAB_HASHED_SHADOW_STACK` — so the outer association
/// must be intact after the function returns, whether or not the local was
/// unset inside it.
#[test]
fn a_local_shadow_leaves_the_outer_association_intact() {
    assert_matches_oracle(
        "local -A over a global association",
        &[
            "typeset -A h=(a 1 b 2); f(){ local -A h; h[x]=9; unset h; \
             print -r -n \"in=${(t)h}|${h[a]-NONE}|\"; }; f; \
             print -r -- \"out=${(t)h}|n=${#h}|k=${(k)h}|a=${h[a]-NONE}\"",
            "typeset -A h=(a 1 b 2); f(){ local -A h; h[x]=9; \
             print -r -n \"in=${(k)h}|\"; }; f; \
             print -r -- \"out=${(k)h}|a=${h[a]-NONE}\"",
            "typeset -A h=(a 1); f(){ local -A h; unset h; \
             print -r -n \"in=${h[a]-NONE}|\"; }; f; print -r -- \"out=${h[a]-NONE}\"",
            // A local ARRAY shadowing a global association: the shadow owns no
            // row, and the outer's must come back.
            "typeset -A h=(a 1); g(){ local -a h; h=(p q r); \
             print -r -n \"arr=${h[2]}|t=${(t)h}|\"; }; g; \
             print -r -- \"out=${(t)h}|a=${h[a]-NONE}\"",
            // Unset from a nested function, two levels below the definition.
            "typeset -A g=(a 1); o(){ i(){ unset g; }; i; \
             print -r -n \"mid=${g[a]-NONE}|\"; }; o; print -r -- \"out=${g[a]-NONE}\"",
        ],
    );
}

/// The `3a31c066de` type-change matrix. `bin_typeset`'s `+a`/`+A` arm used to
/// remove the row itself, immediately before calling `unsetparam`; that copy is
/// gone and `unsetparam` now does it on the way through. Re-declaring an
/// association as an array or a scalar must still leave nothing of the old bag
/// behind for a subscripted read to find.
#[test]
fn redeclaring_an_association_discards_its_pairs() {
    assert_matches_oracle(
        "association type change",
        &[
            "typeset -A H; H[x]=1; typeset -a H; H=(p q); \
             print -r -- \"t=${(t)H}|1=${H[1]}|n=${#H}|k=${(k)H}|v=${(v)H}\"",
            "typeset -A H; H[x]=1; typeset +A H; print -r -- \"t=${(t)H}|v=[$H]|n=${#H}\"",
            "typeset -A H=(k v); typeset +A H; print -r -- \"t=${(t)H}|v=[$H]|n=${#H}\"",
            "typeset -a H=(p q); typeset -A H; print -r -- \"t=${(t)H}|n=${#H}|k=${(k)H}\"",
            "typeset -a A=(1 2 3); typeset +a A; print -r -- \"t=${(t)A}|v=[$A]\"",
            // A plain scalar assignment over an association is the other
            // discard path (`Src/params.c:3242` resetparam) and must be
            // unchanged by this.
            "typeset -A H=(k v); H=str 2>/dev/null; print -r -- \"t=${(t)H}|v=[$H]|n=${#H}\"",
        ],
    );
}
