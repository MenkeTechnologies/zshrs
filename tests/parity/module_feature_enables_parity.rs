//! Parity for the per-module FEATURE-ENABLE table — `zmodload -F`'s
//! `+name`/`-name` state.
//!
//! C stores one enable bit per feature ON THE DESCRIPTOR the module ships:
//! `getfeatureenables` (`Src/module.c:3330-3337`) reads
//! `b->node.flags & BINF_ADDED`, `cd->flags & CONDF_ADDED`,
//! `mf->flags & MFF_ADDED` and `pd->pm != NULL`, and `setfeatureenables`
//! (`c:3358-3381`) writes them back. Which bits are set depends on HOW the
//! module was loaded:
//!
//!   * `zmodload MODULE` passes a NULL `enablesarr`, which `do_module_features`
//!     (`c:2108-2117`) turns into "enable everything";
//!   * a demand-load — `ensurefeature(mod, "b:", name)` from `resolvebuiltin`
//!     (`c:Src/exec.c:2703`) or `ensurefeature(mod, "p:", name)` from
//!     `loadparamnode` (`c:Src/params.c:568`) — passes a ONE-ELEMENT array
//!     (`c:3428`), so `do_module_features` (`c:2081-2107`) enables that single
//!     feature and leaves every sibling off.
//!
//! Most zshrs module ports have no descriptor table to carry the bit, and
//! their `handlefeatures` shims answered the GET direction with a hardcoded
//! all-ones vector — so a plain `compinit`, which reaches `zsh/parameter` and
//! `zsh/zutil` only through the demand path, reported all 33 / all 4 features
//! enabled where zsh reports the two or three that were actually asked for.
//! `_zmodload` (`Completion/Zsh/Command/_zmodload:41-51`) builds its candidate
//! list out of `zmodload -lFP`, so the wrong bits also chose the wrong
//! complement for `zmodload -F MODULE -<TAB>` / `+<TAB>`.
//!
//! Every case below pins the load state EXPLICITLY on both sides — a `-F`
//! reading measured with the module loaded on one side only is a fixture
//! artifact, not a result.

use std::path::PathBuf;
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

/// Prefer a Homebrew zsh: Apple's `/bin/zsh` ships a reduced module set, so
/// several cases here would compare against a shell that cannot load the
/// module at all.
fn zsh_path() -> &'static str {
    use std::path::Path;
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

