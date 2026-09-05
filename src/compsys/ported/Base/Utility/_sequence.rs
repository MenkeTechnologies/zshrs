//! Port of `_sequence` from `Completion/Base/Utility/_sequence`.
//!
//! Full upstream body (40 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  # a separated list where each component uses the same function.
//! sh:13  zparseopts -D -a opts s:=sep n:=num p:=pref i:=pref P:=pref I:=suf S:=suf \
//! sh:14      q=suf r:=suf R:=suf C:=cont F:=garbage d=uniq M+: J+: V+: 1 2 o+: X+: x+:
//! sh:15  (( $#cont )) && curcontext="${curcontext%:*}:$cont[2]"
//! sh:16  (( $#sep )) || sep[2]=,
//! sh:23  qsep="${sep[2]}"
//! sh:24  compquote -p qsep
//! sh:39  (( minus = argv[(ib:2:)-] ))
//! sh:40  "${(@)argv[1,minus-1]}" "$opts[@]" -F dedup "$pref[@]" "$suf[@]" "${(@)argv[minus+1,-1]}"
//! ```
//!
//! Composes a separated-list completer: argv before `-` is the
//! per-element command + args; argv after `-` is the trailing args
//! passed through. Default separator is `,`; `-s <sep>` overrides;
//! `-n <max>` limits count; `-d` allows dupes.

use crate::ported::exec::dispatch_function_call;
use crate::ported::glob::remnulargs;
use crate::ported::lex::{parse_subst_string, untokenize};
use crate::ported::modules::zutil::bin_zparseopts;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use crate::ported::utils::quotestring;
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
use crate::ported::zle::complete::bin_compset;
use crate::ported::zle::computil::bin_compquote;
use crate::ported::zsh_h::{options, MAX_OPS, QT_BACKSLASH};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `compquote [-p] name...` — the C builtin is
/// `BUILTIN("compquote", 0, bin_compquote, 1, -1, 0, "p", NULL)`, so
/// `execbuiltin` parses the leading `-p` into `ops` and `bin_compquote` only
/// ever sees parameter NAMES. Calling `bin_compquote` with a raw
/// `["-p", "qsep"]` would make it treat `-p` as a parameter name and try to
/// quote `$-`, which aborts with "read-only variable: -" — the same trap
/// `_path_files.rs:73-95` documents. Replicate the option parse here.
fn compquote(argv: &[String]) {
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
            names.push(a.clone());
        }
    }
    bin_compquote("compquote", &names, &ops, 0);
}

/// `${(q)s}` — `quotestring(s, QT_BACKSLASH)` (`Src/subst.c` `(q)` arm).
fn q(s: &str) -> String {
    quotestring(s, QT_BACKSLASH)
}

/// sh:34 — the `-r` argument of `suf=( -S ${qsep} -r "$end[1]${(q)qsep[1]} \t\n\-" )`.
///
/// `${qsep[1]}` is the first CHARACTER of the separator compquote just wrote,
/// `${(q)}` quotes it, and the tail is written in DOUBLE quotes, where zsh
/// leaves `\t` / `\n` / `\-` as the two-character sequences they are spelled
/// as. That matters: `compadd -r` runs the string through
/// `getkeystring(s, &i, GETKEYS_SUFFIX, &z)` (`Src/Zle/zle_misc.c:1672`), which
/// is what turns `\t`/`\n` into tab/newline AND what sets `z` from the `\-`,
/// i.e. `suffixnoinsrem` — "remove the suffix on an uninsertable character"
/// (c:1676-1678). Emitting a real tab/newline and a bare `-` instead therefore
/// silently dropped the `\-` flag: `seqcmd al<TAB><RETURN>` ran `seqcmd alpha,`
/// where zsh runs `seqcmd alpha`.
fn remove_chars(end: &str, qsep: &str) -> String {
    let qsep1 = q(&qsep.chars().next().map(String::from).unwrap_or_default());
    format!("{}{} \\t\\n\\-", end, qsep1)
}

/// `${(Q)s}` — `parse_subst_string + remnulargs + untokenize`
/// (`Src/subst.c:4863`). On a parse error C keeps the original
/// (noerrs-tolerant); mirror that.
fn dequote_q(s: &str) -> String {
    match parse_subst_string(s) {
        Ok(mut r) => {
            remnulargs(&mut r);
            untokenize(&r)
        }
        Err(_) => s.to_string(),
    }
}

