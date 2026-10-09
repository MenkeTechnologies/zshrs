//! Port of `_complete_help` from
//! `Completion/Base/Widget/_complete_help`.
//!
//! Upstream body (92 lines, condensed):
//! ```text
//! sh: 1  #compdef -k complete-word \C-xh
//! sh: 3  _complete_help() {
//! sh:  5    eval "$_comp_setup"
//! sh:  7    local _sort_tags=_help_sort_tags text i j k tmp
//! sh:  8    typeset -A help_funcs help_tags help_sfuncs help_styles
//! sh: 10    local -H _help_scan_funcstack="main_complete|complete|approximate|normal"
//! sh: 11    local -H _help_filter_funcstack="alternative|call_function|describe|dispatch|wanted|requested|all_labels|next_label"
//! sh: 13    {
//! sh: 14      compadd() { return 1 }
//! sh: 15      compcall() { _help_sort_tags use-compctl }
//! sh: 16      zstyle() { … capture-via-funcstack-walk … }
//! sh: 50      ${1:-_main_complete}
//! sh: 51    } always {
//! sh: 52      unfunction compadd compcall zstyle
//! sh: 53    }
//! sh: 55    for i in "${(@ok)help_funcs}"; do … zformat -a tmp '  (' … done
//! sh: 66    if [[ ${NUMERIC:-1} -ne 1 ]]; then … styles report … fi
//! sh: 78    compstate[list]='list force'
//! sh: 79    compstate[insert]=''
//! sh: 80    compadd -UX "$text[2,-1]" -n ''
//! sh: 83  _help_sort_tags() { … record help_funcs/help_tags; comptry "$@" … }
//! ```
//!
//! Diagnostic widget: runs the completion machinery with `compadd`,
//! `compcall` and `zstyle` shadowed, then prints which tags (and,
//! with a non-1 numeric argument, which styles) each completion
//! function consulted. The tag report is driven by the real
//! `_help_sort_tags` hook: `$_sort_tags` names it, so `_tags`
//! (`Base/Core/_tags`) invokes it for every tag registration during
//! `_main_complete`, and it accumulates `help_funcs`/`help_tags`.
//!
//! The shared state (`help_funcs`, `help_tags`, `help_sfuncs`,
//! `help_styles`, `_sort_tags`, `_help_scan_funcstack`,
//! `_help_filter_funcstack`) lives in ordinary shell params here — the
//! source keeps them function-local and reaches them from
//! `_help_sort_tags` by zsh dynamic scoping, which the in-process Rust
//! dispatch emulates via the global param table.
//!
//! Not ported: the `zstyle()` override at sh:16-45, and with it the
//! `help_sfuncs`/`help_styles` records behind the `NUMERIC != 1` styles
//! report (which therefore prints nothing). A shell-function `zstyle` is only
//! ever consulted where a command word is resolved (`execcmd`,
//! c:Src/exec.c:3105-3109); the native ports read styles through
//! `lookupstyle` (`src/ported/modules/zutil.rs:1071`) and `bin_zstyle`
//! directly, and neither has a shfunc-override prologue the way
//! `bin_compadd` does (`src/ported/zle/complete.rs:964`, which is why the
//! `compadd() { return 1 }` override below IS honoured). Adding the same
//! prologue/trace hook to `lookupstyle` is a change under `src/ported/`.
//!
//! `_help_sort_tags` derives `$f` from `$funcstack`, so the tag report is as
//! complete as the frames compsys functions leave on `FUNCSTACK`.

use crate::compsys::ported::shared::dispatch_action_command;
use crate::ported::modules::zutil::bin_zformat;
use crate::ported::params::{
    getaparam, gethkparam, gethparam, getsparam, setaparam, sethparam, setsparam, unsetparam,
};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zle::computil::bin_comptry;
use crate::ported::zsh_h::{options, MAX_OPS};

