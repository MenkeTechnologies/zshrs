//! Re-declaring an association with `-a` must discard its pairs.
//!
//! `typeset -A H; H[x]=1; typeset -a H; H=(p q)` is a TYPE CHANGE, and C
//! handles it entirely inside `typeset_single`: `chflags` picks up the
//! PM_HASHED/PM_ARRAY difference (c:Src/builtin.c:2117-2119), so `tc` is set
//! and c:Src/builtin.c:2351-2374 runs `unsetparam_pm(pm, 0, 1)` on the old
//! parameter before the new one is built. That reaches `stdunsetfn`'s
//! `case PM_HASHED: pm->gsu.h->setfn(pm, NULL)` (c:Src/params.c:3922-3925),
//! and `hashsetfn` frees the table (c:Src/params.c:4045-4049 —
//! `deleteparamtable(pm->u.hash)`). The pairs die with the Param because in C
//! they live INSIDE it.
//!
//! zshrs keeps them in `paramtab_hashed_storage`, a map keyed by NAME with no
//! type and no scope dimension, so flipping the node's type bits left the row
//! in place and the SUBSCRIPTED read paths kept resolving through it:
//!
//!     typeset -A ZQW; ZQW[x]=1; typeset -a ZQW; ZQW=(p q)
//!
//!     zsh:   ${ZQW[1]}=p  ${(k)ZQW}="p q"  ${(t)ZQW}=array  ${#ZQW}=2
//!     zshrs: ${ZQW[1]}=    ${(k)ZQW}="x"    ${(t)ZQW}=array  ${#ZQW}=2
//!
//! The type flipped, the elements were stored, and `${ZQW[@]}` / `$ZQW` /
//! `${#ZQW}` all read back correctly — only indexing came back empty, and
//! `${(k)…}` / `${(v)…}` answered with the dead pair. Setting an array and
//! not being able to read it back by index is silent data loss, which is why
//! the every-access-path spread is pinned here rather than one probe.
//!
//! The directions that already agreed are pinned as controls so a future
//! change to the discard cannot quietly take them with it: `-A` over `-A`
//! KEEPS the pairs (`on & ~pm->node.flags` is empty, so `tc` stays 0), while
//! `+A`, `-i`, `-F` and a plain scalar re-declare all drop them through other
//! arms.
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
///
/// Every probe name is unique to this file and the child environment is
/// scrubbed of it, so an inherited export cannot manufacture a divergence
/// (an exported name reports `scalar-export`, not `scalar`).
fn assert_parity_expect(script: &str, expected: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let mut z = Command::new(zsh_path());
    let mut r = Command::new(zshrs_bin());
    for probe in ["ZQW", "ZQA", "ZQF", "ZQG"] {
        z.env_remove(probe);
        r.env_remove(probe);
    }
    let z = z.args(["-fc", script]).output().expect("invoke zsh");
    let r = r
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
// The data loss: which access paths answered from the discarded row.
// ═══════════════════════════════════════════════════════════════════════════

mod association_redeclared_as_an_array {
    use super::*;

    /// Every read of the converted parameter, in one script. The four
    /// subscripted forms are the ones that came back empty; the four
    /// whole-value forms were already right, and are here so a regression
    /// cannot be read as "the array was never stored".
    #[test]
    fn every_access_path_reads_the_new_array() {
        assert_parity_expect(
            r#"typeset -A ZQW; ZQW[x]=1; typeset -a ZQW; ZQW=(p q)
print "idx: [${ZQW[1]}][${ZQW[2]}][${ZQW[-1]}][${ZQW[1,2]}]"
print "scan: [${(k)ZQW}][${(v)ZQW}][${(kv)ZQW}]"
print "whole: [${ZQW[@]}][${ZQW[*]}][$ZQW] t=${(t)ZQW} n=${#ZQW}""#,
            "idx: [p][q][q][p q]\nscan: [p q][p q][p q]\nwhole: [p q][p q][p q] t=array n=2\n",
        );
    }

    /// The row was still answering BEFORE any array value was assigned, so the
    /// empty declaration is its own probe: `${(k)…}` listed the dead key while
    /// `${#…}` already counted the empty array.
    #[test]
    fn the_bare_declaration_already_drops_the_pairs() {
        assert_parity_expect(
            r#"typeset -A ZQW; ZQW[x]=1; typeset -a ZQW
print "[${(k)ZQW}][${(v)ZQW}][${ZQW[1]}] t=${(t)ZQW} n=${#ZQW}""#,
            "[][][] t=array n=0\n",
        );
    }

    /// `-g` takes a different arm on the way to the flag stamp.
    #[test]
    fn the_global_spelling_drops_them_too() {
        assert_parity_expect(
            r#"typeset -gA ZQW; ZQW[x]=1; typeset -ga ZQW; ZQW=(p q)
print "[${ZQW[1]}][${(k)ZQW}] t=${(t)ZQW}""#,
            "[p][p q] t=array\n",
        );
    }

    /// Inside a function the leak compounded: the row outlived the scope that
    /// created it, because the unwind reads the node's type and the node was
    /// no longer PM_HASHED by then. Both halves are asserted — what the
    /// function sees, and what is left behind.
    #[test]
    fn a_local_conversion_does_not_outlive_its_scope() {
        assert_parity_expect(
            r#"ZQF() { typeset -A ZQW; ZQW[x]=1; typeset -a ZQW; ZQW=(p q)
print "in: [${ZQW[1]}][${(k)ZQW}] ${(t)ZQW}" }
ZQF
print "after: [${(t)ZQW}] ${#ZQW} ${+ZQW}""#,
            "in: [p][p q] array-local\nafter: [] 0 0\n",
        );
    }

    /// A local conversion under a GLOBAL association of the same name: the
    /// outer bag must come back untouched, keys and all.
    #[test]
    fn the_shadowed_outer_association_comes_back() {
        assert_parity_expect(
            r#"typeset -A ZQW; ZQW[k]=outer
ZQG() { typeset -A ZQW; ZQW[i]=inner; typeset -a ZQW; ZQW=(p q)
print "in: [${ZQW[1]}][${(k)ZQW}] ${(t)ZQW}" }
ZQG
print "after: [${ZQW[k]}][${(k)ZQW}] ${(t)ZQW} ${#ZQW}""#,
            "in: [p][p q] array-local\nafter: [outer][k] association 1\n",
        );
    }

    /// `-g` from inside a function writes the GLOBAL binding, which the scope
    /// pop must then leave alone.
    #[test]
    fn a_global_conversion_from_a_function_survives_the_return() {
        assert_parity_expect(
            r#"ZQF() { typeset -gA ZQW; ZQW[x]=1; typeset -ga ZQW; ZQW=(p q) }
ZQF
print "[${ZQW[1]}][${(k)ZQW}] ${(t)ZQW} ${#ZQW}""#,
            "[p][p q] array 2\n",
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Controls: the transitions that already agreed.
// ═══════════════════════════════════════════════════════════════════════════

mod the_other_transitions_are_unchanged {
    use super::*;

    /// `-A` over `-A` leaves `on & ~pm->node.flags` empty, so `tc` stays 0
    /// (c:Src/builtin.c:2117-2119) and the pairs are KEPT. This is the one
    /// direction a too-eager discard would break.
    #[test]
    fn redeclaring_as_an_association_keeps_the_pairs() {
        assert_parity_expect(
            r#"typeset -A ZQW; ZQW[x]=1; typeset -A ZQW
print "[${(k)ZQW}][${(v)ZQW}] ${(t)ZQW} ${#ZQW}""#,
            "[x][1] association 1\n",
        );
    }

    /// `typeset +A` turns the attribute OFF, which c:Src/builtin.c:2117-2119
    /// also scores as a type change; an association has no scalar form, so the
    /// recreated scalar is EMPTY.
    #[test]
    fn dropping_the_attribute_leaves_an_empty_scalar() {
        assert_parity_expect(
            r#"typeset -A ZQW; ZQW[x]=1; typeset +A ZQW
print "[$ZQW][${(k)ZQW}][${ZQW[1]}] ${(t)ZQW} ${#ZQW}""#,
            "[][][] scalar 0\n",
        );
    }

    /// Re-declared as a plain scalar, then assigned. An optionless `typeset
    /// NAME` on an existing parameter also PRINTS it
    /// (c:Src/builtin.c:2244-2246), so the listing line is part of the
    /// oracle's answer here.
    #[test]
    fn redeclaring_as_a_scalar_drops_the_pairs() {
        assert_parity_expect(
            r#"typeset -A ZQA; ZQA[x]=1; typeset ZQA; ZQA=plain
print "[$ZQA][${(k)ZQA}] ${(t)ZQA}""#,
            "ZQA=( [x]=1 )\n[plain][plain] scalar\n",
        );
    }

    /// Re-declared as an integer.
    #[test]
    fn redeclaring_as_an_integer_drops_the_pairs() {
        assert_parity_expect(
            r#"typeset -A ZQA; ZQA[x]=1; typeset -i ZQA; ZQA=7
print "[$ZQA][${(k)ZQA}][${ZQA[1]}] ${(t)ZQA}""#,
            "[7][7][7] integer\n",
        );
    }

    /// Re-declared as a float — same arm as the integer, different width.
    #[test]
    fn redeclaring_as_a_float_drops_the_pairs() {
        assert_parity_expect(
            r#"typeset -A ZQA; ZQA[x]=1; typeset -F ZQA; ZQA=2.5
print "[$ZQA][${(k)ZQA}] ${(t)ZQA}""#,
            "[2.5000000000][2.5000000000] float\n",
        );
    }

    /// An `unset` between the two declarations always worked, and is the
    /// shape the fix has to agree with.
    #[test]
    fn an_explicit_unset_between_the_declarations_still_works() {
        assert_parity_expect(
            r#"typeset -A ZQA; ZQA[x]=1; unset ZQA; typeset -a ZQA; ZQA=(p q)
print "[${ZQA[1]}][${(k)ZQA}] ${(t)ZQA}""#,
            "[p][p q] array\n",
        );
    }

    /// Re-declaring a plain ARRAY as an array is not a type change, so its
    /// elements survive — the discard must not key off `-a` alone.
    #[test]
    fn redeclaring_an_array_as_an_array_keeps_its_elements() {
        assert_parity_expect(
            r#"typeset -a ZQA; ZQA=(p q); typeset -a ZQA
print "[${ZQA[1]}][${ZQA[@]}] ${(t)ZQA} ${#ZQA}""#,
            "[p][p q] array 2\n",
        );
    }

    /// The scope repro from the fix that landed before this one: an inner
    /// `typeset -A` under an outer non-hashed local must not be visible
    /// through the outer local, nor outlive it.
    #[test]
    fn a_hashed_local_under_a_scalar_local_still_dies_with_its_scope() {
        assert_parity_expect(
            r#"ZQF() { local ZQW; ZQG; print "in: ${(t)ZQW} ${#ZQW}" }
ZQG() { typeset -A ZQW; ZQW[x]=1 }
ZQF
print "after: ${(t)ZQW} ${#ZQW} ${+ZQW}""#,
            "in: scalar-local 0\nafter:  0 0\n",
        );
    }
}
