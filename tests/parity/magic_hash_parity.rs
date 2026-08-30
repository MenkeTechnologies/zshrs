//! `zsh/parameter` magic-hash parity — the SPECIALPMDEF associations
//! (`commands`, `builtins`, `functions`, `aliases`, `galiases`,
//! `reswords`, `options`, `parameters`, …) whose contents come from a
//! module scan function rather than from stored key/value pairs.
//!
//! C: `createspecialhash` (`Src/module.c`) gives the param a fake
//! `HashTable` whose `getnode` is the module's `getpm*` and whose
//! `scantab` is its `scanpm*`; `Src/params.c:717` then reaches those
//! through `paramvalarr(v->pm->gsu.h->getfn(v->pm), v->scanflags)`.
//! There is no stored map behind any of them.
//!
//! zshrs keeps ORDINARY assoc contents in a name-keyed side map
//! (`paramtab_hashed_storage`) and the magic ones behind `PARTAB`, so
//! every read has to pick the right backing. A seeded EMPTY row in that
//! map used to answer first for a magic name, which is how
//! `compadd -k commands` (`Completion/Unix/Type/_path_commands` sh:103)
//! came back with ZERO matches in a fresh shell — command-name
//! completion offered builtins only, and only started working once some
//! other `$commands` read had gone through the scan (the scan is what
//! runs `fillcmdnamtable` under HASH_LIST_ALL,
//! `Src/Modules/parameter.c:253`).
//!
//! Counts are compared as counts, never as literal numbers: `$commands`
//! is whatever this host's `$PATH` holds. Both shells are launched with
//! the same environment, so the two counts must agree even though
//! neither is predictable.
//!
//! Skip pattern: tests no-op silently when `zsh` isn't on PATH.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

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