// sh:10 — local -H _help_scan_funcstack="main_complete|complete|approximate|normal"
const HELP_SCAN_FUNCSTACK: &str = "main_complete|complete|approximate|normal";
// sh:11 — local -H _help_filter_funcstack="alternative|call_function|describe|dispatch|wanted|requested|all_labels|next_label"
const HELP_FILTER_FUNCSTACK: &str =
    "alternative|call_function|describe|dispatch|wanted|requested|all_labels|next_label";

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

// sh:8 — `typeset -A help_funcs help_tags help_sfuncs help_styles`. The
// four are REAL associative parameters (`declare_locals(..., PM_HASHED)`
// in `_complete_help` below), so they are read through the hash accessors
// `gethkparam`/`gethparam` (c:Src/params.c:3117/3131). `getaparam` answers
// `None` for a PM_HASHED param (c:Src/params.c:3108 bails unless
// `PM_TYPE == PM_ARRAY`), and `setaparam` on one is routed to `sethparam`
// (c:3434 → c:2920 `arrhashsetfn`), so a flat `getaparam(name).chunks(2)`
// reader saw every assoc as empty and each `assoc_set` rewrote the hash
// down to its one new key. Every `^Xh` report came out blank.

/// `(key, value)` pairs of the associative array `name`, in table order.
fn assoc_pairs(name: &str) -> Vec<(String, String)> {
    let keys = gethkparam(name).unwrap_or_default();
    let vals = gethparam(name).unwrap_or_default();
    keys.into_iter().zip(vals).collect()
}

/// `$assoc[$key]` — empty when unset, as the upstream unquoted reads expand.
fn assoc_get(name: &str, key: &str) -> String {
    crate::compsys::ported::shared::assoc_get(name, key).unwrap_or_default()
}

/// `$assoc[$key]=$val`, preserving the other keys.
fn assoc_set(name: &str, key: &str, val: &str) {
    let mut pairs = assoc_pairs(name);
    match pairs.iter_mut().find(|(k, _)| k == key) {
        Some(kv) => kv.1 = val.to_string(),
        None => pairs.push((key.to_string(), val.to_string())),
    }
    let flat: Vec<String> = pairs.into_iter().flat_map(|(k, v)| [k, v]).collect();
    let _ = sethparam(name, flat);
}

/// `${(@ok)assoc}` — the keys of an associative array, sorted.
fn assoc_keys_sorted(name: &str) -> Vec<String> {
    let mut keys = gethkparam(name).unwrap_or_default();
    keys.sort();
    keys
}

/// sh:57-63 / sh:69-75 — build one context's report block into `text`.
///   `funcs_assoc`/`tags_assoc` are the pair of associative arrays
///   (`help_funcs`+`help_tags` for tags, `help_sfuncs`+`help_styles`
///   for styles); `label` is the per-context heading.
fn append_context_report(
    text: &mut String,
    funcs_assoc: &str,
    tags_assoc: &str,
    heading: impl Fn(&str) -> String,
) {
    // sh:55 / sh:68 — for i in "${(@ok)help_funcs}"; do
    for i in assoc_keys_sorted(funcs_assoc) {
        // sh:56 — text+=$'\n'"tags in context :completion:${i}:"
        text.push('\n');
        text.push_str(&heading(&i));
        // sh:57 — tmp=()
        let mut tmp: Vec<String> = Vec::new();
        // sh:58 — for j in "${(@ps.\0.)help_funcs[$i][2,-1]}"; do
        //   value is "\0f1\0f2…"; [2,-1] drops the leading NUL, then
        //   split on NUL.
        let funcs_val = assoc_get(funcs_assoc, &i);
        for j in nul_split_from_2(&funcs_val) {
            // sh:59 — tmp+=( "${(@s.,.)help_tags[${i}${j}][2,-1]}" )
            //   value is ",seg1,seg2…"; [2,-1] drops the leading comma,
            //   then split on comma.
            let tags_val = assoc_get(tags_assoc, &format!("{}{}", i, j));
            tmp.extend(comma_split_from_2(&tags_val));
        }
        // sh:60 — zformat -a tmp '  (' "$tmp[@]"
        let aligned = zformat_align("  (", &tmp);
        // sh:61 — tmp=( $'\n    '${^tmp}')' )
        let wrapped: Vec<String> = aligned.iter().map(|e| format!("\n    {})", e)).collect();
        // sh:62 — text+="${tmp}"  (array joined by IFS space)
        text.push_str(&wrapped.join(" "));
    }
}

