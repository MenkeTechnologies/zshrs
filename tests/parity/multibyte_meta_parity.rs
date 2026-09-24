//! Multibyte / metafication parity: a value holding a high byte or a
//! multibyte character must be measured, subscripted and scanned in
//! METAFIED CHARACTERS, the unit zsh's C source uses throughout
//! (`MB_METACHARLEN`, `MB_METASTRLEN`, `itype_end`).
//!
//! Expectations are the oracle's stdout from `/opt/homebrew/bin/zsh -f -c`
//! under `LC_ALL=en_US.UTF-8`, written as hex because several of them are
//! not valid UTF-8 (a lone `0xdc` byte) and a lossy `String` compare would
//! hide exactly the bytes under test. When a zsh is installed the pin is
//! also re-checked against it.

use std::path::{Path, PathBuf};
use std::process::Command;

const LOCALE: &str = "en_US.UTF-8";

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

fn zsh_path() -> Option<&'static str> {
    ["/opt/homebrew/bin/zsh", "/usr/local/bin/zsh", "/bin/zsh"]
        .into_iter()
        .find(|p| Path::new(p).exists())
}

fn run(cmd: &mut Command, script: &str) -> Vec<u8> {
    cmd.args(["-f", "-c", script])
        .env_remove("LC_CTYPE")
        .env_remove("LANG")
        .env_remove("ZSHRS_CACHE")
        .env("LC_ALL", LOCALE)
        .output()
        .expect("spawn shell")
        .stdout
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn assert_bytes(script: &str, expect_hex: &str) {
    if let Some(z) = zsh_path() {
        let got = run(&mut Command::new(z), script);
        assert_eq!(hex(&got), expect_hex, "pin no longer matches zsh\n  script: {script}");
    }
    let mut c = Command::new(zshrs_bin());
    c.arg("--zsh");
    let got = run(&mut c, script);
    assert_eq!(hex(&got), expect_hex, "zshrs diverged from zsh\n  script: {script}");
}

/// `$#c` inside `$(( ))` counts characters with `MB_METASTRLEN`
/// (c:Src/subst.c:4490 singsub → paramsubst's length arm), so the invalid
/// byte `0xdc` — stored as a `Meta` pair — is ONE character, not two.
#[test]
fn arith_length_of_invalid_high_byte_is_one() {
    let s = r#"c=$'\M-\\'; print $(( $#c )) $[ $#c ]; (( n = $#c )); print $n"#;
    assert_bytes(s, "3120310a310a");
}

/// c:Src/params.c:1634-1663 — a scalar subscript walks `MB_METACHARLEN`
/// units. The unbraced `$c[$i]` / `$c[1,$i]` arm and the nested
/// `${${c}[$i]}` arm split the METAFIED text instead and returned the bare
/// `Meta` byte (`c2 83`).
#[test]
fn dynamic_subscript_keeps_meta_pair_whole() {
    let s = r#"c=$'\M-\\'; i=1; print -r -- $c[$i] $c[1,$i] ${${c}[$i]} ${${c}[1,$i]}"#;
    assert_bytes(s, "dc20dc20dc20dc0a");
}

/// A03quoting "$'-style quote with metafied backslash": both halves of
/// the loop — the `$#chars` bound and `$chars[$i]` — must see seven units.
#[test]
fn metafied_backslash_loop_matches_a03() {
    let s = r#"chars=$(print -r $'BS\\MBS\M-\\'); for (( i = 1; i <= $#chars; i++ )); do char=$chars[$i]; print -n $(( [#16] #char )) ""; done"#;
    assert_bytes(
        s,
        "313623343220313623353320313623354320313623344420313623343220313623353320313623444320",
    );
}

/// With MULTIBYTE unset a unit is one BYTE (c:Src/utils.c:5613), so the
/// second unit of `héllo` is the lead byte `0xc3` on both the unbraced and
/// the nested subscript arms.
#[test]
fn nomultibyte_dynamic_subscript_is_one_byte() {
    let s = r#"s=héllo; i=2; print -r -- $s[$i] $s[2,$i+2] ${${s}[$i]} ${${s}[-1]}; unsetopt multibyte; print -r -- $s[$i] ${${s}[$i]}"#;
    assert_bytes(s, "c3a920c3a96c6c20c3a9206f0ac320c30a");
}

/// c:Src/params.c:2215-2216 — the unbraced name ends at
/// `itype_end(s, IIDENT, 0)`, which accepts `iswalnum` characters unless
/// POSIX_IDENTIFIERS is set (c:Src/utils.c:4347-4350). D07multibyte
/// "POSIX_IDENTIFIERS option".
#[test]
fn unbraced_multibyte_identifier() {
    let s = r#"hähä=3; print -r -- $hähä "$hähä" x$hähä; setopt posixidentifiers; print -r -- $hähä"#;
    assert_bytes(s, "3320332078330ac3a468c3a40a");
}

fn run_stderr(cmd: &mut Command, script: &str) -> String {
    let o = cmd
        .args(["-f", "-c", script])
        .env_remove("LC_CTYPE")
        .env_remove("LANG")
        .env_remove("ZSHRS_CACHE")
        .env("LC_ALL", LOCALE)
        .output()
        .expect("spawn shell");
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// c:Src/utils.c:314-316 — zerrmsg prints every `%s` argument through
/// `nicezputs`, so an invalid byte in a command name, a `cd` target or an
/// `unset` operand is shown as `\M-i`, a control char as `^A`/`\t`. The
/// port formats the message before zerr sees it, so each call site has to
/// nice-format its argument (exec.c:818/903, builtin.c:1080/3877).
/// D07multibyte "Invalid parameter name with following tokenized input".
#[test]
fn error_messages_nice_format_their_argument() {
    let s = r#"x=$'a\xe9b'; $x; cd $x; y=$'a\x01é\tb'; $y; cd $y; typeset -A h; unset "h[$x""#;
    let want = "zsh:1: command not found: a\\M-ib\n\
                zsh:cd:1: no such file or directory: a\\M-ib\n\
                zsh:1: command not found: a^Aé\\tb\n\
                zsh:cd:1: no such file or directory: a^Aé\\tb\n\
                zsh:unset:1: h[a\\M-ib: invalid parameter name\n";
    if let Some(z) = zsh_path() {
        assert_eq!(run_stderr(&mut Command::new(z), s), want, "pin no longer matches zsh");
    }
    let mut c = Command::new(zshrs_bin());
    c.arg("--zsh");
    assert_eq!(run_stderr(&mut c, s), want, "zshrs diverged from zsh");
}

/// The sourced-file form from D07: `$\xe9#` followed by tokenized input
/// is not a parameter, so the whole word is the command name.
#[test]
fn sourced_bad_param_name_error_uses_meta_notation() {
    let dir = std::env::temp_dir().join(format!("zshrs_mb_badparam_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let s = r#"print $'$\xe9#``' >test_bad_param; (setopt nonomatch; . ./test_bad_param)"#;
    let want = "./test_bad_param:1: command not found: $\\M-i#\n";
    if let Some(z) = zsh_path() {
        assert_eq!(run_stderr(Command::new(z).current_dir(&dir), s), want, "pin no longer matches zsh");
    }
    let mut c = Command::new(zshrs_bin());
    c.arg("--zsh").current_dir(&dir);
    assert_eq!(run_stderr(&mut c, s), want, "zshrs diverged from zsh");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `${c:off:len}` and a chained `${c[i][j]}` walk the value with
/// `MB_METACHARLEN` (c:Src/subst.c:3726-3755, c:Src/params.c:1634-1663).
/// Both arms demetafied through a LOSSY UTF-8 decode, so the invalid byte
/// `0xe9` came back as U+FFFD, and they counted `char`s where
/// `unsetopt multibyte` makes every unit a byte.
#[test]
fn substring_and_chained_subscript_keep_raw_bytes() {
    let s = r#"c=$'a\xe9b'; print -r -- ${c:1:1} ${c:1} ${c: -2:1} ${c:1:-1} ${c[2][1]}; unsetopt multibyte; s=héllo; print -r -- ${s:1:2} ${s: -4:1} ${s[2][1]}"#;
    assert_bytes(s, "e920e96220e920e920e90ac3a920a920c30a");
}

fn assert_parity_text(script: &str) {
    let mut c = Command::new(zshrs_bin());
    c.arg("--zsh");
    let dir = std::env::temp_dir().join(format!(
        "zshrs_mb_{}_{}",
        std::process::id(),
        script.len()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let got = run_stderr_and_stdout(c.current_dir(&dir), script);
    if let Some(z) = zsh_path() {
        let want = run_stderr_and_stdout(Command::new(z).current_dir(&dir), script);
        assert_eq!(got, want, "zshrs diverged from zsh\n  script: {script}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn run_stderr_and_stdout(cmd: &mut Command, script: &str) -> (String, String) {
    let o = cmd
        .args(["-f", "-c", script])
        .env_remove("LC_CTYPE")
        .env_remove("LANG")
        .env_remove("ZSHRS_CACHE")
        .env("LC_ALL", LOCALE)
        .output()
        .expect("spawn shell");
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

/// c:Src/Modules/stat.c:574-582 — with `-n` the file name leads each
/// file's entries: an element for `-A`, a `name` pair for `-H`.
#[test]
fn zstat_name_prefix_in_array_and_hash() {
    assert_parity_text(
        r#"zmodload zsh/stat; touch a b; zstat -A arr -n +size -- a b; print -r -- $arr; zstat -nH h +size a; print -r -- ${(kv)h}; touch 50150-é 50150-Ą; zstat +size -A sizes -nor -- 50150-*; print -r -- $sizes"#,
    );
}

/// c:Src/builtin.c:5335-5372 — printf's `%s`/`%b` width and precision
/// count characters with `mbrlen` over the argument's raw bytes, one per
/// byte with MULTIBYTE off. Counting the port's metafied `char`s made the
/// undecodable byte `0xe9` two columns wide, and `unsetopt multibyte` still
/// padded and truncated by character. (A precision that lands on an
/// invalid sequence is a 5.9.2 / dev-tree version split and is not pinned.)
#[test]
fn printf_width_counts_mbrlen_characters() {
    let s = r#"printf "%3s|%-3s|%3b|\n" $'\xe9' $'\xe9' "\xe9"; printf "%4s|%.2s|\n" é éab; unsetopt multibyte; printf "%3s|%.1s|%-3s|\n" é é é"#;
    assert_bytes(
        s,
        "2020e97ce920207c2020e97c0a202020c3a97cc3a9617c0a20c3a97cc37cc3a9207c0a",
    );
}
