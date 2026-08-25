//! Port of `_expand` from `Completion/Base/Completer/_expand`.
//!
//! Line map, verified line-by-line against the copy vendored next to this
//! file (`Base/Completer/_expand`, 246 lines). The previous map in this
//! header was written from an outline and cited lines that do not exist in
//! the source (it put the "expansion equals the word" test at sh:40 — it is
//! sh:128 — and the `substitute` style at sh:10 — it is sh:63).
//!
//! ```text
//! sh: 10  setopt localoptions nonomatch
//! sh: 12  [[ _matcher_num -gt 1 ]] && return 1
//! sh: 14  local exp word sort expr expl subd pref suf=" " force opt asp tmp opre pre epre
//! sh: 15  local continue=0
//! sh: 17  while getopts gsco opt; do force="$force$opt"; done
//! sh: 22  word="$IPREFIX$PREFIX$SUFFIX" (+ "$ISUFFIX" unless funcstack[2] is _prefix)
//! sh: 28  word ends in a bare `$`, an unterminated `${…`, or `$UNSETNAME` -> return 1
//! sh: 36  zstyle -T … suffix
//! sh: 37    && word is `~…/…` | `…$name<sep>` | `…${…}?`
//! sh: 38    && "${(e)word}" holds NO unescaped glob metacharacter        -> return 1
//! sh: 41  zstyle -s … accept-exact tmp || [[ ! -o recexact ]] || tmp=1
//! sh: 44  if tmp is not boolean-true:
//! sh: 45    word is `~` / `~±` / `~±N` (N <= $#dirstack) / `~[…]/…`      -> return 1
//! sh: 48    word is an AMBIGUOUS `~name` or `$name`                      -> continue=1
//! sh: 51    [[ continue -eq 1 && "$tmp" != continue ]]                   -> return 1
//! sh: 56  exp=("$word")
//! sh: 62  substitute style ->
//! sh: 74    brace expansion of the quoted word (`eval exp=( … )`)
//! sh: 90    `(e)`-expand each element, re-escape whitespace
//! sh: 95  else exp=( ${exp:s/\$/$} )
//! sh:100  [[ -z "$exp" ]] && exp=("$word")
//! sh:102  subd=("$exp[@]")
//! sh:109  glob style -> unescape, `${~exp}` (tilde + filename generation), `(q)`
//! sh:126  (( $#exp )) || exp=("$subd[@]")
//! sh:128  one expansion, equal to the word with backslashes stripped     -> return 1
//! sh:133  subst-globs-only && the glob step changed nothing              -> return 1
//! sh:137  keep-prefix -> fold the expanded prefix back to the literal one
//! sh:158  sort style
//! sh:162  add-space style -> asp
//! sh:173  a single expansion gets a `/`, ` ` or empty suffix
//! sh:184  [[ -z "$compstate[insert]" ]] -> one compadd of the whole list
//! sh:192  else:
//! sh:193    _tags all-expansions expansions original
//! sh:195    _requested expansions     -> dir / space / normal partitions
//! sh:222    _requested all-expansions -> the joined string, display-truncated
//! sh:240    _requested original       -> the untouched word
//! sh:242    compstate[insert]=menu
//! sh:245  return continue
//! ```
//!
//! Every expansion above happens inside an `eval '…' 2>/dev/null`
//! (sh:82, sh:90, sh:110, sh:116, sh:145). That is not decoration: it
//! makes a rejected pattern silent AND non-fatal — the `eval` just
//! fails and its assignment never happened. [`eval_quietly`] is that
//! wrapper; the glob and `epre` steps run through it.
//!
//! Deliberately NOT ported (each would be a lie to claim, so it is named
//! here instead):
//!   * sh:108/116 `local -a orig_exp=( $exp )` splits its UNQUOTED argument
//!     on `$IFS`, so an element holding a backslash-escaped space is torn in
//!     two. This port copies `exp` verbatim instead of reproducing that.
//!   * sh:89/93 `setopt aliases` / `setopt NO_aliases` around the `eval` —
//!     no `eval` here, so nothing to guard.
//!   * sh:10 `setopt localoptions nonomatch`. `NOMATCH` is read by
//!     `zglob` (`glob.rs:1478`, the "no matches found: %s" arm at
//!     `glob.rs:1597`); the entry this port globs through is
//!     `glob_path` → `globdata_glob`, which never consults it. So the
//!     option flip has no reader on this path and is left out rather
//!     than faked. `glob_subst` reproduces the OBSERVABLE half —
//!     a pattern matching nothing comes back as itself.

use crate::compsys::ported::_description::_description;
use crate::compsys::ported::_requested::_requested;
use crate::compsys::ported::_tags::_tags;
use crate::compsys::ported::shared::{FnScope, LocalScope, PM_ARRAY};
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getiparam, getsparam, paramtab, setaparam};
use crate::ported::utils::{errflag, noerrs_lock, quotestring};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{
    isset, options, ERRFLAG_ERROR, MAX_OPS, MULTIOS, QT_BACKSLASH, RECEXACT,
};
use std::path::Path;

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `_expand` — substitution/glob expansion completer.
///
/// `compsys::router` dispatches every completer through a
/// `fn(&[String]) -> i32` shim and drops `_expand`'s arguments, so this
/// no-argument entry point is what the completer chain actually calls.
/// [`_expand_with`] carries the `getopts gsco` handling for the day the
/// router forwards `$@`.
pub fn _expand() -> i32 {
    _expand_with(&[])
}

