//! Completers must not leave their own `local` parameters behind.
//!
//! Every stock completer opens with a `local` line, and the names on it are
//! working storage: `expl` (the `compadd` option array `_description` fills
//! in), `mfiles` (`compadd -O`'s output), `_bpf_filters`' `flags` / `subtypes`
//! word tables. zsh unwinds all of them when the function returns, so after a
//! TAB the shell is back to whatever the user had.
//!
//! zshrs answers those names from a NATIVE RUST PORT (`src/compsys/ported/`,
//! dispatched by `src/compsys/router.rs`), and a port that writes a parameter
//! without declaring it local has no `endparamscope` to undo the write. The
//! value then outlives the completion: visible to the user, to a plugin, and
//! to the NEXT completer in the chain — which is the impact, and why the
//! `completer` lists below are chains wherever the case can still attribute
//! the matches to the completer under test.
//!
//! Two shapes of the same bug are pinned here:
//!
//!   * the port never writes the name itself — it hands the NAME to
//!     `_description`, whose last statement is `set -A "$name" …`
//!     (`Completion/Base/Core/_description` sh:94-102), so the array is
//!     created at whatever scope happens to be current (`_expand` sh:14,
//!     `_user_expand` sh:15, `_approximate` sh:91, `_extensions` sh:11);
//!   * the port writes an ASSOCIATIVE array, whose key/value pairs live in
//!     the parallel `paramtab_hashed_storage` map keyed by NAME rather than
//!     in the `param` struct — so putting the `paramtab` node back is only
//!     half the unwind (`_bpf_filters` sh:6).
//!
//! Liveness: each verdict is `yes` only when THE COMPLETER UNDER TEST is the
//! one that produced the matches (`$_lastcomp[completer]`, or the corrected
//! word for the case where the completer is not itself in the chain) *and*
//! the name is unset. A session where the TAB was dropped, or where the
//! completer bailed and the chain's `_complete` answered instead, reports
//! `no`, and `assert_same_verdict` fails that as a broken probe rather than
//! passing it as agreement.
//!
//! Skip pattern: no-ops silently when `zsh` isn't available, when `zsh/zpty`
//! will not load, or when there is no stock function directory to `compinit`
//! against. Harness contract: `zpty_probe`.

#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_verdict, sq, OPEN_PUMPED};

/// Is there a stock `Completion/` tree to run `compinit` over?
///
/// The inherited `$FPATH` on a developer box holds thousands of completers
/// and takes minutes to scan in a debug build, so every driver here pins
/// `/usr/share/zsh/*/functions` the way the rest of the zpty suite does.
fn stock_fpath_exists() -> bool {
    std::fs::read_dir("/usr/share/zsh")
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .any(|e| e.path().join("functions").is_dir())
        })
        .unwrap_or(false)
}

/// A directory holding two files with different extensions, so `ls *.<TAB>`
/// has something for `_extensions` to offer.
fn extension_fixture() -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("parity-leak-fixture");
    let _ = std::fs::create_dir_all(&dir);
    for name in ["leakfx.aaa", "leakfx.bbb"] {
        let _ = std::fs::File::create(dir.join(name));
    }
    dir
}

/// Drive one completion and report whether the completer under test actually
/// ran AND left `$names` unset.
///
/// `setup` is run in the inner shell after `compinit`; `line` is typed as-is
/// and TAB.d. No Return is ever sent — `^U` clears the line first — so a case
/// can safely type a REAL command word, which `tcpdump <TAB>` needs.
///
/// `live` is a `[[ … ]]` condition over `$_lastcomp`, and it is the reason
/// these cases are not self-deceiving. `nmatches > 0` alone is not evidence:
/// every chain here ends in `_complete`, so a run where the completer under
/// test did nothing at all still produces matches — and then the name is
/// legitimately unset on both sides and the case passes while measuring
/// nothing. `$_lastcomp[completer]` names the completer whose matches were
/// kept (`_main_complete` sh:214), so pinning it is what makes the verdict
/// about THIS completer. Measured: removing the `_extensions` fix and
/// re-running with only the `nmatches` guard still passed.
fn leak_driver(setup: &str, line: &str, live: &str, names: &[&str]) -> String {
    // `${+name}` for each name, summed: 0 when every one of them is unset.
    let plus = names
        .iter()
        .map(|n| format!("${{+{n}}}"))
        .collect::<Vec<_>>()
        .join(" + ");
    let setup_q = sq(setup);
    let line_q = sq(line);
    // The marker is COMPUTED, never written literally into the command.
    // The inner shell echoes every line it is given, so a command containing
    // the literal `LEAKFREE=yes` puts that string in the transcript whether or
    // not it ever runs — and the driver below, which searches the transcript,
    // then reports a pass for a session that printed `LEAKFREE=no`. Measured:
    // with the `_extensions` fix removed the case still said `L=yes` while the
    // same session's own debug line read `EX=1 MF=1`.
    let live_q = sq(&format!(
        "integer v=$(( ({plus}) == 0 )); {live} || v=0; print -r -- VERDICT$v"
    ));
    format!(
        "{OPEN_PUMPED}
zpty -w w 'fpath=(/usr/share/zsh/*/functions(N))'; pump
zpty -w w 'autoload -Uz compinit; compinit -u -D'; pump
zpty -w w {setup_q}; pump
zpty -w -n w {line_q}; pump
zpty -w -n w $'\\t'; sleep 3; pump
zpty -w -n w $'\\025'; pump
zpty -w w {live_q}; pump
zpty -d w 2>/dev/null
if [[ $all == *VERDICT1* ]]; then print \"L=yes\"; else print \"L=no\"; fi
"
    )
}