/// `${str[2,-1]}` then `${(ps.\0.)…}` — drop the first char, split on NUL.
fn nul_split_from_2(s: &str) -> Vec<String> {
    let rest: String = s.chars().skip(1).collect();
    if rest.is_empty() {
        return Vec::new();
    }
    rest.split('\0').map(|x| x.to_string()).collect()
}

/// `${str[2,-1]}` then `${(s.,.)…}` — drop the first char, split on comma.
fn comma_split_from_2(s: &str) -> Vec<String> {
    let rest: String = s.chars().skip(1).collect();
    if rest.is_empty() {
        return Vec::new();
    }
    rest.split(',').map(|x| x.to_string()).collect()
}

/// sh:60 — `zformat -a <array> <sep> <specs…>` via the real
///   `bin_zformat` align mode; returns the aligned array.
fn zformat_align(sep: &str, specs: &[String]) -> Vec<String> {
    if specs.is_empty() {
        return Vec::new();
    }
    let tmp_name = ".complete_help.zf";
    let mut argv = vec![tmp_name.to_string(), sep.to_string()];
    argv.extend(specs.iter().cloned());
    let _ = setaparam(tmp_name, Vec::new());
    // `-a` is a parsed flag (BUILTIN optstr "afFqQ", c:Src/Modules/zutil.c:2151).
    let mut zf_ops = make_ops();
    zf_ops.ind[b'a' as usize] = 1;
    let _ = bin_zformat("zformat", &argv, &zf_ops, 0);
    let out = getaparam(tmp_name).unwrap_or_default();
    // Tear the scratch array DOWN, not merely empty it. sh:56 is `zformat
    // -a tmp '  (' "$tmp[@]"` against sh:6's `local … tmp`, so upstream
    // has no parameter left over at all; this port needs a named array to
    // hand `bin_zformat`, and `.complete_help.zf` is its own invention
    // with no upstream counterpart. Clearing it left the NAME in
    // `$parameters`, so `^Xh` put a `.complete_help.zf` in the user's
    // shell that zsh never has — measured with `ls ` + `^Xh`,
    // `${(k)parameters}` diffed after the widget returns. Same teardown
    // the `__compsys_argv` zparseopts bridge does (bug #657).
    unsetparam(tmp_name);
    out
}