/// Every script here needs the module, so prepend the `zmodload` once
/// instead of repeating it in each case.
fn assert_parity(body: &str) {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let script = format!("zmodload zsh/parameter\n{body}");
    let z = Command::new(zsh_path())
        .args(["-f", "-c", &script])
        .output()
        .expect("invoke zsh");
    let r = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", &script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    let zo = String::from_utf8_lossy(&z.stdout).into_owned();
    let ro = String::from_utf8_lossy(&r.stdout).into_owned();
    let ze = String::from_utf8_lossy(&z.stderr).into_owned();
    let re = String::from_utf8_lossy(&r.stderr).into_owned();
    assert_eq!(
        zo, ro,
        "stdout divergence on:\n{script}\n--- zsh ---\n{zo:?}\n--- zshrs ---\n{ro:?}"
    );
    assert_eq!(
        ze, re,
        "stderr divergence on:\n{script}\n--- zsh ---\n{ze:?}\n--- zshrs ---\n{re:?}"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// The scan IS the backing — a magic hash must never read as empty
// ═══════════════════════════════════════════════════════════════════════

mod scan_backing {
    use super::*;

    /// The one that started this: `$commands` enumerates every
    /// executable on `$PATH`, because the scan calls `fillcmdnamtable`
    /// under HASH_LIST_ALL (`Src/Modules/parameter.c:253`). A shell that
    /// answers from an empty stored map reports 0 here and breaks
    /// command-name completion.
    #[test]
    fn commands_enumerates_the_whole_path() {
        assert_parity("print ${#${(k)commands}}");
    }

    /// Reading it TWICE must give the same answer — the scan is not
    /// allowed to be a one-shot that leaves an empty map behind.
    #[test]
    fn commands_count_is_stable_across_reads() {
        assert_parity("print ${#${(k)commands}} ${#${(k)commands}}");
    }

    /// `hash -r` empties `cmdnamtab` AND resets the PATH cursor
    /// (`Src/hashtable.c:623` emptycmdnamtable), so the next scan
    /// refills from the start rather than reporting an empty table.
    #[test]
    fn commands_refills_after_hash_r() {
        assert_parity("hash -r; print ${#${(k)commands}}");
    }

    /// Single-key read: `getpmcommand` (`Src/Modules/parameter.c:213`)
    /// fills the table on a miss under HASH_LIST_ALL, so a hashed
    /// command resolves to its absolute path.
    #[test]
    fn commands_single_key_resolves_a_path() {
        assert_parity(r#"print ${+commands[ls]} ${commands[ls]}"#);
    }

    /// A key the scan never lists reports unset and expands empty.
    #[test]
    fn commands_missing_key_is_unset_and_empty() {
        assert_parity(r#"print "${+commands[zzz_no_such_command_zzz]}" "[${commands[zzz_no_such_command_zzz]}]""#);
    }

    /// `scanpmbuiltins` — the builtin table, independent of `$PATH`.
    #[test]
    fn builtins_keys_and_values_agree() {
        assert_parity("print ${#${(k)builtins}} ${#${(v)builtins}}");
    }

    /// `getpmbuiltin` per-key read.
    #[test]
    fn builtins_single_key() {
        assert_parity("print ${builtins[print]}");
    }

    /// `scanpmfunction` sees shell functions defined in this same
    /// script; `(o)` sorts so the walk order cannot leak in.
    #[test]
    fn functions_lists_defined_shell_functions() {
        assert_parity("f(){ :; }; g(){ :; }; print ${(ok)functions}");
    }

    /// Regular vs global aliases live in two different scans.
    #[test]
    fn aliases_and_galiases_are_separate_scans() {
        assert_parity("alias -g GG=1; print ${(k)galiases}; print ${(ok)aliases}");
    }

    /// `reswords` is an ARRAY-shaped magic row, not an assoc.
    #[test]
    fn reswords_subscript_search() {
        assert_parity("print ${(k)reswords[(r)while]}");
    }

    /// `options` / `parameters` are the other two big scans.
    #[test]
    fn options_and_parameters_enumerate() {
        assert_parity("print ${#${(k)options}} ${options[interactive]} ${#${(k)parameters}}");
    }

    /// An empty magic hash must read as EMPTY, not as "missing" — the
    /// C shape is "param exists, no entries".
    #[test]
    fn an_empty_magic_hash_reads_as_zero_not_unset() {
        assert_parity("print $+dis_builtins ${#${(k)dis_builtins}}");
    }

    /// The type string must still say `special`, so a stored-map answer
    /// cannot masquerade as the real thing.
    #[test]
    fn type_flags_report_special() {
        assert_parity("print ${(t)commands} ${(t)builtins} ${(t)aliases}");
    }

    /// An ordinary assoc keeps using the stored map — the routing must
    /// not swallow normal parameters.
    #[test]
    fn an_ordinary_assoc_is_unaffected() {
        assert_parity("typeset -A h=(a 1 b 2); print ${(k)h} ${(v)h} ${h[a]}");
    }
}

// ═══════════════════════════════════════════════════════════════════════
// Function-local shadowing of a magic name
// ═══════════════════════════════════════════════════════════════════════

mod local_shadow {
    use super::*;

    /// A local ARRAY shadow works, and the special is intact after the
    /// function returns.
    #[test]
    fn local_array_shadow_then_restore() {
        assert_parity(
            "f(){ local -a commands=(x y); print $commands }; f; print ${#${(k)commands}}",
        );
    }

    /// Scoping itself is right: whatever the shadow reads as, popping it
    /// must give the special back.
    #[test]
    fn assoc_shadow_is_popped_on_return() {
        assert_parity("f(){ local -A commands=(x y); }; f; print ${#${(k)commands}}");
    }

    /// zshrs gap: a function-local ASSOC shadow of a magic name reads
    /// EMPTY. zsh prints `x`; zshrs prints nothing.
    ///
    /// Not the stored-map routing — the single-key form below fails the
    /// same way and takes a different path (the bridge's `magic_getnode`
    /// arm sends PARTAB names to `paramsubst`, never to `gethkparam`).
    /// `local -a` (array, above) is unaffected, so it is specific to the
    /// assoc shadow.
    #[test]
    #[ignore = "zshrs gap: a function-local assoc shadow of a magic hash reads empty"]
    fn assoc_shadow_keys() {
        assert_parity("f(){ local -A commands=(x y); print ${(k)commands} }; f");
    }

    /// Same gap, single-key read: zsh `y`, zshrs empty.
    #[test]
    #[ignore = "zshrs gap: a function-local assoc shadow of a magic hash reads empty"]
    fn assoc_shadow_single_key() {
        assert_parity("f(){ local -A commands=(x y); print ${commands[x]} }; f");
    }

    /// Same gap, element count: zsh `1`, zshrs `0`.
    #[test]
    #[ignore = "zshrs gap: a function-local assoc shadow of a magic hash reads empty"]
    fn assoc_shadow_count() {
        assert_parity("f(){ local -A commands=(x y); print ${#commands} }; f");
    }

    /// Same gap on a magic hash that does not depend on `$PATH`, so the
    /// fix cannot be mistaken for a `cmdnamtab` problem.
    #[test]
    #[ignore = "zshrs gap: a function-local assoc shadow of a magic hash reads empty"]
    fn assoc_shadow_of_builtins() {
        assert_parity("f(){ local -A builtins=(x y); print ${(k)builtins} }; f");
    }
}
