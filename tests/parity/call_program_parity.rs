//! `_call_program` parity: the `command` style hook and the `-p`
//! privilege-prefix arm.
//!
//! `_call_program` is the indirection every compsys completer goes through to
//! run an external command, and it is the ONE documented place a user can
//! intercept that command. From the manual entry for the `command` style:
//!
//!     The _strings_ from the call to `_call_program`, or from the style if
//!     set, are concatenated with spaces between them and the resulting
//!     string is evaluated.
//!
//! and, for `gain-privileges`:
//!
//!     To force the use of, e.g. `sudo` or to override any prefix that might
//!     be added due to `gain-privileges`, the `command` style can be used
//!     with a value that begins with a hyphen.
//!     ...
//!     When looking up the `gain-privileges` and `command` styles, the
//!     command component of the zstyle context will end with a slash ("/")
//!     followed by the command that would be used to gain privileges.
//!
//! Upstream (`Completion/Base/Utility/_call_program`) implements that in
//! nine lines, and each of the cases below pins one of them:
//!
//!     sh:10  curcontext="${curcontext%:*}/${${(@M)_comp_priv_prefix:#^*[^\\]=*}[1]}:"
//!     sh:11  zstyle -t ":completion:${curcontext}:${1}" gain-privileges &&
//!     sh:12    prefix=( $_comp_priv_prefix )
//!     sh:26  if zstyle -s ":completion:${curcontext}:${1}" command tmp; then
//!     sh:27    if [[ "$tmp" = -* ]]; then
//!     sh:28      eval $clocale "$tmp[2,-1]" "$argv[2,-1]"
//!     sh:30      eval $clocale $prefix "$tmp"
//!     sh:33      eval $clocale $prefix "$argv[2,-1]"
//!
//! Three of these failed before the fix, in ways a completer sees as "my
//! command produced nothing":
//!
//!   * sh:26 was read as `zstyle -s … | head -1` rather than
//!     `zutil.c:649`'s `sepjoin(vals, " ")`, so a `command` style written as
//!     several words — the ordinary spelling — kept only its first word.
//!   * sh:26 was branched on the VALUE being non-empty rather than on
//!     `zstyle -s`'s STATUS (`zutil.c:648` tests `vals[0]`, a pointer), so
//!     `command ''` fell through to the default command instead of running
//!     nothing.
//!   * sh:9-13 was not implemented at all, so neither the privilege prefix
//!     nor the `/cmd` context mangling that the manual documents existed.
//!
//! Liveness: every case asserts what the ORACLE produced, not just that the
//! two shells agree — a `command` style that silently failed to apply would
//! otherwise make both sides print the default command's output and pass.
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