/// `_expand "$@"` — see [`_expand`]. `args` feeds sh:17-20's
/// `while getopts gsco opt; do force="$force$opt"; done`.
pub fn _expand_with(args: &[String]) -> i32 {
    let _fn_scope = FnScope::enter("_expand");

    // sh:12  [[ _matcher_num -gt 1 ]] && return 1
    if getiparam("_matcher_num") > 1 {
        return 1;
    }

    // sh:14  the array-valued half of the `local` line. These names are
    // handed to `compadd -a` / `compadd -d` by NAME below, so they must be
    // real parameters — and must not survive the call.
    let _scope = LocalScope::declare(&["exp", "dir", "space", "normal", "dstr"], PM_ARRAY);

    // sh:15  local continue=0 — also the exit status at sh:245.
    let mut continue_: i32 = 0;

    // sh:17-20  `while getopts gsco opt; do force="$force$opt"; done`
    let mut force = String::new();
    for arg in args {
        if let Some(letters) = arg.strip_prefix('-') {
            for c in letters.chars() {
                if matches!(c, 'g' | 's' | 'c' | 'o') {
                    force.push(c);
                }
            }
        }
    }

    // sh:22-26  `$ISUFFIX` is dropped when `_prefix` is the caller.
    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    let isuffix = getsparam("ISUFFIX").unwrap_or_default();
    let word = if caller_is_prefix() {
        format!("{}{}{}", iprefix, prefix, suffix) // sh:23
    } else {
        format!("{}{}{}{}", iprefix, prefix, suffix, isuffix) // sh:25
    };

    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);

    // ---------------------------------------------------------------
    // sh:28-52 — the bail-out block. Everything here hands the word to
    // the NEXT completer instead of expanding it.
    // ---------------------------------------------------------------

    // sh:28-30
    //   [[ "$word" = *\$(|\{[^\}]#) ||
    //      ( "$word" = *\$[a-zA-Z0-9_]## && $+parameters[${word##*\$}] -eq 0 ) ]] &&
    //       return 1
    if ends_in_unterminated_dollar(&word) || ends_in_unknown_parameter(&word) {
        return 1;
    }

    // sh:36-39
    //   zstyle -T ":completion:${curcontext}:" suffix &&
    //     [[ "$word" = (\~*/*|*\$(|[=~#^+])[a-zA-Z0-9_\[\]]##[^a-zA-Z0-9_\[\]]|*\$\{*\}?) &&
    //        "${(e)word}" != (#s)(*[^\\]|)[][^*?\(\)\<\>\{\}\|]* ]] &&
    //     return 1
    //
    // "The word names a prefix (`~…/…`, `$var<sep>`, `${…}?`) and, once
    // substituted, still holds no glob metacharacter" — i.e. there is
    // nothing to expand, only a prefix to complete UNDER. This is the test
    // that keeps `cd ~/<TAB>` on `~/` and lets the file completer list the
    // directory; without it the `${~exp}` at sh:110 rewrites `~/` to
    // `$HOME/` and inserts it.
    //
    // `(e)` performs parameter/command/arithmetic substitution only — NOT
    // tilde expansion — so a leading `~` reaches the metacharacter test
    // untouched.
    if style_true_or_unset(&ctx, "suffix")
        && looks_like_prefix(&word)
        && !has_unescaped_glob_meta(
            // A `(e)` zsh cannot parse leaves the word as typed. The only
            // way that happens here is an unclosed `[` subscript — and `[`
            // is itself one of the metacharacters this test looks for, so
            // the untouched word is also the answer the test wants.
            &substitute_params(&word).unwrap_or_else(|| word.clone()),
        )
    {
        return 1;
    }

    // sh:41-42  `zstyle -s … accept-exact tmp || [[ ! -o recexact ]] || tmp=1`
    let mut tmp = {
        let vals = lookupstyle(&ctx, "accept-exact");
        if !vals.is_empty() {
            vals.join(" ")
        } else if isset(RECEXACT) {
            "1".to_string()
        } else {
            String::new()
        }
    };

    // sh:44  `if [[ "$tmp" != (yes|true|on|1) ]]; then`
    if !matches!(tmp.as_str(), "yes" | "true" | "on" | "1") {
        // sh:45-47
        //   { [[ "$word" = \~(|[-+]) ||
        //        ( "$word" = \~[-+][1-9]## && $word[3,-1] -le $#dirstack ) ||
        //      $word = \~\[*\]/* ]] && return 1 }
        if is_bare_tilde_form(&word) {
            return 1;
        }
        // sh:48-50
        //   { [[ ( "$word" = \~* && ${#userdirs[(I)…]}+${#nameddirs[(I)…]} -gt 1 ) ||
        //        ( "$word" = *\$[a-zA-Z0-9_]## && ${#parameters[(I)…]} -ne 1 ) ]] &&
        //       continue=1 }
        if is_ambiguous_prefix(&word) {
            continue_ = 1;
        }
        // sh:51
        if continue_ == 1 && tmp != "continue" {
            return 1;
        }
    }

    // ---------------------------------------------------------------
    // sh:56-118 — build the expansion list.
    // ---------------------------------------------------------------

    // sh:56  exp=("$word")
    let mut exp: Vec<String> = vec![word.clone()];

    // sh:62-96 — substitution.
    if force.contains('s') || style_true_or_unset(&ctx, "substitute") {
        // sh:74-83  brace expansion, BEFORE the `(e)` pass below.
        brace_expand_exp(&word, &mut exp);
        // sh:90-92  `(e)`-expand, then backslash-escape every space, tab and
        // newline so the array assignment cannot split on them.
        //
        // The whole line is one `eval '…' 2>/dev/null`, so an element whose
        // `(e)` expansion zsh cannot parse leaves the ENTIRE assignment
        // undone — `exp` keeps the value sh:56 gave it. `collect()` into an
        // `Option` reproduces that all-or-nothing shape: the first `None`
        // abandons the result.
        if let Some(v) = exp
            .iter()
            .map(|e| substitute_params(e).map(|x| escape_whitespace(&x)))
            .collect::<Option<Vec<String>>>()
        {
            exp = v;
        }
    } else {
        // sh:95  exp=( ${exp:s/\\\$/\$} ) — `:s` replaces the FIRST match only.
        exp = exp.iter().map(|e| e.replacen("\\$", "$", 1)).collect();
    }

    // sh:100  [[ -z "$exp" ]] && exp=("$word") — tests the JOINED array.
    if exp.join(" ").is_empty() {
        exp = vec![word.clone()];
    }

    // sh:102  subd=("$exp[@]")
    let subd = exp.clone();

    // sh:107-118 — globbing. `${~exp}` is tilde expansion FOLLOWED BY
    // filename generation, and the results are re-quoted with `(q)` so the
    // `compadd -Q` below inserts them verbatim.
    let orig_exp = exp.clone();
    let mut done_quote = false; // sh:107  integer done_quote
    if force.contains('g') || style_true_or_unset(&ctx, "glob") {
        // sh:110-111. The whole assignment is one `eval … 2>/dev/null`
        // whose exit status gates `done_quote`, so a pattern the globber
        // REJECTS (`ls *(` — an unterminated qualifier list) must be
        // silent and must count as failure, not as "expanded to itself".
        let (globbed, failed) = eval_quietly(|| {
            orig_exp
                .iter()
                .flat_map(|e| glob_subst(&unescape_ws_and_quotes(e)))
                .collect::<Vec<String>>()
        });
        if !failed && !globbed.is_empty() {
            exp = globbed
                .iter()
                .map(|s| quotestring(s, QT_BACKSLASH))
                .collect();
            done_quote = true;
        }
    }
    // sh:115-118 — no globbing, or globbing produced nothing: same
    // unescape + `(q)` pass with filename generation simply omitted.
    if !done_quote {
        exp = eval_quietly(|| {
            orig_exp
                .iter()
                .map(|e| quotestring(&unescape_ws_and_quotes(e), QT_BACKSLASH))
                .collect::<Vec<String>>()
        })
        .0;
    }

    // sh:126  (( $#exp )) || exp=("$subd[@]")
    if exp.is_empty() {
        exp = subd.clone();
    }

    // sh:128  [[ $#exp -eq 1 && "${exp[1]//\\}" = "${word//\\}"(|\(N\)) ]] && return 1
    if exp.len() == 1 {
        let got = exp[0].replace('\\', "");
        let want = word.replace('\\', "");
        if got == want || got == format!("{}(N)", want) {
            return 1;
        }
    }

    // sh:133-135  subst-globs-only: bail out when the glob step changed
    // nothing, however much the substitution step did.
    // `[[ "$subd" = "$exp"(|\(N\)) ]]` compares the two arrays JOINED.
    let subd_joined = subd.join(" ");
    let exp_joined = exp.join(" ");
    let glob_changed_nothing =
        subd_joined == exp_joined || subd_joined == format!("{}(N)", exp_joined);
    if (force.contains('o') || style_true(&ctx, "subst-globs-only")) && glob_changed_nothing {
        return 1;
    }

    // sh:137-154 — keep-prefix. `opre` is the literal prefix as typed
    // (`~`, `$HOME/`), `pre` the expanded one; folding one back to the
    // other is what turns `/home/u/Documents` back into `~/Documents`.
    let mut opre = String::new();
    let mut pre = String::new();
    let keep_prefix = {
        let vals = lookupstyle(&ctx, "keep-prefix");
        if vals.is_empty() {
            "changed".to_string() // sh:137  `|| tmp=changed`
        } else {
            vals.join(" ")
        }
    };
    // sh:139  [[ "$word" = (\~*/*|*\$*/*) && "$tmp" = (yes|true|on|1|changed) ]]
    if has_expandable_prefix(&word)
        && matches!(
            keep_prefix.as_str(),
            "yes" | "true" | "on" | "1" | "changed"
        )
    {
        // sh:140-144
        opre = if word.contains('$') {
            dollar_prefix(&word) // sh:141  ${(M)word##*\$[^/]##/}
        } else {
            word.split('/').next().unwrap_or_default().to_string() // sh:143  ${word%%/*}
        };
        // sh:145  eval 'epre=( ${(e)~opre} )' 2> /dev/null — same quiet
        // eval as sh:110/116; on failure the assignment never runs, so
        // `epre` keeps the empty value `local` gave it at sh:14.
        let epre = match eval_quietly(|| {
            substitute_params(&opre)
                .map(|e| glob_subst(&e))
                .unwrap_or_default()
        }) {
            (v, false) => v,
            (_, true) => Vec::new(),
        };
        // sh:147
        if epre.len() == 1 && !epre[0].is_empty() {
            pre = quotestring(&epre[0], QT_BACKSLASH); // sh:148  ${(q)epre[1]}
                                                       // sh:149-151
            let unchanged = format!(
                "{}{}",
                opre,
                exp[0].strip_prefix(pre.as_str()).unwrap_or(&exp[0])
            ) == word;
            if (keep_prefix != "changed" || exp.len() > 1 || !unchanged)
                && exp[0].starts_with(pre.as_str())
            {
                exp = exp
                    .iter()
                    .map(|e| format!("{}{}", opre, e.strip_prefix(pre.as_str()).unwrap_or(e)))
                    .collect();
            }
        }
        // sh:153
        if exp.len() == 1 && exp[0] == word {
            return 1;
        }
    }

    // sh:158-160  sort
    let sort = lookupstyle(&ctx, "sort").join(" ");
    if matches!(sort.as_str(), "yes" | "true" | "1" | "on") {
        exp.sort();
    }

    // sh:162-169  add-space
    let mut asp = String::new();
    {
        let vals = lookupstyle(&ctx, "add-space");
        if !vals.is_empty() {
            tmp = vals.join(" ");
            // sh:163
            if !tmp.contains("subst")
                || !word.contains('$')
                || exp.first().map(|e| e.contains('$')).unwrap_or(false)
            {
                if tmp.contains("file") {
                    asp = "file".to_string(); // sh:164
                }
                if ["yes", "true", "1", "on", "subst"]
                    .iter()
                    .any(|k| tmp.contains(k))
                {
                    asp = format!("yes{}", asp); // sh:165
                }
            }
        } else {
            asp = "file".to_string(); // sh:168
        }
    }

    // sh:14 `suf=" "` / sh:173-182 — a lone expansion gets a suffix that
    // says what it is.
    let mut suf = " ".to_string();
    if exp.len() == 1 {
        let j = replace_first(&exp[0], &opre, &pre); // sh:174  ${exp[1]/${opre}/${pre}}
        if Path::new(&j).is_dir() && !exp[0].ends_with('/') {
            suf = "/".to_string(); // sh:175
        } else if asp.starts_with("yes") || (asp.ends_with("file") && Path::new(&j).is_file()) {
            suf = " ".to_string(); // sh:178
        } else {
            suf = String::new(); // sh:180
        }
    }

    // ---------------------------------------------------------------
    // sh:184-243 — emit.
    // ---------------------------------------------------------------

    if get_compstate_str("insert").unwrap_or_default().is_empty() {
        // sh:184-191 — nothing is going to be inserted, so one flat group
        // of every expansion is all that is wanted.
        setaparam("exp", exp.clone());
        let _ = _description(&description_args(&sort, "expansions", "expansions", &word));
        let mut argv = getaparam("expl").unwrap_or_default();
        argv.extend([
            "-UQ".to_string(),
            "-qS".to_string(),
            suf.clone(),
            "-a".to_string(),
            "exp".to_string(),
        ]);
        let _ = bin_compadd("compadd", &argv, &make_ops(), 0); // sh:191
    } else {
        // sh:192-243 — the normal `complete-word` path: `compstate[insert]`
        // is `unambiguous` / `menu` / `automenu`, so the three tags are
        // offered as separate groups and insertion is turned into a menu.

        // sh:193
        let _ = _tags(&[
            "all-expansions".to_string(),
            "expansions".to_string(),
            "original".to_string(),
        ]);

        // sh:195
        if !exp.is_empty() && _requested(&["expansions".to_string()]) == 0 {
            // sh:198-202
            let _ = _description(&description_args(&sort, "expansions", "expansions", &word));

            // sh:203-216 — split by what each expansion IS, because the
            // suffix differs: `/` for directories, a space for files,
            // nothing for the rest.
            let mut normal: Vec<String> = Vec::new();
            let mut space: Vec<String> = Vec::new();
            let mut dir: Vec<String> = Vec::new();
            for i in &exp {
                let j = replace_first(i, &opre, &pre); // sh:208
                if Path::new(&j).is_dir() && !i.ends_with('/') {
                    dir.push(i.clone()); // sh:210
                } else if asp.starts_with("yes")
                    || (asp.ends_with("file") && Path::new(&j).is_file())
                {
                    space.push(i.clone()); // sh:212
                } else {
                    normal.push(i.clone()); // sh:214
                }
            }

            // sh:217  pref="${${word:#[~/]*}:+$PWD}/" — an absolute or
            // tilde word is already rooted, anything else is relative to
            // `$PWD`, and `compadd -W` needs the real directory to stat in.
            let pref = if word.starts_with('~') || word.starts_with('/') {
                "/".to_string()
            } else {
                format!("{}/", getsparam("PWD").unwrap_or_default())
            };

            let expl = getaparam("expl").unwrap_or_default();
            // sh:218
            if !dir.is_empty() {
                setaparam("dir", dir);
                let _ = bin_compadd(
                    "compadd",
                    &partition_argv(&expl, &pref, "/", "dir"),
                    &make_ops(),
                    0,
                );
            }
            // sh:219
            if !space.is_empty() {
                setaparam("space", space);
                let _ = bin_compadd(
                    "compadd",
                    &partition_argv(&expl, &pref, " ", "space"),
                    &make_ops(),
                    0,
                );
            }
            // sh:220
            if !normal.is_empty() {
                setaparam("normal", normal);
                let _ = bin_compadd(
                    "compadd",
                    &partition_argv(&expl, &pref, "", "normal"),
                    &make_ops(),
                    0,
                );
            }
        }

        // sh:222-238 — one match holding EVERY expansion, joined.
        if _requested(&["all-expansions".to_string()]) == 0 {
            // sh:225-229
            let _ = _description(&description_args(
                &sort,
                "all-expansions",
                "all expansions",
                &word,
            ));

            // sh:230-235 — too wide for the terminal: show a truncated
            // display string on its own line instead of the real match.
            let mut disp: Vec<String> = Vec::new();
            let columns = getiparam("COLUMNS");
            let joined_len = exp.join(" ").chars().count() as i64; // sh:230  ${#${exp}}
            if columns > 5 && joined_len >= columns {
                setaparam(
                    "dstr",
                    vec![format!(
                        "{} ...",
                        right_pad_or_truncate(&exp.join(" "), (columns - 5) as usize)
                    )], // sh:232
                );
                disp = vec!["-ld".to_string(), "dstr".to_string()]; // sh:231
            }

            // sh:236  [[ -o multios ]] && exp=($exp[1] $compstate[redirect]${^exp[2,-1]})
            if isset(MULTIOS) {
                let redirect = get_compstate_str("redirect").unwrap_or_default();
                let mut rebuilt: Vec<String> = Vec::new();
                if let Some(first) = exp.first() {
                    rebuilt.push(first.clone());
                }
                rebuilt.extend(exp.iter().skip(1).map(|e| format!("{}{}", redirect, e)));
                exp = rebuilt;
            }

            // sh:237  compadd "$disp[@]" "$expl[@]" -UQ -qS "$suf" - "$exp"
            let mut argv = disp;
            argv.extend(getaparam("expl").unwrap_or_default());
            argv.extend([
                "-UQ".to_string(),
                "-qS".to_string(),
                suf.clone(),
                "-".to_string(),
                exp.join(" "),
            ]);
            let _ = bin_compadd("compadd", &argv, &make_ops(), 0);
        }

        // sh:240  _requested original expl original && compadd "$expl[@]" -UQ - "$word"
        if _requested(&[
            "original".to_string(),
            "expl".to_string(),
            "original".to_string(),
        ]) == 0
        {
            let mut argv = getaparam("expl").unwrap_or_default();
            argv.extend(["-UQ".to_string(), "-".to_string(), word.clone()]);
            let _ = bin_compadd("compadd", &argv, &make_ops(), 0);
        }

        // sh:242 — the groups above are alternatives, not a common prefix.
        set_compstate_str("insert", "menu");
    }

    // sh:245  return continue
    continue_
}