/// `_complete_help` — diagnostic widget. Optional `$1` selects the
/// inner completer (default `_main_complete`).
pub fn _complete_help(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_complete_help");
    // sh:6 `local _sort_tags=_help_sort_tags text i j k tmp`, sh:7
    // `typeset -A help_funcs help_tags help_sfuncs help_styles`, sh:9-10
    // `local -H _help_scan_funcstack=…` / `local -H
    // _help_filter_funcstack=…`.
    //
    // The comment below ("emulate the dynamic scope with global params,
    // cleared at entry so a re-invocation starts fresh") described a
    // scope that was never actually entered: clearing at entry keeps a
    // SECOND `^Xh` honest but does nothing for the shell the user is left
    // holding. Measured with `ls ` + `^Xh`, `${(k)parameters}` diffed
    // after the widget returns:
    //
    //   zsh  : _sort_tags, help_funcs, help_tags, help_sfuncs,
    //          help_styles, _help_scan_funcstack, _help_filter_funcstack
    //          — all seven absent
    //   zshrs: all seven present
    //
    // PM_HASHED for sh:7's `-A`, PM_HIDE for sh:9-10's `-H`, kind 0 for
    // sh:6's bare `local`. `text`, `i`, `j`, `k` and `tmp` stay Rust-side.
    crate::compsys::ported::shared::declare_locals(&["_sort_tags"], 0);
    crate::compsys::ported::shared::declare_locals(
        &["help_funcs", "help_tags", "help_sfuncs", "help_styles"],
        crate::compsys::ported::shared::PM_HASHED,
    );
    crate::compsys::ported::shared::declare_locals(
        &["_help_scan_funcstack", "_help_filter_funcstack"],
        crate::ported::zsh_h::PM_HIDE,
    );
    // sh:5 — eval "$_comp_setup". The `$_comp_setup` snapshot is
    //   installed and evaluated by the completion entry harness (as the
    //   sibling `_complete_debug` widget also relies on); nothing to do
    //   here.

    // sh:7-8 — local _sort_tags=_help_sort_tags text …; typeset -A
    //   help_funcs help_tags help_sfuncs help_styles. These are
    //   function-local upstream; emulate the dynamic scope with global
    //   params, cleared at entry so a re-invocation starts fresh.
    for a in ["help_funcs", "help_tags", "help_sfuncs", "help_styles"] {
        let _ = sethparam(a, Vec::new());
    }
    // sh:10-11 — publish the scan/filter sets for `_help_sort_tags`.
    let _ = setsparam("_help_scan_funcstack", HELP_SCAN_FUNCSTACK);
    let _ = setsparam("_help_filter_funcstack", HELP_FILTER_FUNCSTACK);
    // sh:7 — _sort_tags=_help_sort_tags. Save the prior value so the
    //   hidden-local semantics don't leak past this widget.
    let saved_sort_tags = getsparam("_sort_tags");
    let _ = setsparam("_sort_tags", "_help_sort_tags");

    // sh:12-15 — `{ compadd() { return 1 }; compcall() { … }; zstyle() { … }`.
    //   zsh 5.9.2 defines the overrides directly; the `_shadow compadd
    //   compcall zstyle` / `_unshadow` pair is development-branch only
    //   (not in zsh-5.9.1/5.9.2), and calling it left `_shadow`'s
    //   `.shadow.depth`/`.shadow.stack` globals behind after every ^Xh.
    //   `compadd` returns 1 (suppress real matches during the diagnostic
    //   scan); the tag recording happens via `$_sort_tags=_help_sort_tags`
    //   set above. (`zstyle`'s recording override — sh:16 — is not installed;
    //   see the module header for why.)
    crate::ported::modules::parameter::setfunction("compadd", "return 1".to_string(), 0);
    crate::ported::modules::parameter::setfunction(
        "compcall",
        "_help_sort_tags use-compctl".to_string(),
        0,
    );

    // sh:50 — ${1:-_main_complete}
    let target = args
        .first()
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| "_main_complete".to_string());
    // sh:45 is a COMMAND WORD, so `dispatch_action_command`
    // (shared.rs:1407) resolves it exactly as `execcmd` does:
    // shfunc/port/plugin (c:Src/exec.c:3105-3109), then builtin, then
    // `$PATH`, then c:903's `command not found` with c:908's 127. The
    // `.unwrap_or(1)` this replaces had NO not-found arm, so a name
    // that resolved nowhere returned in silence.
    let ret = dispatch_action_command(&target, &[], 45);

    // sh:45-47 — `} always { unfunction compadd compcall zstyle }`. As in
    // 5.9.2 this removes a `compadd` the user defined before ^Xh as well.
    if let Ok(mut tab) = crate::ported::hashtable::shfunctab_lock().write() {
        tab.remove("compadd");
        tab.remove("compcall");
    }

    // sh:55-64 — tags report.
    let mut text = String::new();
    append_context_report(&mut text, "help_funcs", "help_tags", |i| {
        format!("tags in context :completion:{}:", i)
    });

    // sh:66-77 — styles report, only for a non-1 numeric argument.
    let numeric: i64 = getsparam("NUMERIC")
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    if numeric != 1 {
        // sh:67 — text+=$'\n'
        text.push('\n');
        append_context_report(&mut text, "help_sfuncs", "help_styles", |i| {
            format!("styles in context {}", i)
        });
    }

    // sh:78-79 — compstate[list]='list force'; compstate[insert]=''
    let _ = crate::ported::params::setsparam("compstate[list]", "list force");
    let _ = crate::ported::params::setsparam("compstate[insert]", "");

    // sh:80 — compadd -UX "$text[2,-1]" -n ''
    //   `$text[2,-1]` drops the leading newline accumulated by the
    //   report loops.
    let body: String = text.chars().skip(1).collect();
    let _ = bin_compadd(
        "compadd",
        &[
            "-U".to_string(),
            "-X".to_string(),
            body,
            "-n".to_string(),
            "".to_string(),
        ],
        &make_ops(),
        0,
    );

    // Restore the hidden-local `_sort_tags`.
    match saved_sort_tags {
        Some(v) => {
            let _ = setsparam("_sort_tags", &v);
        }
        None => {
            let _ = setsparam("_sort_tags", "");
        }
    }

    ret
}

