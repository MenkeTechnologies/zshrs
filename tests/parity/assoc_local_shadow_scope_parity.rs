//! An inner `typeset -A NAME` under an OUTER non-hashed local of the same
//! name, and the scope it must not outlive.
//!
//! C keeps an association's pairs in the Param itself (`pm->u.hash`), so the
//! unwind is automatic: `createparam` allocates the local with
//! `zshcalloc` (c:Src/params.c:1157), which leaves its `u.hash` NULL while
//! the outer binding — pairs and all — hangs off `pm->old` (c:1158);
//! `scanendscope`'s non-special arm (c:5969-5970) calls `unsetparam_pm`,
//! whose `pm->gsu.s->unsetfn(pm, exp)` (c:3800) reaches `stdunsetfn`'s
//! `case PM_HASHED: pm->gsu.h->setfn(pm, NULL)` (c:3922-3925) and
//! `hashsetfn` frees that local table (c:4045-4050). Every one of those
//! steps dispatches on the type of the LOCAL being destroyed. The outer
//! param's type is never consulted, because there is no second place the
//! pairs could be.
//!
//! zshrs keeps the pairs in `paramtab_hashed_storage`, a map keyed by NAME
//! with no scope dimension, so both halves have to be done by hand — and the
//! save/restore was gated on the OUTER param being PM_HASHED instead. A
//! hashed local under a scalar or array outer therefore saved nothing and
//! restored nothing, and its row sat in the map for the rest of the process:
//!
//!     outer() { local H; inner; print "in: ${(t)H} ${#H}" }
//!     inner() { typeset -A H; H[x]=1 }
//!     outer; print "after: ${(t)H} ${#H}"
//!
//!     zsh:   in: scalar-local 0   / after:  0
//!     zshrs: in: scalar-local 1   / after: association 1
//!
//! The association was visible THROUGH the outer function's scalar local
//! while `inner` ran, and survived `outer`'s return as a global.
//!
//! The adjacent shapes already agreed and are pinned here as controls: a
//! hashed local with no outer binding at all, a hashed local under a hashed
//! outer, and a non-hashed local under a hashed outer (the shape fixed
//! earlier — the restore of the outer's bag must keep working).
//!
//! Skip pattern: tests no-op silently when `zsh` isn't on PATH.

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