// =====================================================================
// sh:22-26 — who called us
// =====================================================================

/// `[[ "$funcstack[2]" = _prefix ]]` (sh:22). `funcstack[1]` is
/// `_expand` itself, so the caller is index 1 of the innermost-first list
/// `funcstackgetfn` builds (`Src/Modules/parameter.c:627`).
fn caller_is_prefix() -> bool {
    crate::ported::modules::parameter::funcstackgetfn(std::ptr::null_mut())
        .get(1)
        .map(|n| n == "_prefix")
        .unwrap_or(false)
}

// =====================================================================
// sh:28-52 — the bail-out predicates
// =====================================================================

/// sh:28 — `[[ "$word" = *\$(|\{[^\}]#) ]]`: the word ends in a bare `$`
/// or in an unterminated `${…`. Every `$` is a candidate, because the
/// `[^\}]#` run may itself contain a later `$` (`a${b$c` matches).
fn ends_in_unterminated_dollar(word: &str) -> bool {
    word.char_indices()
        .filter(|(_, c)| *c == '$')
        .any(|(i, _)| {
            let tail = &word[i + 1..];
            tail.is_empty() || (tail.starts_with('{') && !tail.contains('}'))
        })
}

/// sh:29 — `[[ "$word" = *\$[a-zA-Z0-9_]## && $+parameters[${word##*\$}] -eq 0 ]]`:
/// the word ends in `$NAME` and no such parameter exists. Both halves key
/// off the LAST `$`, exactly as `${word##*\$}` does.
fn ends_in_unknown_parameter(word: &str) -> bool {
    let Some(i) = word.rfind('$') else {
        return false;
    };
    let name = &word[i + 1..];
    if name.is_empty() || !name.chars().all(is_param_name_char) {
        return false;
    }
    !parameter_exists(name)
}

