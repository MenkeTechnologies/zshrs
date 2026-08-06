//! Port of `_approximate` from
//! `Completion/Base/Completer/_approximate`.
//!
//! Full upstream body (121 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh: 14  [[ _matcher_num -gt 1 || "${#:-$PREFIX$SUFFIX}" -le 1 ]] && return 1
//! sh: 17  local _comp_correct …
//! sh: 21  if [[ "$1" = -a* ]]; then cfgacc="${1[3,-1]}"
//! sh: 24  elif [[ "$1" = -a ]]; then cfgacc="$2"
//! sh: 27  else zstyle -s … max-errors cfgacc || cfgacc='2 numeric'
//! sh: 32  if [[ "$cfgacc" = *numeric* && ${NUMERIC:-1} -ne 1 ]]; then …
//! sh: 41  comax="${NUMERIC:-1}"
//! sh: 44  comax="${cfgacc//[^0-9]}"
//! sh: 47  [[ "$comax" -lt 1 ]] && return 1
//! sh: 50  _tags corrections original
//! sh: 56  _shadow -s _approximate compadd
//! sh: 57  compadd() { … inject (#a${_comp_correct}) prefix into match … }
//! sh: 74  _comp_correct=1
//! sh: 76  [[ -z "$compstate[pattern_match]" ]] && compstate[pattern_match]='*'
//! sh: 79  while [[ _comp_correct -le comax ]]; do
//! sh: 84    if _complete; then …emit corrections list … break …
//! sh:117  (( _comp_correct++ ))
//! sh:118  done
//! sh:120  _unshadow; return ret
//! ```
//!
//! Approximate-match completer: shadows `compadd` to inject the
//! `(#a$n)`-error glob prefix into each match, walks `n` from 1
//! up to the max-errors style. Wraps each pass in
//! `_shadow -s _approximate compadd` + `_unshadow` so the
//! compadd-override layer's eventual install/remove (deferred) is
//! correctly scoped per-iteration.

