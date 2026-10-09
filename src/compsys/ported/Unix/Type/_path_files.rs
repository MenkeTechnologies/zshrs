//! Port of `_path_files` from `Completion/Unix/Type/_path_files`
//! (upstream 895 lines). This is a faithful translation of the shell
//! source: it mirrors the shell's local variable names and control
//! flow and drives the ported C builtins `compfiles`, `compadd`,
//! `compset` and `compquote` instead of reimplementing file
//! generation.
//!
//! Shell → Rust local mapping (names kept from the source):
//!   linepath realpath donepath prepath testpath exppath skips skipped
//!   tmp1 tmp2 tmp3 tmp4 i orig eorig pre suf tpre tsuf opre osuf cpre
//!   pats haspats ignore pfx pfxsfx sopt gopt sdirs ignpar cfopt listsfx
//!   nm menu matcher mopts sort mid accex fake Uopt accept_exact_dirs
//!   path_completion npathcheck Mopts prepaths exppaths
//! Because Rust is typed, where the shell reuses one name for both a
//! scalar and an array we keep the base name for the dominant use and
//! suffix the other (e.g. `tmp1` = the match array, `tmp1s` = scalar
//! `tmp1`, `tmp2` = array, `tmp2s` = scalar `tmp2`).
//!
//! `compfiles` subcommands driven (via `bin_compfiles`):
//!   * `-p$cfopt` / `-P$cfopt`  — cf_pats file generation (sh:463-470)
//!   * `-i`                     — cf_ignore ignore-parents  (sh:580)
//!   * `-r`                     — cf_remove_other ambiguity  (sh:634)
//! The transient parameters `tmp1`, `accex`, `fake`, `ignore` and
//! `_comp_ignore` are materialised in `paramtab` around each builtin
//! call (compfiles/compadd/compquote read/write params by name) and
//! read back into the corresponding Rust locals.
//!
//! The parameter-expansion idioms go through the ported engine: `(z)` via
//! [`split_z`] (the real lexer), `(b)`/`(q)` via `quotestring`, `(Q)` via
//! [`dequote_q`], the `(e)` eval of a parameter-expansion prefix through
//! `execute_script`, and every `[[ … = pattern ]]` test through `matchpat`
//! with the upstream pattern text.
//! `compfiles -p$cfopt` emits the shell's exact option token (`-p` or
//! `-p-`), matching C's accepted forms (computil.c:5011-5015).