fn is_param_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn parameter_exists(name: &str) -> bool {
    paramtab()
        .read()
        .map(|t| t.get(name).is_some())
        .unwrap_or(false)
}

/// sh:37 — `[[ "$word" = (\~*/*|*\$(|[=~#^+])[a-zA-Z0-9_\[\]]##[^a-zA-Z0-9_\[\]]|*\$\{*\}?) ]]`.
///
/// Reads as "the word carries a prefix that names a location": a tilde
/// followed by a path separator, a `$name` followed by exactly one
/// separator character, or a `${…}` followed by exactly one character.
fn looks_like_prefix(word: &str) -> bool {
    tilde_then_slash(word) || dollar_name_then_separator(word) || brace_param_then_one_char(word)
}

/// `\~*/*` — a `~` with a `/` somewhere after it.
fn tilde_then_slash(word: &str) -> bool {
    word.strip_prefix('~')
        .map(|rest| rest.contains('/'))
        .unwrap_or(false)
}

/// `*\$(|[=~#^+])[a-zA-Z0-9_\[\]]##[^a-zA-Z0-9_\[\]]` — the word ENDS with
/// `$`, an optional expansion flag, one or more name characters, and
/// exactly one character that is not a name character. `$HOME/` matches;
/// `$HOME/x` does not (two trailing non-name characters).
fn dollar_name_then_separator(word: &str) -> bool {
    let ch: Vec<char> = word.chars().collect();
    // `[a-zA-Z0-9_\[\]]` — the subscript brackets count as name characters
    // so `$a[1]/` matches.
    let is_name = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '[' | ']');
    let Some(&last) = ch.last() else {
        return false;
    };
    if is_name(last) {
        return false;
    }
    let mut i = ch.len() - 1; // the single trailing non-name character
    let run_end = i;
    while i > 0 && is_name(ch[i - 1]) {
        i -= 1;
    }
    if i == run_end {
        return false; // `##` needs at least one name character
    }
    if i > 0 && matches!(ch[i - 1], '=' | '~' | '#' | '^' | '+') {
        i -= 1; // the optional `(|[=~#^+])` flag
    }
    i > 0 && ch[i - 1] == '$'
}

/// `*\$\{*\}?` — a `${…}` whose closing brace is the second-to-last
/// character, i.e. exactly one character follows it.
fn brace_param_then_one_char(word: &str) -> bool {
    let ch: Vec<char> = word.chars().collect();
    if ch.len() < 4 {
        return false;
    }
    if ch[ch.len() - 2] != '}' {
        return false;
    }
    ch[..ch.len() - 2].iter().collect::<String>().contains("${")
}

/// sh:38 — `[[ "$s" != (#s)(*[^\\]|)[][^*?\(\)\<\>\{\}\|]* ]]`, negated.
///
/// The bracket expression opens with `]`, so the `^` in second position is
/// a LITERAL `^`, not a negation: the class is exactly
/// `] [ ^ * ? ( ) < > { } |`. `(*[^\\]|)` requires the metacharacter to sit
/// at the start of the string or after a non-backslash, i.e. to be
/// unescaped.
fn has_unescaped_glob_meta(s: &str) -> bool {
    const META: &[char] = &[']', '[', '^', '*', '?', '(', ')', '<', '>', '{', '}', '|'];
    let ch: Vec<char> = s.chars().collect();
    ch.iter()
        .enumerate()
        .any(|(i, c)| META.contains(c) && (i == 0 || ch[i - 1] != '\\'))
}