use crate::compsys::ported::_complete::_complete;
use crate::compsys::ported::_description::_description;
use crate::compsys::ported::_requested::_requested;
use crate::compsys::ported::_shadow::{_shadow, _unshadow};
use crate::compsys::ported::_tags::_tags;
use crate::ported::modules::zutil::{lookupstyle, testforstyle};
use crate::ported::params::{getaparam, getiparam, getsparam, setaparam, setsparam, unsetparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
use crate::ported::zle::complete::{
    bin_compadd, clear_compadd_prefix_injector, set_compadd_prefix_injector, COMPADD_ARGV_SHADOW,
};
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:57 — `${argv[(I)-[a-zA-Z]#U[a-zA-Z]#]}`: does this word look like a
/// `compadd` flag bundle containing `-U`? The pattern is a leading `-`
/// followed by ASCII letters only, one of which is `U` (`-U`, `-UQ`,
/// `-Qf`… — `-Uf` yes, `-p/foo` no).
fn is_u_flag_word(a: &str) -> bool {
    match a.strip_prefix('-') {
        Some(rest) => {
            !rest.is_empty()
                && rest.bytes().all(|b| b.is_ascii_alphabetic())
                && rest.bytes().any(|b| b == b'U')
        }
        None => false,
    }
}

/// sh:66/82 — `[(I)-*[JV]]` / `[(R)-*[JV]]` on an ARRAY: a group flag word
/// is one starting with `-` and ending in `J` or `V` (`-J`, `-V`, `-1V`,
/// `-2J`). Verified against zsh subscript semantics — `-2V-default-` does
/// NOT match because it does not END in J/V.
fn is_group_flag_word(a: &str) -> bool {
    a.starts_with('-') && a.len() >= 2 && (a.ends_with('J') || a.ends_with('V'))
}

/// sh:66-67 — `$argv[1,(r)-(|-)]`: the argv slice up to and including the
/// first bare `-` / `--` end-of-options word. zsh's `(r)` range end falls
/// back to the last element when nothing matches, so an argv with no
/// separator yields the whole array.
fn argv_flag_slice(argv: &[String]) -> &[String] {
    match argv.iter().position(|a| a == "-" || a == "--") {
        Some(i) => &argv[..=i],
        None => argv,
    }
}

/// sh:54-70 — the `compadd()` shell function `_approximate` installs over
/// the builtin for the duration of one correction pass.
fn approximate_compadd_shadow(argv: &[String]) -> Option<Vec<String>> {
    // sh:57 — `local ppre="$argv[(I)-p]"` is consumed by the PREFIX
    //   injection at sh:60-64, which lives in `bin_compadd`'s injector.
    // sh:58-59 — without `-U` a candidate cannot be shorter than the number
    //   of errors we are willing to accept, so skip the whole call.
    let comp_correct = getiparam("_comp_correct");
    if !argv.iter().any(|a| is_u_flag_word(a)) {
        let len = getsparam("PREFIX").unwrap_or_default().chars().count()
            + getsparam("SUFFIX").unwrap_or_default().chars().count();
        if (len as i64) <= comp_correct {
            return None;
        }
    }

    let mut expl = getaparam("_correct_expl").unwrap_or_default();

    // sh:66-67 — when the caller names its own group, adopt that word's
    //   J/V VARIANT (sorted `-J` vs unsorted `-V`, plus any `-1`/`-2`
    //   uniquing digits) for the corrections group. Only the flag word is
    //   copied; the group NAME element of `_correct_expl` (`corrections`)
    //   is left alone.
    let correct_group = getiparam("_correct_group");
    if correct_group > 0 {
        let slice = argv_flag_slice(argv);
        // sh:66's guard tests a JOINED string, so it is true whenever some
        //   `-` is followed anywhere by a `J`/`V`; sh:67 then picks the LAST
        //   matching element, which is empty when no element qualifies.
        let repl = slice
            .iter()
            .rfind(|a| is_group_flag_word(a))
            .cloned()
            .unwrap_or_default();
        let joined = slice.join(" ");
        let guard = joined
            .rfind(['J', 'V'])
            .is_some_and(|j| joined[..j].contains('-'));
        if guard {
            if let Some(slot) = expl.get_mut(correct_group as usize - 1) {
                *slot = repl;
                // The shell assigns into `_correct_expl` itself, so the
                // adopted flag survives into the next compadd of this pass.
                setaparam("_correct_expl", expl.clone());
            }
        }
    }

    // sh:69 — `compadd@_approximate "$_correct_expl[@]" "$@"`. The
    //   explanation goes FIRST: compadd takes the first `-J`/`-V`/`-X` it
    //   sees (c:809 `if (!*sp) /* take first option only */`), so the
    //   `corrections` group and its `%e`-formatted description win over
    //   whatever group the underlying completer asked for.
    expl.extend_from_slice(argv);
    Some(expl)
}

/// `_approximate` — spell-correction completer.
pub fn _approximate(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_approximate");
    // sh:14
    if getiparam("_matcher_num") > 1 {
        return 1;
    }
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    if prefix.len() + suffix.len() <= 1 {
        return 1;
    }

    // sh:21-29  -a / max-errors style
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let cfgacc = if let Some(a) = args.first() {
        if let Some(rest) = a.strip_prefix("-a") {
            if !rest.is_empty() {
                rest.to_string()
            } else if args.len() > 1 {
                args[1].clone()
            } else {
                "2 numeric".to_string()
            }
        } else {
            lookupstyle(&format!(":completion:{}:", curcontext), "max-errors")
                .first()
                .cloned()
                .unwrap_or_else(|| "2 numeric".to_string())
        }
    } else {
        lookupstyle(&format!(":completion:{}:", curcontext), "max-errors")
            .first()
            .cloned()
            .unwrap_or_else(|| "2 numeric".to_string())
    };

    // sh:32-44
    let numeric = getiparam("NUMERIC");
    let comax: i64 = if cfgacc.contains("numeric") && numeric != 1 {
        if cfgacc.contains("not-numeric") {
            return 1;
        }
        if numeric < 1 {
            1
        } else {
            numeric
        }
    } else {
        cfgacc
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or(0)
    };
    if comax < 1 {
        return 1;
    }

    // sh:50
    let _ = _tags(&["corrections".to_string(), "original".to_string()]);

    // sh:74-77
    let opm = get_compstate_str("pattern_match").unwrap_or_default();
    if opm.is_empty() {
        set_compstate_str("pattern_match", "*");
    }

    // sh:56  `_shadow -s _approximate compadd` — wrap the entire
    //   loop so the compadd-override (when wired) installs/restores
    //   exactly once, not once per pass.
    // Bare command word at sh:53 — reach it by name. `_shadow` is overridden
    // on a real zpwr host (zsh-more-completions `more_src5/_shadow`), and the
    // `has_fpath_override` gate that honors that lives behind
    // `dispatch_function_call`, not behind a plain Rust call.
    let shargs = [
        "-s".to_string(),
        "_approximate".to_string(),
        "compadd".to_string(),
    ];
    let _ = crate::compsys::ported::shared::call_compfn("_shadow", &shargs, || _shadow(&shargs));

    let mut ret: i32 = 1;
    let mut comp_correct: i64 = 1;
    let oldcontext = curcontext.clone();
    // sh:11-13 — `_comp_correct`, `_correct_expl` and `_correct_group` are
    //   `local` to the shell function. They are read by other completion
    //   code (`_path_files` sh:860 branches on `-z "$_comp_correct"`), so
    //   they MUST be gone again once this completer returns. Without the
    //   unset at the end, one `_approximate` run silently rewired every
    //   LATER completer's path-completion behaviour for the rest of the
    //   completion.
    let pre_suf = format!("{}{}", prefix, suffix);
    let pre_suf_len = pre_suf.chars().count() as i64;
    while comp_correct <= comax {
        let _ = setsparam("_comp_correct", &comp_correct.to_string());
        // sh:77 — `${oldcontext/(#b)([^:]#:[^:]#:)/${match[1][1,-2]}-N:}`:
        //   replace the FIRST two context fields, dropping the trailing `:`
        //   of the match before appending `-N:`. For `:approximate::` that
        //   is `:approximate-1::` — the error count lands on the COMPLETER
        //   field, not on the end of the whole context (which is what the
        //   old `format!("{}-{}")` produced, so every per-pass style lookup
        //   used a context zsh never forms).
        let new_ctx = replace_completer_field(&oldcontext, comp_correct);
        let _ = setsparam("curcontext", &new_ctx);

        let _ = _description(&[
            "corrections".to_string(),
            "_correct_expl".to_string(),
            "corrections".to_string(),
            format!("e:{}", comp_correct),
            format!("o:{}", pre_suf),
        ]);

        // sh:82 — index of the group flag inside the fresh description so
        //   the compadd shadow knows which slot to overwrite at sh:66-67.
        let correct_group = getaparam("_correct_expl")
            .unwrap_or_default()
            .iter()
            .rposition(|a| is_group_flag_word(a))
            .map(|i| i + 1)
            .unwrap_or(0);
        let _ = setsparam("_correct_group", &correct_group.to_string());

        // sh:54-70 — install the `compadd` override for this pass. The
        //   PREFIX half (sh:60-64) is the injector hook; the argv half
        //   (sh:57-58 skip + sh:66-69 expl prepend) is the argv shadow.
        set_compadd_prefix_injector(format!("(#a{})", comp_correct));
        *COMPADD_ARGV_SHADOW.lock().unwrap() = Some(approximate_compadd_shadow);

        let comp_ret = _complete();

        *COMPADD_ARGV_SHADOW.lock().unwrap() = None;
        clear_compadd_prefix_injector();

        if comp_ret == 0 {
            // sh:85-87  insert-unambiguous?
            let unambig = get_compstate_str("unambiguous").unwrap_or_default();
            if testforstyle(&format!(":completion:{}:", new_ctx), "insert-unambiguous") == 0
                && unambig.chars().count() >= pre_suf.chars().count()
            {
                set_compstate_str("pattern_insert", "unambiguous");
            } else if _requested(&["original".to_string()]) == 0 {
                // sh:88-90
                let nm: i64 = get_compstate_str("nmatches")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                if nm > 1 || testforstyle(&format!(":completion:{}:", new_ctx), "original") == 0 {
                    // sh:93
                    let _ = _description(&[
                        "-V".to_string(),
                        "original".to_string(),
                        "expl".to_string(),
                        "original".to_string(),
                    ]);
                    // sh:95 — `builtin compadd`, i.e. deliberately NOT the
                    //   override: the original string is added verbatim.
                    let expl = getaparam("expl").unwrap_or_default();
                    let mut compadd_argv = expl;
                    compadd_argv.push("-U".to_string());
                    compadd_argv.push("-Q".to_string());
                    compadd_argv.push("-".to_string());
                    compadd_argv.push(pre_suf.clone());
                    let _ = bin_compadd("compadd", &compadd_argv, &make_ops(), 0);

                    // sh:100-101
                    let list = get_compstate_str("list").unwrap_or_default();
                    if !list.starts_with("list") {
                        set_compstate_str("list", format!("{} force", list).trim());
                    }
                }
            }
            // sh:103
            set_compstate_str("pattern_match", &opm);
            ret = 0;
            break;
        }

        // sh:109 — one more error than there are characters can never
        //   correct anything, so stop early instead of running the
        //   remaining (expensive) passes.
        if pre_suf_len <= comp_correct + 1 {
            break;
        }
        // sh:110
        comp_correct += 1;
    }

    // sh:114  `_unshadow` — restore the compadd entry. Bare command word;
    // paired with the `_shadow` call above, so it must go by name too or the
    // user's `_shadow` would be torn down by the port's `_unshadow`.
    let _ = crate::compsys::ported::shared::call_compfn("_unshadow", &[], _unshadow);

    // sh:13 — drop the function-locals (see the note above the loop).
    let _ = unsetparam("_comp_correct");
    let _ = unsetparam("_correct_expl");
    let _ = unsetparam("_correct_group");
    let _ = setsparam("curcontext", &oldcontext);

    if ret != 0 {
        // sh:119 — the failure path restores pattern_match too; only the
        //   success path did so before, so a failed `_approximate` left
        //   `compstate[pattern_match]` at `*` for every later completer.
        set_compstate_str("pattern_match", &opm);
    }
    ret
}

/// sh:77 — `${curcontext/(#b)([^:]#:[^:]#:)/${match[1][1,-2]}-N:}`.
/// Rewrites the completer field of a `:completer:command:argument`
/// context to carry the error count (`:approximate:` → `:approximate-1:`).
fn replace_completer_field(ctx: &str, n: i64) -> String {
    // `([^:]#:[^:]#:)` — two colon-terminated (possibly empty) fields; the
    // replacement drops the second field's colon and re-adds it after `-N`.
    match ctx.char_indices().filter(|(_, c)| *c == ':').nth(1) {
        Some((second, _)) => format!("{}-{}:{}", &ctx[..second], n, &ctx[second + 1..]),
        // Fewer than two fields: zsh's substitution finds no match and
        // leaves the context untouched.
        None => ctx.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_input_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "a");
        let _ = setsparam("SUFFIX", "");
        assert_eq!(_approximate(&[]), 1);
    }

    /// sh:77 — the error count replaces the COMPLETER field, it is not
    /// appended to the whole context.
    #[test]
    fn completer_field_carries_the_error_count() {
        assert_eq!(
            replace_completer_field(":approximate::", 1),
            ":approximate-1::"
        );
        assert_eq!(replace_completer_field(":correct:cd:", 2), ":correct-2:cd:");
        // Fewer than two fields: zsh's substitution does not match.
        assert_eq!(replace_completer_field(":approximate", 1), ":approximate");
    }

    /// sh:57 `-[a-zA-Z]#U[a-zA-Z]#` vs sh:66 `-*[JV]` — two different
    /// patterns that both look like "a dash flag" but are not.
    #[test]
    fn flag_word_patterns_match_the_shell_globs() {
        assert!(is_u_flag_word("-U"));
        assert!(is_u_flag_word("-QU"));
        assert!(is_u_flag_word("-Uf"));
        assert!(!is_u_flag_word("-Qf"));
        // `-U` pasted onto a non-letter payload is not the flag pattern.
        assert!(!is_u_flag_word("-p/Users"));
        assert!(!is_u_flag_word("plain"));

        assert!(is_group_flag_word("-J"));
        assert!(is_group_flag_word("-V"));
        assert!(is_group_flag_word("-1V"));
        // Must END in J/V: a pasted group name disqualifies the word.
        assert!(!is_group_flag_word("-2V-default-"));
        assert!(!is_group_flag_word("-X"));
    }

    /// sh:66-67 — the slice stops at the first bare `-`/`--`, so a group
    /// flag among the positional MATCHES must not be picked up.
    #[test]
    fn group_flag_slice_stops_at_end_of_options() {
        let argv: Vec<String> = ["-J", "files", "-", "-V", "x"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let slice = argv_flag_slice(&argv);
        assert_eq!(slice.len(), 3);
        assert_eq!(slice.iter().rfind(|a| is_group_flag_word(a)).unwrap(), "-J");
        // No separator at all: zsh's `(r)` range end falls back to the last
        // element, so the whole array is searched.
        let argv2: Vec<String> = ["-1V", "files"].iter().map(|s| s.to_string()).collect();
        assert_eq!(argv_flag_slice(&argv2).len(), 2);
    }

    /// sh:69 — the corrections description is PREPENDED so compadd's
    /// first-wins flag parsing (c:809) makes `corrections` the group, and
    /// sh:66-67 copies only the caller's J/V variant into that slot.
    #[test]
    fn shadow_prepends_expl_and_adopts_the_callers_group_variant() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "abcdef");
        let _ = setsparam("SUFFIX", "");
        let _ = setsparam("_comp_correct", "1");
        let _ = setsparam("_correct_group", "1");
        setaparam(
            "_correct_expl",
            vec!["-J".into(), "corrections".into(), "-X".into(), "fmt".into()],
        );
        let argv: Vec<String> = ["-1V", "local-directories", "-", "foo"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let out = approximate_compadd_shadow(&argv).expect("call must not be swallowed");
        assert_eq!(
            out,
            vec![
                "-1V",
                "corrections",
                "-X",
                "fmt",
                "-1V",
                "local-directories",
                "-",
                "foo"
            ]
        );
        let _ = unsetparam("_correct_expl");
        let _ = unsetparam("_correct_group");
        let _ = unsetparam("_comp_correct");
    }

    /// sh:58 — a word no longer than the accepted error count can only
    /// produce noise, so the whole compadd is swallowed unless `-U` says
    /// the caller does not want matching at all.
    #[test]
    fn shadow_swallows_calls_that_cannot_beat_the_error_count() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "ab");
        let _ = setsparam("SUFFIX", "");
        let _ = setsparam("_comp_correct", "2");
        let _ = setsparam("_correct_group", "0");
        setaparam("_correct_expl", vec!["-J".into(), "corrections".into()]);

        let plain: Vec<String> = ["-", "foo"].iter().map(|s| s.to_string()).collect();
        assert!(approximate_compadd_shadow(&plain).is_none());

        // sh:57's `-U` escape hatch: unmatched adds still go through.
        let unmatched: Vec<String> = ["-U", "-", "foo"].iter().map(|s| s.to_string()).collect();
        assert!(approximate_compadd_shadow(&unmatched).is_some());

        let _ = unsetparam("_correct_expl");
        let _ = unsetparam("_correct_group");
        let _ = unsetparam("_comp_correct");
    }
}
