//! `$LINENO` and error-line prefixes for code that runs out of a `.zwc`.
//!
//! C executes a dump's wordcode, whose every pipeline carries its original
//! source line (`WCB_PIPE(type, toklineno + 1)`, Src/parse.c:911/935/944,
//! loaded back at Src/exec.c:1356/2057). zshrs deparses the dump and
//! re-parses the text, which used to report the deparse's own line numbers.
//! The source files are removed after `zcompile`, so only the dump can be
//! the text that runs. Expected output is real zsh 5.9's.

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/debug/zshrs")
}

/// Run `script` with `-f` in `dir`, HOME pointed at `dir` so no cache from
/// the user's own HOME can answer. Returns stdout + stderr, merged in order.
fn run(dir: &Path, script: &str) -> String {
    let out = Command::new(zshrs_bin())
        .args(["-f", "-c", &format!("exec 2>&1; {script}")])
        .current_dir(dir)
        .env("HOME", dir)
        .env_remove("ZDOTDIR")
        .output()
        .expect("spawn zshrs");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn write(dir: &Path, rel: &str, text: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

const AUTOLOAD_BODY: &str = "\
echo a $LINENO; echo b $LINENO

echo c $LINENO
if true; then
 echo d $LINENO
fi
# comment
for i in 1; do echo e $LINENO; done
inner() {
  echo i1 $LINENO

  echo i2 $LINENO; echo i3 $LINENO
}
inner
echo f $LINENO | cat
true &&
 echo g $LINENO
: ${undefvar?boom}
";

const AUTOLOAD_EXPECTED: &str = "a 1\nb 1\nc 3\nd 5\ne 8\ni1 1\ni2 3\ni3 3\nf 15\ng 17\nfoo:18: undefvar: boom\n";

#[test]
fn zsh_style_autoload_from_zwc_keeps_source_lines() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "fn/foo", AUTOLOAD_BODY);
    assert_eq!(run(d.path(), "zcompile fn/foo"), "");
    std::fs::remove_file(d.path().join("fn/foo")).unwrap();
    // The second run is served by the autoload cache; it must carry the
    // same lines as the first, which compiled the dump.
    for _ in 0..2 {
        assert_eq!(
            run(d.path(), "fpath=($PWD/fn); autoload foo; foo"),
            AUTOLOAD_EXPECTED
        );
    }
}

#[test]
fn digest_zwc_of_fpath_dir_keeps_source_lines() {
    let d = tempfile::tempdir().unwrap();
    write(d.path(), "fn/foo", AUTOLOAD_BODY);
    assert_eq!(run(d.path(), "zcompile fn.zwc fn/foo"), "");
    std::fs::remove_file(d.path().join("fn/foo")).unwrap();
    assert_eq!(
        run(d.path(), "fpath=($PWD/fn); autoload foo; foo"),
        AUTOLOAD_EXPECTED
    );
}

#[test]
fn ksh_style_zwc_keeps_lines_of_the_defined_function() {
    let d = tempfile::tempdir().unwrap();
    write(
        d.path(),
        "fn/foo",
        "echo x $LINENO\nfunction foo {\n  echo k1 $LINENO\n\n  echo k2 $LINENO\n}\n",
    );
    assert_eq!(run(d.path(), "zcompile -k fn/foo"), "");
    std::fs::remove_file(d.path().join("fn/foo")).unwrap();
    assert_eq!(
        run(d.path(), "fpath=($PWD/fn); autoload foo; foo; foo"),
        "x 1\nk1 1\nk2 3\nk1 1\nk2 3\n"
    );
}

#[test]
fn sourced_zwc_keeps_source_lines() {
    let d = tempfile::tempdir().unwrap();
    write(
        d.path(),
        "s",
        "echo a $LINENO\n\necho b $LINENO; echo c $LINENO\nf(){ echo f $LINENO }; f\n\
         case x in (x) echo h $LINENO;; esac\n{ echo t $LINENO } always { echo u $LINENO }\n\
         nosuch_cmd_q\n",
    );
    assert_eq!(run(d.path(), "zcompile s"), "");
    std::fs::remove_file(d.path().join("s")).unwrap();
    assert_eq!(
        run(d.path(), "source ./s"),
        "a 1\nb 3\nc 3\nf 0\nh 5\nt 6\nu 6\n./s:7: command not found: nosuch_cmd_q\n"
    );
}