/// sh:45-47 — `~`, `~+`, `~-`, `~±N` naming an existing dirstack entry, or
/// `~[…]/…`. All of these are single locations the next completer knows
/// how to offer; expanding them here would erase the notation.
fn is_bare_tilde_form(word: &str) -> bool {
    // `\~(|[-+])`
    if matches!(word, "~" | "~-" | "~+") {
        return true;
    }
    // `\~[-+][1-9]## && $word[3,-1] -le $#dirstack`. `~-` / `~+` are the
    // only two-character leaders, and both are ASCII, so a byte prefix
    // test is safe here; the digit run is checked on the remainder.
    if let Some(rest) = word.strip_prefix("~-").or_else(|| word.strip_prefix("~+")) {
        if !rest.is_empty() && !rest.starts_with('0') && rest.chars().all(|c| c.is_ascii_digit()) {
            let n: i64 = rest.parse().unwrap_or(i64::MAX);
            let depth = getaparam("dirstack").map(|d| d.len()).unwrap_or(0) as i64;
            if n <= depth {
                return true;
            }
        }
    }
    // `\~\[*\]/*`
    if let Some(rest) = word.strip_prefix("~[") {
        if let Some(close) = rest.find(']') {
            return rest[close + 1..].starts_with('/');
        }
    }
    false
}

/// sh:48-50 — the word names a `~prefix` or `$prefix` that MORE THAN ONE
/// name still starts with, so expanding it would pick one arbitrarily.
fn is_ambiguous_prefix(word: &str) -> bool {
    // `( "$word" = \~* && ${#userdirs[(I)${word[2,-1]}*]}+${#nameddirs[(I)${word[2,-1]}*]} -gt 1 )`
    if let Some(stem) = word.strip_prefix('~') {
        // `${#assoc[(I)pat]}` is the NUMBER of keys matching `pat`, and the
        // pattern here is a literal stem plus `*`.
        let hits =
            assoc_keys_with_prefix("userdirs", stem) + assoc_keys_with_prefix("nameddirs", stem);
        if hits > 1 {
            return true;
        }
    }
    // `( "$word" = *\$[a-zA-Z0-9_]## && ${#parameters[(I)${word##*\$}*]} -ne 1 )`
    if let Some(i) = word.rfind('$') {
        let stem = &word[i + 1..];
        if !stem.is_empty() && stem.chars().all(is_param_name_char) {
            let hits = paramtab()
                .read()
                .map(|t| t.keys().filter(|k| k.starts_with(stem)).count())
                .unwrap_or(0);
            if hits != 1 {
                return true;
            }
        }
    }
    false
}

/// `${#<assoc>[(I)<prefix>*]}` — how many keys of the associative array
/// `name` start with `prefix`.
fn assoc_keys_with_prefix(name: &str, prefix: &str) -> usize {
    getaparam(name)
        .unwrap_or_default()
        .chunks(2)
        .filter_map(|kv| kv.first())
        .filter(|k| k.starts_with(prefix))
        .count()
}

// =====================================================================
// sh:62-118 — expansion
// =====================================================================

/// `eval '…' 2>/dev/null` — the wrapper EVERY expansion in this function
/// is written inside (sh:82, sh:90, sh:110, sh:116, sh:145).
///
/// Observed in zsh 5.9.2, the exact sh:110 shape with the pattern the
/// globber rejects:
///
/// ```text
/// % zsh -f -c 'setopt localoptions nonomatch; exp=("*(");
///     if eval "exp=( \${~exp} )" 2>/dev/null && (( $#exp )); then
///       print "GLOB-OK n=$#exp exp=$exp[1]"
///     else print "GLOB-FAILED n=$#exp exp=$exp[1]"; fi
///     print "still-running rc=$?"'
/// GLOB-FAILED n=1 exp=*(
/// still-running rc=0
/// ```
///
/// Three things in that output are the contract: NO diagnostic reaches
/// the terminal, the assignment inside the `eval` did not happen (`exp`
/// is still the original one-element array), and the shell carries ON —
/// the error does not escape the `eval`.
///
/// The port's globber signals the same condition by calling
/// `utils::zerr` (`glob.rs:3948` — the `parsecomplist` returned `None`
/// arm), which BOTH prints `bad pattern: *(` and raises
/// `ERRFLAG_ERROR`. Without this wrapper that message reached the screen
/// (harness row 0 read `ls *(_expand: bad pattern: *(`, shifting the
/// whole match grid), and the raised flag escaped: a probe wrapper
/// around `_expand` logged its pre-call line and never reached its
/// post-call line, i.e. the caller was cut short.
///
/// `noerrs` is the counter `zerr` itself consults before printing
/// (`utils.rs:222`), so holding it up for the duration is the port-side
/// `2>/dev/null`. Restoring `errflag` afterwards is the "carries on"
/// half — an error raised inside the `eval` must not be visible to
/// anything outside it.
///
/// Returns the closure's value and whether the eval FAILED.
fn eval_quietly<T>(f: impl FnOnce() -> T) -> (T, bool) {
    use std::sync::atomic::Ordering::SeqCst;

    // Start from a clean error bit so the flag reports THIS eval, then
    // put the caller's back exactly as it was.
    let saved_errflag = errflag.load(SeqCst);
    errflag.fetch_and(!ERRFLAG_ERROR, SeqCst);
    let saved_noerrs = *noerrs_lock().lock().unwrap();
    *noerrs_lock().lock().unwrap() = saved_noerrs + 1;

    let value = f();

    *noerrs_lock().lock().unwrap() = saved_noerrs;
    let failed = (errflag.load(SeqCst) & ERRFLAG_ERROR) != 0;
    errflag.store(saved_errflag, SeqCst);
    (value, failed)
}

