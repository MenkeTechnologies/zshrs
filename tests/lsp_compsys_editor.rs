//! In-editor compsys completion: the LSP path that answers
//! `git ch<tab>` from the user's real completers.
//!
//! Hermetic by construction. The completion state comes from a
//! synthetic rkyv canonical shard written into a temp `$ZSHRS_HOME`
//! (the same `images/*-recorder.rkyv` shape `zshrs-daemon` writes
//! after `zshrs record`), so the test never reads the developer's
//! `~/.zshrs` and passes on a CI box that has no shard at all.
//!
//! What it pins down:
//!   * the shard's `compdef` row reaches `_comps`, so the fixture
//!     command dispatches to the fixture completer;
//!   * the shard's `fpath` is where the completer body loads from;
//!   * `_arguments` positional actions AND option specs both come
//!     back through the compadd capture shadow.
//!
//! A regression in the shell-thread bootstrap (no session executor →
//! every shell-defined completer silently no-ops) shows up here as an
//! empty match list.

#![cfg(feature = "daemon")]

use std::collections::HashMap;
use std::time::Duration;

use zsh::compsys::in_editor::{complete_at, CompsysRequest};
use zsh::daemon::paths::CachePaths;
use zsh::daemon::shard::{write_canonical_shard, CanonicalShard};

/// The fixture completer. `_arguments` drives both halves of the
/// assertion: a positional action with two literal values, and two
/// option specs.
const ZT_COMPLETER: &str = r#"#compdef zt
_arguments -s \
  '(-v --verbose)'{-v,--verbose}'[be loud]' \
  '1:command:(fixturebuild fixturetest)'
"#;

/// Write `$ZSHRS_HOME/images/<hash>-recorder.rkyv` describing a shell
/// that knows one command (`zt`) completed by one function (`_zt`)
/// autoloaded from `fpath_dir`.
fn seed_shard(root: &std::path::Path, fpath_dir: &std::path::Path) {
    let paths = CachePaths::with_root(root);
    std::fs::create_dir_all(&paths.images).expect("mkdir images");

    let mut shard = CanonicalShard::default();
    shard.header.slug = "recorder".to_string();
    shard.header.source_root = root.to_string_lossy().into_owned();
    shard.header.generation = 1;
    shard.fpath = vec![fpath_dir.to_string_lossy().into_owned()];
    // compdef rows are keyed by FUNCTION, value = space-joined
    // commands (canonical_apply.rs:258-270 replays them through
    // `compinit::compdef`).
    shard.compdef = HashMap::from([("_zt".to_string(), "zt".to_string())]);
    shard.autoload_functions = HashMap::from([("_zt".to_string(), String::new())]);
    write_canonical_shard(&paths, &shard).expect("write canonical shard");
}

/// Collect the completions compsys proposes for `line` with the
/// cursor at its end.
fn matches_for(line: &str) -> Vec<String> {
    // The shell thread bootstraps on first use; a cold request is
    // answered `is_incomplete` by design, so retry until it lands.
    for _ in 0..50 {
        let req = CompsysRequest::new_with_budget(line, line.len(), Duration::from_secs(5));
        let resp = complete_at(req);
        if !resp.matches.is_empty() {
            return resp.matches.into_iter().map(|m| m.completion).collect();
        }
        if !resp.is_incomplete {
            return Vec::new();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Vec::new()
}

#[test]
fn editor_completion_serves_positionals_and_option_specs_from_rkyv_shard() {
    let tmp = std::env::temp_dir().join(format!("zshrs-in-editor-{}", std::process::id()));
    let fpath_dir = tmp.join("fpath");
    std::fs::create_dir_all(&fpath_dir).expect("mkdir fpath");
    std::fs::write(fpath_dir.join("_zt"), ZT_COMPLETER).expect("write _zt");
    seed_shard(&tmp, &fpath_dir);

    // Must be set before the shell thread starts: `CachePaths::resolve`
    // reads it once per bootstrap (daemon/paths.rs:191).
    std::env::set_var("ZSHRS_HOME", &tmp);

    let positional = matches_for("zt fixture");
    assert!(
        positional.iter().any(|m| m == "fixturebuild"),
        "positional action from `_arguments '1:command:(...)'` missing; got {positional:?}",
    );
    assert!(
        positional.iter().any(|m| m == "fixturetest"),
        "second positional value missing; got {positional:?}",
    );

    let options = matches_for("zt --");
    assert!(
        options.iter().any(|m| m == "--verbose"),
        "option spec from `_arguments` missing; got {options:?}",
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
