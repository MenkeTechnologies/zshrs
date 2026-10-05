//! Rust compsys ports against upstream master's shell functions.
//!
//! The reference shell (the oracle built from upstream master) runs the
//! master `Completion/` functions; zshrs, given the same `$fpath`, swaps
//! the stock tree for its bundle and lets `src/compsys/router.rs` run the
//! Rust port in place of each ported function. A difference here is a
//! port that still carries a pre-master body.
//!
//! The `$fpath` entry has to LOOK like an installed tree
//! (`…/share/zsh/<ver>/functions`) or zshrs treats every file in it as a
//! user override and the ports step aside (`has_fpath_override`), which is
//! what the ztst corpus does — so these cases are the only ones that put
//! the ports themselves against master.

use crate::zpty_probe::{assert_same_dump, sq, OPEN_PUMPED};
use std::path::PathBuf;
use std::sync::OnceLock;

/// An installed-shaped copy of the zsh source tree's completion and
/// contrib functions (symlinks), or `None` when no source tree exists.
fn stock_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        let src = std::env::var_os("ZTST_ZSH_SOURCE")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("forkedRepos/zsh")))
            .filter(|p| p.join("Completion").is_dir())?;
        let dir = std::env::temp_dir()
            .join(format!("zshrs-parity-master-{}", std::process::id()))
            .join("share/zsh/master/functions");
        std::fs::create_dir_all(&dir).ok()?;
        let mut stack = vec![src.join("Completion"), src.join("Functions")];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).ok()?.flatten() {
                let p = e.path();
                let name = e.file_name();
                let name = name.to_string_lossy();
                if p.is_dir() {
                    stack.push(p);
                } else if !name.starts_with('.') && !name.starts_with("README") {
                    let _ = std::os::unix::fs::symlink(&p, dir.join(&*name));
                }
            }
        }
        Some(dir)
    })
    .as_ref()
}

/// compinit against the stock-shaped master tree, run `setup`, complete
/// `mytest `, and let the completion function write its verdict to
/// `$OUTFILE`.
fn master_dump_driver(dir: &std::path::Path, setup: &str) -> String {
    let fp = sq(&format!("fpath=({})", dir.display()));
    let setup = sq(setup);
    format!(
        "{OPEN_PUMPED}
zpty -w w {fp}; pump
zpty -w w 'autoload -Uz compinit; compinit -u -D'; pump; pump
zpty -w w {setup}; pump
zpty -w -n w 'mytest '; pump
zpty -w -n w $'\\t'; pump; pump
zpty -w -n w $'\\C-u'; pump
zpty -w -n w $'\\r'; pump
zpty -d w 2>/dev/null
"
    )
}

/// Master `_description` formats with `zformat -Fq` (upstream 74fa234140,
/// Completion/Base/Core/_description:89), so a `%` in the description is
/// doubled and survives the prompt expansion of the group header:
/// `-X '<a%%b>'`. The port ran `zformat -F` and handed `-X '<a%b>'`, whose
/// `%b` the header expansion then ate.
#[test]
fn description_quotes_percent_in_the_description() {
    let Some(dir) = stock_dir() else {
        eprintln!("skip: no zsh source tree for the master completion functions");
        return;
    };
    assert_same_dump(
        &master_dump_driver(
            dir,
            r#"zstyle ":completion:*:descriptions" format "<%d>"; _mytest(){ local expl; _description foo expl "a%b"; print -r -- "${(j:|:)expl}" >! $OUTFILE; compadd x }; compdef _mytest mytest"#,
        ),
        "_description's expl for a description containing %",
    );
}

/// `_complete_help` (`^Xh`) column-aligns each context's tags with
/// `zformat -a tmp '  (' "$tmp[@]"` (Completion/Base/Widget/_complete_help:56)
/// and prints them as `    <tags>  (<functions>)`. The port passed `-a` as
/// an argv word after zformat gained its "afFqQ" optstring, got "one of
/// -afF expected" and an empty array, and printed the context headers with
/// no tag lines under them.
#[test]
fn complete_help_lists_the_aligned_tags() {
    let Some(dir) = stock_dir() else {
        eprintln!("skip: no zsh source tree for the master completion functions");
        return;
    };
    let fp = sq(&format!("fpath=({})", dir.display()));
    let driver = format!(
        "{OPEN_PUMPED}
zpty -w w {fp}; pump
zpty -w w 'autoload -Uz compinit; compinit -u -D'; pump; pump
zpty -w -n w 'print /usr/li'; pump
zpty -w -n w $'\\030h'; sleep 3; pump
zpty -w -n w $'\\025'; pump
zpty -d w 2>/dev/null
setopt extended_glob
if [[ $all == *'tags in context :completion::complete:print::'*$'\\n'*'  (_'* ]]; then print \"H=yes\"; else print \"H=no\"; fi
"
    );
    crate::zpty_probe::assert_same_verdict(&driver, "H", "^Xh printed aligned tag lines");
}
