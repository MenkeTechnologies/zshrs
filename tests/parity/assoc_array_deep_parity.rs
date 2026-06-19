//! Associative-array deep parity:
//! `${(k)H}`, `${(v)H}`, `${(kv)H}`, sorted iteration,
//! `for k v in ${(kv)H}`, ${#H}, delete-key, nested.

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

mod creation {
    use super::*;

    /// `typeset -A H` declares assoc array.
    #[test]
    fn typeset_A_basic() {
        assert_parity(r#"typeset -A H; H[k]=v; echo "$H[k]""#);
    }

    /// `H=(k v k v)` bulk-init.
    #[test]
    fn bulk_init_pairs() {
        assert_parity(r#"typeset -A H=(k1 v1 k2 v2); echo "${H[k1]}|${H[k2]}""#);
    }

    /// Empty H.
    #[test]
    fn empty_assoc_count_zero() {
        assert_parity(r#"typeset -A H; echo "${#H}""#);
    }
}

mod lookup {
    use super::*;

    #[test]
    fn lookup_existing_key() {
        assert_parity(r#"typeset -A H; H[name]=jacob; echo "${H[name]}""#);
    }

    #[test]
    fn lookup_missing_key_empty() {
        assert_parity(r#"typeset -A H; H[a]=1; echo "[${H[nonexistent]}]""#);
    }

    /// Key with special chars.
    #[test]
    fn lookup_key_with_space() {
        assert_parity(r#"typeset -A H; H["a b"]=value; echo "${H["a b"]}""#);
    }
}

mod count {
    use super::*;

    #[test]
    fn count_three_entries() {
        assert_parity(r#"typeset -A H=(a 1 b 2 c 3); echo "${#H}""#);
    }

    #[test]
    fn count_after_add() {
        assert_parity(r#"typeset -A H=(a 1); H[b]=2; echo "${#H}""#);
    }

    #[test]
    fn count_after_delete() {
        assert_parity(r#"typeset -A H=(a 1 b 2 c 3); unset 'H[b]'; echo "${#H}""#);
    }
}

mod keys_values {
    use super::*;

    /// `${(k)H}` returns keys.
    #[test]
    fn flag_k_returns_keys_sorted() {
        // Use sort to make order deterministic across shells.
        assert_parity(r#"typeset -A H=(c 1 a 2 b 3); print -l "${(@k)H}" | sort"#);
    }

    /// `${(v)H}` returns values.
    #[test]
    fn flag_v_returns_values_sorted() {
        assert_parity(r#"typeset -A H=(c 30 a 10 b 20); print -l "${(@v)H}" | sort -n"#);
    }

    /// `${(kv)H}` interleaves keys+values.
    #[test]
    fn flag_kv_pairs() {
        // Sorted alternate-line output.
        assert_parity(r#"typeset -A H=(b 2 a 1); print -l "${(@kv)H}" | sort"#);
    }
}

mod iteration {
    use super::*;

    /// `for k v in ${(kv)H}` iterates pairs.
    #[test]
    fn for_loop_over_kv_pairs() {
        assert_parity(
            r#"
typeset -A H=(a 1 b 2 c 3)
for k v in "${(@kv)H}"; do
  echo "$k=$v"
done | sort
"#,
        );
    }

    /// Iterate keys only.
    #[test]
    fn for_loop_over_keys() {
        assert_parity(
            r#"
typeset -A H=(a 1 b 2)
for k in "${(@k)H}"; do
  echo "$k"
done | sort
"#,
        );
    }
}

mod delete {
    use super::*;

    /// `unset 'H[k]'` removes one key.
    #[test]
    fn unset_single_key() {
        assert_parity(
            r#"
typeset -A H=(a 1 b 2 c 3)
unset 'H[b]'
echo "${(@k)H}" | tr ' ' '\n' | sort
"#,
        );
    }

    /// `unset H` clears whole hash.
    #[test]
    fn unset_whole_hash() {
        assert_parity(r#"typeset -A H=(a 1 b 2); unset H; echo "${#H}""#);
    }
}

mod overwrite_and_extend {
    use super::*;

    /// Overwriting value.
    #[test]
    fn overwrite_existing_value() {
        assert_parity(r#"typeset -A H; H[k]=v1; H[k]=v2; echo "${H[k]}""#);
    }

    /// `H+=( k v )` add pair.
    #[test]
    fn extend_with_plus_eq() {
        assert_parity(r#"typeset -A H=(a 1); H+=(b 2 c 3); echo "${#H}""#);
    }
}

mod subscript_flags {
    use super::*;

    /// `${H[(I)pat]}` pattern-key lookup.
    #[test]
    fn flag_I_pattern_lookup() {
        assert_parity(
            r#"
typeset -A H=(apple 1 banana 2 cherry 3)
echo "${H[(I)b*]}"
"#,
        );
    }

    /// `${(M)H[(I)*a*]}` match-only modifier.
    #[test]
    fn flag_I_returns_all_matches() {
        assert_parity(
            r#"
typeset -A H=(apple 1 banana 2 cherry 3)
print -l "${(@k)H[(I)*a*]}" | sort
"#,
        );
    }
}

mod special_keys {
    use super::*;

    /// Empty string key.
    #[test]
    fn empty_string_key() {
        assert_parity(r#"typeset -A H; H[""]=emptykey; echo "[${H[""]}]""#);
    }

    /// Numeric-looking key.
    #[test]
    fn numeric_key_treated_as_string() {
        assert_parity(r#"typeset -A H; H[42]=meaning; echo "${H[42]}""#);
    }

    /// Key with $ char (literal).
    #[test]
    fn key_with_dollar_literal() {
        assert_parity(r#"typeset -A H; H['$weird']=ok; echo "${H['$weird']}""#);
    }
}

mod array_of_keys_in_subst {
    use super::*;

    /// `${H[$KEY]}` indirect via var.
    #[test]
    fn indirect_key_via_var() {
        assert_parity(r#"typeset -A H=(a 1 b 2); K=a; echo "${H[$K]}""#);
    }
}

mod assoc_in_function {
    use super::*;

    #[test]
    fn assoc_inside_function_local() {
        assert_parity(
            r#"
f() {
  typeset -A LOCAL_H
  LOCAL_H[x]=10
  echo "${LOCAL_H[x]}"
}
f
echo "outside=[${LOCAL_H[x]}]"
"#,
        );
    }
}

/// Nested subscript on a `(P)`-indirect reference to an ASSOC does key
/// lookup on the referenced param: `${${(P)n}[key]}` ≡ `${h[key]}` when
/// $n names assoc h (c:Src/subst.c (P) named-ref). The port flattened
/// the inner `${(P)n}` to its values first, so the outer string-key
/// subscript returned all values instead of indexing.
mod p_flag_indirect_assoc_subscript {
    use super::*;

    #[test]
    fn string_key_lookup() {
        assert_parity(r#"typeset -A h=(a 1 b 2 c 3); n=h; print -r - ${${(P)n}[b]}"#);
    }

    #[test]
    fn variable_key_lookup() {
        assert_parity(r#"typeset -A h=(a 1 b 2); n=h; k=a; print -r - ${${(P)n}[$k]}"#);
    }

    #[test]
    fn quote_flagged_outer() {
        assert_parity(r#"typeset -A h=(a 1 b 2); n=h; print -r - ${(q-)${(P)n}[b]}"#);
    }

    #[test]
    fn positional_ref_name() {
        assert_parity(r#"typeset -A opts=(a 1 b 2); set -- opts; print -r - ${${(P)1}[b]}"#);
    }

    /// `@`/`*` on a `(P)`-assoc still yields the values (regression guard).
    #[test]
    fn splat_yields_values() {
        assert_parity(r#"typeset -A h=(a 1 b 2); n=h; print -r - "${(@kP)n}""#);
    }

    /// Numeric subscript on a `(P)`-indexed-array still indexes (guard).
    #[test]
    fn p_array_numeric_subscript() {
        assert_parity(r#"typeset -a arr=(x y z); n=arr; print -r - ${${(P)n}[2]}"#);
    }

    /// Plain nested array subscript unaffected (regression guard).
    #[test]
    fn plain_nested_array_subscript() {
        assert_parity(r#"arr=(hello world); print -r - ${${arr}[2]}"#);
    }
}