use crate::compsys::ported::shared::{PM_ARRAY, PM_UNIQUE};
use crate::compsys::ported::shared::{dequote_q, dispatch_action_command, split_z};
use crate::ported::glob::matchpat;
use crate::ported::utils::quotestring;
use crate::ported::zsh_h::{QT_BACKSLASH, QT_BACKSLASH_PATTERN};
use crate::ported::glob::{shtokenize, tokenize, zglob};
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, gethkparam, gethparam, getsparam, setaparam, setsparam};
use crate::ported::subst::{filesubstr, singsub};
use crate::ported::zle::compcore::get_compstate_str;
use crate::ported::zle::complete::{bin_compadd, bin_compadd_body, bin_compset};
use crate::ported::zle::computil::{bin_compfiles, bin_compquote};
use crate::ported::zsh_h::{isset, options, CASEGLOB, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

// ---- small helpers -------------------------------------------------

fn compadd(argv: Vec<String>) -> i32 {
    bin_compadd("compadd", &argv, &make_ops(), 0)
}
/// `builtin compadd …` — the real builtin with shell-function lookup
/// bypassed (`Src/exec.c:3402-3406` gates the `shfunctab` lookup on
/// `!(cflags & BINF_BUILTIN)`), so the `compadd()` function that
/// `_approximate` / `_correct` install never sees the call.
fn compadd_builtin(argv: Vec<String>) -> i32 {
    bin_compadd_body("compadd", &argv, &make_ops(), 0)
}
fn compset(argv: Vec<String>) -> i32 {
    bin_compset("compset", &argv, &make_ops(), 0)
}
/// `compquote [-p] name...` — sync each named local into paramtab is
/// the caller's job; this just fires the builtin.
fn compquote(argv: Vec<String>) {
    // The C builtin is `BUILTIN("compquote", 0, bin_compquote, 1, -1, 0,
    // "p", NULL)`: execbuiltin parses the leading `-p` into `ops` and hands
    // bin_compquote only the parameter names. Calling bin_compquote directly
    // bypasses that parser, so a raw `["-p","tmp1"]` made bin_compquote treat
    // `-p` as a PARAMETER NAME and try to quote `$-` → "read-only variable: -"
    // aborting every `foo ../<TAB>` path completion. Replicate the option
    // parse here: leading `-<flags>` tokens set ops.ind, the rest are names.
    let mut ops = make_ops();
    let mut names: Vec<String> = Vec::with_capacity(argv.len());
    let mut opts_done = false;
    for a in argv {
        if !opts_done && a.len() > 1 && a.starts_with('-') {
            for ch in &a.as_bytes()[1..] {
                ops.ind[*ch as usize] = 1;
            }
        } else {
            opts_done = true;
            names.push(a);
        }
    }
    bin_compquote("compquote", &names, &ops, 0);
}
fn compfiles(argv: Vec<String>) -> i32 {
    bin_compfiles("compfiles", &argv, &make_ops(), 0)
}

/// Dispatch a compsys function BY NAME from upstream line `line`.
///
/// Every upstream site this stands in for writes a plain COMMAND WORD, so
/// `shared::dispatch_action_command` (shared.rs:1407) — `execcmd`'s own
/// resolution: shfunc/port/plugin (c:Src/exec.c:3105-3109), then builtin, then
/// `$PATH`, then c:903's `command not found` with c:908's 127 — is what they
/// mean. The `.unwrap_or(1)` this replaces had NO not-found arm, so a name the
/// shell DIAGNOSES (`_list_files` and `_description` are ordinary `$fpath`
/// functions a user can shadow) produced no output at all.
fn dispatch0(name: &str, args: &[String], line: u64) -> i32 {
    crate::compsys::ported::shared::dispatch_action_command(name, args, line)
}

fn get_arr(name: &str) -> Vec<String> {
    getaparam(name).unwrap_or_default()
}
fn get_str(name: &str) -> String {
    getsparam(name).unwrap_or_default()
}

fn cs_i(key: &str) -> i64 {
    get_compstate_str(key)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
fn cs_s(key: &str) -> String {
    get_compstate_str(key).unwrap_or_default()
}

/// `zstyle -s ctx style name [sep]` — Src/Modules/zutil.c:643-658:
///   `if ((vals = lookupstyle(args[1], args[2])) && vals[0]) {`
///   `    ret = sepjoin(vals, (args[4] ? args[4] : " "), 0); val = 0; }`
///   `else { ret = ztrdup(""); val = 1; }`
/// so ALL values are joined with `sep` (default a single space), not
/// just the first one; `None` mirrors the `val = 1` (style unset) arm.
/// A style set to one empty string is still a hit (`vals[0]` is a valid
/// pointer), which `Some(String::new())` reproduces.
/// No `_path_files` call site passes the optional `sep` argument, so the
/// separator is fixed at the C default `" "`.
fn zstyle_s(ctx: &str, style: &str) -> Option<String> {
    let vals = lookupstyle(ctx, style);
    if vals.is_empty() {
        None
    } else {
        Some(crate::ported::utils::sepjoin(&vals, Some(" ")))
    }
}
/// zstyle -a: all values.
fn zstyle_a(ctx: &str, style: &str) -> Vec<String> {
    lookupstyle(ctx, style)
}
/// zstyle -t: present and true-ish.
fn zstyle_t(ctx: &str, style: &str) -> bool {
    match lookupstyle(ctx, style).first() {
        Some(w) => matches!(w.as_str(), "yes" | "true" | "on" | "1"),
        None => false,
    }
}
/// `zstyle -t ctx style word...` — Src/Modules/zutil.c:707-717:
///   `if (args[3]) { … while (*p) if (!strcmp(*ap, *p++)) return 0; … return 1; }`
/// With value words given, the boolean spelling is NOT consulted at all:
/// the test is "does any listed word appear among the style's values".
/// `_path_files` uses this for `expand suffix` (sh:681) and
/// `expand prefix` (sh:887) — reading those as a plain boolean made the
/// documented `prefix`/`suffix` values of `expand` inert.
fn zstyle_t_word(ctx: &str, style: &str, words: &[&str]) -> bool {
    let vals = lookupstyle(ctx, style);
    words.iter().any(|w| vals.iter().any(|v| v == w))
}
/// zstyle -T: default-true (true unless explicitly false-ish).
fn zstyle_t_default(ctx: &str, style: &str) -> bool {
    match lookupstyle(ctx, style).first() {
        Some(w) => !matches!(w.as_str(), "no" | "false" | "off" | "0"),
        None => true,
    }
}

/// Flat-assoc lookup for `_comp_caller_options[key]` style access.
fn assoc_get(name: &str, key: &str) -> Option<String> {
    // `_comp_caller_options` is PM_HASHED (`_main_complete` publishes it
    // with `sethparam`); `getaparam` returns None for anything that is not
    // PM_ARRAY, so the hash must be read through gethkparam/gethparam
    // (c:params.c:3117/3131). Flat key/value arrays remain supported.
    let keys = gethkparam(name).unwrap_or_default();
    if !keys.is_empty() {
        let vals = gethparam(name).unwrap_or_default();
        return keys
            .iter()
            .position(|k| k == key)
            .and_then(|i| vals.get(i).cloned());
    }
    get_arr(name)
        .chunks(2)
        .find(|kv| kv.first().map(|k| k == key).unwrap_or(false))
        .and_then(|kv| kv.get(1).cloned())
}

/// `${(b)s}` — backslash-quote pattern metacharacters so `s` matches
/// literally when used as a pattern (`QT_BACKSLASH_PATTERN`, c:Src/subst.c
/// `case 'b'`).
fn quote_b(s: &str) -> String {
    quotestring(s, QT_BACKSLASH_PATTERN)
}

/// `[[ s = (|*[^\\])[][*?#~^\|\<\>]* ]]` — a pattern metacharacter that is
/// not preceded by a backslash, or one at the very start.
fn has_active_glob(s: &str) -> bool {
    let mut prev: Option<char> = None;
    for c in s.chars() {
        if matches!(
            c,
            ']' | '[' | '*' | '?' | '#' | '~' | '^' | '|' | '<' | '>'
        ) && prev != Some('\\')
        {
            return true;
        }
        prev = Some(c);
    }
    false
}

/// `${(M)tpre##${~skips}}` — longest leading run of `./`, `../` (and,
/// when squeeze, bare `/`) components. Returns that prefix.
fn match_skips_prefix(s: &str, squeeze: bool) -> String {
    let b = s.as_bytes();
    let mut i = 0;
    loop {
        if b[i..].starts_with(b"./") {
            i += 2;
        } else if b[i..].starts_with(b"../") {
            i += 3;
        } else if squeeze && b.get(i) == Some(&b'/') {
            i += 1;
        } else {
            break;
        }
    }
    s[..i].to_string()
}

/// `tmp1=( $~tmp1 )` — tokenise + glob-expand each element.
///
/// `$~` forces GLOBSUBST (c:Src/subst.c:2373 `globsubst = 2`), so each element
/// is `shtokenize`d as it is copied out (`strcatsub`, c:823/830) and the word then
/// goes through `globlist` → `zglob` (c:Src/glob.c:1214) like any other
/// command-line word. `zglob` owns the no-match arm (c:1872-1888): the
/// word is DROPPED under NULL_GLOB or the `(N)` qualifier, and otherwise,
/// with NOMATCH set, it is `zerr("no matches found: %s")` — which sets
/// errflag, and that is what unwinds `_path_files` (see the errflag check
/// at the sh:472 call site). `_main_complete`'s `$_comp_options` turn
/// NULL_GLOB on, so the normal completion path drops the word silently;
/// a completion widget whose function is the completer itself
/// (`zle -C`, `compdef -k`) runs with the user's options, gets NOMATCH,
/// and in zsh the completion stops there. Routing through the bare
/// `glob_path` match list had no no-match arm at all, so that stop
/// never happened and `_files` went on trying its remaining patterns.
fn tilde_glob(pats: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for p in pats {
        let mut word = p.clone();
        shtokenize(&mut word);
        let mut list = vec![word];
        zglob(&mut list, 0, 0);
        out.extend(list);
    }
    out
}

/// basename of each element (`${(@)arr:t}`).
fn tails(arr: &[String]) -> Vec<String> {
    arr.iter()
        .map(|s| {
            let t = s.trim_end_matches('/');
            match t.rfind('/') {
                Some(i) => t[i + 1..].to_string(),
                None => t.to_string(),
            }
        })
        .collect()
}

// ---- zparseopts ----------------------------------------------------

/// Result of the sh:59-62 `zparseopts -a mopts ...` parse. Each field
/// is the array the shell binds via `=name`; `mopts` is the `-a`
/// default (everything without an explicit `=name`).
#[derive(Default, Debug)]
pub struct Parsed {
    pub mopts: Vec<String>,    // -a mopts (J V x X 1 2 o n)
    pub pfx: Vec<String>,      // P:=pfx
    pub pfxsfx: Vec<String>,   // S: q r: R: => pfxsfx
    pub prepaths: Vec<String>, // W:=prepaths
    pub ignore: Vec<String>,   // F:=ignore
    pub matcher: Vec<String>,  // M+:=matcher
    pub tmp1: Vec<String>,     // f= /= g+:-= tmp1
}

// (takes_arg, dest, concat) for each single-char option.
enum Dest {
    Mopts,
    Pfx,
    Pfxsfx,
    Prepaths,
    Ignore,
    Matcher,
    Tmp1,
}

fn opt_spec(c: u8) -> Option<(bool, Dest, bool)> {
    // concat=true only for `g` (the `:-` ZOF_SAME form).
    Some(match c {
        b'P' => (true, Dest::Pfx, false),
        b'S' => (true, Dest::Pfxsfx, false),
        b'q' => (false, Dest::Pfxsfx, false),
        b'r' => (true, Dest::Pfxsfx, false),
        b'R' => (true, Dest::Pfxsfx, false),
        b'W' => (true, Dest::Prepaths, false),
        b'F' => (true, Dest::Ignore, false),
        b'M' => (true, Dest::Matcher, false),
        b'J' | b'V' | b'x' | b'X' | b'o' => (true, Dest::Mopts, false),
        b'1' | b'2' | b'n' => (false, Dest::Mopts, false),
        b'f' | b'/' => (false, Dest::Tmp1, false),
        b'g' => (true, Dest::Tmp1, true),
        _ => return None,
    })
}

/// Faithful port of the sh:59-62 `zparseopts` invocation for this exact
/// spec. Follows the zsh short-option scan (`bin_zparseopts`,
/// `add_opt_val`): options bundle; a value can be attached or taken
/// from the next argv; `g` stores option+value concatenated
/// (ZOF_SAME), every other value-taking option stores option and value
/// as two array elements. Parsing stops at the first non-option, `-`
/// or `--`.
pub fn zparse_pathfiles(args: &[String]) -> Parsed {
    let mut p = Parsed::default();
    let mut i = 0;
    while i < args.len() {
        let tok = &args[i];
        // Not an option / bare `-` / `--` ends the parse.
        if !tok.starts_with('-') || tok == "-" {
            break;
        }
        if tok == "--" {
            i += 1;
            break;
        }
        let rest = &tok[1..];
        let rb = rest.as_bytes();
        let mut j = 0;
        let mut consumed_next = false;
        while j < rb.len() {
            let c = rb[j];
            let Some((takes_arg, dest, concat)) = opt_spec(c) else {
                // bad option — stop (zparseopts default aborts; we halt).
                j = rb.len();
                break;
            };
            let optname = format!("-{}", c as char);
            let push = |p: &mut Parsed, dest: &Dest, vals: Vec<String>| {
                let d = match dest {
                    Dest::Mopts => &mut p.mopts,
                    Dest::Pfx => &mut p.pfx,
                    Dest::Pfxsfx => &mut p.pfxsfx,
                    Dest::Prepaths => &mut p.prepaths,
                    Dest::Ignore => &mut p.ignore,
                    Dest::Matcher => &mut p.matcher,
                    Dest::Tmp1 => &mut p.tmp1,
                };
                d.extend(vals);
            };
            if takes_arg {
                let value = if j + 1 < rb.len() {
                    let v = rest[j + 1..].to_string();
                    j = rb.len();
                    v
                } else if i + 1 < args.len() {
                    consumed_next = true;
                    args[i + 1].clone()
                } else {
                    // missing mandatory arg — bind empty.
                    String::new()
                };
                if concat {
                    push(&mut p, &dest, vec![format!("{}{}", optname, value)]);
                } else {
                    push(&mut p, &dest, vec![optname, value]);
                }
                break;
            } else {
                push(&mut p, &dest, vec![optname]);
                j += 1;
            }
        }
        i += 1;
        if consumed_next {
            i += 1;
        }
    }
    p
}

// ---- main ----------------------------------------------------------

/// Reach `_path_files` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_path_files -/ -g '*(-*)' -P / -W /` (Completion/Unix/Type/_absolute_command_paths sh:22) — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_path_files_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _path_files(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_path_files", args, || _path_files_impl(args))
}

/// `_path_files` — file/directory completion entry point.
pub fn _path_files_impl(argv: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_path_files");
    // sh:3 — match/mbegin/mend are populated by _have_glob_qual.
    let curcontext = get_str("curcontext");
    let ctx = format!(":completion:{}:", curcontext);
    let paths_ctx = format!(":completion:{}:paths", curcontext);

    // sh:5-8 — file-split-chars.
    if let Some(splitchars) = zstyle_s(&ctx, "file-split-chars") {
        // sh:7 `compset -P "*[${(q)splitchars}]"`
        compset(vec![
            "-P".into(),
            format!("*[{}]", quotestring(&splitchars, QT_BACKSLASH)),
        ]);
    }

    // sh:22-39 — glob-qualifier dispatch.
    let prefix = get_str("PREFIX");
    // sh:22 `_have_glob_qual …` is a COMMAND WORD like any other, so
    // route it through `dispatch_action_command` (shared.rs:1407):
    // shfunc/port/plugin (c:Src/exec.c:3105-3109), then builtin, then
    // `$PATH`, then c:903's `command not found` with c:908's 127. The
    // branch is taken on 0 either way, so only the SWALLOWED diagnostic
    // changes — `== Some(0)` turned a name the shell diagnoses into a
    // silent false.
    if dispatch_action_command("_have_glob_qual", &[prefix.clone()], 22) == 0 {
        let mut ret = 1;
        let mtch = get_arr("match");
        let m1len = mtch.first().map(|s| s.chars().count()).unwrap_or(0);
        compset(vec!["-p".into(), m1len.to_string()]);
        compset(vec!["-S".into(), r"[^\)\|\~]#(|\))".into()]);
        let eg_on = assoc_get("_comp_caller_options", "extendedglob").as_deref() == Some("on");
        if eg_on && compset(vec!["-P".into(), r"\#".into()]) == 0 {
            if dispatch0("_globflags", &[], 27) == 0 {
                ret = 0;
            }
        } else {
            if eg_on {
                // sh:30 — `local -a flags`. The port hands `flags` to
                // `_describe` BY NAME, so it has to exist in paramtab; without
                // the declaration it would be born at level 0 and outlive the
                // call.
                let _flags_scope =
                    crate::compsys::ported::shared::LocalScope::declare(&["flags"], PM_ARRAY);
                // sh:31-34 — flags=( '#:introduce glob flag' ); _describe...
                setaparam("flags", vec!["#:introduce glob flag".into()]);
                if dispatch0(
                    "_describe",
                    &[
                        "-t".into(),
                        "globflags".into(),
                        "glob flag".into(),
                        "flags".into(),
                        "-Q".into(),
                        "-S".into(),
                        "".into(),
                    ],
                    34,
                ) == 0
                {
                    ret = 0;
                }
            }
            if dispatch0("_globquals", &[], 36) == 0 {
                ret = 0;
            }
        }
        return ret;
    }

    // sh:44-53 — the function's `local` block.
    //
    // Only the names this port round-trips through `paramtab` need a real
    // declaration; everything else upstream lists (`linepath`, `realpath`,
    // `pre`, `suf`, …) is a Rust local here and never reaches the parameter
    // table. The ones below DO reach it, because the C builtins this port
    // drives — `compfiles` (computil.c:4998), `compquote`, `compadd` — and
    // the `_describe` port take their operands BY NAME.
    //
    // Without the declaration `setaparam` created them at level 0, so they
    // survived the completion: after `- <TAB><TAB>` the shell was left holding
    // global arrays `accex`, `fake`, `tmp1` and `tmp2`, and the user's
    // `_parameters` (~/.zpwr/autoload/comp_utils/_parameters:34) — which keeps
    // every name whose `$parameters` type string does NOT contain `local` —
    // then offered all four in the `parameters` group. `ls <TAB>` leaked
    // `accex` the same way. See shared::declare_locals for the general case.
    //
    // The list below is CLOSED, and re-measured: the scan that produced
    // `5bbd9c2e11` (`_groups`) and `ddadab02f3` (nine more) re-flags this file
    // on every pass because it matches sh:45/48 `local` names against
    // `set[as]param` call sites and cannot see this declaration. There is
    // nothing left for it to find. Every name this port hands to
    // `set[as]param` is either declared here — `tmp1` `tmp2` `tmp4` `i`
    // `tmpdisp` `ignore` `accex` `fake` `exppaths` `expl`, plus `listfiles`
    // `listopts` written by the `_list_files` callee and `flags` scoped to the
    // sh:30 branch above — or deliberately caller-visible: `PREFIX`/`SUFFIX`
    // (the compsys specials the whole function drives) and `_comp_ignore`,
    // which sh:141/581 append to on purpose, in `_main_complete`'s scope
    // (`Base/Core/_main_complete:54 typeset -U … _comp_ignore`). The rest of
    // sh:44-53 — `linepath` `realpath` `donepath` `prepath` `testpath`
    // `exppath` `skips` `skipped` `tmp3` `orig` `eorig` `pre` `suf` `tpre`
    // `tsuf` `opre` `osuf` `cpre` `pats` `haspats` `pfx` `pfxsfx` `sopt`
    // `gopt` `opt` `sdirs` `ignpar` `cfopt` `listsfx` `nm` `menu` `matcher`
    // `mopts` `sort` `mid` `origtmp1` `Uopt` `accept_exact_dirs`
    // `path_completion` `npathcheck` `Mopts` `prepaths` — never reaches
    // `paramtab` at all, so declaring any of them would only create and unwind
    // a shadow with no counterpart in this port's execution.
    //
    // Measured through a pty, both shells `-f -i` on one generated init
    // (`fpath=( $fpath )`, `compinit -C -d <pinned zpwr dump>`), snapshotting
    // `${(ok)parameters}` to a file before and after the completion and
    // diffing the names each shell GAINED, over 21 shapes: `ls `, `ls /usr/`,
    // `cat /etc/pas`, `cd /`, `chmod `, `find -`, `cp `, `ls ~/`,
    // `cat /usr/share/../`, `ls //usr/`, `cd /usr/lo`, `ls /u/l/b`, `ls *(`,
    // `ls *(-`, `ls **/`, plus five bare wrappers that call `_path_files`
    // with NO caller locals of their own (`_path_files`, `-W /usr`, `-g '*.h'`,
    // `-/`, and one under `zstyle … file-list all`). Zero zshrs-only names on
    // every one. The mirror-image probe — seed all 21 names with a sentinel at
    // TOP level, complete, read them back — is also 21/21 identical to zsh, so
    // the declaration restores as well as it hides.
    //
    // `LocalScope`, not a bare `declare_locals`, and that is deliberate here
    // even though `declare_locals` is the default for a port whose only entry
    // is through `doshfunc`. `_path_files_impl` has a DIRECT Rust caller that
    // never runs `endparamscope`: the `_path_files` wrapper's own fallback,
    // taken whenever `dispatch_function_call` finds no executor. That is the
    // path `empty_line_returns_one` (bottom of this file) takes, at
    // `locallevel == 0`, where `declare_locals` is a documented no-op — so
    // with a bare declaration that test would strand `tmp1`/`accex`/`fake`/
    // `exppaths` in the process-wide `paramtab` for every later test sharing
    // `test_util::global_state_lock`. `LocalScope`'s unconditional restore is
    // what covers that, and it is safe to use here for the reason it was NOT
    // safe in `_description`: nothing this port hands back to a Rust caller is
    // in this list (matches go out through `compadd`, and `PREFIX`/`SUFFIX`/
    // `_comp_ignore` are excluded above).
    let mut _locals = crate::compsys::ported::shared::LocalScope::declare(
        // sh:45 `local tmp1 tmp2 tmp3 tmp4 i …`, sh:48 `local … tmpdisp …`.
        // Declared PM_ARRAY rather than as bare scalars because the paramtab
        // copy only ever carries the ARRAY use of each name (the scalar uses
        // are the Rust locals `tmp1s`/`tmp2s`/`tmp4s`), so the type never has
        // to change under `setaparam`.
        &["tmp1", "tmp2", "tmp4", "i", "tmpdisp"],
        PM_ARRAY,
    );
    _locals.also(&["ignore"], PM_ARRAY); // sh:46
    _locals.also(&["accex", "fake"], PM_ARRAY); // sh:47
                                                // sh:48 `local listfiles listopts tmpdisp origtmp1 Uopt` — `_list_files`
                                                // (Unix/Type/_list_files:15-16) assigns both without `local` and relies on
                                                // this declaration. Without it `setaparam` created them at level 0 and they
                                                // survived the completion, so `_parameters` — which keeps every name whose
                                                // `$parameters` type string does NOT contain `local` — offered `listfiles`
                                                // and `listopts` in the `parameters` group, two matches zsh never lists.
    _locals.also(&["listfiles", "listopts"], PM_ARRAY);
    _locals.also(&["exppaths"], PM_ARRAY | PM_UNIQUE); // sh:53 `typeset -U … exppaths`
                                                       // sh:116 `local expl` — declared inside the `if (( ! $mopts[(I)-[JVX]] ))`
                                                       // block, handed to `_description` at sh:119/121 and folded into `mopts`
                                                       // at sh:131. Upstream's `local` SAVES and RESTORES it, so a caller's own
                                                       // `expl` is untouched by the call; the port wrote it straight into the
                                                       // global table and handed the callee's value back. The stock-utility
                                                       // sweep read `expl[2] = '-J' '-default-'` after `_path_files` where zsh
                                                       // reads `expl[0] =`, on all four `_path_files` cells.
                                                       //
                                                       // Declared for the whole body rather than for the sh:115-132 block: the
                                                       // window is invisible from outside, and `LocalScope` unwinds on return
                                                       // either way.
    _locals.also(&["expl"], PM_ARRAY); // sh:116

    // sh:59-62 — option parse.
    let parsed = zparse_pathfiles(argv);
    let mut mopts = parsed.mopts;
    let pfx = parsed.pfx;
    let mut pfxsfx = parsed.pfxsfx;
    let mut prepaths = parsed.prepaths;
    let mut ignore = parsed.ignore;
    let mut matcher = parsed.matcher;
    let topt = parsed.tmp1; // sh `tmp1` (the -f/-/-g flag array)

    // sh:64 — sopt = "-" + first char of each topt element.
    let mut sopt: Option<String> = {
        let mut s = String::from("-");
        for e in &topt {
            let stripped = e.strip_prefix('-').unwrap_or(e);
            if let Some(c) = stripped.chars().next() {
                s.push(c);
            }
        }
        Some(s)
    };
    // sh:65-66
    let haspats_flags = topt
        .iter()
        .any(|e| e.starts_with("-/") || e.starts_with("-g"));
    let gopt = topt.iter().any(|e| e.starts_with("-g"));

    // sh:67-74 — build pats.
    let g_pats: Vec<String> = topt
        .iter()
        .filter(|e| e.starts_with("-g"))
        .map(|e| e[2..].to_string())
        .collect();
    let mut pats: Vec<String> = {
        // sh:69/72 `pats=( ${${(z):-x $pats}[2,-1]} )` — the joined -g
        // patterns, split by the shell lexer with the sentinel word dropped.
        let split: Vec<String> = split_z(&format!("x {}", g_pats.join(" ")))
            .into_iter()
            .skip(1)
            .collect();
        if topt.iter().any(|e| e == "-/") {
            let mut v = vec!["*(-/)".to_string()];
            v.extend(split);
            v
        } else {
            split
        }
    };
    // sh:74 — drop empty/blank elements.
    pats.retain(|p| !p.trim().is_empty());
    let haspats = haspats_flags;

    // sh:76-78 — leading literal prefix.
    if !pfx.is_empty() {
        let pfx2 = pfx.get(1).cloned().unwrap_or_default();
        if compset(vec!["-P".into(), quote_b(&pfx2)]) != 0 {
            let mut np = pfx.clone();
            np.extend(pfxsfx.clone());
            pfxsfx = np;
        }
    }

    // ZLE-special scoping (c:Src/Zle/complete.c:1307-1317 `addcompparams`,
    // c:Src/Zle/zle_params.c:200-206 `makezleparams`): PREFIX / SUFFIX /
    // IPREFIX / ISUFFIX are created `PM_SPECIAL|PM_REMOVABLE|PM_LOCAL` with
    // `pm->level = locallevel + 1`, i.e. they belong to the scope of the
    // TOP completion function. Any deeper shell function that ASSIGNS one
    // gets a local shadow (c:Src/params.c:1130 — `oldpm->level ==
    // locallevel` fails, so a new param with `pm->old = oldpm` is pushed),
    // and popping that scope restores the old value through the special's
    // gsu setter. Net effect upstream: `PREFIX=...` inside `_path_files` is
    // visible to the builtins it calls but is UNDONE for its caller. That is
    // why upstream _path_files never restores PREFIX explicitly and still
    // leaves it untouched (measured: a probe completer around
    // `_path_files -/` for `foo /sr` reads `/sr` before AND after in zsh).
    //
    // The port runs ported compsys functions as plain Rust fns against one
    // global parameter store, so the assignments below leaked out: after the
    // first (empty) matcher-list pass, PREFIX was left as `sr` instead of
    // `/sr`, so the second pass completed relative to $PWD and `cd /sr<TAB>`
    // answered `src/` instead of `/usr`. Snapshot here — AFTER sh:7/24-26/77
    // `compset`, which writes compprefix/compiprefix through the BUILTIN and
    // therefore is NOT scoped — and restore at the single exit below.
    let entry_prefix = get_str("PREFIX");
    let entry_suffix = get_str("SUFFIX");

    // sh:80-93 — resolve -W into prepaths (`typeset -U prepaths`).
    if !prepaths.is_empty() {
        // `${x%/}/` — drop ONE trailing slash, then append one.
        let slashed = |w: &str| format!("{}/", w.strip_suffix('/').unwrap_or(w));
        let tmp1s = prepaths.get(1).cloned().unwrap_or_default();
        if tmp1s.starts_with('(') {
            // sh:83 `prepaths=( ${^=tmp1[2,-2]%/}/ )`
            let inner: String = tmp1s
                .chars()
                .skip(1)
                .take(tmp1s.chars().count().saturating_sub(2))
                .collect();
            prepaths = inner.split_whitespace().map(slashed).collect();
        } else if tmp1s.starts_with('/') {
            // sh:85 `prepaths=( "${tmp1%/}/" )`
            prepaths = vec![slashed(&tmp1s)];
        } else {
            // sh:87 `prepaths=( ${(P)^tmp1%/}/ )` — an unset target still
            // yields one empty word, hence `/`; only an empty ARRAY yields
            // none and falls through to sh:88.
            let vals = getaparam(&tmp1s)
                .unwrap_or_else(|| vec![getsparam(&tmp1s).unwrap_or_default()]);
            prepaths = vals.iter().map(|v| slashed(v)).collect();
            if prepaths.is_empty() {
                prepaths = vec![slashed(&tmp1s)]; // sh:88
            }
        }
        prepaths = dedup(prepaths);
        if prepaths.is_empty() {
            prepaths = vec![String::new()]; // sh:90
        }
    } else {
        prepaths = vec![String::new()];
    }

    // sh:95-101 — resolve -F ignore.
    if !ignore.is_empty() {
        let ig2 = ignore.get(1).cloned().unwrap_or_default();
        if ig2.starts_with('(') {
            ignore = ig2[1..ig2.len().saturating_sub(1)]
                .split_whitespace()
                .map(String::from)
                .collect();
        } else {
            // sh:99 `ignore=( ${(P)ignore[2]} )` — an empty scalar vanishes.
            ignore = getaparam(&ig2)
                .or_else(|| getsparam(&ig2).map(|s| vec![s]))
                .unwrap_or_default()
                .into_iter()
                .filter(|v| !v.is_empty())
                .collect();
        }
    }

    // sh:106-113 — default file selection.
    if matches!(sopt.as_deref(), Some("-f") | Some("-")) {
        if !gopt {
            sopt = Some("-f".into());
            pats = vec!["*".into()];
        } else {
            sopt = None; // unset sopt
        }
    }

    // sh:115-132 — description / matcher from _description.
    let has_jvx = mopts.iter().any(|e| e == "-J" || e == "-V" || e == "-X");
    if !has_jvx {
        if !gopt && sopt.as_deref() == Some("-/") {
            dispatch0(
                "_description",
                &["directories".into(), "expl".into(), "directory".into()],
                119,
            );
        } else {
            dispatch0(
                "_description",
                &["files".into(), "expl".into(), "file".into()],
                121,
            );
        }
        let expl = get_arr("expl");
        // sh:123 — highest index of a -M* element.
        if let Some(pos) = expl.iter().rposition(|e| e.starts_with("-M")) {
            let spec = if expl[pos] == "-M" {
                expl.get(pos + 1).cloned().unwrap_or_default()
            } else {
                expl[pos][2..].to_string()
            };
            if !matcher.is_empty() {
                let m2 = matcher.get(1).cloned().unwrap_or_default();
                if matcher.len() >= 2 {
                    matcher[1] = format!("{} {}", m2, spec);
                } else {
                    matcher = vec!["-M".into(), spec];
                }
            } else {
                matcher = vec!["-M".into(), spec];
            }
        }
        mopts.extend(expl);
    }

    // sh:136-138 — fold $fignore into ignore patterns.
    let fignore = get_arr("fignore");
    let comp_no_ignore = get_str("_comp_no_ignore");
    let fignore_env = get_str("FIGNORE");
    // sh:137 `"$pats" = \ #\*\ #` — spaces around a lone `*`.
    let pats_is_star = pats.join(" ").trim_matches(' ') == "*";
    if comp_no_ignore.is_empty()
        && ignore.is_empty()
        && (!gopt || pats_is_star)
        && !fignore_env.is_empty()
    {
        ignore = fignore.iter().map(|f| format!("?*{}", f)).collect();
    }

    // sh:140-143 — install ignore into _comp_ignore + mopts -F.
    if !ignore.is_empty() {
        let mut ci = get_arr("_comp_ignore");
        ci.extend(ignore.clone());
        setaparam("_comp_ignore", ci);
        if !mopts.iter().any(|e| e == "-F") {
            mopts.push("-F".into());
            mopts.push("_comp_ignore".into());
        }
    }

    // sh:145-149 — case-insensitive matcher under nocaseglob.
    if matcher.is_empty() && !isset(CASEGLOB) {
        matcher = vec!["-M".into(), "m:{a-zA-Z}={A-Za-z}".into()];
    }

    // sh:151-154 — add matcher to mopts.
    if !matcher.is_empty() {
        mopts.extend(matcher.clone());
    }

    // sh:156-185 — file-sort.
    if let Some(fs) = zstyle_s(&ctx, "file-sort") {
        let mut sort = if fs.contains("size") {
            "oL".to_string()
        } else if fs.contains("links") {
            "ol".to_string()
        } else if fs.contains("time") || fs.contains("date") || fs.contains("modi") {
            "om".to_string()
        } else if fs.contains("access") {
            "oa".to_string()
        } else if fs.contains("inode") || fs.contains("change") {
            "oc".to_string()
        } else {
            "on".to_string()
        };
        if fs.contains("rev") {
            // sort[1]=O — replace first char.
            let mut c: Vec<char> = sort.chars().collect();
            c[0] = 'O';
            sort = c.into_iter().collect();
        }
        if fs.contains("follow") {
            sort = format!("-{}-", sort);
        }
        if sort == "on" {
            sort.clear();
        } else {
            let mut nm = vec!["-o".to_string(), "nosort".to_string()];
            nm.extend(mopts.clone());
            mopts = nm;
            let mut tmp2v = Vec::new();
            for t in &pats {
                // sh:175 — a COMMAND WORD; see the sh:22 note above.
                if dispatch_action_command(
                    "_have_glob_qual",
                    &[t.clone(), "complete".into()],
                    175,
                ) == 0
                {
                    let m = get_arr("match");
                    let m1 = m.first().cloned().unwrap_or_default();
                    let m5 = m.get(4).cloned().unwrap_or_default();
                    tmp2v.push(format!("{}#q{})({})", m1, sort, m5));
                } else {
                    tmp2v.push(format!("{}({})", t, sort));
                }
            }
            pats = tmp2v;
        }
    }

    // sh:191-195 — squeeze-slashes.
    let squeeze = zstyle_t(&paths_ctx, "squeeze-slashes");

    // sh:197-212 — assorted styles.
    let sdirs = zstyle_s(&paths_ctx, "special-dirs").unwrap_or_default();
    let listsfx = zstyle_t(&paths_ctx, "list-suffixes");
    // sh:202 — `sopt=$sopt/` when the joined patterns are a bare `*`, end or
    // start in `*(…)`, or carry a `(…/…)` qualifier. An unset `sopt` becomes `/`.
    if matchpat(
        r"((|*[[:blank:]])\*(|[[:blank:]]*|\([^[:blank:]]##\))|*\([^[:blank:]]#/[^[:blank:]]#\)*)",
        &pats.join(" "),
        true,
        true,
    ) {
        sopt = Some(format!("{}/", sopt.clone().unwrap_or_default()));
    }
    let accex = zstyle_a(&paths_ctx, "accept-exact");
    let fake = zstyle_a(&ctx, "fake-files");
    // sh:207 reads the *unsuffixed* context, not the `:paths` one:
    //   `zstyle -s ":completion:${curcontext}:" ignore-parents ignpar`
    let ignpar = zstyle_s(&ctx, "ignore-parents").unwrap_or_default();
    let accept_exact_dirs = zstyle_t(&paths_ctx, "accept-exact-dirs");
    let path_completion = zstyle_t_default(&paths_ctx, "path-completion");

    // sh:214-237 — copy glob qualifiers from the line into the patterns.
    if !cs_s("pattern_match").is_empty() {
        let suffix0 = get_str("SUFFIX");
        let prefix0 = get_str("PREFIX");
        // sh:215 / sh:216 — two COMMAND WORDS; see the sh:22 note above.
        let hit = (suffix0.is_empty()
            && dispatch_action_command(
                "_have_glob_qual",
                &[prefix0.clone(), "complete".into()],
                215,
            ) == 0)
            || dispatch_action_command(
                "_have_glob_qual",
                &[suffix0.clone(), "complete".into()],
                216,
            ) == 0;
        if hit {
            let m = get_arr("match");
            let tmp3 = m.get(4).cloned().unwrap_or_default(); // match[5]
            if !suffix0.is_empty() {
                setsparam("SUFFIX", &m.get(1).cloned().unwrap_or_default()); // match[2]
            } else {
                setsparam("PREFIX", &m.get(1).cloned().unwrap_or_default());
            }
            let mut tmp2v = Vec::new();
            for t in &pats {
                // sh:227 — a COMMAND WORD; see the sh:22 note above.
                if dispatch_action_command(
                    "_have_glob_qual",
                    &[t.clone(), "complete".into()],
                    227,
                ) == 0
                {
                    let mm = get_arr("match");
                    let m1 = mm.first().cloned().unwrap_or_default();
                    let m5 = mm.get(4).cloned().unwrap_or_default();
                    tmp2v.push(format!("{}{}{})", m1, tmp3, m5));
                } else {
                    tmp2v.push(format!("{}({})", t, tmp3));
                }
            }
            pats = tmp2v;
        }
    }

    // sh:242-247 — snapshot prefix/suffix/orig.
    let mut pre = get_str("PREFIX");
    let mut suf = get_str("SUFFIX");
    let opre = get_str("PREFIX");
    let osuf = get_str("SUFFIX");
    let mut orig = format!("{}{}", pre, suf);
    let eorig = orig.clone();

    // sh:249-257 — menu? correction options?
    let comp_correct = get_str("_comp_correct");
    let insert = cs_s("insert");
    let pattern_match = cs_s("pattern_match");
    let orig_no_tilde = orig.strip_prefix('~').unwrap_or(&orig);
    let menu = insert.ends_with("menu")
        || insert
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        || !comp_correct.is_empty()
        || (!pattern_match.is_empty() && !has_active_glob(orig_no_tilde));
    let _ = menu;
    let mut cfopt = String::new();
    let mut uopt = String::new();
    let mut mopts_r: Vec<String> = Vec::new(); // Mopts
    if !comp_correct.is_empty() {
        cfopt = "-".into();
        uopt = "-U".into();
    } else {
        mopts_r = vec!["-M".into(), "r:|/=* r:|=*".into()];
    }

    // sh:259-359 — split line into linepath + working prefix.
    let mut linepath = String::new();
    let mut realpath = String::new();
    let mut donepath;
    let quote = cs_s("quote");

    // sh:261 — a parameter expansion (or command substitution) in the word
    // from the line, with a slash somewhere after it.
    if quote != "'"
        && matchpat(
            r"[^][*?#^\|\<\>\\]#(\`[^\`]#\`|\$)*/*",
            &pre,
            true,
            true,
        )
    {
        // sh:269 `linepath="${(M)pre##*\$[^/]##/}"` — the LONGEST prefix
        // matching `*$<non-slashes>/`.
        let slashes: Vec<usize> = pre.match_indices('/').map(|(i, _)| i).collect();
        linepath = slashes
            .iter()
            .rev()
            .map(|&i| &pre[..=i])
            .find(|cand| matchpat(r"*\$[^/]##/", cand, true, true))
            .unwrap_or("")
            .to_string();
        // sh:270-274 — `eval 'realpath=${(e)~linepath}' 2>/dev/null` under
        // `setopt localoptions nounset`.
        realpath = eval_e_glob(&linepath);
        if realpath.is_empty() || realpath == linepath {
            return 1; // sh:275
        }
        // sh:276 `pre="${pre#${linepath}}"`
        pre = strip_prefix_literal(&pre, &linepath);
        // sh:277-279 `i="${#linepath//$i}"` with `i='[^/]'` counts the slashes;
        // `orig="${orig[1,(in:i:)/][1,-2]}"`.
        let nslash = linepath.matches('/').count();
        orig = truncate_after_nth_slash(&orig, nslash);
        donepath = String::new();
        prepaths = vec![String::new()];
    } else if pre.starts_with('~') && (quote.is_empty() || quote == "`") {
        // sh:282-327 — ~ prefix.
        let after_tilde = &pre[1..];
        let lp = after_tilde.split('/').next().unwrap_or("").to_string();
        if lp.is_empty() {
            let home = get_str("HOME");
            realpath = format!("{}/", home.trim_end_matches('/'));
        } else if is_numeric_dirstack(&lp) {
            // sh:294-312 — directory stack index.
            let dirstack = get_arr("dirstack");
            let mut tmp1n: i64;
            if !lp.starts_with(['-', '+']) {
                tmp1n = lp.parse().unwrap_or(0);
            } else if lp.starts_with('-') {
                tmp1n = dirstack.len() as i64 + lp.parse::<i64>().unwrap_or(0);
            } else {
                tmp1n = lp[1..].parse().unwrap_or(0);
            }
            if isset(crate::ported::zsh_h::PUSHDMINUS) {
                tmp1n = dirstack.len() as i64 - tmp1n;
            }
            if tmp1n == 0 {
                realpath = format!("{}/", get_str("PWD"));
            } else if tmp1n <= dirstack.len() as i64 {
                // sh:306 `$dirstack[tmp1]/` — a negative subscript counts from the end.
                let at = if tmp1n < 0 {
                    dirstack.len() as i64 + tmp1n
                } else {
                    tmp1n - 1
                };
                realpath = format!(
                    "{}/",
                    usize::try_from(at)
                        .ok()
                        .and_then(|i| dirstack.get(i))
                        .map(String::as_str)
                        .unwrap_or("")
                );
            } else {
                dispatch0("_message", &["not enough directory stack entries".into()], 310);
                return 1;
            }
        } else if lp == "-" || lp == "+" {
            realpath = format!("{}/", expand_tilde(&format!("~{}", lp)).unwrap_or_default());
        } else {
            // sh:316 — eval "realpath=~user/"
            realpath = expand_tilde(&format!("~{}/", lp)).unwrap_or_default();
            if realpath.is_empty() {
                dispatch0("_message", &[format!("unknown user `{}'", lp)], 318);
                return 1;
            }
        }
        linepath = format!("~{}/", lp);
        if realpath == linepath {
            return 1;
        }
        pre = pre.splitn(2, '/').nth(1).unwrap_or("").to_string();
        orig = orig.splitn(2, '/').nth(1).unwrap_or("").to_string();
        donepath = String::new();
        prepaths = vec![String::new()];
    } else {
        // sh:328-358 — no ~ prefix.
        linepath.clear();
        realpath.clear();
        let preserve = zstyle_s(&ctx, "preserve-prefix");
        if let Some(pp) = preserve.filter(|s| !s.is_empty()).and_then(|pp| {
            // pre = (#b)(${~pp})*  — leading match of pp.
            match_leading_pattern(&pre, &pp).map(|m1| m1)
        }) {
            pre = pre[pp.len()..].to_string();
            orig = orig[pp.len().min(orig.len())..].to_string();
            donepath = pp;
            prepaths = vec![String::new()];
        } else if pre.starts_with('/') {
            pre = pre[1..].to_string();
            orig = orig[1..].to_string();
            donepath = "/".into();
            prepaths = vec![String::new()];
        } else {
            if pre.starts_with("./") || pre.starts_with("../") {
                prepaths = vec![String::new()];
            }
            donepath = String::new();
        }
    }

    // sh:361-877 — generate matches, looping over prepaths.
    let mut exppaths: Vec<String> = Vec::new();
    let nm = cs_i("nmatches");
    let skips_squeeze = squeeze;

    // sh:35 `local … mid …` — function-level, so it survives across prepaths.
    let mut mid = String::new();

    for prepath in prepaths.clone() {
        let mut skipped = String::new();
        let mut cpre = String::new();

        // sh:373-410 — accept an exact directory prefix immediately.
        if (accept_exact_dirs || !path_completion) && pre.contains('/') {
            // `${pre} = (#b)(*)/([^/]#)`
            if let Some(cut) = pre.rfind('/') {
                // The file generator strips quotes only from pattern
                // characters, so tmp1/tpre/tmp3 are unquoted copies while
                // tmp2 keeps the line's own spelling (sh:386-396).
                let tmp2 = pre[..cut].to_string();
                let mut tmp1s = unquote_where(&tmp2, |_| true);
                let mut tpre = unquote_where(&pre[cut + 1..], |c| !is_quoted_pattern_char(c));
                let mut tmp3 = unquote_where(&donepath, |_| true);
                loop {
                    let candidate = format!("{}{}{}{}", prepath, realpath, tmp3, tmp2);
                    if !path_completion || is_dir(&candidate) {
                        tmp3 = format!("{}{}/", tmp3, tmp1s);
                        donepath = requote_pattern_chars(&tmp3); // sh:401
                        pre = tpre.clone();
                        break;
                    } else if let Some(cut2) = tmp1s.rfind('/') {
                        // `$tmp1 = (#b)(*)/([^/]#)`
                        tpre = format!("{}/{}", &tmp1s[cut2 + 1..], tpre);
                        tmp1s.truncate(cut2);
                    } else {
                        break;
                    }
                }
            }
        }

        let mut tpre = pre.clone();
        let mut tsuf = suf.clone();
        // sh:421 — testpath is used as a literal string, so the quoting of
        // pattern characters comes off donepath.
        let mut testpath = unquote_where(&donepath, is_quoted_pattern_char);

        // sh:423-426 — strip leading skips.
        let mut tmp2s = match_skips_prefix(&tpre, skips_squeeze);
        tpre = tpre[tmp2s.len()..].to_string();
        let mut tmp1: Vec<String> = vec![format!("{}{}{}{}", prepath, realpath, donepath, tmp2s)];

        let mut npathcheck: i32 = 0;
        let mut hit_continue_outer = false;

        // sh:430-610 — walk path components generating matches.
        loop {
            let origtmp1 = tmp1.clone();

            // sh:435-441 — prefix/suffix for this component.
            if tpre.contains('/') {
                setsparam("PREFIX", tpre.split('/').next().unwrap_or(""));
                setsparam("SUFFIX", "");
            } else {
                setsparam("PREFIX", &tpre);
                setsparam("SUFFIX", tsuf.split('/').next().unwrap_or(""));
            }

            let tmp2: Vec<String> = tmp1.clone(); // sh:452

            let matcher_str = format!(
                "{} {}",
                get_str("_matcher"),
                matcher.get(1).cloned().unwrap_or_default()
            );

            // sh:454-471 — drive compfiles.
            setaparam("tmp1", tmp1.clone());
            setaparam("accex", accex.clone());
            setaparam("fake", fake.clone());
            let concat = format!("{}{}", tpre, tsuf);
            if concat.contains('/') {
                let tail = concat.rsplit('/').next().unwrap_or("");
                let use_sdirs = if !fake.is_empty() || !tail.is_empty() {
                    sdirs.clone()
                } else {
                    String::new()
                };
                compfiles(vec![
                    format!("-P{}", cfopt),
                    "tmp1".into(),
                    "accex".into(),
                    skipped.clone(),
                    matcher_str.clone(),
                    use_sdirs,
                    "fake".into(),
                ]);
            } else if sopt
                .as_deref()
                .map(|s| s.contains('/') || s.contains('f'))
                .unwrap_or(false)
            {
                let mut a = vec![
                    format!("-p{}", cfopt),
                    "tmp1".into(),
                    "accex".into(),
                    skipped.clone(),
                    matcher_str.clone(),
                    sdirs.clone(),
                    "fake".into(),
                ];
                a.extend(pats.clone());
                compfiles(a);
            } else {
                let mut a = vec![
                    format!("-p{}", cfopt),
                    "tmp1".into(),
                    "accex".into(),
                    skipped.clone(),
                    matcher_str.clone(),
                    "".into(),
                    "fake".into(),
                ];
                a.extend(pats.clone());
                compfiles(a);
            }
            // sh:472 — `tmp1=( $~tmp1 ) 2> /dev/null`. The redirection is
            // load-bearing and was dropped: a pattern zsh rejects still raises
            // errflag — that is what unwinds `_files` and stops it trying the
            // remaining `-g` file-patterns — but the DIAGNOSTIC must never
            // reach the terminal mid-completion. `noerrs = 1` is exactly that
            // split (c:Src/utils.c:179-183 — `if (errflag || noerrs) { if
            // (noerrs < 2) errflag |= ERRFLAG_ERROR; return; }`: message
            // suppressed, errflag still set). `noerrs = 2` would wrongly
            // swallow errflag too and defeat the unwind.
            let saved_noerrs = {
                let mut ne = crate::ported::utils::noerrs_lock().lock().unwrap();
                let old = *ne;
                *ne = 1;
                old
            };
            tmp1 = tilde_glob(&get_arr("tmp1"));
            *crate::ported::utils::noerrs_lock().lock().unwrap() = saved_noerrs;
            // c:Src/utils.c:179-184 — `zerr` sets `errflag |= ERRFLAG_ERROR`,
            // and in C every statement after it in the enclosing shell
            // function is skipped: `execlist` bails on `errflag`, so the
            // frame unwinds up through `_files` and the completion stops with
            // whatever it had already added. That unwind is FREE in C because
            // `_path_files` is a shell function; this is a NATIVE port, so
            // nothing propagates it and the remaining `-g` patterns kept
            // being tried against a glob that can no longer succeed. Check it
            // explicitly at the one site that can raise it.
            if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
                return 1;
            }

            let cur_prefix = get_str("PREFIX");
            let cur_suffix = get_str("SUFFIX");
            if !format!("{}{}", cur_prefix, cur_suffix).is_empty() {
                // sh:487-502 — pws non-canonical hack.
                if tmp1.is_empty() && npathcheck == 0 {
                    npathcheck = 1;
                    for tmp3 in &tmp2 {
                        let mut base = tmp3.clone();
                        if !base.is_empty() && !base.ends_with('/') {
                            base.push('/');
                        }
                        let probe =
                            format!("{}{}{}", base, dequote_q(&cur_prefix), dequote_q(&cur_suffix));
                        if path_exists(&probe) {
                            npathcheck = 2;
                        }
                    }
                    if npathcheck == 2 {
                        tmp1 = origtmp1.clone();
                        continue;
                    }
                }

                let tmp2b: Vec<String>;
                if tmp1.is_empty() {
                    // sh:505 — tmp2=( ${^${tmp2:#/}}/$PREFIX$SUFFIX )
                    tmp2b = tmp2
                        .iter()
                        .filter(|e| e.as_str() != "/")
                        .map(|e| format!("{}/{}{}", e, cur_prefix, cur_suffix))
                        .collect();
                } else if tmp1.first().map(|s| s.contains('/')).unwrap_or(false) {
                    // sh:506-518 — reduce to basenames via compadd -D.
                    if !comp_correct.is_empty() {
                        // sh:507-514 — while a correcting completer is
                        // active, narrow EXACTLY first: sh:509 is
                        // `builtin compadd`, which bypasses the
                        // `compadd()` shell function `_approximate`
                        // installs (and therefore its `(#a$N)` PREFIX
                        // injection). Only if that leaves nothing does
                        // sh:513 retry through the shadowed `compadd`
                        // to let approximation widen the set.
                        // sh:508  tmp2=( "$tmp1[@]" )
                        tmp2b = tmp1.clone();
                        setaparam("tmp1", tmp1.clone());
                        // sh:509  builtin compadd -D tmp1 "$matcher[@]" - "${(@)tmp1:t}"
                        let mut a: Vec<String> = vec!["-D".into(), "tmp1".into()];
                        a.extend(matcher.clone());
                        a.push("-".into());
                        a.extend(tails(&tmp1));
                        compadd_builtin(a);
                        tmp1 = get_arr("tmp1");
                        // sh:511  if [[ $#tmp1 -eq 0 ]]
                        if tmp1.is_empty() {
                            // sh:512  tmp1=( "$tmp2[@]" )
                            tmp1 = tmp2b.clone();
                            setaparam("tmp1", tmp1.clone());
                            // sh:513  compadd -D tmp1 "$matcher[@]" - "${(@)tmp2:t}"
                            let mut a2: Vec<String> = vec!["-D".into(), "tmp1".into()];
                            a2.extend(matcher.clone());
                            a2.push("-".into());
                            a2.extend(tails(&tmp2b));
                            compadd(a2);
                            tmp1 = get_arr("tmp1");
                        }
                    } else {
                        // sh:515-518 — no correcting completer active.
                        // sh:516  tmp2=( "$tmp1[@]" )
                        tmp2b = tmp1.clone();
                        setaparam("tmp1", tmp1.clone());
                        // sh:517  compadd -D tmp1 "$matcher[@]" - "${(@)tmp1:t}"
                        let mut a: Vec<String> = vec!["-D".into(), "tmp1".into()];
                        a.extend(matcher.clone());
                        a.push("-".into());
                        a.extend(tails(&tmp1));
                        compadd(a);
                        tmp1 = get_arr("tmp1");
                    }
                } else {
                    // sh:519-522
                    tmp2b = vec![String::new()];
                    setaparam("tmp1", tmp1.clone());
                    let mut a: Vec<String> = vec!["-D".into(), "tmp1".into()];
                    a.extend(matcher.clone());
                    a.push("-a".into());
                    a.push("tmp1".into());
                    compadd(a);
                    tmp1 = get_arr("tmp1");
                }

                // sh:527-544 — no file matched: save expanded path.
                if tmp1.is_empty() {
                    if tmp2b.first().map(|s| s.contains('/')).unwrap_or(false) {
                        let pr = format!("{}{}", prepath, realpath);
                        let mut tt: Vec<String> = tmp2b
                            .iter()
                            .map(|s| s.strip_prefix(&pr).unwrap_or(s).to_string())
                            .collect();
                        if tt.first().map(|s| s.contains('/')).unwrap_or(false) {
                            // ${(@)tmp2:h}
                            tt = tt.iter().map(|s| head_dir(s)).collect();
                            setaparam("tmp2", tt.clone());
                            compquote(vec!["tmp2".into()]);
                            tt = get_arr("tmp2");
                            for t in &tt {
                                if t.ends_with('/') {
                                    exppaths.push(format!("{}{}{}", t, tpre, tsuf));
                                } else {
                                    exppaths.push(format!("{}/{}{}", t, tpre, tsuf));
                                }
                            }
                        } else if concat.contains('/') {
                            exppaths.push(format!("{}{}", tpre, tsuf));
                        }
                    }
                    hit_continue_outer = true;
                    break;
                }
            } else if tmp1.is_empty() {
                // sh:546-573 — empty dir hacks.
                if concat.is_empty() && !format!("{}{}", pre, suf).is_empty() {
                    let mut np = vec!["-S".to_string(), "".to_string()];
                    np.extend(pfxsfx.clone());
                    pfxsfx = np;
                } else if haspats
                    && format!("{}{}{}", tpre, tsuf, suf).is_empty()
                    && pre.ends_with('/')
                {
                    setsparam("PREFIX", &opre);
                    setsparam("SUFFIX", &osuf);
                    compadd(vec![
                        "-nQS".into(),
                        "".into(),
                        "-".into(),
                        format!("{}{}{}", linepath, donepath, orig),
                    ]);
                }
                hit_continue_outer = true;
                break;
            }

            // sh:575-585 — ignore-parents.
            if !ignpar.is_empty()
                && comp_no_ignore.is_empty()
                && !concat.contains('/')
                && !tmp1.is_empty()
                && (!ignpar.contains("dir") || pats.first().map(|s| s == "*(-/)").unwrap_or(false))
                && (!ignpar.contains("..")
                    || tmp1.first().map(|s| s.contains("../")).unwrap_or(false))
            {
                let base = format!("{}{}{}", prepath, realpath, donepath);
                setaparam("tmp1", tmp1.clone());
                setaparam("ignore", ignore.clone());
                compfiles(vec![
                    "-i".into(),
                    "tmp1".into(),
                    "ignore".into(),
                    ignpar.clone(),
                    base.clone(),
                ]);
                ignore = get_arr("ignore");
                let mut ci = get_arr("_comp_ignore");
                ci.extend(
                    ignore
                        .iter()
                        .map(|e| e.strip_prefix(&base).unwrap_or(e).to_string()),
                );
                setaparam("_comp_ignore", ci.clone());
                if !ci.is_empty() && !mopts.iter().any(|e| e == "-F") {
                    mopts.push("-F".into());
                    mopts.push("_comp_ignore".into());
                }
            }

            // sh:589-596 — advance to next component.
            if tpre.contains('/') {
                tpre = tpre.splitn(2, '/').nth(1).unwrap_or("").to_string();
            } else if tsuf.contains('/') {
                tpre = tsuf.splitn(2, '/').nth(1).unwrap_or("").to_string();
                tsuf.clear();
            } else {
                break;
            }

            // sh:602-608 — skip over next components.
            tmp2s = match_skips_prefix(&tpre, skips_squeeze);
            if !tmp2s.is_empty() {
                skipped = format!("/{}", tmp2s);
                tpre = tpre[tmp2s.len()..].to_string();
            } else {
                skipped = "/".into();
            }
            npathcheck = 0;
        }

        if hit_continue_outer {
            continue; // continue 2
        }

        // sh:612-625 — the first-ambiguous-component search.
        let mut tmp3 = format!("{}{}", pre, suf);
        tpre = pre.clone();
        tsuf = suf.clone();
        let anchor = format!("{}{}{}", prepath, realpath, testpath);
        if !anchor.is_empty() {
            // sh:619-623 — the strip is CASE-INSENSITIVE under NO_CASE_GLOB:
            // `tmp1=( "${(@)tmp1#(#i)${prepath}${realpath}${testpath}}" )`.
            // The anchor is built from `donepath`, i.e. the components as the
            // user TYPED them, while `tmp1` holds what globbing returned, i.e.
            // the components as they are spelled on disk. Those differ in case
            // exactly when `nocaseglob` (or a case-insensitive filesystem) let
            // an unmatched-case component through, and a case-sensitive strip
            // then leaves the whole absolute path in the match: with
            // `accept-exact-dirs` set, `ls /tmp/probeci/abcdir/<TAB>` inserted
            // `/tmp/probeci/abcdir//tmp/probeci/AbcDir/` where zsh lists the
            // directory's files.
            let ignore_case = !isset(CASEGLOB); // sh:619
            tmp1 = tmp1
                .iter()
                .map(|s| {
                    if let Some(rest) = s.strip_prefix(&anchor) {
                        return rest.to_string(); // sh:623
                    }
                    if ignore_case {
                        // sh:621 — `(#i)` over a literal anchor: fold both
                        // sides per character so a multibyte anchor can never
                        // split `s` on a non-char boundary.
                        let mut it = s.char_indices();
                        let mut end = 0usize;
                        let mut ok = true;
                        for ac in anchor.chars() {
                            match it.next() {
                                Some((i, sc)) if sc.to_lowercase().eq(ac.to_lowercase()) => {
                                    end = i + sc.len_utf8();
                                }
                                _ => {
                                    ok = false;
                                    break;
                                }
                            }
                        }
                        if ok {
                            return s[end..].to_string();
                        }
                    }
                    s.clone()
                })
                .collect();
        }

        let mut tmp4 = String::new();
        loop {
            // sh:634-635 — compfiles -r.
            setaparam("tmp1", tmp1.clone());
            let amb = compfiles(vec!["-r".into(), "tmp1".into(), dequote_q(&tmp3)]);
            tmp1 = get_arr("tmp1");
            tmp4 = amb.to_string();

            let tmp2s2;
            if tpre.contains('/') {
                tmp2s2 = format!("{}{}", cpre, tpre.split('/').next().unwrap_or(""));
                setsparam("PREFIX", &format!("{}{}{}", linepath, donepath, tmp2s2));
                setsparam(
                    "SUFFIX",
                    &format!(
                        "/{}{}",
                        tpre.splitn(2, '/').nth(1).unwrap_or(""),
                        tsuf.splitn(2, '/').nth(1).unwrap_or("")
                    ),
                );
            } else {
                tmp2s2 = format!("{}{}", cpre, tpre);
                setsparam("PREFIX", &format!("{}{}{}", linepath, donepath, tmp2s2));
                setsparam("SUFFIX", &tsuf);
            }

            if amb != 0 {
                // sh:651-757 — ambiguous component: add candidates.
                let mut tmp2s3 = testpath.clone();
                if !linepath.is_empty() {
                    setaparam("tmp2", vec![tmp2s3.clone()]);
                    setaparam("tmp1", tmp1.clone());
                    compquote(vec!["-p".into(), "tmp2".into(), "tmp1".into()]);
                    tmp2s3 = get_arr("tmp2").into_iter().next().unwrap_or_default();
                    tmp1 = get_arr("tmp1");
                } else if !tmp2s3.is_empty() {
                    setaparam("tmp1", tmp1.clone());
                    compquote(vec!["-p".into(), "tmp1".into()]);
                    tmp1 = get_arr("tmp1");
                    setaparam("tmp2", vec![tmp2s3.clone()]);
                    compquote(vec!["tmp2".into()]);
                    tmp2s3 = get_arr("tmp2").into_iter().next().unwrap_or_default();
                } else {
                    setaparam("tmp1", tmp1.clone());
                    setaparam("tmp2", vec![tmp2s3.clone()]);
                    compquote(vec!["tmp1".into(), "tmp2".into()]);
                    tmp1 = get_arr("tmp1");
                    tmp2s3 = get_arr("tmp2").into_iter().next().unwrap_or_default();
                }

                if comp_correct.is_empty()
                    && pattern_match == "*"
                    && listsfx
                    && has_active_glob(&tmp2s3)
                {
                    setsparam("PREFIX", &opre);
                    setsparam("SUFFIX", &osuf);
                }

                let ipx = get_str("IPREFIX");
                let isx = get_str("ISUFFIX");
                let anchor2 = format!("{}{}{}", prepath, realpath, testpath);
                let listing = cs_s("insert").is_empty()
                    // sh:681 `! zstyle -t "…:paths" expand suffix`
                    || (!zstyle_t_word(&paths_ctx, "expand", &["suffix"])
                        && !listsfx
                        && (!comp_correct.is_empty()
                            || pattern_match.is_empty()
                            || !get_str("SUFFIX").contains('/')
                            || has_active_glob(
                                get_str("SUFFIX").splitn(2, '/').nth(1).unwrap_or(""),
                            )));

                if listing {
                    if amb != 0 && zstyle_t(&paths_ctx, "ambiguous") {
                        crate::ported::zle::compcore::set_compstate_str("to_end", "");
                    }
                    if tmp3.contains('/') {
                        if !listsfx
                            || !tmp3
                                .split('/')
                                .nth(1)
                                .map(|s| !s.is_empty())
                                .unwrap_or(false)
                        {
                            // sh:694-702
                            tmp1 = tmp1
                                .iter()
                                .map(|s| s.split('/').next().unwrap_or("").to_string())
                                .collect();
                            setaparam("tmp1", tmp1.clone());
                            dispatch0("_list_files", &["tmp1".into(), anchor2.clone()], 695);
                            let listopts = get_arr("listopts");
                            let mut a = vec![uopt.clone()];
                            a.retain(|s| !s.is_empty());
                            a.push("-Qf".into());
                            a.extend(mopts.clone());
                            a.push("-p".into());
                            a.push(format!(
                                "{}{}{}",
                                if uopt.is_empty() { "" } else { ipx.as_str() },
                                linepath,
                                tmp2s3
                            ));
                            a.push("-s".into());
                            a.push(format!(
                                "/{}{}",
                                tmp3.splitn(2, '/').nth(1).unwrap_or(""),
                                if uopt.is_empty() {
                                    String::new()
                                } else {
                                    isx.clone()
                                }
                            ));
                            a.push("-W".into());
                            a.push(anchor2.clone());
                            a.extend(pfxsfx.clone());
                            a.extend(mopts_r.clone());
                            a.extend(listopts.clone());
                            a.push("-a".into());
                            a.push("tmp1".into());
                            compadd(a);
                        } else {
                            // sh:704-713
                            tmp1 = tmp1
                                .iter()
                                .map(|s| {
                                    format!(
                                        "{}/{}",
                                        s.split('/').next().unwrap_or(""),
                                        tmp3.splitn(2, '/').nth(1).unwrap_or("")
                                    )
                                })
                                .collect();
                            setaparam("tmp1", tmp1.clone());
                            dispatch0("_list_files", &["tmp1".into(), anchor2.clone()], 706);
                            let listopts = get_arr("listopts");
                            let mut a = vec![uopt.clone()];
                            a.retain(|s| !s.is_empty());
                            a.push("-Qf".into());
                            a.extend(mopts.clone());
                            a.push("-p".into());
                            a.push(format!(
                                "{}{}{}",
                                if uopt.is_empty() { "" } else { ipx.as_str() },
                                linepath,
                                tmp2s3
                            ));
                            a.push("-s".into());
                            a.push(if uopt.is_empty() {
                                String::new()
                            } else {
                                isx.clone()
                            });
                            a.push("-W".into());
                            a.push(anchor2.clone());
                            a.extend(pfxsfx.clone());
                            a.extend(mopts_r.clone());
                            a.extend(listopts.clone());
                            a.push("-a".into());
                            a.push("tmp1".into());
                            compadd(a);
                        }
                    } else {
                        // sh:716-722
                        setaparam("tmp1", tmp1.clone());
                        dispatch0("_list_files", &["tmp1".into(), anchor2.clone()], 716);
                        let listopts = get_arr("listopts");
                        let mut a = vec![uopt.clone()];
                        a.retain(|s| !s.is_empty());
                        a.push("-Qf".into());
                        a.extend(mopts.clone());
                        a.push("-p".into());
                        a.push(format!(
                            "{}{}{}",
                            if uopt.is_empty() { "" } else { ipx.as_str() },
                            linepath,
                            tmp2s3
                        ));
                        a.push("-s".into());
                        a.push(if uopt.is_empty() {
                            String::new()
                        } else {
                            isx.clone()
                        });
                        a.push("-W".into());
                        a.push(anchor2.clone());
                        a.extend(pfxsfx.clone());
                        a.extend(mopts_r.clone());
                        a.extend(listopts.clone());
                        a.push("-a".into());
                        a.push("tmp1".into());
                        compadd(a);
                    }
                } else {
                    // sh:724-753 — inserting the match.
                    if tmp3.contains('/') {
                        let mut base = vec![uopt.clone()];
                        base.retain(|s| !s.is_empty());
                        base.push("-Qf".into());
                        base.extend(mopts.clone());
                        base.push("-p".into());
                        base.push(format!(
                            "{}{}{}",
                            if uopt.is_empty() { "" } else { ipx.as_str() },
                            linepath,
                            tmp2s3
                        ));
                        base.push("-W".into());
                        base.push(anchor2.clone());
                        base.extend(pfxsfx.clone());
                        base.extend(mopts_r.clone());
                        if !listsfx {
                            for it in tmp1.clone() {
                                setaparam("tmpdisp", vec![it.clone()]);
                                dispatch0("_list_files", &["tmpdisp".into(), anchor2.clone()], 733);
                                let disp = get_arr("tmpdisp").into_iter().next().unwrap_or(it);
                                let listopts = get_arr("listopts");
                                let mut a = base.clone();
                                a.push("-s".into());
                                a.push(if uopt.is_empty() {
                                    String::new()
                                } else {
                                    isx.clone()
                                });
                                a.extend(listopts);
                                a.push("-".into());
                                a.push(disp);
                                compadd(a);
                            }
                        } else {
                            if !pattern_match.is_empty() {
                                // SUFFIX gs./.*/ + '*'
                                // sh:733 `SUFFIX="${SUFFIX:gs./.*/}*"` — old `/`, new `*/`.
                                let cs = get_str("SUFFIX").replace('/', "*/") + "*";
                                setsparam("SUFFIX", &cs);
                            }
                            for it in tmp1.clone() {
                                setaparam("i", vec![it.clone()]);
                                dispatch0("_list_files", &["i".into(), anchor2.clone()], 740);
                                let disp = get_arr("i").into_iter().next().unwrap_or(it);
                                let listopts = get_arr("listopts");
                                let mut a = base.clone();
                                a.extend(listopts);
                                a.push("-".into());
                                a.push(disp);
                                compadd(a);
                            }
                        }
                    } else {
                        setaparam("tmp1", tmp1.clone());
                        dispatch0("_list_files", &["tmp1".into(), anchor2.clone()], 745);
                        let listopts = get_arr("listopts");
                        let mut a = vec![uopt.clone()];
                        a.retain(|s| !s.is_empty());
                        a.push("-Qf".into());
                        a.extend(mopts.clone());
                        a.push("-p".into());
                        a.push(format!(
                            "{}{}{}",
                            if uopt.is_empty() { "" } else { ipx.as_str() },
                            linepath,
                            tmp2s3
                        ));
                        a.push("-s".into());
                        a.push(if uopt.is_empty() {
                            String::new()
                        } else {
                            isx.clone()
                        });
                        a.push("-W".into());
                        a.push(anchor2.clone());
                        a.extend(pfxsfx.clone());
                        a.extend(mopts.clone());
                        a.extend(mopts_r.clone());
                        a.extend(listopts);
                        a.push("-a".into());
                        a.push("tmp1".into());
                        compadd(a);
                    }
                }
                tmp4 = "-".into();
                break;
            }

            // sh:762-765 — all components checked.
            if !tmp3.contains('/') {
                tmp4.clear();
                break;
            }

            // sh:770-797 — commit the unambiguous component.
            let head = tmp1
                .first()
                .map(|s| s.split('/').next().unwrap_or(""))
                .unwrap_or("");
            testpath = format!("{}{}/", testpath, head);
            tmp3 = tmp3.splitn(2, '/').nth(1).unwrap_or("").to_string();

            let use_line_head =
                comp_correct.is_empty() && !pattern_match.is_empty() && has_active_glob(&tmp2s2);
            if tpre.contains('/') {
                if use_line_head {
                    cpre = format!(
                        "{}{}/",
                        cpre,
                        tmp1.first()
                            .map(|s| s.split('/').next().unwrap_or(""))
                            .unwrap_or("")
                    );
                } else {
                    cpre = format!("{}{}/", cpre, tpre.split('/').next().unwrap_or(""));
                }
                tpre = tpre.splitn(2, '/').nth(1).unwrap_or("").to_string();
            } else if tsuf.contains('/') {
                // sh:785 `[[ "$tsuf" != /* ]] && mid="$testpath"`
                if !tsuf.starts_with('/') {
                    mid = testpath.clone();
                }
                if use_line_head {
                    cpre = format!(
                        "{}{}/",
                        cpre,
                        tmp1.first()
                            .map(|s| s.split('/').next().unwrap_or(""))
                            .unwrap_or("")
                    );
                } else {
                    cpre = format!("{}{}/", cpre, tpre);
                }
                tpre = tsuf.splitn(2, '/').nth(1).unwrap_or("").to_string();
                tsuf.clear();
            } else {
                tpre.clear();
                tsuf.clear();
            }

            tmp1 = tmp1
                .iter()
                .map(|s| s.splitn(2, '/').nth(1).unwrap_or("").to_string())
                .collect();
        }

        // sh:800-876 — final add of collected matches (non-ambiguous).
        if tmp4.is_empty() && mid.ends_with('/') {
            // Completing in the middle of the word, not in the last
            // component (upstream `if [[ "$mid" = */ ]]`).
            setsparam("PREFIX", &opre);
            setsparam("SUFFIX", &osuf);
            let mut tmp4v = strip_prefix_literal(&testpath, &mid); // `${testpath#${mid}}`
            // `${mid%/*/}`: shortest suffix matching `/*/`, i.e. from the last
            // slash before the final one; unchanged when there is none.
            let mid_dir = match mid[..mid.len() - 1].rfind('/') {
                Some(i) => mid[..i].to_string(),
                None => mid.clone(),
            };
            let mut tmp2v = mid
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_string(); // `${${mid%/}##*/}`
            let ipx = get_str("IPREFIX");
            let isx = get_str("ISUFFIX");
            let multi = mid.matches('/').count() >= 2; // `$mid = */*/*`
            let mut tmp3v = mid_dir.clone();
            if multi {
                setaparam("tmp3", vec![tmp3v.clone()]);
                if !linepath.is_empty() {
                    compquote(vec!["-p".into(), "tmp3".into()]);
                } else {
                    compquote(vec!["tmp3".into()]);
                }
                tmp3v = get_arr("tmp3").into_iter().next().unwrap_or_default();
            }
            setaparam("tmp4", vec![tmp4v.clone()]);
            setaparam("tmp2", vec![tmp2v.clone()]);
            setaparam("tmp1", tmp1.clone());
            compquote(vec!["tmp4".into(), "tmp2".into(), "tmp1".into()]);
            tmp4v = get_arr("tmp4").into_iter().next().unwrap_or_default();
            tmp2v = get_arr("tmp2").into_iter().next().unwrap_or_default();
            tmp1 = get_arr("tmp1");
            let anchor_dir = format!("{}{}{}", prepath, realpath, mid_dir);
            for i in tmp1.clone() {
                setaparam("tmp2", vec![tmp2v.clone()]);
                dispatch0("_list_files", &["tmp2".into(), anchor_dir.clone()], 822);
                tmp2v = get_arr("tmp2").into_iter().next().unwrap_or(tmp2v);
                let listopts = get_arr("listopts");
                let mut a: Vec<String> = Vec::new();
                if !uopt.is_empty() {
                    a.push(uopt.clone());
                }
                a.push("-Qf".into());
                a.extend(mopts.clone());
                a.push("-p".into());
                let ip = if uopt.is_empty() { "" } else { ipx.as_str() };
                a.push(if multi {
                    format!("{}{}{}/", ip, linepath, tmp3v)
                } else {
                    format!("{}{}", ip, linepath)
                });
                a.push("-s".into());
                a.push(format!(
                    "/{}{}{}",
                    tmp4v,
                    i,
                    if uopt.is_empty() { "" } else { isx.as_str() }
                ));
                a.push("-W".into());
                a.push(if multi {
                    format!("{}/", anchor_dir)
                } else {
                    format!("{}{}", prepath, realpath)
                });
                a.extend(pfxsfx.clone());
                a.extend(mopts_r.clone());
                a.extend(listopts);
                a.push("-".into());
                a.push(tmp2v.clone());
                compadd(a);
            }
        } else if tmp4.is_empty() {
            if osuf.contains('/') {
                setsparam("PREFIX", &format!("{}{}", opre, osuf));
                setsparam("SUFFIX", "");
            } else {
                setsparam("PREFIX", &opre);
                setsparam("SUFFIX", &osuf);
            }
            let mut tmp4s = testpath.clone();
            if !linepath.is_empty() {
                setaparam("tmp4", vec![tmp4s.clone()]);
                setaparam("tmp1", tmp1.clone());
                compquote(vec!["-p".into(), "tmp4".into(), "tmp1".into()]);
                tmp4s = get_arr("tmp4").into_iter().next().unwrap_or_default();
                tmp1 = get_arr("tmp1");
            } else if !tmp4s.is_empty() {
                setaparam("tmp1", tmp1.clone());
                compquote(vec!["-p".into(), "tmp1".into()]);
                tmp1 = get_arr("tmp1");
                setaparam("tmp4", vec![tmp4s.clone()]);
                compquote(vec!["tmp4".into()]);
                tmp4s = get_arr("tmp4").into_iter().next().unwrap_or_default();
            } else {
                setaparam("tmp4", vec![tmp4s.clone()]);
                setaparam("tmp1", tmp1.clone());
                compquote(vec!["tmp4".into(), "tmp1".into()]);
                tmp4s = get_arr("tmp4").into_iter().next().unwrap_or_default();
                tmp1 = get_arr("tmp1");
            }

            let prefix_now = get_str("PREFIX");
            let suffix_now = get_str("SUFFIX");
            let px = format!(
                "{}{}",
                prefix_now.strip_prefix('~').unwrap_or(&prefix_now),
                suffix_now
            );
            let ipx = get_str("IPREFIX");
            let isx = get_str("ISUFFIX");
            let anchor3 = format!("{}{}{}", prepath, realpath, testpath);
            if comp_correct.is_empty() && !pattern_match.is_empty() && has_active_glob(&px) {
                // sh:862-866 — pattern match.
                tmp1 = tmp1
                    .iter()
                    .map(|s| format!("{}{}{}", linepath, tmp4s, s))
                    .collect();
                setaparam("tmp1", tmp1.clone());
                dispatch0(
                    "_list_files",
                    &["tmp1".into(), format!("{}{}", prepath, realpath)],
                    864,
                );
                let listopts = get_arr("listopts");
                let mut a = vec![
                    "-Qf".to_string(),
                    "-W".into(),
                    format!("{}{}", prepath, realpath),
                ];
                a.extend(pfxsfx.clone());
                a.extend(mopts.clone());
                a.push("-M".into());
                a.push("r:|/=* r:|=*".into());
                a.extend(listopts);
                a.push("-a".into());
                a.push("tmp1".into());
                compadd(a);
            } else {
                // sh:868-873 — normal add.
                setaparam("tmp1", tmp1.clone());
                dispatch0("_list_files", &["tmp1".into(), anchor3.clone()], 869);
                let listopts = get_arr("listopts");
                let mut a = vec![uopt.clone()];
                a.retain(|s| !s.is_empty());
                a.push("-Qf".into());
                a.push("-p".into());
                a.push(format!(
                    "{}{}{}",
                    if uopt.is_empty() { "" } else { ipx.as_str() },
                    linepath,
                    tmp4s
                ));
                a.push("-s".into());
                a.push(if uopt.is_empty() {
                    String::new()
                } else {
                    isx.clone()
                });
                a.push("-W".into());
                a.push(anchor3.clone());
                a.extend(pfxsfx.clone());
                a.extend(mopts.clone());
                a.extend(mopts_r.clone());
                a.extend(listopts);
                a.push("-a".into());
                a.push("tmp1".into());
                compadd(a);
            }
        }
    }

    // sh:886-893 — expand-paths.
    let matcher_num = getsparam("_matcher_num")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let matchers = get_arr("_matchers").len() as i64;
    if matcher_num == matchers
        // sh:887 `zstyle -t "…:paths" expand prefix`
        && zstyle_t_word(&paths_ctx, "expand", &["prefix"])
        && nm == cs_i("nmatches")
        && !exppaths.is_empty()
        && format!("{}{}", linepath, dedup(exppaths.clone()).join(" ")) != eorig
    {
        setsparam("PREFIX", &opre);
        setsparam("SUFFIX", &osuf);
        setaparam("exppaths", dedup(exppaths.clone()));
        let mut a = vec!["-Q".to_string()];
        a.extend(mopts.clone());
        a.push("-S".into());
        a.push("".into());
        a.push("-M".into());
        a.push("r:|/=* r:|=*".into());
        a.push("-p".into());
        a.push(linepath.clone());
        a.push("-a".into());
        a.push("exppaths".into());
        compadd(a);
    }

    // Pop the ZLE-special scope (see the snapshot comment above): upstream
    // gets this for free from PM_LOCAL, the port has to do it by hand.
    setsparam("PREFIX", &entry_prefix);
    setsparam("SUFFIX", &entry_suffix);

    // sh:895 — return status.
    if nm != cs_i("nmatches") {
        0
    } else {
        1
    }
}