/// sh:13-14 — bridge zparseopts with the dense spec list.
fn run_zparseopts_sequence(
    args: &[String],
) -> (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
) {
    let src = "__compsys_argv";
    crate::compsys::ported::shared::set_bridge_argv(src, args);
    setaparam("opts", Vec::new());
    setaparam("sep", Vec::new());
    setaparam("num", Vec::new());
    setaparam("pref", Vec::new());
    setaparam("suf", Vec::new());
    setaparam("cont", Vec::new());
    setaparam("uniq", Vec::new());
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-D".to_string(),
            "-v".to_string(),
            src.to_string(),
            "-a".to_string(),
            "opts".to_string(),
            "s:=sep".to_string(),
            "n:=num".to_string(),
            "p:=pref".to_string(),
            "i:=pref".to_string(),
            "P:=pref".to_string(),
            "I:=suf".to_string(),
            "S:=suf".to_string(),
            "q=suf".to_string(),
            "r:=suf".to_string(),
            "R:=suf".to_string(),
            "C:=cont".to_string(),
            "F:=cont".to_string(), // garbage→cont
            "d=uniq".to_string(),
            "M+:".to_string(),
            "J+:".to_string(),
            "V+:".to_string(),
            "1".to_string(),
            "2".to_string(),
            "o+:".to_string(),
            "X+:".to_string(),
            "x+:".to_string(),
        ],
        &make_ops(),
        0,
    );
    let remaining = getaparam(src).unwrap_or_default();
    // Tear down `__compsys_argv` — the zparseopts-bridge scratch array, not a
    // real zsh identifier (zsh operates on positional $argv). It is declared
    // FUNCTION-LOCAL by `shared::set_bridge_argv`; this unset is what clears it
    // when the port runs outside any function scope. Bug #657.
    crate::ported::params::unsetparam(src);
    (
        remaining,
        getaparam("opts").unwrap_or_default(),
        getaparam("sep").unwrap_or_default(),
        getaparam("num").unwrap_or_default(),
        getaparam("pref").unwrap_or_default(),
        getaparam("suf").unwrap_or_default(),
        getaparam("uniq").unwrap_or_default(),
    )
}

