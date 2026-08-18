//! The rkyv autoload chunk cache must be invisible.
//!
//! `~/.zshrs/autoloads.rkyv` stores the compiled definition program for
//! each autoloaded function, so the second process to call `_git` (or any
//! other completer) installs it without re-parsing the file. That is a
//! pure speed change — the loaded function has to behave identically, and
//! an edited definition file has to win over the cached chunk.
//!
//! Both properties are checked against the shell's own observable state:
//! `$LINENO` inside the body and `$funcsourcetrace`, which is where a
//! wrong compile shows up first (a chunk compiled as a top-level script
//! instead of a function body reports different line numbers). Verified
//! against `zsh -f` at the time of writing: `lineno=2`,
//! `trace=<file>:0`.

use std::path::PathBuf;
use std::process::Command;

fn zshrs_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        let pb = PathBuf::from(p);
        if pb.exists() {
            return Some(pb);
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    [
        manifest.join("target/debug/zshrs"),
        manifest.join("target/release/zshrs"),
    ]
    .into_iter()
    .find(|cand| cand.exists())
}

/// Body whose output pins down how it was compiled: `$LINENO` is relative
/// to the function's definition line, and `funcsourcetrace` names the
/// definition file plus that line.
const BODY: &str = "# comment on line 1\n\
                    print \"lineno=$LINENO\"\n\
                    print \"trace=${funcsourcetrace[1]##*/}\"\n\
                    print \"args=$*\"\n";

fn run(bin: &PathBuf, home: &PathBuf, fpath: &PathBuf, script: &str) -> String {
    let out = Command::new(bin)
        .args(["-c", script])
        .env("ZSHRS_HOME", home)
        .env("FPATH", fpath)
        .env_remove("ZDOTDIR")
        .output()
        .expect("spawn zshrs");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn cached_autoload_chunk_matches_a_fresh_compile_and_yields_to_an_edit() {
    let Some(bin) = zshrs_bin() else {
        eprintln!("skip: zshrs binary not built");
        return;
    };
    let tmp = std::env::temp_dir().join(format!("zshrs-autoload-cache-{}", std::process::id()));
    let home = tmp.join("home");
    let fpath = tmp.join("fpath");
    std::fs::create_dir_all(&home).expect("mkdir home");
    std::fs::create_dir_all(&fpath).expect("mkdir fpath");
    std::fs::write(fpath.join("zt_cachefn"), BODY).expect("write fn");

    let script = "autoload -Uz zt_cachefn; zt_cachefn a b";
    // First process: nothing cached, so this compiles the file.
    let cold = run(&bin, &home, &fpath, script);
    assert!(
        cold.contains("lineno=2")
            && cold.contains("trace=zt_cachefn:0")
            && cold.contains("args=a b"),
        "fresh compile output unexpected: {cold:?}",
    );
    let shard = home.join("autoloads.rkyv");
    assert!(
        shard.exists(),
        "loader did not write the chunk to {}",
        shard.display(),
    );

    // Next processes read the chunk instead of parsing. Byte-identical
    // output is the whole contract.
    for i in 0..2 {
        let warm = run(&bin, &home, &fpath, script);
        assert_eq!(warm, cold, "cached run {i} diverged from the fresh compile");
    }

    // An edited definition file must beat the cached chunk: the appended
    // line is on line 5, and `$LINENO` proves the recompile happened
    // against the new bytes rather than the stale ones.
    let mut edited = BODY.to_string();
    edited.push_str("print \"appended=$LINENO\"\n");
    std::fs::write(fpath.join("zt_cachefn"), &edited).expect("rewrite fn");
    let after = run(&bin, &home, &fpath, script);
    assert!(
        after.contains("appended=5"),
        "edited body was served from the stale chunk: {after:?}",
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
