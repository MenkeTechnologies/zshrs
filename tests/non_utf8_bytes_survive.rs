//! A byte that is not valid UTF-8 has to survive the completer load path.
//!
//! zsh has no encoding requirement on anything it reads. `Src/input.c`
//! reads script text as bytes and `Src/utils.c metafy` escapes only the
//! reserved IMETA bytes, so a lone ISO-8859-1 `0xE9` travels through the
//! lexer, through `$( … )` capture, through `read`, and back out to the
//! terminal unchanged. The legacy completion corpus depends on that: a
//! `#compdef` file whose author line or `_describe` text predates UTF-8
//! is ordinary input to zsh, and the helper programs `_call_program`
//! shells out to emit such bytes routinely.
//!
//! zshrs carries shell text in a Rust `String`, which cannot hold the
//! byte directly, so it Meta-encodes it (`src/script_bytes.rs`) and
//! decodes at the output boundary (`utils::unmetafy_str`). These tests
//! pin the property that matters — the byte that goes in is the byte
//! that comes out — at each place bytes ENTER the shell. They assert on
//! raw `stdout` bytes, never on a `String`, because every lossy
//! conversion this file guards against is invisible once the output has
//! been through `from_utf8_lossy`.
//!
//! Locale is pinned to `LC_ALL=C`, which exists everywhere including a
//! headless Linux CI box, and which is also the setting under which the
//! length assertion below is byte-valued (verified against
//! `zsh 5.9.2 -f`: `caf\xe9` is four).

use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::process::Command;

/// ISO-8859-1 `é` — a single byte that is not a valid UTF-8 sequence,
/// and the one a latin-1 completion file is most likely to carry.
const LATIN1_E: u8 = 0xE9;

/// UTF-8 replacement character. Its appearance anywhere in the output is
/// the exact failure this file exists to catch: a `from_utf8_lossy` that
/// silently substituted the byte instead of carrying it.
const REPLACEMENT: &[u8] = "\u{FFFD}".as_bytes();

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

/// Run a script whose text is RAW BYTES (it contains `0xE9`, so it is not
/// a `str`) and return raw stdout. `-f` skips every startup file, so the
/// only thing under test is the shell itself.
fn run(bin: &PathBuf, script: &[u8], tmp: &PathBuf) -> Vec<u8> {
    let out = Command::new(bin)
        .arg("-f")
        .arg("-c")
        .arg(std::ffi::OsStr::from_bytes(script))
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .env("ZSHRS_HOME", tmp)
        .env_remove("ZDOTDIR")
        .output()
        .expect("spawn zshrs");
    out.stdout
}

/// A private directory for the fixture files and for `ZSHRS_HOME`, so a
/// concurrent test run never shares either.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("zshrs-nonutf8-{}-{}", tag, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn assert_clean(got: &[u8], want: &[u8], what: &str) {
    assert!(
        !got.windows(3).any(|w| w == REPLACEMENT),
        "{what}: output was corrupted to U+FFFD: {got:?}"
    );
    assert_eq!(got, want, "{what}: bytes differ");
}

/// The headline case: a completion file carrying a latin-1 byte in a
/// comment AND inside a description string loads, and both the stored
/// body and the description it prints come back byte-for-byte.
///
/// This is the whole bug in one test. Reading the file with
/// `fs::read_to_string` failed the ENTIRE file on the first bad byte, so
/// the completer did not merely lose its accent — it did not exist, and
/// `autoload` reported nothing.
#[test]
fn latin1_completion_file_loads_and_round_trips() {
    let Some(bin) = zshrs_bin() else {
        eprintln!("skipping: no zshrs binary built");
        return;
    };
    let dir = scratch("autoload");
    let fpath = dir.join("fpath");
    std::fs::create_dir_all(&fpath).unwrap();

    // `# author: Jos<0xE9>` is the comment case; `"caf<0xE9>"` is the
    // description case. Written as bytes — this is not valid UTF-8 and
    // must not be routed through anything that assumes it is.
    let mut body = Vec::new();
    body.extend_from_slice(b"#compdef _latin1\n# author: Jos");
    body.push(LATIN1_E);
    body.extend_from_slice(b"\nprint -rn -- \"caf");
    body.push(LATIN1_E);
    body.extend_from_slice(b"\"\n");
    std::fs::write(fpath.join("_latin1"), &body).unwrap();

    let mut script = Vec::new();
    script.extend_from_slice(b"fpath=(");
    script.extend_from_slice(fpath.to_str().unwrap().as_bytes());
    // `+X` loads the definition file without running it, which is the
    // step `compinit`/`_main_complete` perform; then the body is printed
    // so a byte lost at PARSE time is visible separately from one lost at
    // RUN time.
    script.extend_from_slice(b"); autoload -Uz +X _latin1; print -rn -- \"$functions[_latin1]\"; print -rn -- '|'; _latin1");

    let out = run(&bin, &script, &dir);

    let (stored, printed) = {
        let bar = out
            .iter()
            .position(|&b| b == b'|')
            .expect("separator missing — the completer did not load at all");
        (&out[..bar], &out[bar + 1..])
    };
    assert!(
        stored.contains(&LATIN1_E),
        "stored function body lost the latin-1 byte: {stored:?}"
    );
    assert!(
        !stored.windows(3).any(|w| w == REPLACEMENT),
        "stored function body was corrupted to U+FFFD: {stored:?}"
    );
    assert_clean(printed, b"caf\xe9", "description printed by the completer");

    let _ = std::fs::remove_dir_all(&dir);
}

