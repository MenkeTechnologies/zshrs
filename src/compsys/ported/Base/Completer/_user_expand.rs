//! Port of `_user_expand` from
//! `Completion/Base/Completer/_user_expand`.
//!
//! Upstream body (147 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh:13  [[ _matcher_num -gt 1 ]] && return 1
//! sh:18  if [[ "$funcstack[2]" = _prefix ]]; then
//! sh:19    word="$IPREFIX$PREFIX$SUFFIX"
//! sh:20  else
//! sh:21    word="$IPREFIX$PREFIX$SUFFIX$ISUFFIX"
//! sh:22  fi
//! sh:26  exp=("$word")
//! sh:30  zstyle -a … user-expand specs || return 1
//! sh:32  for spec in $specs; do
//! sh:33    REPLY=
//! sh:34    case $spec in
//! sh:36    ($IDENT) eval tmp='${'$spec[2,-1]'[$word]}' …  # assoc lookup
//! sh:42    (_*) reply=(); $spec $word; if reply nonempty: exp=("$reply[@]"); break
//! sh:54    esac
//! sh:55  done
//! sh:57  [[ $#exp -eq 1 && "$exp[1]" = "$word" ]] && return 1
//! sh:61  sort style
//! sh:65  add-space style -> asp
//! sh:76  a single expansion gets a `/`, ` ` or empty suffix
//! sh:87  [[ -z "$compstate[insert]" ]] -> one compadd of the whole list
//! sh:95  else:
//! sh:96    _tags all-expansions expansions original
//! sh:98    _requested expansions     -> dir / space / normal partitions
//! sh:124   _requested all-expansions -> the joined string, display-truncated
//! sh:142   _requested original       -> the untouched word
//! sh:144   compstate[insert]=menu
//! sh:147 return 0
//! ```
//!
//! sh:61-145 is `_expand` sh:158-243 with two things taken out, because a
//! user expansion is an arbitrary string rather than a path:
//!   * no `${opre}` / `${pre}` keep-prefix rewrite — sh:77 and sh:112 stat
//!     `${exp[1]}` / `"$i"` directly, where `_expand` sh:174/sh:208 stat
//!     `${exp[1]/${opre}/${pre}}`;
//!   * no `-fW "$pref"` on the three partition `compadd`s (sh:120-122 vs
//!     `_expand` sh:218-220), and correspondingly no `pref=` line.
//! The three pieces the two blocks DO share live in `shared`.
//!
//! Deliberately NOT ported, named here rather than claimed:
//!   * sh:11 `setopt localoptions nonomatch` — no `zglob` call on this path,
//!     but `$-` reads NOMATCH (`zshletters[]`, c:Src/options.c:296 maps `3`
//!     to `-NOMATCH`), so a spec function that inspects `$-` sees a
//!     difference. `_expand_with` carries a targeted guard for the same line;
//!     this port has no glob step to protect, so the flip is not reproduced.