/// sh:74-83 — brace expansion, the first half of the `substitute` arm.
///
/// ```text
/// if [[ ! $_comp_caller_options[ignorebraces] == on &&
///       "${#${exp}//[^\{]}" = "${#${exp}//[^\}]}" ]]; then
///   local otmp
///   tmp=${(q)word}
///   while [[ $#tmp != $#otmp ]]; do
///     otmp=$tmp
///     tmp=${tmp//(#b)\\\$\\\{(([^\{\}]|\\\\{|\\\\})#)([^\\])\\\}/…}
///   done
///   eval exp\=\( ${tmp:gs/\\{/\{/:gs/\\}/\}/} \) 2>/dev/null
/// fi
/// ```
///
/// `${(q)word}` backslash-escapes every shell-special character in the
/// word — braces and commas included — so the `eval` at sh:82 cannot do
/// anything except what the `:gs` pass hands back to it. That pass strips
/// the backslash off `\{` and `\}` only, which leaves brace EXPANSION as
/// the single active piece of syntax in the evaluated word. The fixpoint
/// loop is what keeps a `${…}` the user typed behind a literal backslash
/// out of it, by doubling those two backslashes so `:gs` skips them.
///
/// The `else` at sh:65-72 in the source is a comment, not code: the
/// commented-out one-liner it replaced expanded `${foo}` as a brace group
/// too, which is the bug this loop exists to avoid.
fn brace_expand_exp(word: &str, exp: &mut Vec<String>) {
    // sh:74  `[[ ! $_comp_caller_options[ignorebraces] == on && … ]]`
    if caller_option_on("ignorebraces") {
        return;
    }
    // sh:74  `"${#${exp}//[^\{]}" = "${#${exp}//[^\}]}"` — the count of
    // `{` and the count of `}` in the JOINED array must agree, so a
    // half-typed `ls /usr/{b` is left for the file completer.
    let joined = exp.join(" ");
    if joined.matches('{').count() != joined.matches('}').count() {
        return;
    }

    // sh:77  tmp=${(q)word}
    let mut tmp = quotestring(word, QT_BACKSLASH);

    // sh:78-81 — iterate until the length stops changing. The test is on
    // `$#tmp` vs `$#otmp`, i.e. on LENGTH, and `otmp` starts out unset, so
    // the body always runs at least once.
    let mut otmp = String::new();
    while tmp.chars().count() != otmp.chars().count() {
        otmp = tmp.clone(); // sh:79
        tmp = protect_quoted_param_braces(&tmp); // sh:80
    }

    // sh:82  `${tmp:gs/\{/\{/:gs/\}/\}/}` — two chained global string
    // substitutions, `\{` -> `{` then `\}` -> `}`. Both are plain
    // left-to-right non-overlapping replacements, which is exactly what
    // `str::replace` does.
    let unquoted_braces = tmp.replace("\\{", "{").replace("\\}", "}");

    // sh:82  `eval exp\=\( … \) 2>/dev/null`. `exp` is the array
    // `LocalScope::declare` created at sh:14, so the assignment lands on
    // the same parameter the rest of this function publishes. sh:56 has
    // already put `("$word")` in the Rust-side vector; mirror it onto the
    // parameter first so a FAILING eval leaves that value in place, which
    // is what the shell's `2>/dev/null`-swallowed parse error does.
    setaparam("exp", exp.clone()); // sh:56
    let (_, failed) = eval_quietly(|| {
        crate::ported::exec::execute_script_zsh_pipeline(&format!("exp=( {} )", unquoted_braces))
    });
    if !failed {
        *exp = getaparam("exp").unwrap_or_default();
    }
}

/// `$_comp_caller_options[<key>] == on`. `_comp_caller_options` is
/// PM_HASHED (`_main_complete` publishes it with `sethparam`), so it has
/// to be read through `gethkparam`/`gethparam` rather than `getaparam`.
fn caller_option_on(key: &str) -> bool {
    use crate::ported::params::{gethkparam, gethparam};
    let keys = gethkparam("_comp_caller_options").unwrap_or_default();
    let vals = gethparam("_comp_caller_options").unwrap_or_default();
    keys.iter()
        .position(|k| k == key)
        .and_then(|i| vals.get(i))
        .map(|v| v == "on")
        .unwrap_or(false)
}

/// sh:80 — one pass of
/// `${tmp//(#b)\$\{(([^{}]|\\{|\\})#)([^\])\}/\$\\{$match[1]$match[3]\\}}`.
///
/// The pattern reaches the matcher AFTER the shell has removed its own
/// quoting, so `\\` is a LITERAL backslash and `\$` a literal `$` — the
/// head is the four-character sequence `\`, `$`, `\`, `{`, i.e. a `${`
/// that `${(q)…}` has escaped. What follows is
///
///   * `(([^{}]|\\{|\\})#)`  — `$match[1]`: any run of characters in
///     which every `{` / `}` is preceded by TWO backslashes (an
///     already-protected brace from an earlier pass); a bare `{` or `}`
///     ends the run.
///   * `([^\])`               — `$match[3]`: one character that is not a
///     backslash, so the `\}` below cannot be borrowed from it.
///   * `\}`                    — the escaped closing brace.
///
/// `#` is greedy, so the closing `\}` is the LAST one that still leaves a
/// well-formed body; `//` then continues scanning after the match.
///
/// The replacement re-emits the same text with `\{` -> `\\{` and
/// `\}` -> `\\}`.
fn protect_quoted_param_braces(s: &str) -> String {
    let ch: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len() + 8);
    let mut i = 0usize;
    while i < ch.len() {
        match match_escaped_dollar_brace(&ch, i) {
            Some((body, tail, end)) => {
                out.push_str("\\$\\\\{"); // `\$\\{`
                out.extend(body);
                out.push(tail);
                out.push_str("\\\\}"); // `\\}`
                i = end;
            }
            None => {
                out.push(ch[i]);
                i += 1;
            }
        }
    }
    out
}

/// One match of the sh:80 pattern anchored at `i`. Returns
/// `($match[1], $match[3], index just past the match)`.
fn match_escaped_dollar_brace(ch: &[char], i: usize) -> Option<(&[char], char, usize)> {
    // `\$\{` — the escaped `${`.
    if i + 4 > ch.len() || ch[i] != '\\' || ch[i + 1] != '$' || ch[i + 2] != '\\' || ch[i + 3] != '{'
    {
        return None;
    }
    let body = i + 4;
    // `#` is greedy: walk the candidate closing `\}` from the right.
    let mut close = ch.len().saturating_sub(2);
    while close >= body + 1 {
        if ch[close] == '\\' && ch[close + 1] == '}' {
            let tail = ch[close - 1]; // `([^\])`
            if tail != '\\' && every_brace_double_escaped(&ch[body..close - 1]) {
                return Some((&ch[body..close - 1], tail, close + 2));
            }
        }
        close -= 1;
    }
    None
}

/// `(([^{}]|\\{|\\})#)` — the run is valid exactly when every `{` and
/// `}` inside it is preceded by two backslashes. Any other character is
/// admitted by the `[^{}]` alternative on its own.
fn every_brace_double_escaped(seg: &[char]) -> bool {
    seg.iter().enumerate().all(|(k, c)| {
        if *c != '{' && *c != '}' {
            return true;
        }
        k >= 2 && seg[k - 1] == '\\' && seg[k - 2] == '\\'
    })
}