/// `_sequence` — wrap a completer to run per-element of a
/// separator-delimited list.
pub fn _sequence(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_sequence");
    // sh:11 — `local -a opts sep num pref suf cont end uniq dedup garbage`.
    //
    // Eight of those ten are created as shell parameters by this port:
    // seven are `zparseopts` targets (sh:13-14, seeded in
    // `run_zparseopts_sequence`) and `dedup` is published for the
    // duplicate filter (sh:159 in the port). `end`/`garbage` stay
    // Rust-side, so they are left out. Measured on a `_sequence`-backed
    // spec, all eight outlived the completion:
    //
    //   zsh  : opts=[][0] sep=[][0] num=[][0] pref=[][0] suf=[][0]
    //          cont=[][0] uniq=[][0] dedup=[][0]
    //   zshrs: every one of them [array][0]
    //
    // `num` and `curcontext` are on the same upstream lines but are
    // already declared local by `_main_complete` (sh:27-30); `num` is
    // listed here anyway because sh:11 declares it in THIS function and
    // the port writes it here, and the extra shadow is unwound by the same
    // `endparamscope`.
    crate::compsys::ported::shared::declare_locals(
        &[
            "opts", "sep", "num", "pref", "suf", "cont", "uniq", "dedup",
        ],
        crate::compsys::ported::shared::PM_ARRAY,
    );
    // sh:10 — `local … pre qsep …`. `qsep` is a SCALAR and, unlike the eight
    // above, it is not bookkeeping: sh:24 hands its NAME to `compquote`, which
    // rewrites the parameter in place, so it has to be a real parameter and it
    // has to be local or it outlives the completion.
    crate::compsys::ported::shared::declare_locals(&["qsep"], 0);
    // sh:13
    let (mut argv, opts, sep, num, mut pref, mut suf, uniq) = run_zparseopts_sequence(args);

    // sh:16
    let sep_char = sep.get(1).cloned().unwrap_or_else(|| ",".to_string());

    // sh:18-21
    let mut nosep = false;
    // sh:19 — `end="${(q)suf[suf[(i)-S]+1]}"`. The `${(q)}` is what makes the
    // sh:20 pattern match the SUFFIX as it is actually spelled on the line.
    let mut end = String::new();
    if let Some(s_pos) = suf.iter().position(|s| s == "-S") {
        end = q(&suf.get(s_pos + 1).cloned().unwrap_or_default());
        // sh:20
        if !end.is_empty()
            && bin_compset(
                "compset",
                &["-S".to_string(), format!("{}*", end)],
                &make_ops(),
                0,
            ) == 0
        {
            suf.clear();
            nosep = true;
        }
    }

    // sh:23-24 — `qsep="${sep[2]}"` / `compquote -p qsep`.
    //
    // compquote quotes for the CURRENT quoting context: `comp_quote` quotes
    // with `*compqstack` (`Src/Zle/computil.c:3691-3705`), so the separator's
    // spelling is NOT a constant and cannot be baked in. Measured in real zsh
    // for `_sequence -s '|' …`:
    //
    //     unquoted word    qsep=\|      inside " or '   qsep=|
    //
    // Every use below reads `qsep` back out of the parameter, because that is
    // the value compquote left there. Skipping this step is not cosmetic: the
    // sh:35 test is `compset -S ${(q)qsep}\*`, and `compset -S '|*'` returns 0
    // — `|*` is a pattern ALTERNATION that matches the empty suffix — where
    // `compset -S '\|*'` returns 1. So with the raw separator the port cleared
    // `suf` on every unquoted word and passed neither `-S` nor `-r` down to the
    // element completer: `seqcmd al<TAB>` against `_sequence -s '|' _elems -`
    // left the line at `seqcmd al` where zsh writes `seqcmd alpha\|`.
    setsparam("qsep", &sep_char);
    compquote(&["-p".to_string(), "qsep".to_string()]);
    let qsep = getsparam("qsep").unwrap_or_default();
    // `${(q)qsep}` — the second, ordinary shell-quoting pass sh:31/35/36 apply
    // on top of compquote's result before using it as a compset PATTERN.
    let qqsep = q(&qsep);

    // sh:25-29  dedup list build (only when -d not given)
    let dedup: Vec<String> = if uniq.is_empty() {
        // sh:26 — `pre="${(q)pref[pref[(i)-P]+1]}"`
        let mut pre = String::new();
        if let Some(p_pos) = pref.iter().position(|s| s == "-P") {
            if let Some(v) = pref.get(p_pos + 1) {
                pre = q(v);
            }
        }
        // sh:27
        let prefix = getsparam("PREFIX").unwrap_or_default();
        let suffix = getsparam("SUFFIX").unwrap_or_default();
        let trimmed_prefix = prefix.strip_prefix(&pre as &str).unwrap_or(&prefix);
        let mut dd: Vec<String> = trimmed_prefix.split(&qsep).map(|s| s.to_string()).collect();
        if dd.len() > 1 {
            dd.pop(); // drop the LAST partial token
        } else {
            dd.clear();
        }
        for tail in suffix.split(&qsep).skip(1) {
            dd.push(tail.to_string());
        }
        // sh:28 — `[[ -n $compstate[quoting] ]] || dedup=( ${(Q)dedup} )`.
        // Outside a quoting context the words carry compquote's backslashes;
        // the matches they are compared against do not, so they are unquoted
        // again. Inside one there is nothing to strip and the test skips it.
        if get_compstate_str("quoting")
            .unwrap_or_default()
            .is_empty()
        {
            dd = dd.iter().map(|e| dequote_q(e)).collect();
        }
        dd
    } else {
        Vec::new()
    };
    setaparam("dedup", dedup);

    // sh:31-37
    let num_val: i64 = num.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    // sh:31 — `compset -P $(( num[2] - 1 )) \*${(q)qsep}`. The count and the
    // pattern are two SEPARATE arguments (`compset -P [ number ] pattern`,
    // `Src/Zle/computil.c` `bin_compset`); the port used to concatenate them
    // into one word, which made the pattern begin with a digit.
    if !num.is_empty()
        && bin_compset(
            "compset",
            &[
                "-P".to_string(),
                (num_val - 1).to_string(),
                format!("*{}", qqsep),
            ],
            &make_ops(),
            0,
        ) == 0
    {
        pref.clear();
    } else {
        // sh:34 — `suf=( -S ${qsep} -r "$end[1]${(q)qsep[1]} \t\n\-" )`.
        // `${qsep[1]}` is the first CHARACTER of the separator, and the tail is
        // written in DOUBLE quotes, where zsh leaves `\t` / `\n` / `\-` as the
        // two-character sequences they are spelled as — not as tab/newline.
        if !nosep && (num.is_empty() || num_val > 1) {
            suf = vec![
                "-S".to_string(),
                qsep.clone(),
                "-r".to_string(),
                remove_chars(&end, &qsep),
            ];
        }
        // sh:35
        if bin_compset(
            "compset",
            &["-S".to_string(), format!("{}*", qqsep)],
            &make_ops(),
            0,
        ) == 0
        {
            suf.clear();
        }
        // sh:36
        if bin_compset(
            "compset",
            &["-P".to_string(), format!("*{}", qqsep)],
            &make_ops(),
            0,
        ) == 0
        {
            pref.clear();
        }
    }
    tracing::debug!(
        target: "compsys::_sequence",
        qsep = %qsep,
        qqsep = %qqsep,
        quote = %get_compstate_str("quote").unwrap_or_default(),
        suf = ?suf,
        "sh:23-36 separator text for this quoting context",
    );

    // Apply -F dedup to compstate ignored-prefix (approx by setting
    //   compstate[ignored])
    set_compstate_str("ignored", "");

    // sh:39-40  split argv on bare `-`; left part is command, right
    //   is trailing args.
    //
    // sh:39 is `(( minus = argv[(ib:2:)-] ))` — the search starts at element
    // TWO, so a command word spelled `-` is not mistaken for the separator.
    let minus = argv
        .iter()
        .skip(1)
        .position(|s| s == "-")
        .map(|i| i + 1)
        .unwrap_or(argv.len());
    let cmd_chunk = &argv[..minus];
    let extras: Vec<String> = if minus < argv.len() {
        argv[minus + 1..].to_vec()
    } else {
        Vec::new()
    };
    if cmd_chunk.is_empty() {
        return 1;
    }
    let cmd = cmd_chunk[0].clone();
    let mut call_argv: Vec<String> = cmd_chunk[1..].to_vec();
    call_argv.extend(opts);
    call_argv.push("-F".to_string());
    call_argv.push("dedup".to_string());
    call_argv.extend(pref);
    call_argv.extend(suf);
    call_argv.extend(extras);
    // `compadd` is a BUILTIN, so `dispatch_function_call` finds no shell
    // function and the per-element completer adds NOTHING — silently, with no
    // diagnostic. Same omission `9caf16845d` fixed on `_alternative` sh:61 and
    // `_arguments` sh:453, and `_sequence` is written with a bare `compadd` by
    // ten upstream callers (`_abcde:5`, `_cu:44`, `_gem:106`, `_ipfw:168`,
    // `_luarocks:40`, `_rsync:192` …). Measured before the fix:
    // `_sequence -s '|' compadd - alpha beta gamma` returned 1 with
    // `compstate[nmatches]` still 0.
    if cmd == "compadd" {
        return crate::ported::zle::complete::bin_compadd(
            "compadd",
            &call_argv,
            &make_ops(),
            0,
        );
    }
    dispatch_function_call(&cmd, &call_argv).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// sh:34's `-r` string, against the three values REAL zsh passes down.
    ///
    /// Measured under a pty with `_elems () { print -r -- "ARGS=${(j. .)${(qq)@}}" }`
    /// as the element completer, so these are the bytes `_sequence` actually
    /// handed `compadd`:
    ///
    /// ```text
    ///   _sequence -s '|' …   seqcmd al<TAB>    -r '\\ \t\n\-'
    ///   _sequence -s '|' …   seqcmd 'al<TAB>   -r '\| \t\n\-'
    ///   _sequence …          seqcmd al<TAB>    -r ', \t\n\-'
    /// ```
    ///
    /// The first two are the SAME separator in two quoting contexts —
    /// compquote wrote `\|` on the unquoted word and `|` inside the quotes —
    /// which is the whole reason sh:24 cannot be skipped.
    #[test]
    fn remove_chars_matches_zsh() {
        // `${(q)}` reads the character-type table, so the shared init has to
        // have run or every character looks unspecial and nothing is quoted.
        let _g = crate::test_util::global_state_lock();
        crate::ported::utils::inittyptab();
        // unquoted context: compquote turned `|` into `\|`, first char `\`.
        assert_eq!(remove_chars("", "\\|"), "\\\\ \\t\\n\\-");
        // inside quotes: compquote left `|` alone.
        assert_eq!(remove_chars("", "|"), "\\| \\t\\n\\-");
        // the default separator needs no quoting in either context.
        assert_eq!(remove_chars("", ","), ", \\t\\n\\-");
        // sh:34 prefixes `$end[1]`, the `${(q)}`-quoted `-S` value.
        assert_eq!(remove_chars("\\]", ","), "\\], \\t\\n\\-");
    }

    #[test]
    fn returns_one_for_empty_command() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PREFIX", "");
        let _ = crate::ported::params::setsparam("SUFFIX", "");
        let r = _sequence(&["-".to_string()]);
        assert_eq!(r, 1);
    }
}
