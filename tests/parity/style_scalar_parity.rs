//! `zstyle -s` read parity: the whole value array, and "set" vs "non-empty".
//!
//! `zstyle -s` is `bin_zstyle` case `'s'`, `Src/Modules/zutil.c:643-658`:
//!
//! ```text
//! c:648      if ((vals = lookupstyle(args[1], args[2])) && vals[0]) {
//! c:649          ret = sepjoin(vals, (args[4] ? args[4] : " "), 0);
//! c:650          val = 0;
//! c:651      } else {
//! c:652          ret = ztrdup("");
//! c:653          val = 1;
//! c:654      }
//! ```
//!
//! A compsys port that spells this
//! `lookupstyle(ctx, style).first().cloned().unwrap_or_default()` and then
//! branches on `!value.is_empty()` loses BOTH halves:
//!
//!   * c:649 joins the WHOLE value array with a space. A style written
//!     unquoted — which is the ordinary spelling for anything that is a
//!     command line, a format string, a prompt, a matcher spec or a path with
//!     a space in it — is several elements, and element 1 alone silently
//!     truncates it.
//!   * c:648 tests `vals[0]`, a POINTER. `zstyle <ctx> <style> ''` is SET and
//!     `zstyle -s` returns 0 for it, which is how a user turns a style OFF
//!     for a narrower context than the one that turned it on. Reading an
//!     empty value as "no style" inverts that.
//!
//! `_call_program`'s `command` style was the first instance
//! (`tests/parity/call_program_parity.rs`); the cases below pin the rest of
//! the sweep. Each names the upstream line it comes from.
//!
//! Liveness: every case asserts the ORACLE's own answer first. Without that a
//! style that silently failed to apply at all would make both shells print
//! the same default and pass vacuously.
//!
//! Skip pattern: no-ops silently when no zsh binary or no stock function
//! directory is present.

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

fn zsh_path() -> &'static str {
    for p in [
        "/opt/homebrew/bin/zsh",
        "/usr/local/bin/zsh",
        "/bin/zsh",
        "/usr/bin/zsh",
    ] {
        if Path::new(p).exists() {
            return p;
        }
    }
    "zsh"
}