/// `${(e)s}` — parameter substitution only. `(e)` does NOT tilde-expand
/// and does NOT glob, which is exactly why the sh:38 test can look at a
/// leading `~` and still see it.
///
/// Command substitution and arithmetic expansion are not covered; a word
/// carrying `$(…)` or `$((…))` comes back unchanged.
///
/// `None` means "zsh could not have performed this substitution at all".
/// The case that reaches here is a subscript that is opened and never
/// closed (`$commands[`): `Src/params.c` `getindex` rejects it as
/// `invalid subscript`, so the `${(e)…}` is a parse error rather than an
/// expansion. sh:90 wraps the whole array assignment in
/// `eval '…' 2>/dev/null`, which means the assignment simply does not
/// happen and `exp` keeps the word — which sh:128 then recognises as
/// "the expansion equals the word" and hands on to the next completer.
/// Substituting `$commands` and dropping the orphan `[` instead made
/// `echo $commands[<LEFT><TAB>` render three expansion groups where zsh
/// renders nothing.
fn substitute_params(s: &str) -> Option<String> {
    let ch: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < ch.len() {
        if ch[i] == '\\' && i + 1 < ch.len() {
            out.push(ch[i]);
            out.push(ch[i + 1]);
            i += 2;
            continue;
        }
        if ch[i] != '$' || i + 1 >= ch.len() {
            out.push(ch[i]);
            i += 1;
            continue;
        }
        let (name, next) = if ch[i + 1] == '{' {
            match ch[i + 2..].iter().position(|&c| c == '}') {
                Some(rel) => (
                    ch[i + 2..i + 2 + rel].iter().collect::<String>(),
                    i + 3 + rel,
                ),
                None => {
                    out.push('$');
                    i += 1;
                    continue;
                }
            }
        } else {
            let mut j = i + 1;
            while j < ch.len() && is_param_name_char(ch[j]) {
                j += 1;
            }
            if j == i + 1 {
                out.push('$');
                i += 1;
                continue;
            }
            // `$name[` with no closing `]` — `invalid subscript`.
            if ch.get(j) == Some(&'[') && !ch[j + 1..].contains(&']') {
                return None;
            }
            (ch[i + 1..j].iter().collect::<String>(), j)
        };
        if let Some(v) = getsparam(&name).or_else(|| std::env::var(&name).ok()) {
            out.push_str(&v);
        }
        i = next;
    }
    Some(out)
}

/// sh:90-92 — `${${(e)exp//\\[ \t\n]/ }//(#b)([ \t\n])/\\$match[1]}`.
/// Net effect: every space, tab and newline ends up backslash-escaped
/// exactly once, whether or not it already was.
fn escape_whitespace(s: &str) -> String {
    let is_ws = |c: char| matches!(c, ' ' | '\t' | '\n');
    let mut out = String::with_capacity(s.len() + 8);
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.peek() {
                Some(&w) if is_ws(w) => {
                    out.push('\\');
                    out.push(w);
                    it.next();
                }
                _ => out.push('\\'),
            }
        } else if is_ws(c) {
            out.push('\\');
            out.push(c);
        } else {
            out.push(c);
        }
    }
    out
}

/// sh:110/116 — `${exp//(#b)\\([ \t\"'\n])/$match[1]}`: drop the backslash
/// in front of a space, tab, double quote, single quote or newline, so the
/// globber sees the real character.
fn unescape_ws_and_quotes(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            if let Some(&n) = it.peek() {
                if matches!(n, ' ' | '\t' | '"' | '\'' | '\n') {
                    out.push(n);
                    it.next();
                    continue;
                }
            }
        }
        out.push(c);
    }
    out
}

/// `${~s}` — tilde expansion followed by filename generation. `nonomatch`
/// (sh:10) means a pattern that matches nothing survives as itself, so the
/// fallback is the tilde-expanded word.
fn glob_subst(s: &str) -> Vec<String> {
    let expanded = tilde_expand(s);
    if !has_glob_meta(&expanded) {
        return vec![expanded];
    }
    let hits = crate::ported::glob::glob_path(&expanded);
    if hits.is_empty() {
        vec![expanded]
    } else {
        hits
    }
}

/// The leading-`~` half of `${~s}`, via the canonical `filesubstr`
/// (`Src/subst.c:737`) so `~`, `~+`, `~-`, `~±N`, `~user` and `~[dyn]` all
/// behave as they do elsewhere. `filesubstr` dispatches on the Tilde TOKEN
/// (`Src/zsh.h:189`) the lexer produces; a word rebuilt from `$PREFIX` /
/// `$SUFFIX` carries a plain ASCII `~`, so retokenize it first.
fn tilde_expand(s: &str) -> String {
    if !s.starts_with('~') {
        return s.to_string();
    }
    let tokenized = format!("\u{98}{}", &s[1..]);
    crate::ported::subst::filesubstr(&tokenized, false).unwrap_or_else(|| s.to_string())
}

/// Does the word carry an UNESCAPED filename-generation metacharacter?
/// Used only to decide whether to run the globber at all — an ordinary
/// path must not be handed to `glob_path`, which would drop it when the
/// file does not exist.
fn has_glob_meta(s: &str) -> bool {
    let ch: Vec<char> = s.chars().collect();
    ch.iter()
        .enumerate()
        .any(|(i, c)| matches!(*c, '*' | '?' | '[' | '(') && (i == 0 || ch[i - 1] != '\\'))
}

// =====================================================================
// sh:137-154 — keep-prefix helpers
// =====================================================================

/// sh:139 — `[[ "$word" = (\~*/*|*\$*/*) ]]`: there is a prefix worth
/// folding back, i.e. a `~` or a `$` with a `/` after it.
fn has_expandable_prefix(word: &str) -> bool {
    if tilde_then_slash(word) {
        return true;
    }
    match word.find('$') {
        Some(i) => word[i..].contains('/'),
        None => false,
    }
}

/// sh:141 — `${(M)word##*\$[^/]##/}`: the LONGEST prefix of the form
/// "anything, `$`, one or more non-`/`, `/`". For `a$FOO/b/c` that is
/// `a$FOO/`, not `a$FOO/b/`, because `b` is not preceded by a `$`.
fn dollar_prefix(word: &str) -> String {
    let ch: Vec<char> = word.chars().collect();
    let mut best: Option<usize> = None;
    for (k, c) in ch.iter().enumerate() {
        if *c != '/' {
            continue;
        }
        // Walk back over the name run; it must be non-empty and led by `$`.
        let mut j = k;
        while j > 0 && ch[j - 1] != '/' && ch[j - 1] != '$' {
            j -= 1;
        }
        if j < k && j > 0 && ch[j - 1] == '$' {
            best = Some(k);
        }
    }
    match best {
        Some(k) => ch[..=k].iter().collect(),
        None => String::new(),
    }
}

/// `${s/pat/rep}` — replace the FIRST occurrence. An empty `pat` matches
/// the empty string at position 0, so the result is `s` unchanged when
/// `rep` is empty too, which is the only way this is reached with an empty
/// `$opre`.
fn replace_first(s: &str, pat: &str, rep: &str) -> String {
    if pat.is_empty() {
        return s.to_string();
    }
    s.replacen(pat, rep, 1)
}

// =====================================================================
// sh:184-243 — emit helpers
// =====================================================================

/// sh:185-189 / sh:198-202 / sh:225-229 — the same three-way
/// `_description` call, with `-V` (keep insertion order) unless the `sort`
/// style asked for a menu.
fn description_args(sort: &str, tag: &str, descr: &str, word: &str) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    if sort != "menu" {
        args.push("-V".to_string());
    }
    args.push(tag.to_string());
    args.push("expl".to_string());
    args.push(descr.to_string());
    args.push(format!("o:{}", word));
    args
}

/// sh:218-220 — `compadd "$expl[@]" -fW "$pref" -UQ -qS <suf> -a <array>`.
fn partition_argv(expl: &[String], pref: &str, suf: &str, array: &str) -> Vec<String> {
    let mut argv: Vec<String> = expl.to_vec();
    argv.extend([
        "-fW".to_string(),
        pref.to_string(),
        "-UQ".to_string(),
        "-qS".to_string(),
        suf.to_string(),
        "-a".to_string(),
        array.to_string(),
    ]);
    argv
}