/// The stock `Completion/` tree, wherever this zsh installed it. zsh needs it
/// to autoload `_call_program`; zshrs answers the name from its own router
/// and would run without one, but both shells must see the SAME upstream
/// function or the comparison means nothing.
fn stock_fpath() -> Option<PathBuf> {
    // `zsh -f` starts with the compiled-in module fpath, which is exactly the
    // directory holding the stock functions.
    let out = Command::new(zsh_path())
        .args(["-fc", "print -rl -- $fpath"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(PathBuf::from)
        .find(|d| d.join("_call_program").is_file())
}

/// Preamble both shells run: pin the stock fpath, load compsys, and turn on
/// the one option sh:10's pattern needs.
///
/// `EXTENDED_GLOB` is not a test convenience — sh:10's `:#^*[^\\]=*` is an
/// extended-glob negation, and upstream gets the option from `_comp_options`
/// (`compinit` sh:141), which `_main_complete` applies with
/// `setopt localoptions`. Pinning it here reproduces the state the function
/// actually runs in.
fn preamble(fpath: &Path, dump_tag: &str) -> String {
    format!(
        "fpath=( {} )\n\
         autoload -Uz compinit\n\
         compinit -u -d ${{TMPDIR:-/tmp}}/zshrs_callprog_parity_{}_$$\n\
         autoload -Uz _call_program\n\
         setopt extendedglob\n",
        fpath.display(),
        dump_tag,
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
    // stderr is deliberately dropped: sh:19-22 sends a helper's stderr to
    // /dev/null when fd 2 is a terminal and through when it is not, and under
    // `Command` it is never a terminal — so a diagnostic from one shell's
    // compinit would show up as a divergence that has nothing to do with the
    // case.
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// Run `body` under the preamble in both shells and require identical stdout.
/// `expect` is the oracle's own answer, asserted first: without it a case
/// where the style never applied at all would pass on both sides.
fn assert_parity(dump_tag: &str, body: &str, expect: &str) {
    let Some(fpath) = stock_fpath() else { return };
    let script = format!("{}{}", preamble(&fpath, dump_tag), body);

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

/// sh:33 — no `command` style: the arguments after the tag ARE the command.
#[test]
fn no_style_runs_the_arguments() {
    assert_parity(
        "nostyle",
        "curcontext=mycmd\n\
         print -r -- \"$(_call_program mytag echo BASE one two)\"\n",
        "BASE one two",
    );
}

/// sh:26-30 — a single-word `command` style REPLACES the command.
#[test]
fn command_style_replaces_the_command() {
    assert_parity(
        "repl1",
        "curcontext=mycmd\n\
         zstyle ':completion:mycmd:mytag' command 'echo REPL'\n\
         print -r -- \"$(_call_program mytag echo BASE one two)\"\n",
        "REPL",
    );
}

/// `zutil.c:649` — `zstyle -s` is `sepjoin(vals, " ")` over the WHOLE value
/// array, which is how the manual's "concatenated with spaces" reads in C.
///
/// This is the case a `.first()` spelling loses: the style below is three
/// elements, element 1 is the bare word `echo`, and running that alone
/// prints an empty line instead of `REPL2 extra`. Every multi-word `command`
/// style a user writes unquoted lands here.
#[test]
fn command_style_joins_all_of_its_values() {
    assert_parity(
        "replarr",
        "curcontext=mycmd\n\
         zstyle ':completion:mycmd:mytag' command echo REPL2 extra\n\
         print -r -- \"$(_call_program mytag echo BASE one two)\"\n",
        "REPL2 extra",
    );
}

/// sh:27-28 — a leading `-` makes the style a PREFIX: the dash is stripped
/// and the original arguments are appended, rather than replaced.
#[test]
fn leading_dash_wraps_instead_of_replacing() {
    assert_parity(
        "wrap1",
        "curcontext=mycmd\n\
         zstyle ':completion:mycmd:mytag' command '-print -r -- WRAPPED'\n\
         print -r -- \"$(_call_program mytag echo BASE one two)\"\n",
        "WRAPPED echo BASE one two",
    );
}

/// sh:27-28 with a multi-element style — the join at `zutil.c:649` happens
/// BEFORE the `-*` test, so the dash belongs to the joined string and the
/// rest of the elements are part of the prefix.
#[test]
fn leading_dash_wrap_joins_all_of_its_values() {
    assert_parity(
        "wraparr",
        "curcontext=mycmd\n\
         zstyle ':completion:mycmd:mytag' command '-print' -r -- WRAP2\n\
         print -r -- \"$(_call_program mytag echo BASE one two)\"\n",
        "WRAP2 echo BASE one two",
    );
}

/// `zutil.c:648` tests `vals[0]`, a POINTER, so a style set to the empty
/// string is SET: sh:30 evaluates nothing and the default command at sh:33
/// never runs. Reading "empty value" as "no style" silently inverts this and
/// runs the very command the user disabled.
#[test]
fn empty_command_style_is_set_and_suppresses_the_command() {
    assert_parity(
        "replempty",
        "curcontext=mycmd\n\
         zstyle ':completion:mycmd:mytag' command ''\n\
         print -r -- \"OUT<$(_call_program mytag echo BASE)>\"\n",
        "OUT<>",
    );
}

/// The return status is the command's, through the style (`_pick_variant`
/// and `_arguments` both branch on it).
#[test]
fn status_propagates_through_the_style() {
    assert_parity(
        "replrc",
        "curcontext=mycmd\n\
         zstyle ':completion:mycmd:mytag' command 'false'\n\
         _call_program mytag echo BASE >/dev/null\n\
         print -r -- \"rc=$?\"\n",
        "rc=1",
    );
}

/// sh:9-12 — with `-p` and a non-empty `$_comp_priv_prefix`, a true
/// `gain-privileges` prepends the whole prefix to the command.
///
/// The context it is looked up under is sh:10's, not the caller's: the last
/// `:`-component of `$curcontext` is replaced by `/<command word of the
/// prefix>`, and sh:10 leaves a trailing `:` that sh:26 then doubles. The
/// prefix here is `( FOO=bar echo PRIV )`, so sh:10's `:#^*[^\\]=*` skips the
/// assignment and picks `echo`, giving `:completion:mycmd/echo::mytag`.
#[test]
fn dash_p_prepends_the_privilege_prefix() {
    assert_parity(
        "pgain",
        "curcontext='mycmd:zzz'\n\
         typeset -ga _comp_priv_prefix\n\
         _comp_priv_prefix=( FOO=bar echo PRIV )\n\
         zstyle ':completion:mycmd/echo::mytag' gain-privileges yes\n\
         print -r -- \"$(_call_program -p mytag echo ARG)\"\n",
        "PRIV echo ARG",
    );
}

/// sh:11 — the prefix is applied only when `gain-privileges` is TRUE for
/// that context. Unset means no prefix, so `-p` is inert by default and a
/// completer cannot silently acquire `sudo`.
#[test]
fn dash_p_without_gain_privileges_adds_nothing() {
    assert_parity(
        "pnogain",
        "curcontext='mycmd:zzz'\n\
         typeset -ga _comp_priv_prefix\n\
         _comp_priv_prefix=( FOO=bar echo PRIV )\n\
         print -r -- \"$(_call_program -p mytag echo ARG)\"\n",
        "ARG",
    );
}

/// sh:10 + sh:26 — the mangled context is used for the `command` style too,
/// which is what lets a user override the command for the privileged case
/// alone. A port that computes the prefix but keeps the caller's context
/// passes the previous case and fails this one.
#[test]
fn dash_p_command_style_uses_the_mangled_context() {
    assert_parity(
        "pctx",
        "curcontext='mycmd:zzz'\n\
         typeset -ga _comp_priv_prefix\n\
         _comp_priv_prefix=( FOO=bar echo PRIV )\n\
         zstyle ':completion:mycmd/echo::mytag' command 'echo STYLED'\n\
         print -r -- \"$(_call_program -p mytag echo ARG)\"\n",
        "STYLED",
    );
}

/// sh:28's missing `$prefix` — the documented override. With
/// `gain-privileges` on, a `command` style that starts with `-` still wins
/// and the privilege prefix is NOT added, so a user can force their own
/// escalation command.
#[test]
fn leading_dash_style_overrides_the_privilege_prefix() {
    assert_parity(
        "poverride",
        "curcontext='mycmd:zzz'\n\
         typeset -ga _comp_priv_prefix\n\
         _comp_priv_prefix=( FOO=bar echo PRIV )\n\
         zstyle ':completion:mycmd/echo::mytag' gain-privileges yes\n\
         zstyle ':completion:mycmd/echo::mytag' command '-print -r -- OVERRIDE'\n\
         print -r -- \"$(_call_program -p mytag echo ARG)\"\n",
        "OVERRIDE echo ARG",
    );
}
