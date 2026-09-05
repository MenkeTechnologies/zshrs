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

/// Run `zshrs --prewarm-autoloads DIR` and return its JSON summary line.
fn prewarm(bin: &PathBuf, home: &PathBuf, fpath: &PathBuf) -> String {
    let out = Command::new(bin)
        .arg("--prewarm-autoloads")
        .arg(fpath)
        .env("ZSHRS_HOME", home)
        .env_remove("ZDOTDIR")
        .output()
        .expect("spawn prewarm");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn prewarm_fills_the_shard_so_the_first_call_never_compiles() {
    // The point of the pass: after it runs, a shell that has never seen
    // the function still installs it from bytecode. Proven by the shard
    // not being rewritten on that first call — a compile would
    // write-through and change it.
    let Some(bin) = zshrs_bin() else {
        eprintln!("skip: zshrs binary not built");
        return;
    };
    let tmp = std::env::temp_dir().join(format!("zshrs-prewarm-{}", std::process::id()));
    let home = tmp.join("home");
    let fpath = tmp.join("fpath");
    std::fs::create_dir_all(&home).expect("mkdir home");
    std::fs::create_dir_all(&fpath).expect("mkdir fpath");
    std::fs::write(fpath.join("_zt_prewarmed"), BODY).expect("write fn");
    // A `.zwc` digest and a non-`_` file must both be ignored — the
    // filename rule is compinit's.
    std::fs::write(fpath.join("_zt_prewarmed.zwc"), b"not source").expect("write zwc");
    std::fs::write(fpath.join("notacompleter"), BODY).expect("write other");

    let summary = prewarm(&bin, &home, &fpath);
    assert!(
        summary.contains("\"seen\":1") && summary.contains("\"compiled\":1"),
        "prewarm should have seen exactly the one `_` file: {summary}",
    );
    let shard = home.join("autoloads.rkyv");
    let before = std::fs::metadata(&shard).expect("shard written").len();

    let script = "autoload -Uz _zt_prewarmed; _zt_prewarmed a b";
    let out = run(&bin, &home, &fpath, script);
    assert!(
        out.contains("lineno=2") && out.contains("args=a b"),
        "prewarmed function misbehaved: {out:?}",
    );
    let after = std::fs::metadata(&shard).expect("shard still there").len();
    assert_eq!(
        before, after,
        "shard was rewritten — the call recompiled instead of using the prewarmed chunk",
    );

    // Re-running the pass must not recompile what is already current.
    let again = prewarm(&bin, &home, &fpath);
    assert!(
        again.contains("\"compiled\":0") && again.contains("\"fresh\":1"),
        "second pass should be a no-op: {again}",
    );

    let _ = std::fs::remove_dir_all(&tmp);
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

/// A cache entry whose chunk does not define the function it is stored
/// under must not be able to break the autoload.
///
/// This is the failure the shard actually produced: a chunk emitted by a
/// different build passed every freshness test, ran, defined nothing, and
/// the loader reported `function not defined by file`. On a real `$fpath`
/// that meant `_megacomplete` failing four times per `<TAB>` and the
/// completion system returning zero matches. Nothing in the suite caught
/// it, because every test only ever asked whether a *correct* chunk was
/// reused.
///
/// The entry is poisoned in place — chunk swapped, key and binary stamp
/// left untouched — so the loader is guaranteed to accept it and the
/// recovery is what is being measured.
#[test]
fn a_cached_chunk_that_defines_nothing_is_discarded_and_recompiled() {
    let Some(bin) = zshrs_bin() else {
        eprintln!("skip: zshrs binary not built");
        return;
    };
    let tmp = std::env::temp_dir().join(format!("zshrs-poisoned-{}", std::process::id()));
    let home = tmp.join("home");
    let fpath = tmp.join("fpath");
    std::fs::create_dir_all(&home).expect("mkdir home");
    std::fs::create_dir_all(&fpath).expect("mkdir fpath");
    std::fs::write(fpath.join("zt_victim"), "print victim-ok\n").expect("write victim");
    std::fs::write(fpath.join("zt_donor"), "print donor-ok\n").expect("write donor");

    // Warm both entries with chunks this binary really produced.
    let warm = run(
        &bin,
        &home,
        &fpath,
        "autoload -Uz zt_victim zt_donor; zt_victim; zt_donor",
    );
    assert!(
        warm.contains("victim-ok") && warm.contains("donor-ok"),
        "warm-up run misbehaved: {warm:?}",
    );

    let shard_path = home.join("autoloads.rkyv");
    let bytes = std::fs::read(&shard_path).expect("shard written");
    let archived = rkyv::check_archived_root::<zsh::autoload_cache::AutoloadShard>(&bytes[..])
        .expect("shard is a valid archive");
    let mut shard: zsh::autoload_cache::AutoloadShard =
        rkyv::Deserialize::deserialize(archived, &mut rkyv::Infallible).expect("deserialize shard");
    let donor_blob = shard
        .entries
        .get("zt_donor")
        .expect("donor entry present")
        .chunk_blob
        .clone();
    let victim = shard.entries.get_mut("zt_victim").expect("victim entry");
    assert_ne!(
        victim.chunk_blob, donor_blob,
        "the two functions compiled to the same chunk — poison would be a no-op",
    );
    // Key, directory and producing-binary stamp all stay as the loader
    // wrote them; only the bytecode is now somebody else's.
    victim.chunk_blob = donor_blob.clone();
    let poisoned = rkyv::to_bytes::<_, 4096>(&shard).expect("re-serialize shard");
    std::fs::write(&shard_path, &poisoned[..]).expect("write poisoned shard");

    let out = Command::new(&bin)
        .args(["-c", "autoload -Uz zt_victim; zt_victim"])
        .env("ZSHRS_HOME", &home)
        .env("FPATH", &fpath)
        .env_remove("ZDOTDIR")
        .output()
        .expect("spawn zshrs");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stdout.contains("victim-ok"),
        "loader did not recover from the poisoned chunk: stdout={stdout:?} stderr={stderr:?}",
    );
    assert!(
        !stderr.contains("function not defined by file"),
        "poisoned chunk surfaced as a load failure: {stderr:?}",
    );

    // And the bad entry is gone rather than waiting to fail again.
    let after = std::fs::read(&shard_path).expect("shard still there");
    let archived_after =
        rkyv::check_archived_root::<zsh::autoload_cache::AutoloadShard>(&after[..])
            .expect("shard still a valid archive");
    let shard_after: zsh::autoload_cache::AutoloadShard =
        rkyv::Deserialize::deserialize(archived_after, &mut rkyv::Infallible)
            .expect("deserialize shard");
    if let Some(entry) = shard_after.entries.get("zt_victim") {
        assert_ne!(
            entry.chunk_blob, donor_blob,
            "the poisoned chunk is still cached — the next shell fails again",
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

/// `$funcsourcetrace[1]` names the fpath FILE, not the file with the
/// function's own name appended a second time.
///
/// The `##*/` in [`BODY`] hides this class outright: `<dir>/f/f:0` and
/// `<dir>/f:0` both tail-strip to `f:0`, so every existing assertion in
/// this file passes either way. The whole path is what completers read.
///
/// The failure this pins: `dispatch_function_call` builds a synthetic
/// `shfunc` for `doshfunc`, sets its `filename` to `getshfuncfile`'s
/// ANSWER (the resolved source file), and used to copy the real node's
/// flags verbatim — PM_LOADDIR included. PM_LOADDIR means "filename holds
/// the fpath DIRECTORY" (c:Src/hashtable.c:1061
/// `zhtricat(shf->filename, "/", shf->node.nam)`), so `doshfunc` appended
/// `/name` to a path that already ended in it and the funcstack frame read
/// `<dir>/f/f:0`.
///
/// Not cosmetic: git ships its own `_git` completion wrapper that finds
/// git-completion.bash with
/// `"$(dirname ${funcsourcetrace[1]%:*})"/git-completion.bash`. The extra
/// component sent that search into a directory that does not exist, so
/// `$script` stayed empty and `. "$script"` failed with
/// `_git:.:48: no such file or directory:` on every `git-cvsserver <TAB>`.
///
/// Reference (`zsh -f`, 5.9.2): `trace=<fpath dir>/zt_fsttrace:0`.
#[test]
fn funcsourcetrace_names_the_fpath_file_without_doubling_the_function_name() {
    let Some(bin) = zshrs_bin() else {
        eprintln!("skip: zshrs binary not built");
        return;
    };
    let tmp = std::env::temp_dir().join(format!("zshrs-fsttrace-{}", std::process::id()));
    let home = tmp.join("home");
    let fpath = tmp.join("fpath");
    std::fs::create_dir_all(&home).expect("mkdir home");
    std::fs::create_dir_all(&fpath).expect("mkdir fpath");
    std::fs::write(
        fpath.join("zt_fsttrace"),
        "print -r -- \"trace=${funcsourcetrace[1]}\"\n\
         print -r -- \"src=${functions_source[zt_fsttrace]}\"\n",
    )
    .expect("write fn");

    let want_file = fpath.join("zt_fsttrace");
    let want_trace = format!("trace={}:0", want_file.display());
    let want_src = format!("src={}", want_file.display());

    // Cold (compiles the file) and warm (serves the cached chunk) must
    // both agree: the cache is not allowed to change the attribution.
    for pass in ["cold", "warm"] {
        let out = run(
            &bin,
            &home,
            &fpath,
            "zmodload zsh/parameter 2>/dev/null; autoload -Uz zt_fsttrace; zt_fsttrace",
        );
        assert!(
            out.contains(&want_trace),
            "{pass}: funcsourcetrace is not the fpath file: want {want_trace:?}, got {out:?}",
        );
        // `functions_source` reads the real shfunctab node through
        // `getshfuncfile` and was always right; asserting it here keeps a
        // future "fix" from making the two disagree in the other direction.
        assert!(
            out.contains(&want_src),
            "{pass}: functions_source moved: want {want_src:?}, got {out:?}",
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
}
