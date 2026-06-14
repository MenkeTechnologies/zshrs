//! Port of `_sequence` from `Completion/Base/Utility/_sequence`.
//!
//! Full upstream body (40 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  # a separated list where each component uses the same function.
//! sh:11  zparseopts -D -a opts s:=sep n:=num p:=pref i:=pref P:=pref I:=suf S:=suf \
//! sh:12      q=suf r:=suf R:=suf C:=cont F:=garbage d=uniq M+: J+: V+: 1 2 o+: X+: x+:
//! sh:13  (( $#cont )) && curcontext="${curcontext%:*}:$cont[2]"
//! sh:14  (( $#sep )) || sep[2]=,
//! sh:31  (( minus = argv[(ib:2:)-] ))
//! sh:32  "${(@)argv[1,minus-1]}" "$opts[@]" -F dedup "$pref[@]" "$suf[@]" "${(@)argv[minus+1,-1]}"
//! ```
//!
//! Composes a separated-list completer: argv before `-` is the
//! per-element command + args; argv after `-` is the trailing args
//! passed through. Default separator is `,`; `-s <sep>` overrides;
//! `-n <max>` limits count; `-d` allows dupes.

use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::bin_zparseopts;
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::zle::compcore::set_compstate_str;
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:11-12 — bridge zparseopts with the dense spec list.
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
    setaparam(src, args.to_vec());
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
    (
        getaparam(src).unwrap_or_default(),
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
    // sh:11
    let (mut argv, opts, sep, num, mut pref, mut suf, uniq) = run_zparseopts_sequence(args);

    // sh:14
    let sep_char = sep.get(1).cloned().unwrap_or_else(|| ",".to_string());
    let qsep = sep_char.clone();

    // sh:18-19
    let mut nosep = false;
    let s_pos = suf.iter().position(|s| s == "-S");
    if s_pos.is_some() {
        if let Some(end) = suf.get(s_pos.unwrap() + 1).cloned() {
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
    }

    // sh:23-29  dedup list build (only when -d not given)
    let dedup: Vec<String> = if uniq.is_empty() {
        let mut pre = String::new();
        if let Some(p_pos) = pref.iter().position(|s| s == "-P") {
            if let Some(v) = pref.get(p_pos + 1).cloned() {
                pre = v;
            }
        }
        let prefix = getsparam("PREFIX").unwrap_or_default();
        let suffix = getsparam("SUFFIX").unwrap_or_default();
        let trimmed_prefix = prefix.trim_start_matches(&pre as &str).to_string();
        let mut dd: Vec<String> = trimmed_prefix.split(&qsep).map(|s| s.to_string()).collect();
        if dd.len() > 1 {
            dd.pop(); // drop the LAST partial token
        } else {
            dd.clear();
        }
        for tail in suffix.split(&qsep).skip(1) {
            dd.push(tail.to_string());
        }
        dd
    } else {
        Vec::new()
    };
    setaparam("dedup", dedup);

    // sh:31-37
    let num_val: i64 = num.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let mut consumed_pref = false;
    if num_val > 0
        && bin_compset(
            "compset",
            &["-P".to_string(), format!("{}*{}", num_val - 1, qsep)],
            &make_ops(),
            0,
        ) == 0
    {
        pref.clear();
        consumed_pref = true;
    } else if !nosep && (num_val == 0 || num_val > 1) {
        let _ = &mut suf;
        suf = vec![
            "-S".to_string(),
            qsep.clone(),
            "-r".to_string(),
            format!("{} \t\n-", qsep),
        ];
    }
    if bin_compset(
        "compset",
        &["-S".to_string(), format!("{}*", qsep)],
        &make_ops(),
        0,
    ) == 0
    {
        suf.clear();
    }
    if bin_compset(
        "compset",
        &["-P".to_string(), format!("*{}", qsep)],
        &make_ops(),
        0,
    ) == 0
    {
        pref.clear();
    }
    let _ = consumed_pref;

    // Apply -F dedup to compstate ignored-prefix (approx by setting
    //   compstate[ignored])
    set_compstate_str("ignored", "");

    // sh:39-40  split argv on bare `-`; left part is command, right
    //   is trailing args.
    let minus = argv.iter().position(|s| s == "-").unwrap_or(argv.len());
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
    dispatch_function_call(&cmd, &call_argv).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_for_empty_command() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PREFIX", "");
        let _ = crate::ported::params::setsparam("SUFFIX", "");
        let r = _sequence(&["-".to_string()]);
        assert_eq!(r, 1);
    }
}