/// Run `script` under both shells and require identical stdout. `expected` is
/// the oracle's own answer, written out here so the assertion still says what
/// the right answer IS when a diff shows up.
fn assert_parity_expect(script: &str, expected: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let z = Command::new(zsh_path())
        .args(["-fc", script])
        .output()
        .expect("invoke zsh");
    let r = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");

    let z_out = String::from_utf8_lossy(&z.stdout).into_owned();
    let r_out = String::from_utf8_lossy(&r.stdout).into_owned();
    assert_eq!(
        z_out, expected,
        "the recorded oracle answer is stale for script:\n{script}\n--- zsh ---\n{z_out:?}"
    );
    assert_eq!(
        z_out, r_out,
        "stdout divergence on script:\n{script}\n--- zsh ---\n{z_out:?}\n--- zshrs ---\n{r_out:?}"
    );
    assert_eq!(
        z.status.code().unwrap_or(-1),
        r.status.code().unwrap_or(-1),
        "exit divergence on script:\n{script}"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// The leak: a hashed local under a NON-hashed outer local.
// ═══════════════════════════════════════════════════════════════════════════

mod hashed_local_under_a_non_hashed_outer {
    use super::*;

    /// The reported repro, both halves: what the outer function sees while the
    /// inner one runs, and what is left at top level afterwards.
    #[test]
    fn typeset_A_under_a_scalar_local_dies_with_its_scope() {
        assert_parity_expect(
            r#"outer() { local H; inner; print "in: [${(t)H}] ${#H} [${(k)H}]" }
inner() { typeset -A H; H[x]=1 }
outer
print "after: [${(t)H}] ${#H} [${(k)H}] ${+H}""#,
            "in: [scalar-local] 0 []\nafter: [] 0 [] 0\n",
        );
    }

    /// Same shape spelled `local -A`, which takes a different arm of
    /// `bin_typeset` on the way to `createparam`.
    #[test]
    fn local_A_under_a_scalar_local_dies_with_its_scope() {
        assert_parity_expect(
            r#"outer() { local H; inner; print "in: [${(t)H}] ${#H}" }
inner() { local -A H; H[x]=1 }
outer
print "after: [${(t)H}] ${#H} ${+H}""#,
            "in: [scalar-local] 0\nafter: [] 0 0\n",
        );
    }

    /// An ARRAY outer: the leaked row answered subscripted reads of the outer
    /// ARRAY, so `${(k)H}` listed the inner's keys while `${#H}` still counted
    /// the (empty) array.
    #[test]
    fn typeset_A_under_an_array_local_dies_with_its_scope() {
        assert_parity_expect(
            r#"outer() { local -a H; inner; print "in: [${(t)H}] ${#H} [${(k)H}]" }
inner() { typeset -A H; H[x]=1 }
outer
print "after: [${(t)H}] ${#H} ${+H}""#,
            "in: [array-local] 0 []\nafter: [] 0 0\n",
        );
    }

    /// The outer local's own value must survive the inner declaration —
    /// `${#H}` over the scalar is a STRING length, which is what the leaked
    /// association was answering instead.
    #[test]
    fn the_outer_scalar_keeps_its_value_and_its_length() {
        assert_parity_expect(
            r#"inner() { typeset -A H; H[s]=2 }
outer() { local H=one; inner; print "a: [$H] ${#H}"; inner; print "b: [$H] ${#H}" }
outer
print "after: ${+H}""#,
            "a: [one] 3\nb: [one] 3\nafter: 0\n",
        );
    }

    /// Three deep, one type per level: the array in the middle must come back
    /// with ITS OWN elements, not with the innermost function's pairs.
    #[test]
    fn three_deep_scalar_array_assoc() {
        assert_parity_expect(
            r#"l3() { typeset -A H; H[z]=3; print "l3: [${(t)H}] ${(kv)H}" }
l2() { local -a H; H=(a b); l3; print "l2: [${(t)H}] ${#H} ${H}" }
l1() { local H=s; l2; print "l1: [${(t)H}] [$H]" }
l1
print "top: [${(t)H}] ${+H}""#,
            "l3: [association-local] z 3\nl2: [array-local] 2 a b\nl1: [scalar-local] [s]\ntop: [] 0\n",
        );
    }

    /// `unset` inside the inner function drops the inner's binding only; the
    /// outer's scalar is still there when the scope pops.
    #[test]
    fn unset_inside_the_inner_function() {
        assert_parity_expect(
            r#"u2() { typeset -A H; H[x]=1; unset H; print "u2: [${(t)H}] ${+H}" }
u1() { local H=keepme; u2; print "u1: [${(t)H}] [$H]" }
u1
print "top: [${(t)H}] ${+H}""#,
            "u2: [] 0\nu1: [scalar-local] [keepme]\ntop: [] 0\n",
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Controls — shapes that already agreed. They are here so a future change to
// the save/restore gate cannot buy the case above by breaking one of these.
// ═══════════════════════════════════════════════════════════════════════════

mod shapes_that_already_agreed {
    use super::*;

    /// No outer binding at all: the local's row is removed by the
    /// `!had_outer` arm of endparamscope, not by a shadow frame.
    #[test]
    fn hashed_local_with_no_outer_binding() {
        assert_parity_expect(
            r#"f() { typeset -A M; M[k]=v; print "in: [${(t)M}] ${#M}" }; f; print "after: [${(t)M}] ${+M}""#,
            "in: [association-local] 1\nafter: [] 0\n",
        );
    }

    /// Hashed under hashed: the outer's pairs come back untouched.
    #[test]
    fn hashed_local_over_a_hashed_outer() {
        assert_parity_expect(
            r#"m2() { typeset -A H; H[b]=2; print "m2: ${(kv)H}" }
m1() { typeset -A H; H[a]=1; m2; print "m1: ${(kv)H}" }
typeset -A H; H[g]=0; m1; print "top: [${(t)H}] ${(kv)H}""#,
            "m2: b 2\nm1: a 1\ntop: [association] g 0\n",
        );
    }

    /// Non-hashed local over a hashed GLOBAL — the direction fixed earlier.
    /// The global's pairs must be readable again after the scope pops.
    #[test]
    fn plain_local_over_a_hashed_global_restores_the_pairs() {
        assert_parity_expect(
            r#"typeset -A h=(a 1); f(){ local h; }; f; print "[${h[a]}] [${(t)h}]""#,
            "[1] [association]\n",
        );
    }

    /// A `local -a` inside a `local -A` scope: the array's subscripted read
    /// must not route to the hash storage.
    #[test]
    fn array_local_inside_an_assoc_local_scope() {
        assert_parity_expect(
            r#"g() { local -a h; h=(a b c); print "g: ${h[2]}" }
f() { local -A h; h[x]=1; g; print "f: ${(kv)h}" }
f
print "top: ${+h}""#,
            "g: b\nf: x 1\ntop: 0\n",
        );
    }
}