/// The stock `Completion/` tree, wherever this zsh installed it. Both shells
/// must autoload the SAME upstream function or the comparison means nothing.
fn stock_fpath() -> Option<PathBuf> {
    let out = Command::new(zsh_path())
        .args(["-fc", "print -rl -- $fpath"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(PathBuf::from)
        .find(|d| d.join("_description").is_file())
}

/// Preamble both shells run: pin the stock fpath and load compsys.
///
/// `_setup` is stubbed out. It is the one thing `_description` sh:19 calls
/// that needs a live completion widget (it assigns into `$compstate`, which
/// outside `zle` is "assignment to invalid subscript range" in zsh and aborts
/// the function). Both shells get the same stub, and every assertion here is
/// on `$expl`, which `_description` builds itself at sh:92-104 — nothing the
/// stub covers. Without it the ORACLE prints nothing and the cases would be
/// vacuous on both sides.
fn preamble(fpath: &Path, dump_tag: &str, stub_setup: bool) -> String {
    format!(
        "fpath=( {} )\n\
         autoload -Uz compinit\n\
         compinit -u -d ${{TMPDIR:-/tmp}}/zshrs_style_scalar_{}_$$\n\
         setopt extendedglob\n\
         {}",
        fpath.display(),
        dump_tag,
        if stub_setup { "_setup() { : }\n" } else { "" },
    )
}

fn run(bin: &[&str], script: &str) -> String {
    let o = Command::new(bin[0])
        .args(&bin[1..])
        .arg("-c")
        .arg(script)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("shell");
    // stderr is dropped: one shell's compinit diagnostics are not the subject
    // of any case here.
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// Run `body` under the preamble in both shells and require identical stdout.
/// `expect` is the oracle's own answer, asserted FIRST.
fn assert_parity_inner(dump_tag: &str, body: &str, expect: &str, stub_setup: bool) {
    let Some(fpath) = stock_fpath() else { return };
    let script = format!("{}{}", preamble(&fpath, dump_tag, stub_setup), body);

    let z = run(&[zsh_path(), "-f"], &script);
    assert_eq!(
        z.trim_end(),
        expect,
        "ORACLE did not produce the documented answer — the case is not \
         exercising what it claims.\n--- script ---\n{script}"
    );

    let bin = zshrs_bin();
    let r = run(&[bin.to_str().unwrap(), "--zsh", "-f"], &script);
    assert_eq!(
        z, r,
        "divergence\n--- script ---\n{script}\n--- zsh ---\n{z:?}\n--- zshrs ---\n{r:?}"
    );
}

/// Cases that go through `_description`, which needs the `_setup` stub.
fn assert_descr_parity(dump_tag: &str, body: &str, expect: &str) {
    assert_parity_inner(dump_tag, body, expect, true);
}

/// Cases whose function runs standalone.
fn assert_parity(dump_tag: &str, body: &str, expect: &str) {
    assert_parity_inner(dump_tag, body, expect, false);
}

// ---------------------------------------------------------------------------
// _description sh:23-24 — `format` chain
// ---------------------------------------------------------------------------

/// `_description` sh:23-24:
///
/// ```text
/// zstyle -s ":completion:${curcontext}:$1" format format ||
///     zstyle -s ":completion:${curcontext}:descriptions" format format
/// ```
///
/// The `||` runs on c:648's STATUS. A tag-specific `format ''` is SET, so
/// upstream STOPS there and that tag gets no header — this is the documented
/// way to silence one tag while a global `:descriptions` format stays in
/// force for everything else. Falling through on an empty value applied the
/// global format to the very tag that had turned it off.
#[test]
fn description_empty_tag_format_does_not_fall_through_to_descriptions() {
    assert_descr_parity(
        "dfmtempty",
        "curcontext=mycmd\n\
         zstyle ':completion:mycmd:descriptions' format 'GLOBAL %d'\n\
         zstyle ':completion:mycmd:mytag' format ''\n\
         local -a expl\n\
         _description mytag expl 'a thing'\n\
         print -r -- \"expl=${(j:|:)expl}\"\n",
        "expl=-J|-default-",
    );
}

/// c:649 joins the whole value array, so a `format` written unquoted keeps
/// every word. Element 1 alone left `--` as the entire format string and the
/// description text vanished from the header.
#[test]
fn description_format_joins_all_of_its_values() {
    assert_descr_parity(
        "dfmtjoin",
        "curcontext=mycmd\n\
         zstyle ':completion:mycmd:mytag' format -- TAGFMT %d\n\
         local -a expl\n\
         _description mytag expl 'a thing'\n\
         print -r -- \"expl=${(j:|:)expl}\"\n",
        "expl=-J|-default-|-X|-- TAGFMT a thing",
    );
}

// ---------------------------------------------------------------------------
// _description sh:31-32 — `matcher`
// ---------------------------------------------------------------------------

/// `_description` sh:31-32 is `zstyle -s … matcher match && opts=($opts -M
/// "$match")`. The `&&` is c:648's status, so `matcher ''` appends a
/// deliberately empty `-M` — which is how a narrower context cancels a
/// matcher a broader one set. Gating on the value being non-empty dropped
/// the `-M` entirely and the broader matcher kept applying.
#[test]
fn description_empty_matcher_still_emits_dash_m() {
    assert_descr_parity(
        "dmatchempty",
        "curcontext=mycmd\n\
         zstyle ':completion:mycmd:mytag' matcher ''\n\
         local -a expl\n\
         _description mytag expl 'a thing'\n\
         print -r -- \"expl=${(j:|:)expl}\"\n",
        "expl=-M||-J|-default-",
    );
}

/// A matcher spec written as several unquoted words is joined by c:649 into
/// the ONE argument `-M` takes. Element 1 alone silently dropped every
/// matcher rule after the first.
#[test]
fn description_matcher_joins_all_of_its_values() {
    assert_descr_parity(
        "dmatchjoin",
        "curcontext=mycmd\n\
         zstyle ':completion:mycmd:mytag' matcher 'm:{a-z}={A-Z}' 'r:|=*'\n\
         local -a expl\n\
         _description mytag expl 'a thing'\n\
         print -r -- \"expl=${(j:|:)expl}\"\n",
        "expl=-M|m:{a-z}={A-Z} r:|=*|-J|-default-",
    );
}

// ---------------------------------------------------------------------------
// the caching layer — `cache-path`
// ---------------------------------------------------------------------------

/// `_cache_invalid` sh:12-13:
///
/// ```text
/// zstyle -s ":completion:${curcontext}:" cache-path _cache_dir
/// : ${_cache_dir:=${ZDOTDIR:-$HOME}/.zcompcache}
/// ```
///
/// sh:13's `:=` is an EMPTINESS test applied AFTER the lookup, so a
/// `cache-path ''` still takes the default. A port that folds the default
/// into the lookup's `unwrap_or_else` gets the opposite: c:648 says "set",
/// the value is `""`, and the cache path becomes `/<ident>` at the
/// filesystem root.
#[test]
fn cache_invalid_empty_cache_path_takes_the_default_directory() {
    assert_parity(
        "civempty",
        "curcontext=mycmd\n\
         zstyle ':completion:mycmd:' use-cache yes\n\
         zstyle ':completion:mycmd:' cache-path ''\n\
         zstyle ':completion:mycmd:' cache-policy pol\n\
         pol() { print -r -- \"POLICY got: ${1#${ZDOTDIR:-$HOME}}\"; return 1 }\n\
         _cache_invalid ident\n",
        "POLICY got: /.zcompcache/ident",
    );
}

/// c:649 joins the whole value array, so a cache directory whose path has a
/// space in it survives. Element 1 alone cut it at the first word and the
/// cache silently went somewhere else.
#[test]
fn cache_invalid_cache_path_joins_all_of_its_values() {
    assert_parity(
        "civjoin",
        "curcontext=mycmd\n\
         zstyle ':completion:mycmd:' use-cache yes\n\
         zstyle ':completion:mycmd:' cache-path /tmp/one two\n\
         zstyle ':completion:mycmd:' cache-policy pol\n\
         pol() { print -r -- \"POLICY got: $1\"; return 1 }\n\
         _cache_invalid ident\n",
        "POLICY got: /tmp/one two/ident",
    );
}

/// `_store_cache` sh:10-11 is the same two lines. The written file has to
/// land under the joined directory, not under its first word.
#[test]
fn store_cache_cache_path_joins_all_of_its_values() {
    assert_parity(
        "scjoin",
        "curcontext=mycmd\n\
         base=${TMPDIR:-/tmp}/zshrs_style_scalar_sc_$$\n\
         zstyle ':completion:mycmd:' use-cache yes\n\
         eval \"zstyle ':completion:mycmd:' cache-path $base/a b\"\n\
         typeset -a myvar=( one two )\n\
         _store_cache ident myvar\n\
         print -rl -- ${(f)\"$(cd $base 2>/dev/null && print -rl -- **/ident(N))\"}\n\
         command rm -rf $base\n",
        "a b/ident",
    );
}

/// `_retrieve_cache` sh:10-11, the read half: the cache file is found and
/// sourced only when the joined directory is used.
#[test]
fn retrieve_cache_cache_path_joins_all_of_its_values() {
    assert_parity(
        "rcjoin",
        "curcontext=mycmd\n\
         base=${TMPDIR:-/tmp}/zshrs_style_scalar_rc_$$\n\
         command mkdir -p \"$base/a b\"\n\
         print -r 'CACHEVAR=hit' > \"$base/a b/ident\"\n\
         zstyle ':completion:mycmd:' use-cache yes\n\
         eval \"zstyle ':completion:mycmd:' cache-path $base/a b\"\n\
         _retrieve_cache ident\n\
         print -r -- \"rc=$? CACHEVAR=$CACHEVAR\"\n\
         command rm -rf $base\n",
        "rc=0 CACHEVAR=hit",
    );
}