/// sh:83-93 — `_help_sort_tags`. Installed as `$_sort_tags` so
/// `_tags` invokes it for each tag registration; records the
/// responsible completion function (`$f`, derived from `$funcstack`)
/// into `help_funcs`/`help_tags`, then forwards to `comptry` so normal
/// completion still proceeds.
pub fn _help_sort_tags(args: &[String]) -> i32 {
    // sh:84 — f="${${(@)${(@)funcstack[3,(i)_($~_help_scan_funcstack)]}
    //           :#(_($~_help_filter_funcstack)|\((eval|anon)\))}% *}"
    let f = derive_responsible_func();

    // sh:86 — curcontext key.
    let curcontext = getsparam("curcontext").unwrap_or_default();
    // sh:85 — ${argv} joined with space (scalar use).
    let argv_joined = args.join(" ");

    // sh:86-87 — condition:
    //   help_funcs[$curcontext] != *${f}*  ||
    //   help_tags[${curcontext}${f}] != *(${(j:|:)~argv})*
    let funcs_val = assoc_get("help_funcs", &curcontext);
    let tags_key = format!("{}{}", curcontext, f);
    let tags_val = assoc_get("help_tags", &tags_key);

    let f_recorded = funcs_val.contains(&f);
    // `*(t1|t2|…)*` — true when any tag already appears in tags_val.
    let any_tag_present = !args.is_empty() && args.iter().any(|t| tags_val.contains(t.as_str()));

    if !f_recorded || !any_tag_present {
        // sh:88-89 — [[ … != *${f}* ]] && help_funcs[$curcontext]+=$'\0'"${f}"
        if !f_recorded {
            assoc_set("help_funcs", &curcontext, &format!("{}\0{}", funcs_val, f));
        }
        // sh:90 — help_tags[${curcontext}${f}]+=",${argv}:${f}"
        assoc_set(
            "help_tags",
            &tags_key,
            &format!("{},{}:{}", tags_val, argv_joined, f),
        );
        // sh:91 — comptry "$@" 2>/dev/null
        return bin_comptry("comptry", args, &make_ops(), 0);
    }
    0
}

/// `$funcstack` — the shell special is computed from the canonical
/// `FUNCSTACK` Vec (a direct `getaparam` read misses it), innermost
/// frame first, matching `Src/Modules/parameter.c`'s `funcstackgetfn`
/// (mirrored in `subst.rs`). During a real completion the dispatched
/// compsys functions run inside `doshfunc`, so `_help_sort_tags` and
/// `_tags` are the top two frames — exactly what the `[3,…]` slice
/// skips.
fn read_funcstack() -> Vec<String> {
    crate::ported::modules::parameter::FUNCSTACK
        .lock()
        .map(|f| f.iter().rev().map(|fs| fs.name.clone()).collect())
        .unwrap_or_default()
}