// ---- misc string helpers ------------------------------------------

fn dedup(v: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    v.into_iter().filter(|e| seen.insert(e.clone())).collect()
}

/// `eval "x=~spec"` — tilde expansion (`~`, `~user`, `~-`, `~+`). Routes
/// through the ported `filesubstr` by converting the leading ASCII `~`
/// to the Tilde token it expects.
fn expand_tilde(spec: &str) -> Option<String> {
    crate::compsys::ported::shared::tilde_expand(spec)
}

fn is_dir(p: &str) -> bool {
    std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}
fn path_exists(p: &str) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}
fn is_numeric_dirstack(s: &str) -> bool {
    // ([-+]|)[0-9]##
    let body = s.strip_prefix(['-', '+']).unwrap_or(s);
    !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit())
}

/// `${(@)s:h}` — dirname.
fn head_dir(s: &str) -> String {
    let t = s.trim_end_matches('/');
    match t.rfind('/') {
        Some(i) if i > 0 => t[..i].to_string(),
        Some(_) => "/".to_string(),
        None => ".".to_string(),
    }
}

/// `${orig[1,(in:i:)/][1,-2]}` — keep everything up to and including the
/// n-th slash, then drop the final char. With fewer than n slashes `(in:i:)`
/// is one past the end, so the whole string is kept and its last char dropped.
fn truncate_after_nth_slash(s: &str, n: usize) -> String {
    let mut count = 0;
    for (idx, c) in s.char_indices() {
        if c == '/' {
            count += 1;
            if count == n {
                return s[..idx].to_string();
            }
        }
    }
    let mut t = s.to_string();
    t.pop();
    t
}

