//! Port of `_zargs` from `Completion/Zsh/Function/_zargs`.
//!
//! Completion for the `zargs` autoloadable function (an `xargs`-alike).
//! The command line is `zargs [opts] [eofstr input-args eofstr] command
//! [args]`, where `eofstr` (default `--`, overridable via `--eof=`/`-e`)
//! separates zargs options, the input-args, and the command. The number
//! of `eofstr` separators *before the cursor* selects what to complete:
//! 0 → zargs options, 1 → input files, 2+ → the command and its args.
//!
//! Full upstream body (49 lines, abridged — head is `#compdef`):
//! ```text
//! sh: 3  local arguments eofstr pos=$((CURRENT)) numeofs=0 ret=1 cmdpos=1
//! sh: 9  eofstr=${${${${words[(r)(--eof=*|-e*)]}#--eof=}#-e}:---}
//! sh:10  while {
//! sh:11    pos=$(( words[(b:pos-1:I)$eofstr] ))
//! sh:12    (( numeofs == 0 )) && (( cmdpos = pos ))
//! sh:13    (( pos )) && (( numeofs++ ))
//! sh:14    (( pos ))
//! sh:15  } {}
//! sh:16  case $numeofs in
//! sh:17    0)  arguments=( … )               # zargs option specs
//! sh:34        _arguments -S -s $arguments[@] && ret=0 ;;
//! sh:36    1)  _files && ret=0 ;;            # input-args
//! sh:40    *)  shift cmdpos words            # command + command args
//! sh:43        (( CURRENT -= cmdpos )); _normal ;;
//! sh:46  esac
//! sh:48  return ret
//! ```
//!
//! Note on sh:40-45: the `*)` branch does NOT do `&& ret=0`, so the
//! function returns the initial `ret=1` regardless of `_normal`'s exit
//! status. This is faithful to upstream — `_normal` is invoked for its
//! completion side effects and its return value is discarded.

use crate::compsys::ported::_arguments::arguments_byname;
use crate::compsys::ported::_files::_files;
use crate::compsys::ported::shared::glob_matches;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};

