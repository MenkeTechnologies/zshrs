//! The completion special parameters exist only while a completion
//! function runs.
//!
//! `callcompfunc` brackets the call with `startparamscope();
//! makecompparams();` … `endparamscope();` (c:Src/Zle/compcore.c:816-839).
//! `makecompparams` creates `$words`, `$CURRENT`, `$PREFIX`, `$SUFFIX`,
//! `$IPREFIX`, `$ISUFFIX`, `$QIPREFIX`, `$QISUFFIX` and the `$compstate`
//! association at `locallevel + 1` (c:Src/Zle/complete.c:1297-1352), so the
//! scope end deletes them again. Outside completion `compstate` is an
//! ordinary undeclared name, and `compstate[list]=list` takes assignsparam's
//! `createparam(t, PM_ARRAY)` path (c:Src/params.c:3061): the string key is
//! evaluated arithmetically and the assignment fails with "assignment to
//! invalid subscript range". zshrs special-cased the NAME `compstate` as an
//! association in assignsparam, so the write succeeded and silently created
//! a global association.
//!
//! Inside the widget, `makecompparams` creates every `compkparams` row and
//! `comp_setunset` hides the rows outside `kset` (c:Src/Zle/compcore.c:562-
//! 819), so `${(k)compstate}` lists the getter-backed keys (`unambiguous`,
//! `list_lines`, …) and not `exact` unless REC_EXACT is on. `CURRENT` is a
//! `VAL()` integer row, which gets `pm->base = 10` (c:complete.c:1313-1315)
//! and so prints as `typeset -i10`.
//!
//! Driven through `zle -C` without `compinit` (as in
//! compset_pattern_parity): the special parameters are created by the
//! widget call itself. Harness contract: `zpty_probe`.

use std::path::Path;
use std::process::Command;

use crate::zpty_probe::{assert_same_dump, sq, zsh_available, zsh_path, zshrs_bin, OPEN_PUMPED};

const SPECIALS: &str = "compstate words CURRENT PREFIX SUFFIX IPREFIX ISUFFIX QIPREFIX QISUFFIX";

/// A driver whose `^X^G` widget is `zle -C … complete-word <body>`; `after`
/// runs at the prompt once the widget has returned.
fn widget_driver(body: &str, after: &str) -> String {
    let setup = sq(&format!(
        "cw() {{ {body} }}; zle -C cw complete-word cw; bindkey '^X^G' cw"
    ));
    let after = if after.is_empty() {
        String::new()
    } else {
        format!("zpty -w w {}; pump\n", sq(after))
    };
    format!(
        "{OPEN_PUMPED}
zpty -w w {setup}; pump
zpty -w -n w 'echo ab'; pump
zpty -w -n w $'\\C-x\\C-g'; pump
zpty -w -n w $'\\C-a\\C-k'; pump
{after}zpty -d w
"
    )
}

/// c:Src/params.c:3061 — outside completion `compstate` is undeclared, so
/// a string subscript is an arithmetic index and the write fails.
#[test]
fn compstate_subscript_write_outside_completion_fails() {
    if !zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let script = "compstate[list]=list; print rc=$? t=${(t)compstate}";
    let run = |shell: &Path, zshrs: bool| {
        let mut cmd = Command::new(shell);
        if zshrs {
            cmd.arg("--zsh");
        }
        let out = cmd.args(["-f", "-c", script]).output().expect("invoke shell");
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    };
    let reference = run(Path::new(zsh_path()), false);
    assert!(
        reference.contains("assignment to invalid subscript range"),
        "reference zsh did not reject the write: {reference:?}"
    );
    assert_eq!(reference, run(&zshrs_bin(), true));
}

/// Inside a `zle -C` widget: types of every special, `typeset -p CURRENT`
/// (`-i10`), and the key set of `$compstate`.
#[test]
fn completion_specials_inside_a_completion_widget() {
    let body = format!(
        r#"{{ for p in {SPECIALS}; do print -r -- "$p=${{(tP)p}}"; done; typeset -p CURRENT; local -a ks; ks=(${{(k)compstate}}); print -r -- K= ${{(o)ks}} }} >! $OUTFILE 2>&1"#
    );
    assert_same_dump(
        &widget_driver(&body, ""),
        "completion special parameters inside a zle -C widget",
    );
}

/// After the widget returns the specials are gone again, so a subscript
/// write to `compstate` fails exactly as it does in a fresh shell.
#[test]
fn completion_specials_are_gone_after_the_widget() {
    let after = format!(
        r#"{{ for p in {SPECIALS}; do print -r -- "$p=${{(tP)p}}"; done; compstate[list]=list; print -r -- "rc=$?" }} >! $OUTFILE 2>&1"#
    );
    assert_same_dump(
        &widget_driver("compadd -U - x", &after),
        "completion special parameters after a zle -C widget returned",
    );
}

/// The stock `_suffix_alias_files` directory: the reference shell's own
/// function directory, which is where both shells autoload it from.
fn stock_function_dir() -> Option<String> {
    let out = Command::new(zsh_path())
        .args([
            "-fc",
            "for d in $fpath; do [[ -f $d/_suffix_alias_files ]] && { print -r -- $d; break }; done",
        ])
        .output()
        .ok()?;
    let d = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!d.is_empty()).then_some(d)
}

/// `_suffix_alias_files` (zsh-5.9.1 tag) has no `(#q^/)` qualifier — that
/// line is a dev-branch change (workers/50307) — and backslash-quotes each
/// alias name with `(q)`, so the candidate count is the verdict.
fn suffix_alias_driver(alias: &str) -> Option<String> {
    let fdir = stock_function_dir()?;
    let dir = std::env::temp_dir().join(format!(
        "zshrs-sfx-{}-{}",
        std::process::id(),
        alias.len()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("d.txt")).ok()?;
    for f in ["a.txt", "a.xzy", "a.x*y", "b.c"] {
        std::fs::write(dir.join(f), "").ok()?;
    }
    let setup = sq(&format!(
        "cd {}; fpath=({fdir}); autoload -U $fpath[1]/_*(:t); {alias}",
        dir.display()
    ));
    let body = r#"_suffix_alias_files; print -r -- "rc=$? N=$compstate[nmatches]" >! $OUTFILE 2>&1"#;
    let driver = widget_driver(body, "");
    Some(driver.replacen(
        "zpty -w -n w 'echo ab'",
        &format!("zpty -w w {setup}; pump\nzpty -w -n w 'echo '"),
        1,
    ))
}

/// With AUTO_CD unset the 5.9.1 pattern still matches the directory
/// `d.txt`, so both `a.txt` and `d.txt` are candidates.
#[test]
fn suffix_alias_files_keeps_directories() {
    let Some(driver) = suffix_alias_driver("alias -s txt=cat") else {
        eprintln!("skip: stock _suffix_alias_files not found");
        return;
    };
    assert_same_dump(&driver, "_suffix_alias_files with a directory named *.txt");
}

/// `(kq)` turns the alias name `x*y` into `x\*y`, so only the file
/// literally named `a.x*y` matches, not `a.xzy`.
#[test]
fn suffix_alias_files_quotes_the_alias_name() {
    let Some(driver) = suffix_alias_driver("alias -s 'x*y=cat'") else {
        eprintln!("skip: stock _suffix_alias_files not found");
        return;
    };
    assert_same_dump(&driver, "_suffix_alias_files with a pattern character in the suffix");
}