/// `_expand` sh:14 declares `expl`, and sh:186/188 hand that name to
/// `_description`.
///
/// `_expand` is the one case that runs ALONE in the `completer` list.
/// Measured with `_expand _complete`, `$_lastcomp[completer]` for this word
/// is `complete` — `_expand` offers its expansion as a menu and the chain
/// carries on — so the chained spelling cannot tell "`_expand` ran" from
/// "`_expand` did nothing"; with `_expand` alone it reads `expand`.
#[test]
fn expand_does_not_leak_expl() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    let driver = leak_driver(
        r#"zstyle ":completion:*" completer _expand"#,
        "print $HOME/",
        r#"[[ $_lastcomp[completer] == expand ]]"#,
        &["expl"],
    );
    assert_same_verdict(&driver, "L", "_expand left `expl` set after the completion");
}

/// `_user_expand` sh:15 declares `expl` and sh:89/91 hand it to
/// `_description`. The `user-expand` style needs a spec to fire at all, so
/// the setup installs a one-line expander function first.
#[test]
fn user_expand_does_not_leak_expl() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    let driver = leak_driver(
        r#"_leakexp() { reply=(AAA BBB) }; zstyle ":completion:*" user-expand _leakexp; zstyle ":completion:*" completer _user_expand _complete"#,
        "print leakword",
        r#"[[ $_lastcomp[completer] == user-expand ]]"#,
        &["expl"],
    );
    assert_same_verdict(
        &driver,
        "L",
        "_user_expand left `expl` set after the completion",
    );
}

/// `_approximate` sh:91 declares `expl` inside the correction branch and
/// sh:93 fills it through `_description -V original`. Reached by misspelling
/// a path so `_complete` finds nothing and the correction pass runs.
///
/// The misspelling corrects to ONE directory on purpose. A word whose
/// correction has hundreds of candidates (`/usr/binn/`) makes the inner
/// shell paint a listing the driver then has to drain before its next write
/// lands, and in a debug build that write was simply lost — the case failed
/// with `v` unset rather than with a verdict.
#[test]
fn approximate_does_not_leak_expl() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    let driver = leak_driver(
        r#"zstyle ":completion:*" completer _complete _approximate; zstyle ":completion:*:approximate:*" max-errors 2"#,
        "print /usr/lbi",
        r#"[[ $_lastcomp[completer] == approximate ]]"#,
        &["expl"],
    );
    assert_same_verdict(
        &driver,
        "L",
        "_approximate left `expl` set after the correction pass",
    );
}

/// `_extensions` sh:11 declares `expl` and `mfiles`; sh:30's
/// `compadd -O mfiles` writes the second one BY NAME, so it is a direct
/// write rather than a `_description` fill.
#[test]
fn extensions_does_not_leak_expl_or_mfiles() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    let dir = extension_fixture();
    let setup = format!(
        r#"cd {}; zstyle ":completion:*" completer _extensions _complete"#,
        dir.display()
    );
    let driver = leak_driver(
        &setup,
        "print *.",
        r#"[[ $_lastcomp[completer] == extensions ]]"#,
        &["expl", "mfiles"],
    );
    assert_same_verdict(
        &driver,
        "L",
        "_extensions left `expl` / `mfiles` set after the completion",
    );
}

/// `_bpf_filters` sh:6 declares `flags` and `subtypes` `local -A`.
///
/// This is the associative-array half of the class, and it needs more than a
/// `paramtab` restore: an assoc's pairs live in the parallel
/// `paramtab_hashed_storage` map keyed by name, so a scope that put the node
/// back but left the row answered `${(t)flags} == association` with the node
/// long gone.
#[test]
fn bpf_filters_does_not_leak_its_word_tables() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    let driver = leak_driver(
        &format!(
            r#"cd {}; zstyle ":completion:*" completer _complete"#,
            extension_fixture().display()
        ),
        "tcpdump tc",
        r#"[[ $_lastcomp[unambiguous] == tcp* ]]"#,
        &["flags", "subtypes"],
    );
    assert_same_verdict(
        &driver,
        "L",
        "_bpf_filters left `flags` / `subtypes` set after the completion",
    );
}