/// sh:19-33 — the zargs option specs, with each `{--long,-short}` brace
/// expansion already flattened into its two resulting `_arguments`
/// specs (exclusion group prefix + word + description suffix).
fn build_arguments() -> Vec<String> {
    [
        // sh:20  '(--eof -e)'{--eof=,-e+}'[…]'
        "(--eof -e)--eof=[change the end-of-input-args string from \"--\" to eof-str]",
        "(--eof -e)-e+[change the end-of-input-args string from \"--\" to eof-str]",
        // sh:21  '(--exit -x)'{--exit,-x}'[…]'
        "(--exit -x)--exit[exit if the size (see --max-chars) is exceeded]",
        "(--exit -x)-x[exit if the size (see --max-chars) is exceeded]",
        // sh:22  '--help[…]'
        "--help[print summary and exit]",
        // sh:23  '(--interactive -p)'{--interactive,-p}'[…]'
        "(--interactive -p)--interactive[prompt before executing each command line]",
        "(--interactive -p)-p[prompt before executing each command line]",
        // sh:24  '(--max-args -n)'{--max-args=,-n+}'[…]'
        "(--max-args -n)--max-args=[use at most max-args arguments per command line]",
        "(--max-args -n)-n+[use at most max-args arguments per command line]",
        // sh:25  '(--max-chars -s)'{--max-chars=,-s+}'[…]'
        "(--max-chars -s)--max-chars=[use at most max-chars characters per command line]",
        "(--max-chars -s)-s+[use at most max-chars characters per command line]",
        // sh:26  '(--max-lines -l)'{--max-lines=,-l+}'[…]'
        "(--max-lines -l)--max-lines=[use at most max-lines of the input-args per command line]",
        "(--max-lines -l)-l+[use at most max-lines of the input-args per command line]",
        // sh:27  '(--max-procs -P)'{--max-procs=,-P+}'[…]'
        "(--max-procs -P)--max-procs=[run up to max-procs command lines in the background at once]",
        "(--max-procs -P)-P+[run up to max-procs command lines in the background at once]",
        // sh:28  '(--no-run-if-empty -r)'{--no-run-if-empty,-r}'[…]'
        "(--no-run-if-empty -r)--no-run-if-empty[do nothing if there are no input arguments before the eof-str]",
        "(--no-run-if-empty -r)-r[do nothing if there are no input arguments before the eof-str]",
        // sh:29  '(--null -0)'{--null,-0}'[…]'
        "(--null -0)--null[split each input-arg at null bytes, for xargs compatibility]",
        "(--null -0)-0[split each input-arg at null bytes, for xargs compatibility]",
        // sh:30  '(--replace -i)'{--replace=,-i}'[…]'
        "(--replace -i)--replace=[substitute replace-str in the initial-args by each initial-arg]",
        "(--replace -i)-i[substitute replace-str in the initial-args by each initial-arg]",
        // sh:31  '(--verbose -t)'{--verbose,-t}'[…]'
        "(--verbose -t)--verbose[print each command line to stderr before executing it]",
        "(--verbose -t)-t[print each command line to stderr before executing it]",
        // sh:32  '--version[…]'
        "--version[print the version number of zargs and exit]",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// sh:9 — derive `eofstr` from `$words`.
///
/// `${words[(r)(--eof=*|-e*)]}` = the first word matching `--eof=*` or
/// `-e*`; then `#--eof=` and `#-e` strip the option prefix; then
/// `:---` substitutes `--` when the result is empty/unset.
fn compute_eofstr(words: &[String]) -> String {
    // sh:9  words[(r)(--eof=*|-e*)] — first (forward) matching word.
    let found = words
        .iter()
        .find(|w| w.starts_with("--eof=") || w.starts_with("-e"))
        .cloned()
        .unwrap_or_default();
    // sh:9  #--eof=  then  #-e  (each strips only a leading match).
    let s = found.strip_prefix("--eof=").unwrap_or(&found);
    let s = s.strip_prefix("-e").unwrap_or(s);
    // sh:9  :---  → default to "--" when empty.
    if s.is_empty() {
        "--".to_string()
    } else {
        s.to_string()
    }
}

/// sh:11 — `words[(b:begin:I)$eofstr]`: index (1-based) of the last
/// element at index `<= begin` that pattern-matches `eofstr`, searching
/// backward. Returns 0 when there is no match (or `begin < 1`).
fn find_eof_backward(words: &[String], begin: i64, eofstr: &str) -> i64 {
    if begin < 1 {
        return 0;
    }
    let n = words.len() as i64;
    let mut i = begin.min(n);
    while i >= 1 {
        if glob_matches(eofstr, &words[(i - 1) as usize]) {
            return i;
        }
        i -= 1;
    }
    0
}

/// sh:10-15 — walk backward from the cursor counting `eofstr`
/// separators. Returns `(numeofs, cmdpos)`: `numeofs` is the number of
/// separators at index `< CURRENT`, and `cmdpos` is the index of the
/// rightmost such separator (set on the first loop iteration, when
/// `numeofs == 0`).
fn scan_eofs(words: &[String], current: i64, eofstr: &str) -> (i64, i64) {
    let mut pos = current; // sh:3  pos=$((CURRENT))
    let mut numeofs = 0i64; // sh:3
    let mut cmdpos = 1i64; // sh:3
    loop {
        // sh:11  pos=$(( words[(b:pos-1:I)$eofstr] ))
        pos = find_eof_backward(words, pos - 1, eofstr);
        // sh:12  (( numeofs == 0 )) && (( cmdpos = pos ))
        if numeofs == 0 {
            cmdpos = pos;
        }
        // sh:13  (( pos )) && (( numeofs++ ))
        if pos != 0 {
            numeofs += 1;
        }
        // sh:14  (( pos )) — loop while pos != 0.
        if pos == 0 {
            break;
        }
    }
    (numeofs, cmdpos)
}

/// `_zargs` — completion for the `zargs` function: dispatches to
/// `_arguments` (options), `_files` (input-args) or `_normal` (the
/// wrapped command) based on how many `eofstr` separators precede the
/// cursor.
pub fn _zargs(_args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_zargs");
    let mut ret = 1; // sh:3  ret=1

    // sh:3  pos=$((CURRENT)) (read here; the scan owns its own copy).
    let words = getaparam("words").unwrap_or_default();
    let current: i64 = getsparam("CURRENT")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // sh:9  eofstr=…
    let eofstr = compute_eofstr(&words);

    // sh:10-15  count separators before the cursor.
    let (numeofs, cmdpos) = scan_eofs(&words, current, &eofstr);

    // sh:16  case $numeofs in
    match numeofs {
        0 => {
            // sh:19-34  zargs option specs.
            let mut call = vec!["-S".to_string(), "-s".to_string()];
            call.extend(build_arguments());
            // By NAME (matching the `_files` call below) so `_arguments` gets
            // its own `comp_wrapper` frame (c:1556); that frame contains its
            // `compstate[restore]=''` (`_arguments.rs:1130`), which would
            // otherwise cancel the restore owed to `_zargs`' caller.
            if crate::compsys::ported::shared::call_compfn("_arguments", &call, || {
                arguments_byname(&call)
            }) == 0
            {
                ret = 0; // sh:34  && ret=0
            }
        }
        1 => {
            // sh:38  _files && ret=0
            if crate::compsys::ported::shared::call_compfn("_files", &[], || _files(&[])) == 0 {
                ret = 0;
            }
        }
        _ => {
            // sh:42  shift cmdpos words
            let drop = cmdpos.max(0) as usize;
            let mut w = words;
            if drop <= w.len() {
                w.drain(0..drop);
            } else {
                w.clear();
            }
            setaparam("words", w);
            // sh:43  (( CURRENT -= cmdpos ))
            let _ = setsparam("CURRENT", &(current - cmdpos).to_string());
            // sh:44  _normal (return value discarded; ret stays 1).
            let _ = dispatch_function_call("_normal", &[]);
        }
    }

    ret // sh:48  return ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eofstr_defaults_to_dashdash() {
        // No --eof/-e word present → default "--".
        assert_eq!(compute_eofstr(&["zargs".into(), "-x".into()]), "--");
        // `-e` / `--eof=` with an empty value also default to "--".
        assert_eq!(compute_eofstr(&["zargs".into(), "-e".into()]), "--");
        assert_eq!(compute_eofstr(&["zargs".into(), "--eof=".into()]), "--");
    }

    #[test]
    fn eofstr_from_long_and_short_forms() {
        assert_eq!(compute_eofstr(&["zargs".into(), "--eof=EOF".into()]), "EOF");
        assert_eq!(compute_eofstr(&["zargs".into(), "-eEOF".into()]), "EOF");
        // `(r)` picks the FIRST matching word, forward order.
        assert_eq!(
            compute_eofstr(&["zargs".into(), "-eFIRST".into(), "--eof=SECOND".into()]),
            "FIRST"
        );
    }

    #[test]
    fn find_eof_backward_returns_highest_match_at_or_below_begin() {
        let w: Vec<String> = ["zargs", "--", "a", "b", "--", "cmd"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // begin=6 → last "--" at or before index 6 is index 5.
        assert_eq!(find_eof_backward(&w, 6, "--"), 5);
        // begin=4 → next "--" backward is index 2.
        assert_eq!(find_eof_backward(&w, 4, "--"), 2);
        // begin=1 → nothing matches at/below index 1.
        assert_eq!(find_eof_backward(&w, 1, "--"), 0);
        // begin < 1 → 0.
        assert_eq!(find_eof_backward(&w, 0, "--"), 0);
    }

    #[test]
    fn scan_eofs_classifies_option_file_and_command_positions() {
        // 0 separators before cursor → complete zargs options.
        let w0: Vec<String> = ["zargs", "-"].iter().map(|s| s.to_string()).collect();
        let (n0, _c0) = scan_eofs(&w0, 2, "--");
        assert_eq!(n0, 0);

        // 1 separator before cursor → complete input files.
        let w1: Vec<String> = ["zargs", "--"].iter().map(|s| s.to_string()).collect();
        let (n1, c1) = scan_eofs(&w1, 3, "--");
        assert_eq!(n1, 1);
        assert_eq!(c1, 2);

        // 2 separators before cursor → complete the wrapped command.
        // words: zargs -- a b -- cmd ; cursor at position 7.
        let w2: Vec<String> = ["zargs", "--", "a", "b", "--", "cmd"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (n2, c2) = scan_eofs(&w2, 7, "--");
        assert_eq!(n2, 2);
        // cmdpos = rightmost separator before cursor = index 5.
        assert_eq!(c2, 5);
    }

    #[test]
    fn build_arguments_flattens_brace_specs() {
        let a = build_arguments();
        // 13 upstream lines, 11 of which brace-expand to 2 → 24 specs.
        assert_eq!(a.len(), 24);
        // Brace pair flattened to both forms.
        assert!(a.contains(
            &"(--eof -e)--eof=[change the end-of-input-args string from \"--\" to eof-str]"
                .to_string()
        ));
        assert!(a.contains(
            &"(--eof -e)-e+[change the end-of-input-args string from \"--\" to eof-str]"
                .to_string()
        ));
        // Non-brace specs kept verbatim.
        assert!(a.contains(&"--help[print summary and exit]".to_string()));
        assert!(a.contains(&"--version[print the version number of zargs and exit]".to_string()));
    }

    #[test]
    fn shift_command_words_matches_precommand_shape() {
        // The `*)` branch shifts cmdpos words and decrements CURRENT.
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        setaparam(
            "words",
            ["zargs", "--", "a", "b", "--", "cmd"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        );
        let _ = setsparam("CURRENT", "7");
        let _ = _zargs(&[]);
        // After the shift, words start at the wrapped command and
        // CURRENT is offset by cmdpos (5): 7 - 5 = 2.
        assert_eq!(
            getaparam("words").unwrap_or_default(),
            vec!["cmd".to_string()]
        );
        assert_eq!(getsparam("CURRENT").unwrap_or_default(), "2");
    }
}