/// True when the reference zsh can actually load `module`. `zsh/pcre` and
/// `zsh/regex` are configure-time optional, and zshrs links both in
/// unconditionally, so a case naming one has to be skipped rather than
/// scored against a shell that does not have it.
fn zsh_has_module(module: &str) -> bool {
    Command::new(zsh_path())
        .args(["-fc", &format!("zmodload {} 2>/dev/null", module)])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn run(bin: &str, args: &[&str], script: &str) -> (String, String, i32) {
    let out = Command::new(bin)
        .args(args)
        .arg(script)
        .env_remove("ZSHRS_CACHE")
        .output()
        .unwrap_or_else(|e| panic!("invoke {}: {}", bin, e));
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// Byte-compare stdout + exit status of `zsh -fc SCRIPT` and
/// `zshrs --zsh -fc SCRIPT`. `-f` on BOTH sides is mandatory: an rc file can
/// demand-load a module and silently change the very state under test.
///
/// A both-sides-empty stdout is rejected outright — that is the vacuous pass
/// this whole file exists to avoid.
fn assert_feature_parity(script: &str) {
    if !zsh_available() {
        eprintln!("skip: no zsh on this host");
        return;
    }
    let (zo, _ze, zx) = run(zsh_path(), &["-fc"], script);
    let (ro, _re, rx) = run(zshrs_bin().to_str().unwrap(), &["--zsh", "-fc"], script);
    assert!(
        !zo.trim().is_empty(),
        "reference zsh produced no stdout for:\n{}\n(a silent reference makes the comparison vacuous)",
        script
    );
    assert_eq!(
        zo, ro,
        "feature-enable divergence on:\n{}\n--- zsh ---\n{}--- zshrs ---\n{}",
        script, zo, ro
    );
    assert_eq!(zx, rx, "exit-status divergence on:\n{}", script);
}

// ---------------------------------------------------------------------------
// The load-path discriminator: demand-load enables ONE feature, explicit
// `zmodload` enables all of them.
// ---------------------------------------------------------------------------

/// `${commands+…}` is a `p:` demand-load: `loadparamnode` (c:Src/params.c:568)
/// calls `ensurefeature(mn, "p:", "commands")`, which reaches
/// `do_module_features` with a ONE-element array (c:3428). Exactly one of
/// zsh/parameter's 33 features may come back `+`.
///
/// This is the minimal form of the `compinit` case: compinit touches
/// `$commands`/`$funcstack` and nothing else in zsh/parameter, and zshrs used
/// to answer that with all 33 enabled.
#[test]
fn parameter_param_demand_enables_only_the_demanded_feature() {
    assert_feature_parity(": ${commands+x}; zmodload -lF zsh/parameter");
}

/// The `b:` counterpart: running `zstyle` fires `resolvebuiltin`
/// (c:Src/exec.c:2703) → `ensurefeature(mod, "b:", "zstyle")`, so zsh/zutil's
/// other three builtins stay `-`.
#[test]
fn zutil_builtin_demand_enables_only_the_demanded_feature() {
    assert_feature_parity("zstyle -L >/dev/null; zmodload -lF zsh/zutil");
}

/// The other half of the discriminator, and the half that already agreed
/// before the fix: a NULL `enablesarr` means "enable all features"
/// (c:2108-2117). Pinned so a fix to the demand path cannot regress it.
#[test]
fn explicit_load_enables_every_feature() {
    assert_feature_parity("zmodload zsh/parameter; zmodload -lF zsh/parameter");
    assert_feature_parity("zmodload zsh/zutil; zmodload -lF zsh/zutil");
    assert_feature_parity("zmodload zsh/system; zmodload -lF zsh/system");
    assert_feature_parity("zmodload zsh/files; zmodload -lF zsh/files");
}

/// `zmodload -F MODULE feature` builds a non-NULL array (c:3252-3259), so the
/// listed feature is enabled and every sibling is explicitly left off — the
/// same bitmap shape as the demand path, reached from the command line.
#[test]
fn single_feature_zmodload_f_leaves_siblings_disabled() {
    assert_feature_parity("zmodload -F zsh/parameter p:functions; zmodload -lF zsh/parameter");
    assert_feature_parity("zmodload -F zsh/zutil b:zstyle; zmodload -lF zsh/zutil");
    assert_feature_parity("zmodload -F zsh/stat b:zstat; zmodload -lF zsh/stat");
    assert_feature_parity("zmodload -F zsh/system b:syserror; zmodload -lF zsh/system");
    assert_feature_parity("zmodload -F zsh/files b:zf_ln; zmodload -lF zsh/files");
    assert_feature_parity("zmodload -F zsh/terminfo p:terminfo; zmodload -lF zsh/terminfo");
}

/// `zmodload -F MODULE` with NO feature words still allocates the array
/// (c:3252), so it means "enable NOTHING" — the case that distinguishes an
/// empty non-NULL `enablesarr` from a NULL one.
#[test]
fn bare_dash_f_enables_nothing() {
    assert_feature_parity("zmodload -F zsh/datetime; zmodload -lF zsh/datetime");
    assert_feature_parity("zmodload -F zsh/zutil; zmodload -lF zsh/zutil");
}

/// Turning a feature back off has to move the bit, not just the listing:
/// `setfeatureenables` (c:3358) is the same call in both directions.
#[test]
fn feature_can_be_disabled_after_a_full_load() {
    assert_feature_parity(
        "zmodload zsh/zutil; zmodload -F zsh/zutil -b:zformat; zmodload -lF zsh/zutil",
    );
    assert_feature_parity(
        "zmodload zsh/parameter; zmodload -F zsh/parameter -p:aliases -p:builtins; \
         zmodload -lF zsh/parameter",
    );
}

// ---------------------------------------------------------------------------
// The downstream consumer: `_zmodload` reads `zmodload -lFP`, not `-lF`.
// ---------------------------------------------------------------------------

/// `_zmodload:41` assigns the signed feature list into an array and
/// `:41-51` slices it three ways. Pin the array itself — it is the input the
/// completer's `compset -P -` / `compset -P +` arms transform, so a wrong bit
/// here picks the wrong complement for `zmodload -F MODULE -<TAB>`.
#[test]
fn lfp_array_is_the_completers_input() {
    assert_feature_parity(
        "zstyle -L >/dev/null; typeset -a f; zmodload -lFP f zsh/zutil; \
         print -r -- \"n=${#f}\"; print -rl -- \"${f[@]}\"",
    );
    assert_feature_parity(
        ": ${commands+x}; typeset -a f; zmodload -lFP f zsh/parameter; \
         print -r -- \"n=${#f}\"; print -rl -- \"${f[@]}\"",
    );
    // `_zmodload:44` — the `compset -P -` arm keeps only the ENABLED ones.
    assert_feature_parity(
        "zstyle -L >/dev/null; typeset -a f; zmodload -lFP f zsh/zutil; \
         print -rl -- ${${f:#-*}##?}",
    );
    // `_zmodload:47` — the `compset -P +` arm keeps only the DISABLED ones.
    assert_feature_parity(
        "zstyle -L >/dev/null; typeset -a f; zmodload -lFP f zsh/zutil; \
         print -rl -- ${${f:#+*}##?}",
    );
}

// ---------------------------------------------------------------------------
// The feature ARRAY itself: `featuresarray` (c:3283) decides the names and
// their order, and the enable bitmap is positional against it.
// ---------------------------------------------------------------------------

/// `featuresarray` walks `bn_list` in table order (c:3295-3296), and
/// compctl's `bintab` (c:Src/Zle/compctl.c:4005-4008) is `compcall` then
/// `compctl`.
#[test]
fn feature_order_follows_the_module_table() {
    assert_feature_parity("zmodload zsh/compctl; zmodload -lF zsh/compctl");
}

/// `featuresarray` prefixes a conddef `C:` when it carries `CONDF_INFIX` and
/// `c:` otherwise (c:3298-3299). `zsh/regex` and `zsh/pcre` each declare their
/// single conddef `CONDF_INFIX`; `zsh/complete`'s four do not.
#[test]
fn infix_conddefs_use_the_uppercase_prefix() {
    for m in ["zsh/regex", "zsh/pcre"] {
        if !zsh_has_module(m) {
            eprintln!("skip: reference zsh has no {}", m);
            continue;
        }
        assert_feature_parity(&format!("zmodload {m}; zmodload -lF {m}"));
        assert_feature_parity(&format!("zmodload -F {m} C:{}-match; zmodload -lF {m}", {
            let (_, tail) = m.split_at(4);
            tail
        }));
    }
}

/// zsh/zle's `module_features` (c:Src/Zle/zle_main.c:2234-2240) carries
/// `bintab` ALONE. ZLE's special parameters (`$BUFFER`, `$CURSOR`, …) come
/// from `zleparams` (c:Src/Zle/zle_params.c), NOT the feature interface, so
/// they must not appear in this listing.
#[test]
fn zle_feature_surface_is_its_bintab_only() {
    assert_feature_parity("zmodload zsh/zle; zmodload -lF zsh/zle");
}

/// Every module that answers `features_module` must answer `enables_module`
/// too — `bin_zmodload_features` (c:3163-3167) treats a module that does not
/// as having nothing enabled, which listed a LOADED zsh/computil /
/// zsh/complete as all-`-`.
#[test]
fn loaded_completion_modules_report_their_builtins_enabled() {
    assert_feature_parity("zmodload zsh/computil; zmodload -lF zsh/computil");
    assert_feature_parity("zmodload zsh/complete; zmodload -lF zsh/complete");
    assert_feature_parity("zmodload zsh/zleparameter; zmodload -lF zsh/zleparameter");
    assert_feature_parity("zmodload zsh/sched; zmodload -lF zsh/sched");
    assert_feature_parity("zmodload zsh/rlimits; zmodload -lF zsh/rlimits");
}

/// `zmodload -LF` renders the same bits as a re-runnable statement
/// (c:3198-3216) and drops the disabled ones. A module left in a partial
/// state has to round-trip through that form.
#[test]
fn capital_l_form_lists_only_enabled_features() {
    assert_feature_parity("zmodload -F zsh/zutil b:zstyle b:zformat; zmodload -LF zsh/zutil");
    assert_feature_parity(": ${commands+x}; zmodload -LF zsh/parameter");
}
