//! The `compsys.db` rebuild under concurrency.
//!
//! One SQLite file is shared by every `zshrs` on the machine, and the
//! rebuild is a whole-database build-aside plus `rename`. Without a lock
//! two shells rebuilding at once each install a complete database over
//! the other's: the last `rename` wins and every earlier builder's
//! `$fpath` scan is discarded. This file pins the two properties that
//! makes wrong:
//!
//!   1. a plain `compinit` publishes the scan of ITS OWN `$fpath`, never
//!      a neighbour's — upstream compinit sh:504-528 scans `$fpath`
//!      unconditionally when `-C` is absent;
//!   2. a cold `compinit -C` storm runs exactly ONE scan, and every
//!      shell in it still ends up with the full table.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

/// `n` completion files shared by every shell, plus one named after
/// `uniq` that only this shell's `$fpath` holds.
fn make_fpath(dir: &Path, shared: usize, uniq: &str) {
    fs::create_dir_all(dir).expect("fpath dir");
    for i in 0..shared {
        fs::write(
            dir.join(format!("_cmd{i}")),
            format!("#compdef cmd{i}\n\n_cmd{i}() {{ :; }}\n"),
        )
        .expect("write completion");
    }
    fs::write(
        dir.join(format!("_{uniq}")),
        format!("#compdef {uniq}\n\n_{uniq}() {{ :; }}\n"),
    )
    .expect("write unique completion");
}

/// `$HOME` and `$ZDOTDIR` are pinned to `home` alongside `$ZSHRS_HOME`.
///
/// `compinit -C` sources `$_comp_dumpfile`, which defaults to
/// `${ZDOTDIR:-$HOME}/.zcompdump` (compinit sh:133, ported at
/// `src/compsys/ported/compinit.rs:735`), and sh:512-517 then sets
/// `_i_done=yes` so the whole sh:523-550 `$fpath` scan is skipped — the
/// dump alone defines `$_comps`. With only `ZSHRS_HOME` redirected the
/// child still read the DEVELOPER'S real `~/.zcompdump`, so
/// `cold_dash_c_storm_scans_fpath_once` measured his 1729 completers
/// instead of the 301 files this test wrote. Verified directly: same
/// storm, same fpath, `COMPS=1729` with the ambient `$HOME` and
/// `COMPS=301` with `HOME=$ZDOTDIR=<tempdir>`. All eight shells share the
/// one `home`, which is the arrangement under test — one dump, one cache,
/// one lock.
fn spawn(home: &Path, fpath: &Path, script: &str) -> Child {
    Command::new(zshrs_bin())
        .args(["-f", "-c", script])
        .env("ZSHRS_HOME", home)
        .env("HOME", home)
        .env("ZDOTDIR", home)
        .env("FPATH", fpath)
        .env_remove("ZSHRS_CACHE")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn zshrs")
}

fn wait_stdout(child: Child) -> String {
    let out = child.wait_with_output().expect("wait zshrs");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Eight shells rebuild at once, each with one completion no other shell
/// can see. Each must publish its own.
///
/// This failed while the rebuild lock's post-wait revalidation was not
/// restricted to `-C`: every one of the eight came back holding shell
/// 8's unique entry, because each had accepted the cache the previous
/// lock holder had just installed from a DIFFERENT `$fpath`.
#[test]
fn concurrent_rebuilds_each_publish_their_own_fpath() {
    const SHELLS: usize = 8;
    const SHARED: usize = 200;
    let root = tempfile::tempdir().expect("tempdir");
    let home = root.path().join("home");
    fs::create_dir_all(&home).expect("home");

    let mut fpaths = Vec::new();
    for s in 0..SHELLS {
        let dir = root.path().join(format!("fp{s}"));
        make_fpath(&dir, SHARED, &format!("uniq{s}"));
        fpaths.push(dir);
    }

    let children: Vec<Child> = fpaths
        .iter()
        .map(|fp| {
            spawn(
                &home,
                fp,
                r#"autoload -Uz compinit
                   compinit -u
                   print "COMPS=${#_comps}"
                   print "UNIQ=${(k)_comps[(I)uniq*]}""#,
            )
        })
        .collect();

    for (s, child) in children.into_iter().enumerate() {
        let out = wait_stdout(child);
        assert!(
            out.contains(&format!("COMPS={}", SHARED + 1)),
            "shell {s} must publish its whole scan ({} entries); got `{out}`",
            SHARED + 1,
        );
        assert!(
            out.contains(&format!("UNIQ=uniq{s}")),
            "shell {s} must publish the completion only IT can see, \
             not a concurrent rebuilder's; got `{out}`",
        );
    }
}

/// A cold `compinit -C` storm scans `$fpath` once, not once per shell,
/// and every shell still gets the full table.
///
/// The count comes from the log rather than from timing: `compinit`
/// writes one "background scan complete" line per scan that actually
/// ran, so eight lines means seven databases were built and thrown away.
#[test]
fn cold_dash_c_storm_scans_fpath_once() {
    const SHELLS: usize = 8;
    const SHARED: usize = 300;
    let root = tempfile::tempdir().expect("tempdir");
    let home = root.path().join("home");
    fs::create_dir_all(&home).expect("home");
    let fpath = root.path().join("fp");
    make_fpath(&fpath, SHARED, "solo");

    let children: Vec<Child> = (0..SHELLS)
        .map(|_| {
            spawn(
                &home,
                &fpath,
                r#"autoload -Uz compinit
                   compinit -C -u
                   print "COMPS=${#_comps}""#,
            )
        })
        .collect();

    for child in children {
        let out = wait_stdout(child);
        assert!(
            out.contains(&format!("COMPS={}", SHARED + 1)),
            "every shell in the storm must see the whole table; got `{out}`",
        );
    }

    let log = fs::read_to_string(home.join("zshrs.log")).unwrap_or_default();
    let scans = log.matches("background scan complete").count();
    assert_eq!(
        scans, 1,
        "a cold `-C` storm must scan $fpath once; {scans} scans ran, so \
         {} complete databases were built and discarded. log:\n{log}",
        scans.saturating_sub(1),
    );
}
