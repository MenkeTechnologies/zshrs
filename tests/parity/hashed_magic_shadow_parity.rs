//! `local -A NAME` over one of zsh/parameter's magic hashes.
//!
//! `commands`, `aliases`, `galiases`, `functions`, `options`, `nameddirs`,
//! `userdirs` are `partab[]` rows (c:Src/Modules/parameter.c:2235-2298) backed
//! by a C hash table rather than by parameter storage. A `local NAME` inside a
//! function replaces the special's paramtab node with a plain one
//! (c:Src/params.c:1090-1115 createparam), and until the scope pops every
//! later lookup finds the PLAIN node — so the magic getfn/scanfn is
//! unreachable. zshrs keeps the magic rows in separate static tables matched
//! BY NAME, so it re-imposes that shadow explicitly.
//!
//! The guard was too broad: it bailed on ANY shadowed magic name. But C
//! dispatches on the TYPE of the node paramtab actually returned —
//! c:Src/params.c:2270 `PM_TYPE(pm->node.flags) & (PM_ARRAY|PM_HASHED)`, then
//! getarg at c:1597-1606 `ht = v->pm->gsu.h->getfn(v->pm); …
//! ht->getnode(ht, s)` — so a shadow that is ITSELF PM_HASHED owns a table and
//! must be served from it. Only a NON-hash shadow (`local options`,
//! `local -a options`) hides the row.
//!
//! The values were stored correctly the whole time (`${NAME[@]}` found them);
//! each reader in turn simply refused to look, so `${#NAME}`, `${(k)NAME}`,
//! `${(v)NAME}`, `${(kv)NAME}` and `${NAME[(I)pat]}` all came back empty.
//!
//! Stock caller: `Completion/Zsh/Type/_command_names:70` (`local -A +h
//! commands`).
//!
//! The control rows are the reason the guard existed: `local options` and
//! `local -a options` (git's `git-completion.bash` `__git_resolve_builtins`
//! does exactly that) must still shadow the magic row completely.
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

fn assert_parity(script: &str) {
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
// A PM_HASHED shadow is served from its own table.
// ═══════════════════════════════════════════════════════════════════════════

mod hashed_shadow_is_readable {
    use super::*;

    /// The reported repro: whole-map reads over a `local -A` shadow.
    #[test]
    fn count_and_kv_over_local_A_commands() {
        assert_parity(
            r#"f() { local -A commands; commands=(a b); print "n=${#commands} kv=(${(kv)commands})" }; f"#,
        );
    }

    #[test]
    fn keys_and_values_flags() {
        assert_parity(
            r#"f() { local -A commands; commands=(a b); print "k=(${(k)commands}) v=(${(v)commands})" }; f"#,
        );
    }

    /// The `(I)` scan reads through the raw-value arm, a fourth guard.
    #[test]
    fn index_scan_subscript() {
        assert_parity(
            r#"f() { local -A commands; commands=(a b c d); print "[${commands[(I)*]}]" }; f"#,
        );
    }

    /// The keyed read — the half fixed in params.rs; pinned here so the two
    /// halves cannot drift apart again.
    #[test]
    fn keyed_read_and_element_write() {
        assert_parity(
            r#"f() { local -A commands; commands=(fake /bin/fake); commands[k]=v; print "[$commands[fake]][$commands[k]]" }; f"#,
        );
    }

    #[test]
    fn splat_agrees_with_the_flag_reads() {
        assert_parity(
            r#"f() { local -A commands; commands=(a b); print "[${commands[@]}] n=${#commands}" }; f"#,
        );
    }

    /// `_command_names:70` uses the `+h` (no-export/hide) spelling.
    #[test]
    fn local_A_plus_h_spelling() {
        assert_parity(
            r#"f() { local -A +h commands; commands=(a b); print "n=${#commands} kv=(${(kv)commands})" }; f"#,
        );
    }

    #[test]
    fn typeset_A_spelling() {
        assert_parity(
            r#"f() { typeset -A commands; commands=(a b); print "n=${#commands} kv=(${(kv)commands})" }; f"#,
        );
    }

    #[test]
    fn aliases_row() {
        assert_parity(
            r#"f() { local -A aliases; aliases=(a b); print "n=${#aliases} kv=(${(kv)aliases})" }; f"#,
        );
    }

    #[test]
    fn galiases_row() {
        assert_parity(r#"f() { local -A galiases; galiases=(a b); print "n=${#galiases}" }; f"#);
    }

    #[test]
    fn functions_row() {
        assert_parity(r#"f() { local -A functions; functions=(a b); print "n=${#functions}" }; f"#);
    }

    #[test]
    fn options_row() {
        assert_parity(
            r#"f() { local -A options; options=(a b); print "n=${#options} kv=(${(kv)options})" }; f"#,
        );
    }

    #[test]
    fn nameddirs_and_userdirs_rows() {
        assert_parity(
            r#"f() { local -A nameddirs; nameddirs=(a b); print "n=${#nameddirs}" }; g() { local -A userdirs; userdirs=(a b); print "n=${#userdirs}" }; f; g"#,
        );
    }

    /// A hashed shadow with nothing written to it is EMPTY, not the magic row.
    #[test]
    fn empty_hashed_shadow_reads_empty() {
        assert_parity(r#"f() { local -A commands; print "n=${#commands}" }; f"#);
    }

    #[test]
    fn loop_over_kv_pairs() {
        assert_parity(
            r#"f() { local -A commands; commands=(a b); for k v in ${(kv)commands}; do print "$k=$v"; done }; f"#,
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// CONTROLS — a NON-hash shadow still hides the magic row, and no shadow at
// all still reaches it.
// ═══════════════════════════════════════════════════════════════════════════

mod non_hash_shadow_still_hides {
    use super::*;

    /// `git-completion.bash`'s `__git_resolve_builtins` shape.
    #[test]
    fn local_scalar_options_reads_empty() {
        assert_parity(r#"f() { local options; print "[${options}] n=${#options}" }; f"#);
    }

    #[test]
    fn local_array_options_reads_the_array() {
        assert_parity(r#"f() { local -a options; options=(x y); print "[${options}] n=${#options}" }; f"#);
    }

    #[test]
    fn local_array_commands_reads_the_array() {
        assert_parity(
            r#"f() { local -a commands; commands=(x y); print "[${commands}] n=${#commands}" }; f"#,
        );
    }

    /// A scalar shadow's numeric subscript is a CHARACTER index, not a key.
    #[test]
    fn scalar_shadow_takes_the_scalar_subscript_arm() {
        assert_parity(r#"f() { local options=abc; print "[${options[2]}]" }; f"#);
    }
}

mod unshadowed_magic_rows_still_work {
    use super::*;

    #[test]
    fn global_options_key_read() {
        assert_parity(r#"print "[${options[monitor]}]""#);
    }

    #[test]
    fn global_commands_is_set() {
        assert_parity(r#"print "[${+commands}]""#);
    }

    #[test]
    fn global_functions_key_read() {
        assert_parity(r#"aa() { print hi }; print "[${functions[aa]}]""#);
    }

    /// The magic row is large; a count of it must not read as the empty map.
    #[test]
    fn global_options_count_is_large() {
        assert_parity(r#"print "[$(( ${#options} > 100 ))]""#);
    }
}