/// The characters upstream's quoting substitutions treat as pattern
/// characters: `\\ ] [ ^ ~ ( ) # * ?` (sh:392, sh:400, sh:421).
fn is_quoted_pattern_char(c: char) -> bool {
    matches!(c, '\\' | ']' | '[' | '^' | '~' | '(' | ')' | '#' | '*' | '?')
}

/// `${s//(#b)\\(X)/$match[1]}` — drop the backslash in front of every
/// character for which `pred` holds, scanning left to right without overlap.
fn unquote_where(s: &str, pred: impl Fn(char) -> bool) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match (c, it.peek()) {
            ('\\', Some(&n)) if pred(n) => {
                out.push(n);
                it.next();
            }
            _ => out.push(c),
        }
    }
    out
}

/// `${s//(#b)([\\\]\[\^\~\(\)\#\*\?])/\\$match[1]}` — backslash every
/// pattern character (sh:401).
fn requote_pattern_chars(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if is_quoted_pattern_char(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `eval 'realpath=${(e)~linepath}' 2>/dev/null` inside
/// `function { setopt localoptions nounset; … }` (sh:270-274), run by the
/// shell engine. A failing expansion (an unset parameter under `nounset`)
/// assigns nothing, which is the empty string here.
fn eval_e_glob(linepath: &str) -> String {
    crate::compsys::ported::shared::declare_locals(&["_cs_pf_linepath", "_cs_pf_realpath"], 0);
    let _ = setsparam("_cs_pf_linepath", linepath);
    let _ = setsparam("_cs_pf_realpath", "");
    let _ = crate::ported::exec::execute_script(
        r#"function { setopt localoptions nounset; eval '_cs_pf_realpath=${(e)~_cs_pf_linepath}' 2>/dev/null }"#,
    );
    let out = get_str("_cs_pf_realpath");
    let _ = crate::ported::params::unsetparam("_cs_pf_linepath");
    let _ = crate::ported::params::unsetparam("_cs_pf_realpath");
    out
}

/// `${s#${prefix}}` — a parameter expansion inside a `#` pattern is literal
/// (no GLOB_SUBST), so this removes `prefix` verbatim when `s` starts with it.
fn strip_prefix_literal(s: &str, prefix: &str) -> String {
    s.strip_prefix(prefix).unwrap_or(s).to_string()
}

/// `pre = (#b)(${~pp})*` — return the leading match of pattern `pp`
/// against `pre` (the matched prefix), if any; `(#b)(${~pp})*` binds the
/// group greedily, so the LONGEST matching prefix wins.
fn match_leading_pattern(pre: &str, pp: &str) -> Option<String> {
    if let Some(prog) = crate::ported::pattern::patcompile(
        &{
            let mut s = pp.to_string();
            tokenize(&mut s);
            s
        },
        0,
        None,
    ) {
        // Longest leading prefix of `pre` that matches `pp`.
        let mut best: Option<String> = None;
        for (i, _) in pre.char_indices().chain(std::iter::once((pre.len(), ' '))) {
            if crate::ported::pattern::pattry(&prog, &pre[..i]) {
                best = Some(pre[..i].to_string());
            }
        }
        best
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `tmp1=( $~tmp1 )` (sh:472) must keep every path the glob really
    /// matched, including files whose NAME contains a pattern
    /// metacharacter. The previous result-side `has_active_glob` filter
    /// deleted them: `/etc` holds 29 such files (`profile~orig`,
    /// `group~previous`, …), so `cat /etc/<TAB>` generated 87 matches
    /// instead of 116 — below LISTMAX (100), which silently swallowed
    /// zsh's "do you wish to see all 116 possibilities (58 lines)?"
    /// query. Regression guard for that whole chain.
    #[test]
    fn tilde_glob_keeps_files_whose_names_contain_glob_metachars() {
        // `zglob` reads the option table and `errflag`, both shell-global.
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        // One name per metacharacter class the old filter tripped on.
        let names = [
            "plain", "a~orig", "c#hash", "d^caret", "e*star", "f?q", "g[br]", "i|pipe", "j<lt>",
        ];
        for n in &names {
            std::fs::write(dir.path().join(n), b"").expect("write");
        }
        let pat = format!("{}/*", dir.path().display());
        let got = tilde_glob(&[pat]);
        let mut got_names: Vec<String> = got
            .iter()
            .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
            .collect();
        got_names.sort();
        let mut want: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        want.sort();
        assert_eq!(got_names, want, "tilde_glob dropped real files");
    }

    /// A pattern that matches nothing contributes nothing, and under the
    /// default NOMATCH it is `zglob`'s c:1876-1880 error: errflag is SET.
    /// That errflag is what unwinds `_path_files` at sh:472 when the
    /// completion runs with the user's options (a `zle -C` / `compdef -k`
    /// widget whose function is the completer, so no `_main_complete`
    /// NULL_GLOB). The old `glob_path` route returned the empty list and
    /// raised nothing, so `_files` kept trying its remaining patterns and
    /// `_alternative` its remaining actions after zsh had stopped
    /// (spec-fuzz 9503/case0022).
    #[test]
    fn tilde_glob_drops_non_matching_pattern() {
        use std::sync::atomic::Ordering;
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("only"), b"").expect("write");
        let pat = format!("{}/nosuchprefix*", dir.path().display());
        let saved_noerrs = std::mem::replace(&mut *crate::ported::utils::noerrs_lock().lock().unwrap(), 1);
        crate::ported::utils::errflag.store(0, Ordering::Relaxed);
        let got = tilde_glob(&[pat]);
        let ef = crate::ported::utils::errflag.load(Ordering::Relaxed);
        crate::ported::utils::errflag.store(0, Ordering::Relaxed);
        *crate::ported::utils::noerrs_lock().lock().unwrap() = saved_noerrs;
        assert!(got.is_empty());
        assert_ne!(ef, 0, "c:1876-1880 — NOMATCH must raise errflag");
    }

    #[test]
    fn zparse_dirs_only() {
        let p = zparse_pathfiles(&["-/".to_string()]);
        assert_eq!(p.tmp1, vec!["-/".to_string()]);
        assert!(p.mopts.is_empty());
    }

    #[test]
    fn zparse_glob_separate_concats() {
        // -g <pat> stores option+value concatenated (ZOF_SAME).
        let p = zparse_pathfiles(&["-g".to_string(), "*.rs".to_string()]);
        assert_eq!(p.tmp1, vec!["-g*.rs".to_string()]);
    }

    #[test]
    fn zparse_glob_attached() {
        let p = zparse_pathfiles(&["-g*.txt".to_string()]);
        assert_eq!(p.tmp1, vec!["-g*.txt".to_string()]);
    }

    #[test]
    fn zparse_value_options_split_into_two_elements() {
        // -W stores option and value as two elements (not concatenated).
        let p = zparse_pathfiles(&["-W".to_string(), "/tmp".to_string()]);
        assert_eq!(p.prepaths, vec!["-W".to_string(), "/tmp".to_string()]);
        // -P → pfx, -M → matcher.
        let p2 = zparse_pathfiles(&[
            "-P".to_string(),
            "pre".to_string(),
            "-M".to_string(),
            "m:{a-z}={A-Z}".to_string(),
        ]);
        assert_eq!(p2.pfx, vec!["-P".to_string(), "pre".to_string()]);
        assert_eq!(
            p2.matcher,
            vec!["-M".to_string(), "m:{a-z}={A-Z}".to_string()]
        );
    }

    #[test]
    fn zparse_flags_go_to_mopts() {
        let p = zparse_pathfiles(&[
            "-J".to_string(),
            "grp".to_string(),
            "-1".to_string(),
            "-n".to_string(),
        ]);
        assert_eq!(
            p.mopts,
            vec![
                "-J".to_string(),
                "grp".to_string(),
                "-1".to_string(),
                "-n".to_string()
            ]
        );
    }

    #[test]
    fn zparse_stops_at_bare_dash() {
        // A bare `-` (compadd terminator) ends option parsing.
        let p = zparse_pathfiles(&["-f".to_string(), "-".to_string(), "x".to_string()]);
        assert_eq!(p.tmp1, vec!["-f".to_string()]);
    }

    #[test]
    fn empty_line_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "/nonexistent/path/here_");
        let _ = setsparam("SUFFIX", "");
        // No active completion => nmatches unchanged => rc 1.
        assert_eq!(_path_files_impl(&[]), 1);
    }

    /// sh:392/400 — `\\(?)` drops every backslash, `\\([^\\\]\[\^\~\(\)\#\*\?])`
    /// only those in front of a NON-pattern character, and sh:421's
    /// `\\([\\\]\[\^\~\(\)\#\*\?])` only those in front of a pattern one.
    #[test]
    fn quoting_substitutions_of_the_accept_exact_dirs_block() {
        assert_eq!(unquote_where(r"a\ b\*c", |_| true), "a b*c");
        assert_eq!(
            unquote_where(r"a\ b\*c", |c| !is_quoted_pattern_char(c)),
            r"a b\*c"
        );
        assert_eq!(
            unquote_where(r"a\ b\*c", is_quoted_pattern_char),
            r"a\ b*c"
        );
        // a doubled backslash is one escaped backslash, not two escapes
        assert_eq!(unquote_where(r"a\\b", |_| true), r"a\b");
        assert_eq!(requote_pattern_chars("a*b(c)"), r"a\*b\(c\)");
    }

    /// `(|*[^\\])[][*?#~^\|\<\>]*` — an unescaped metacharacter anywhere.
    #[test]
    fn has_active_glob_ignores_backslash_escaped_metachars() {
        assert!(has_active_glob("*"));
        assert!(has_active_glob("a?b"));
        assert!(!has_active_glob(r"a\*b"));
        // the pattern only looks at the single preceding character
        assert!(!has_active_glob(r"a\\*b"));
        assert!(!has_active_glob("plain/path"));
    }

    /// `${orig[1,(in:i:)/][1,-2]}` — everything before the i-th slash; with
    /// fewer slashes the whole string minus its last character.
    #[test]
    fn truncate_after_nth_slash_cuts_before_the_slash() {
        assert_eq!(truncate_after_nth_slash("a/b/c", 1), "a");
        assert_eq!(truncate_after_nth_slash("a/b/c", 2), "a/b");
        assert_eq!(truncate_after_nth_slash("abc", 1), "ab");
    }

    /// `${s#${prefix}}` — the prefix comes out of a parameter expansion, so
    /// it is literal and glob characters in it are not special.
    #[test]
    fn strip_prefix_literal_takes_the_text_verbatim() {
        assert_eq!(strip_prefix_literal("$foo/bar", "$foo/"), "bar");
        assert_eq!(strip_prefix_literal("xyz", "*"), "xyz");
    }
}