use crate::compsys::ported::_description::_description;
use crate::compsys::ported::_requested::_requested;
use crate::compsys::ported::_tags::_tags;
use crate::compsys::ported::shared::{
    caller_is_prefix, expansion_description_args, expansion_partition_argv, right_pad_or_truncate,
    LocalScope, PM_ARRAY,
};
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getiparam, getsparam, setaparam, setsparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{isset, options, MAX_OPS, MULTIOS};
use std::path::Path;

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `_user_expand` — completer that applies user-defined expansions.
pub fn _user_expand() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_user_expand");
    // sh:13
    if getiparam("_matcher_num") > 1 {
        return 1;
    }

    // sh:15-16 — the part of the `local` line this port newly writes.
    // `dir`, `space`, `normal` and `dstr` are handed to `compadd -a` /
    // `compadd -d` BY NAME below, so they have to be real parameters, and
    // `REPLY` is clobbered per spec iteration at sh:33 and read back at sh:89.
    // Upstream declares them local, and none of them may survive into the next
    // completer.
    //
    // The rest of sh:15-16 (`exp`, `reply`, `specs`, `word`, `sort`, `suf`,
    // `asp`, `tmp`, `spec`) is left as this port already had it — those writes
    // predate this change and scoping them is its own edit.
    //
    // `expl` is in the list even though this port never writes it: sh:89/91,
    // sh:102/104 and sh:128/130 hand the NAME to `_description`, whose last
    // statement is `set -A "$name" …` (`_description` sh:100/102), so the array
    // is born at whatever level is current. Measured with a `user-expand` style
    // and `ls foo<TAB>`, /opt/homebrew/bin/zsh 5.9.2 leaves `expl` unset where
    // zshrs left `expl=(-J -default-)` behind for the next completer.
    let mut _scope = LocalScope::declare(&["dir", "space", "normal", "dstr", "expl"], PM_ARRAY);
    _scope.also(&["REPLY"], 0);

    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    let isuffix = getsparam("ISUFFIX").unwrap_or_default();
    // sh:18-22 — `_prefix` has already moved the whole `$SUFFIX` into
    // `$ISUFFIX` (`_prefix` sh:18-23) before re-running the completer list,
    // so appending `$ISUFFIX` under that caller hands the word back the
    // suffix `_prefix` just removed. `_expand` sh:22 and `_expand_alias`
    // sh:9 carry the same branch; this port had only the else arm.
    let word = if caller_is_prefix() {
        format!("{}{}{}", iprefix, prefix, suffix) // sh:19
    } else {
        format!("{}{}{}{}", iprefix, prefix, suffix, isuffix) // sh:21
    };

    let ctx = format!(":completion:{}:", getsparam("curcontext").unwrap_or_default());
    // sh:30
    let specs = lookupstyle(&ctx, "user-expand");
    if specs.is_empty() {
        return 1;
    }

    // sh:26
    let mut exp: Vec<String> = vec![word.clone()];
    for spec in &specs {
        // sh:33 — cleared per iteration, so `$REPLY` at sh:89 is whatever the
        // spec that matched left behind and nothing older.
        let _ = setsparam("REPLY", "");
        if let Some(name) = spec.strip_prefix('$') {
            // sh:36  assoc-lookup
            let arr = getaparam(name).unwrap_or_default();
            let val = arr
                .chunks(2)
                .find(|kv| kv.first().map(|k| k == &word).unwrap_or(false))
                .and_then(|kv| kv.get(1).cloned())
                .unwrap_or_default();
            if !val.is_empty() {
                exp = vec![val]; // sh:41
                break;
            }
        } else if spec.starts_with('_') {
            // sh:47-52  shell-fn dispatch — fn writes into $reply
            setaparam("reply", Vec::new());
            let _ = dispatch_function_call(spec, &[word.clone()]);
            let reply = getaparam("reply").unwrap_or_default();
            if !reply.is_empty() {
                exp = reply;
                break;
            }
        }
    }

    // sh:57
    if exp.len() == 1 && exp[0] == word {
        return 1;
    }

    // sh:61-63  sort
    let sort = lookupstyle(&ctx, "sort").join(" ");
    if matches!(sort.as_str(), "yes" | "true" | "1" | "on") {
        exp.sort();
    }

    // sh:65-72  add-space
    let mut asp = String::new();
    {
        let vals = lookupstyle(&ctx, "add-space");
        if !vals.is_empty() {
            let tmp = vals.join(" ");
            // sh:66
            if !tmp.contains("subst")
                || !word.contains('$')
                || exp.first().map(|e| e.contains('$')).unwrap_or(false)
            {
                if tmp.contains("file") {
                    asp = "file".to_string(); // sh:67
                }
                if ["yes", "true", "1", "on", "subst"]
                    .iter()
                    .any(|k| tmp.contains(k))
                {
                    asp = format!("yes{}", asp); // sh:68
                }
            }
        } else {
            asp = "file".to_string(); // sh:71
        }
    }

    // sh:15 `suf=" "` / sh:76-85 — a lone expansion gets a suffix that says
    // what it is. Unlike `_expand` sh:174 there is no `${opre}`/`${pre}`
    // rewrite: the expansion is stat'd exactly as the spec produced it.
    let mut suf = " ".to_string();
    if exp.len() == 1 {
        if Path::new(&exp[0]).is_dir() && !exp[0].ends_with('/') {
            suf = "/".to_string(); // sh:78
        } else if asp.starts_with("yes")
            || (asp.ends_with("file") && Path::new(&exp[0]).is_file())
        {
            suf = " ".to_string(); // sh:81
        } else {
            suf = String::new(); // sh:83
        }
    }

    // sh:89 / sh:102 / sh:128 — `expansions${REPLY:+: $REPLY}`: a spec
    // function may name what it expanded, and that name goes in the group
    // description.
    let reply_tail = match getsparam("REPLY").unwrap_or_default() {
        r if r.is_empty() => String::new(),
        r => format!(": {}", r),
    };

    // ---------------------------------------------------------------
    // sh:87-145 — emit.
    // ---------------------------------------------------------------

    if get_compstate_str("insert").unwrap_or_default().is_empty() {
        // sh:87-94 — nothing is going to be inserted, so one flat group of
        // every expansion is all that is wanted.
        setaparam("exp", exp.clone());
        let _ = _description(&expansion_description_args(
            &sort,
            "expansions",
            &format!("expansions{}", reply_tail),
            &word,
        ));
        let mut argv = getaparam("expl").unwrap_or_default();
        argv.extend([
            "-UQ".to_string(),
            "-qS".to_string(),
            suf.clone(),
            "-a".to_string(),
            "exp".to_string(),
        ]);
        return bin_compadd("compadd", &argv, &make_ops(), 0); // sh:94
    }

    // sh:95-145 — the normal `complete-word` path: `compstate[insert]` is
    // `unambiguous` / `menu` / `automenu`, so the three tags are offered as
    // separate groups and insertion is turned into a menu.

    // sh:96
    let _ = _tags(&[
        "all-expansions".to_string(),
        "expansions".to_string(),
        "original".to_string(),
    ]);

    // sh:98
    if !exp.is_empty() && _requested(&["expansions".to_string()]) == 0 {
        // sh:101-105
        let _ = _description(&expansion_description_args(
            &sort,
            "expansions",
            &format!("expansions{}", reply_tail),
            &word,
        ));

        // sh:106-119 — split by what each expansion IS, because the suffix
        // differs: `/` for directories, a space for files, nothing for the
        // rest. sh:111's `j="${i}"` is the whole of `_expand` sh:208 minus
        // the keep-prefix rewrite, so `i` is stat'd as it stands.
        let mut normal: Vec<String> = Vec::new();
        let mut space: Vec<String> = Vec::new();
        let mut dir: Vec<String> = Vec::new();
        for i in &exp {
            if Path::new(i).is_dir() && !i.ends_with('/') {
                dir.push(i.clone()); // sh:113
            } else if asp.starts_with("yes") || (asp.ends_with("file") && Path::new(i).is_file()) {
                space.push(i.clone()); // sh:115
            } else {
                normal.push(i.clone()); // sh:117
            }
        }

        let expl = getaparam("expl").unwrap_or_default();
        // sh:120
        if !dir.is_empty() {
            setaparam("dir", dir);
            let _ = bin_compadd(
                "compadd",
                &expansion_partition_argv(&expl, None, "/", "dir"),
                &make_ops(),
                0,
            );
        }
        // sh:121
        if !space.is_empty() {
            setaparam("space", space);
            let _ = bin_compadd(
                "compadd",
                &expansion_partition_argv(&expl, None, " ", "space"),
                &make_ops(),
                0,
            );
        }
        // sh:122
        if !normal.is_empty() {
            setaparam("normal", normal);
            let _ = bin_compadd(
                "compadd",
                &expansion_partition_argv(&expl, None, "", "normal"),
                &make_ops(),
                0,
            );
        }
    }

    // sh:124-140 — one match holding EVERY expansion, joined.
    if _requested(&["all-expansions".to_string()]) == 0 {
        // sh:127-131
        let _ = _description(&expansion_description_args(
            &sort,
            "all-expansions",
            &format!("all expansions{}", reply_tail),
            &word,
        ));

        // sh:132-137 — too wide for the terminal: show a truncated display
        // string on its own line instead of the real match.
        let mut disp: Vec<String> = Vec::new();
        let columns = getiparam("COLUMNS");
        let joined_len = exp.join(" ").chars().count() as i64; // sh:132  ${#${exp}}
        if columns > 5 && joined_len >= columns {
            setaparam(
                "dstr",
                vec![format!(
                    "{} ...",
                    right_pad_or_truncate(&exp.join(" "), (columns - 5) as usize)
                )], // sh:134
            );
            disp = vec!["-ld".to_string(), "dstr".to_string()]; // sh:133
        }

        // sh:138  [[ -o multios ]] && exp=($exp[1] $compstate[redirect]${^exp[2,-1]})
        if isset(MULTIOS) {
            let redirect = get_compstate_str("redirect").unwrap_or_default();
            let mut rebuilt: Vec<String> = Vec::new();
            if let Some(first) = exp.first() {
                rebuilt.push(first.clone());
            }
            rebuilt.extend(exp.iter().skip(1).map(|e| format!("{}{}", redirect, e)));
            exp = rebuilt;
        }

        // sh:139  compadd "$disp[@]" "$expl[@]" -UQ -qS "$suf" - "$exp"
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

    // sh:142  _requested original expl original && compadd "$expl[@]" -UQ - "$word"
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

    // sh:144 — the groups above are alternatives, not a common prefix.
    set_compstate_str("insert", "menu");

    // sh:147
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::modules::zutil::bin_zstyle;
    use crate::ported::params::setsparam;
    use crate::ported::zle::comp_h::Cmatch;

    /// Set up the `user-expand` spec, the word, and `$compstate[insert]`, run
    /// the completer, and hand back what reached `compadd` plus the resulting
    /// `$compstate[insert]`.
    ///
    /// The spec is sh:36's `$IDENT` assoc arm, which reads the named parameter
    /// and pairs it up as key/value — a flat array stands in for the assoc, as
    /// in `prefix_caller_drops_isuffix_from_the_word` above. `compadd` refuses
    /// to run outside a completion function (`bin_compadd`, c:Src/Zle/complete.c),
    /// so `INCOMPFUNC` is raised for the call.
    fn drive(word: &str, expansion: &str, insert: &str) -> (Vec<Cmatch>, String) {
        use crate::ported::zle::compcore::set_compstate_str;
        use std::sync::atomic::Ordering;

        let _ = setsparam("IPREFIX", "");
        let _ = setsparam("PREFIX", word);
        let _ = setsparam("SUFFIX", "");
        let _ = setsparam("ISUFFIX", "");
        let _ = setsparam("curcontext", "");
        crate::ported::params::setiparam("_matcher_num", 1);
        setaparam(
            "ZZQ_UE_MAP",
            vec![word.to_string(), expansion.to_string()],
        );
        let _ = bin_zstyle(
            "zstyle",
            &[
                ":completion:*".to_string(),
                "user-expand".to_string(),
                "$ZZQ_UE_MAP".to_string(),
            ],
            &make_ops(),
            0,
        );
        set_compstate_str("insert", insert);

        crate::comp_match_handles::matches_arc().lock().unwrap().clear();
        crate::ported::zle::complete::INCOMPFUNC.store(1, Ordering::Relaxed);
        let _ = _user_expand();
        crate::ported::zle::complete::INCOMPFUNC.store(0, Ordering::Relaxed);

        let arc = crate::comp_match_handles::matches_arc();
        let got = arc.lock().unwrap().clone();
        (
            got,
            crate::ported::zle::compcore::get_compstate_str("insert").unwrap_or_default(),
        )
    }

    /// sh:76-85 — a LONE expansion is given a suffix that says what it is:
    /// `/` for a directory (sh:78), a space for a file or `add-space` (sh:81),
    /// nothing otherwise (sh:83). It reaches `compadd` as `-qS "$suf"`
    /// (sh:94). None of sh:61-85 was ported, so every expansion arrived with
    /// no `-qS` at all and a directory expansion lost its trailing slash:
    ///
    /// ```text
    ///   user-expand spec: zzdir -> /usr/share/zsh, buffer `echo zzdir` + TAB
    ///     zsh     echo /usr/share/zsh/
    ///     zshrs   echo /usr/share/zsh     <- before
    ///     zshrs   echo /usr/share/zsh/    <- after
    /// ```
    #[test]
    fn lone_directory_expansion_gets_the_slash_suffix() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = crate::ported::zle::zle_main::zle_test_setup();

        // sh:77 `[[ -d ${exp[1]} ]]` — a directory that exists everywhere.
        let dir = std::env::temp_dir();
        let dir = dir.to_string_lossy().trim_end_matches('/').to_string();
        let (matches, _) = drive("zzqdir", &dir, "");
        assert_eq!(
            matches.last().and_then(|m| m.suf.clone()),
            Some("/".to_string()),
            "sh:78 — a lone directory expansion is added with `-qS /`"
        );

        // sh:83 — not a directory, not a file, and `add-space` unset means
        // sh:71's `asp=file`, so the `-f` test at sh:80 fails too: no suffix.
        let (matches, _) = drive("zzqplain", "zzqNOTAPATH", "");
        // `-qS ""` still REACHES compadd, so the match carries an empty
        // suffix rather than none at all — which is exactly what separates
        // this from the unported state, where no `-qS` was passed and `suf`
        // came back `None` for every expansion including the directory above.
        assert_eq!(
            matches.last().and_then(|m| m.suf.clone()),
            Some(String::new()),
            "sh:83 — a lone non-path expansion is added with an empty `-qS`"
        );
    }

    /// sh:95-145 — when `$compstate[insert]` is NOT empty the completer takes
    /// the second arm: the expansions are offered as separate tag groups and
    /// sh:144 turns insertion into a menu, because the groups are alternatives
    /// rather than a common prefix. The whole arm was unported, so the word
    /// was inserted with no listing and `$compstate[insert]` kept whatever
    /// `_main_complete` had put there.
    #[test]
    fn non_empty_compstate_insert_takes_the_menu_arm() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = crate::ported::zle::zle_main::zle_test_setup();

        let (_, insert) = drive("zzqmenu", "zzqEXPANSION", "automenu");
        assert_eq!(insert, "menu", "sh:144 — `compstate[insert]=menu`");

        // sh:87 — the other arm leaves it alone.
        let (_, insert) = drive("zzqmenu", "zzqEXPANSION", "");
        assert_eq!(insert, "", "sh:87 — the empty-insert arm never sets it");
    }

    #[test]
    fn returns_one_when_user_expand_unset() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "");
        let _ = setsparam("SUFFIX", "");
        let _ = setsparam("IPREFIX", "");
        let _ = setsparam("ISUFFIX", "");
        crate::ported::params::setiparam("_matcher_num", 1);
        assert_eq!(_user_expand(), 1);
    }

    /// sh:18-22 — under `_prefix` the word is `$IPREFIX$PREFIX$SUFFIX` with
    /// NO `$ISUFFIX`, because `_prefix` moved the suffix there itself
    /// (`_prefix` sh:18-23) before re-running the completer list. The port
    /// always appended `$ISUFFIX`, so it looked up the wrong key and the
    /// expansion silently did not fire.
    ///
    /// Measured on a PTY with `setopt complete_in_word`,
    /// `completer _user_expand _prefix` and the cursor three left of the end
    /// of `echo foobar`, logging `$1` from the `user-expand` spec function:
    ///
    /// ```text
    ///   pass 2 (funcstack _user_expand,_prefix,_main_complete)
    ///     zsh    word=<foo>
    ///     zshrs  word=<foobar>     <- before
    ///     zshrs  word=<foo>        <- after
    /// ```
    #[test]
    fn prefix_caller_drops_isuffix_from_the_word() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("IPREFIX", "");
        let _ = setsparam("PREFIX", "foo");
        let _ = setsparam("SUFFIX", "");
        let _ = setsparam("ISUFFIX", "bar");
        let _ = setsparam("curcontext", "");
        crate::ported::params::setiparam("_matcher_num", 1);
        // sh:36's `$IDENT` arm reads the named parameter and pairs it up as
        // key/value, so a flat array stands in for the assoc here.
        setaparam(
            "ZZQ_UE_MAP",
            vec!["foo".to_string(), "ZZQ_HIT".to_string()],
        );
        setaparam("exp", Vec::new());
        let _ = bin_zstyle(
            "zstyle",
            &[
                ":completion:*".to_string(),
                "user-expand".to_string(),
                "$ZZQ_UE_MAP".to_string(),
            ],
            &make_ops(),
            0,
        );

        // No `_prefix` frame: word is `foo` + `bar`, the map misses, `exp`
        // still holds the word and sh:57 bails.
        assert_eq!(_user_expand(), 1, "sh:21 arm must look up `foobar`");

        // Under `_prefix`: word is `foo`, the map hits, and sh:26's `exp`
        // reaches the compadd at sh:94 carrying the expansion.
        // `funcstackgetfn` (c:Src/Modules/parameter.c:627) hands the frames
        // back innermost-first, so `funcstack[2]` is index 1 — the CALLER of
        // the completer. Reproduce the live shape the PTY run showed,
        // `_user_expand,_prefix,_main_complete`, with the two frames that
        // matter; only `doshfunc` pushes these, and no executor runs here.
        {
            let mut stack = crate::ported::modules::parameter::FUNCSTACK.lock().unwrap();
            for name in ["_prefix", "_user_expand"] {
                stack.push(crate::ported::zsh_h::funcstack {
                    prev: None,
                    name: name.to_string(),
                    filename: None,
                    caller: None,
                    flineno: 0,
                    lineno: 0,
                    tp: 0,
                });
            }
        }
        let _ = _user_expand();
        {
            let mut stack = crate::ported::modules::parameter::FUNCSTACK.lock().unwrap();
            stack.pop();
            stack.pop();
        }
        assert_eq!(
            getaparam("exp").unwrap_or_default(),
            vec!["ZZQ_HIT".to_string()],
            "sh:19 arm must look up `foo`, not `foo`+$ISUFFIX"
        );
    }
}