/// `${(r:n:)s}` — pad on the right with spaces to `n` characters, or cut
/// to the first `n` when it is already longer.
fn right_pad_or_truncate(s: &str, n: usize) -> String {
    let len = s.chars().count();
    if len >= n {
        s.chars().take(n).collect()
    } else {
        format!("{}{}", s, " ".repeat(n - len))
    }
}

// =====================================================================
// zstyle helpers (Src/Modules/zutil.c:700-724)
// =====================================================================

/// `zstyle -T <ctx> <style>` — true when the style is UNSET, or set with a
/// boolean-true first value.
fn style_true_or_unset(ctx: &str, style: &str) -> bool {
    match lookupstyle(ctx, style).first() {
        Some(v) => matches!(v.as_str(), "true" | "yes" | "on" | "1"),
        None => true,
    }
}

/// `zstyle -t <ctx> <style>` — true only when the style is set with a
/// boolean-true first value.
fn style_true(ctx: &str, style: &str) -> bool {
    matches!(
        lookupstyle(ctx, style).first().map(|v| v.as_str()),
        Some("true" | "yes" | "on" | "1")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::{setiparam, setsparam};

    #[test]
    fn matcher_num_gt_one_returns_one() {
        let _g = crate::test_util::global_state_lock();
        setiparam("_matcher_num", 5);
        assert_eq!(_expand(), 1);
        setiparam("_matcher_num", 0);
    }

    #[test]
    fn plain_word_no_substitution_returns_one() {
        let _g = crate::test_util::global_state_lock();
        setiparam("_matcher_num", 1);
        let _ = setsparam("PREFIX", "plain");
        let _ = setsparam("SUFFIX", "");
        let _ = setsparam("IPREFIX", "");
        let _ = setsparam("ISUFFIX", "");
        assert_eq!(_expand(), 1);
    }

    /// sh:37-38 — the pair of tests that decides whether `cd ~/<TAB>`
    /// expands or is handed on. Values checked against zsh 5.9 with
    /// `setopt extendedglob`:
    ///
    /// ```text
    /// word=~/     prefix=1 meta=0  ->  return 1
    /// word=~/*    prefix=1 meta=1  ->  expand
    /// word=~/Do   prefix=1 meta=0  ->  return 1
    /// word=~      prefix=0
    /// word=$HOME/ prefix=1
    /// ```
    #[test]
    fn suffix_guard_matches_zsh() {
        assert!(looks_like_prefix("~/"));
        assert!(looks_like_prefix("~/Do"));
        assert!(looks_like_prefix("~/*"));
        assert!(looks_like_prefix("~[x]/y"));
        assert!(looks_like_prefix("$HOME/"));
        assert!(looks_like_prefix("$a[1]/"));
        assert!(looks_like_prefix("$=foo/"));
        assert!(looks_like_prefix("${foo}x"));

        // `$HOME/x` has TWO trailing non-name characters, `${foo}xy` two
        // trailing characters after the brace — neither alternative fits.
        assert!(!looks_like_prefix("~"));
        assert!(!looks_like_prefix("$HOME/x"));
        assert!(!looks_like_prefix("${foo}xy"));
        assert!(!looks_like_prefix("x$y"));
        assert!(!looks_like_prefix("a$b=c/"));

        assert!(!has_unescaped_glob_meta("~/"));
        assert!(!has_unescaped_glob_meta("~/Do"));
        assert!(has_unescaped_glob_meta("~/*"));
        assert!(has_unescaped_glob_meta("~[x]/y"));
        assert!(!has_unescaped_glob_meta("a\\*b"));
        assert!(has_unescaped_glob_meta("*"));
    }

    /// sh:28-29 — an unfinished parameter reference is never expanded.
    /// Checked against zsh 5.9: `a$`, `a${b`, `a${b$c` and an unset
    /// `a$NAME` all return 1; `${a}` and a SET `a$HOME` do not.
    #[test]
    fn unfinished_parameter_reference_bails() {
        assert!(ends_in_unterminated_dollar("a$"));
        assert!(ends_in_unterminated_dollar("a${b"));
        // The `[^\}]#` run may swallow a later `$`, so the LAST `$` is not
        // the only candidate.
        assert!(ends_in_unterminated_dollar("a${b$c"));
        assert!(!ends_in_unterminated_dollar("${a}"));
        assert!(!ends_in_unterminated_dollar("plain"));
    }

    /// sh:141 — `${(M)word##*\$[^/]##/}` stops at the first `/` that
    /// actually terminates a `$name`, however many `/` follow.
    #[test]
    fn dollar_prefix_matches_zsh() {
        assert_eq!(dollar_prefix("$HOME/x"), "$HOME/");
        assert_eq!(dollar_prefix("a$FOO/b/c"), "a$FOO/");
        assert_eq!(dollar_prefix("no/dollar/here"), "");
    }

    /// sh:230-232 — `${(r:COLUMNS-5:)exp}` pads the JOINED list, and cuts
    /// it when it is already wider. Checked against zsh 5.9 with
    /// `COLUMNS=20`: `aaa bbb ccc` becomes `aaa bbb ccc    ` and
    /// `aaaaaaaaaa bbbbbbbbbb cccccccccc` becomes `aaaaaaaaaa bbbb`.
    #[test]
    fn all_expansions_display_is_padded_then_cut() {
        assert_eq!(right_pad_or_truncate("aaa bbb ccc", 15), "aaa bbb ccc    ");
        assert_eq!(
            right_pad_or_truncate("aaaaaaaaaa bbbbbbbbbb cccccccccc", 15),
            "aaaaaaaaaa bbbb"
        );
    }

    /// sh:110/116/145 — a pattern the globber REJECTS must be silent and
    /// must count as a FAILED eval. `utils::zerr` is what the globber
    /// calls for that case (`glob.rs:3948`); under [`eval_quietly`] it
    /// must print nothing, report the failure back, and leave the
    /// caller's `errflag` untouched — `bin_eval`'s
    /// `errflag &= ~ERRFLAG_ERROR`. Without the clear, `ls *(<TAB>`
    /// aborted the shell function that called `_expand`.
    #[test]
    fn eval_quietly_reports_the_error_and_swallows_the_flag() {
        let _g = crate::test_util::global_state_lock();
        use std::sync::atomic::Ordering::SeqCst;

        errflag.store(0, SeqCst);
        let (value, failed) = eval_quietly(|| {
            crate::ported::utils::zerr("bad pattern: *(");
            7
        });
        assert_eq!(value, 7);
        assert!(failed, "a zerr inside the eval is an eval failure");
        assert_eq!(
            errflag.load(SeqCst) & ERRFLAG_ERROR,
            0,
            "the error bit must not outlive the eval"
        );

        assert!(!eval_quietly(|| ()).1, "a clean eval never reports failure");
    }

    /// sh:45-47 — `~`, `~+`, `~-` are locations, not expansions.
    #[test]
    fn bare_tilde_forms_bail() {
        assert!(is_bare_tilde_form("~"));
        assert!(is_bare_tilde_form("~+"));
        assert!(is_bare_tilde_form("~-"));
        assert!(!is_bare_tilde_form("~/"));
        assert!(!is_bare_tilde_form("~user"));
    }
}