/// sh:84 — derive `$f`, the completion function responsible for the
/// current tag registration, from `$funcstack`.
fn derive_responsible_func() -> String {
    let funcstack = read_funcstack();
    let len = funcstack.len();

    // `funcstack[3,(i)_($~_help_scan_funcstack)]`: 1-based slice from
    //   index 3 up to (and including) the first element matching
    //   `_(main_complete|complete|approximate|normal)`.
    let scan: Vec<String> = HELP_SCAN_FUNCSTACK
        .split('|')
        .map(|s| format!("_{}", s))
        .collect();
    // 1-based index of first scan match, or len+1 when none.
    let mut end_1based = len + 1;
    for (idx, el) in funcstack.iter().enumerate() {
        if scan.iter().any(|s| s == el) {
            end_1based = idx + 1;
            break;
        }
    }
    // 1-based [3, end] → rust [2, end).
    let start = 2usize;
    let end = end_1based.min(len);
    let slice: &[String] = if start < end {
        &funcstack[start..end]
    } else {
        &[]
    };

    // `:#(_($~_help_filter_funcstack)|\((eval|anon)\))` — drop filtered
    //   completion helpers and eval/anon frames.
    let filter: Vec<String> = HELP_FILTER_FUNCSTACK
        .split('|')
        .map(|s| format!("_{}", s))
        .collect();
    let kept: Vec<String> = slice
        .iter()
        .filter(|el| {
            let e = el.as_str();
            !(filter.iter().any(|f| f == e) || e == "(eval)" || e == "(anon)")
        })
        .cloned()
        .collect();

    // `"${…% *}"` — the outer expansion carries no `(@)`, so inside the
    // quotes the array is joined with a space FIRST and `% *` then removes
    // the shortest ` *` suffix of that one string: the last word, i.e. the
    // `_(main_complete|complete|approximate|normal)` frame the `(i)`
    // subscript stopped on. Stripping each element instead left that frame
    // in, and every tag was attributed to `_normal` too. Measured on zsh
    // 5.9.2: `c=("_x y" _z); print "${${(@)c}% *}"` prints `_x y`.
    let joined = kept.join(" ");
    match joined.rfind(' ') {
        Some(pos) => joined[..pos].to_string(),
        None => joined,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seed `FUNCSTACK` so `$funcstack` (innermost-first) equals
    /// `names`. `FUNCSTACK` stores push-order (oldest first) and is
    /// reversed on read, so push the reverse here.
    fn set_test_funcstack(names_innermost_first: &[&str]) {
        let mut stack = crate::ported::modules::parameter::FUNCSTACK.lock().unwrap();
        stack.clear();
        for name in names_innermost_first.iter().rev() {
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

    fn clear_test_funcstack() {
        crate::ported::modules::parameter::FUNCSTACK
            .lock()
            .unwrap()
            .clear();
    }

    #[test]
    /// With no executor wired the command word this path ends in resolves
    /// to no shell function, no builtin and nothing on `$PATH` — the
    /// `Src/exec.c:903` case — so it reports `command not found` and the
    /// status is c:908's 127. It used to return a silent 1.
    fn unresolvable_command_word_reports_not_found() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_complete_help(&[]), 127);
    }

    #[test]
    fn clears_help_assocs_before_dispatch() {
        // sh:8 — the four `typeset -A` assocs are function-local
        //   upstream; here they are cleared at entry so stale entries
        //   from a prior invocation never bleed into a fresh report.
        let _g = crate::test_util::global_state_lock();
        let _ = setaparam("help_funcs", vec!["ctx".to_string(), "\0stale".to_string()]);
        let _ = _complete_help(&[]);
        let after = getaparam("help_funcs").unwrap_or_default();
        assert!(
            !after.iter().any(|s| s == "\0stale"),
            "help_funcs must be cleared at widget entry"
        );
    }

    #[test]
    fn sets_sort_tags_hook_during_run_and_restores_after() {
        // sh:7 — `_sort_tags=_help_sort_tags` is the mechanism by which
        //   `_tags` routes tag registrations into the report. It is a
        //   hidden local upstream, so it must not survive the widget.
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("_sort_tags", "");
        let _ = _complete_help(&[]);
        assert_eq!(
            getsparam("_sort_tags").unwrap_or_default(),
            "",
            "_sort_tags must be restored to its prior (empty) value"
        );
    }

    #[test]
    fn help_sort_tags_records_func_and_tags() {
        // sh:84-91 — with a curcontext and a funcstack naming a
        //   completer, `_help_sort_tags` records the function into
        //   help_funcs and the tag list into help_tags.
        let _g = crate::test_util::global_state_lock();
        let _ = setaparam("help_funcs", Vec::new());
        let _ = setaparam("help_tags", Vec::new());
        let _ = setsparam("curcontext", ":completion::complete:mycmd:");
        // funcstack: [ _help_sort_tags, _tags, _files, _main_complete ]
        //   → slice[3,(i)_main_complete] = (_files _main_complete),
        //   joined, and `% *` drops the terminating `_main_complete`: `_files`.
        set_test_funcstack(&["_help_sort_tags", "_tags", "_files", "_main_complete"]);
        let _ = _help_sort_tags(&["files".to_string(), "directories".to_string()]);
        let funcs = assoc_get("help_funcs", ":completion::complete:mycmd:");
        assert!(
            funcs.contains("_files"),
            "help_funcs must record the responsible completer, got {:?}",
            funcs
        );
        let tags = assoc_get(
            "help_tags",
            &format!(":completion::complete:mycmd:{}", "_files"),
        );
        assert!(
            tags.contains("files directories"),
            "help_tags must record the tag list, got {:?}",
            tags
        );
        clear_test_funcstack();
    }

    #[test]
    /// sh:8 declares the four report tables `typeset -A`, and the widget
    /// creates them PM_HASHED. Two tag registrations from two contexts
    /// must BOTH survive into the report. With the old flat-array reader
    /// `getaparam` answered `None` for the hash, every `assoc_set` wrote the
    /// table back with just its one new key, and the final read found
    /// nothing at all: `^Xh` printed an empty report for every command.
    fn help_sort_tags_accumulates_into_real_assocs() {
        let _g = crate::test_util::global_state_lock();
        let _ = unsetparam("help_funcs");
        let _ = unsetparam("help_tags");
        let _ = sethparam("help_funcs", Vec::new());
        let _ = sethparam("help_tags", Vec::new());
        set_test_funcstack(&["_help_sort_tags", "_tags", "_arguments", "_ls", "_main_complete"]);
        let _ = setsparam("curcontext", ":complete:ls:");
        let _ = _help_sort_tags(&["argument-rest".to_string(), "options".to_string()]);
        set_test_funcstack(&[
            "_help_sort_tags",
            "_tags",
            "_files",
            "_arguments",
            "_ls",
            "_main_complete",
        ]);
        let _ = setsparam("curcontext", ":complete:ls:argument-rest");
        let _ = _help_sort_tags(&["globbed-files".to_string()]);

        assert_eq!(
            assoc_keys_sorted("help_funcs"),
            vec![":complete:ls:", ":complete:ls:argument-rest"],
            "both contexts must be recorded in the PM_HASHED help_funcs"
        );
        let mut text = String::new();
        append_context_report(&mut text, "help_funcs", "help_tags", |i| {
            format!("tags in context :completion:{}:", i)
        });
        assert_eq!(
            text,
            "\ntags in context :completion::complete:ls::\
             \n    argument-rest options  (_arguments _ls)\
             \ntags in context :completion::complete:ls:argument-rest:\
             \n    globbed-files  (_files _arguments _ls)"
        );
        let _ = unsetparam("help_funcs");
        let _ = unsetparam("help_tags");
        clear_test_funcstack();
    }

    #[test]
    fn derive_responsible_func_slices_and_filters() {
        // sh:84 — filter set drops `_dispatch`/`_wanted`; scan set
        //   terminates the slice at `_main_complete`, and `% *` on the
        //   joined words then drops that terminating frame. zsh 5.9.2 over
        //   this exact stack prints `_files`.
        let _g = crate::test_util::global_state_lock();
        set_test_funcstack(&[
            "_help_sort_tags",
            "_tags",
            "_wanted",
            "_files",
            "_main_complete",
            "_normal",
        ]);
        let f = derive_responsible_func();
        assert_eq!(f, "_files");
        clear_test_funcstack();
    }
}