/// `$( … )` is how every completer harvests candidates and descriptions
/// (`_call_program` is a `$( … )` in a trenchcoat), so a helper that
/// emits a legacy byte must not have it replaced on the way in.
#[test]
fn command_substitution_carries_a_latin1_byte() {
    let Some(bin) = zshrs_bin() else {
        eprintln!("skipping: no zshrs binary built");
        return;
    };
    let dir = scratch("cmdsubst");

    // Captured, then re-emitted: both the pipe read and the write-back
    // have to be lossless for this to hold.
    let out = run(&bin, br#"print -rn -- "$(printf 'caf\xe9')""#, &dir);
    assert_clean(&out, b"caf\xe9", "$( … ) capture");

    // Split on newlines the way `_describe`'s input arrays are built —
    // the byte must survive field splitting too, not just capture.
    let out = run(
        &bin,
        br#"a=( ${(f)"$(printf 'one\ncaf\xe9\nthree')"} ); print -rn -- "${a[2]}"; print -rn -- "/${#a}""#,
        &dir,
    );
    assert_clean(&out, b"caf\xe9/3", "$( … ) capture split on newlines");

    let _ = std::fs::remove_dir_all(&dir);
}

/// `$(< file)` and `read` are the other two ways a completer ingests
/// bytes it did not produce — a cache file, a `.desc` sidecar, a
/// `while read` over a generated list.
#[test]
fn file_reads_carry_a_latin1_byte() {
    let Some(bin) = zshrs_bin() else {
        eprintln!("skipping: no zshrs binary built");
        return;
    };
    let dir = scratch("fileread");
    let data = dir.join("cache");
    std::fs::write(&data, b"caf\xe9\n").unwrap();
    let path = data.to_str().unwrap().as_bytes();

    let mut script = Vec::new();
    script.extend_from_slice(b"print -rn -- \"$(<");
    script.extend_from_slice(path);
    script.extend_from_slice(b")\"");
    assert_clean(&run(&bin, &script, &dir), b"caf\xe9", "$(< file)");

    let mut script = Vec::new();
    script.extend_from_slice(b"read -r l < ");
    script.extend_from_slice(path);
    script.extend_from_slice(b"; print -rn -- \"$l\"");
    assert_clean(&run(&bin, &script, &dir), b"caf\xe9", "read < file");

    let mut script = Vec::new();
    script.extend_from_slice(b"zmodload zsh/mapfile; print -rn -- \"$mapfile[");
    script.extend_from_slice(path);
    script.extend_from_slice(b"]\"");
    assert_clean(&run(&bin, &script, &dir), b"caf\xe9\n", "$mapfile[file]");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Under `LC_ALL=C` every length op counts BYTES
/// (`Src/utils.c mb_metastrlenend`: `MB_CUR_MAX == 1` returns
/// `ztrlen`, the length of the DEMETAFIED stream). A captured
/// `caf<0xE9>` is four bytes to zsh; counting zshrs's Meta encoding
/// instead reports seven, which is how a completer's column arithmetic
/// goes wrong without any visible corruption.
#[test]
fn c_locale_length_counts_the_byte_not_its_encoding() {
    let Some(bin) = zshrs_bin() else {
        eprintln!("skipping: no zshrs binary built");
        return;
    };
    let dir = scratch("length");

    let out = run(
        &bin,
        br#"v="$(printf 'caf\xe9')"; print -rn -- "${#v}""#,
        &dir,
    );
    assert_eq!(out, b"4", "${{#}} under LC_ALL=C must count bytes");

    // Same value reached through `$'…'` rather than a capture: the two
    // spellings must agree, or only one of the two encoders is honest.
    let out = run(&bin, br#"v=$'caf\xe9'; print -rn -- "${#v}""#, &dir);
    assert_eq!(out, b"4", "${{#}} of a $'…' byte must count bytes");

    let _ = std::fs::remove_dir_all(&dir);
}
