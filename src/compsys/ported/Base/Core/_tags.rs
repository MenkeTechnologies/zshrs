//! Port of `_tags` from `Completion/Base/Core/_tags`.
//!
//! Full upstream body (67 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local prev
//! sh: 4
//! sh: 5  # A `--' as the first argument says that we should tell comptags to use
//! sh: 6  # the preceding function nesting level. This is only documented here because
//! sh: 7  # if everything goes well, users won't have to worry about it and should
//! sh: 8  # not mess with it.
//! sh: 9
//! sh:10  if [[ "$1" = -- ]]; then
//! sh:11    prev=-
//! sh:12    shift
//! sh:13  fi
//! sh:14
//! sh:15  if (( $# )); then
//! sh:16
//! sh:17    # We have arguments: the tags supported in this context.
//! sh:18
//! sh:19    local curcontext="$curcontext" order tag nodef tmp
//! sh:20
//! sh:21    if [[ "$1" = -C?* ]]; then
//! sh:22      curcontext="${curcontext%:*}:${1[3,-1]}"
//! sh:23      shift
//! sh:24    elif [[ "$1" = -C ]]; then
//! sh:25      curcontext="${curcontext%:*}:${2}"
//! sh:26      shift 2
//! sh:27    fi
//! sh:28
//! sh:29    [[ "$1" = -(|-) ]] && shift
//! sh:30
//! sh:31    zstyle -a ":completion:${curcontext}:" group-order order &&
//! sh:32        compgroups "$order[@]"
//! sh:33
//! sh:34    # Set and remember offered tags.
//! sh:35
//! sh:36    comptags "-i$prev" "$curcontext" "$@"
//! sh:37
//! sh:38    # Sort the tags.
//! sh:39
//! sh:40    if [[ -n "$_sort_tags" ]]; then
//! sh:41      "$_sort_tags" "$@"
//! sh:42    else
//! sh:43      zstyle -a ":completion:${curcontext}:" tag-order order ||
//! sh:44          (( ! ${@[(I)options]} )) ||
//! sh:45          order=('(|*-)argument-* (|*-)option[-+]* values' options)
//! sh:46
//! sh:47      for tag in $order; do
//! sh:48        case $tag in
//! sh:49        -)     nodef=yes;;
//! sh:50        \!*)   comptry "${(@)argv:#(${(j:|:)~${=~tag[2,-1]}})}";;
//! sh:51        ?*)    comptry -m "$tag";;
//! sh:52        esac
//! sh:53      done
//! sh:54
//! sh:55      [[ -z "$nodef" ]] && comptry "$@"
//! sh:56  fi
//! sh:57
//! sh:58    # Return non-zero if at least one set of tags should be used.
//! sh:59
//! sh:60    comptags "-T$prev"
//! sh:61
//! sh:62    return
//! sh:63  fi
//! sh:64
//! sh:65  # The other mode: switch to the next set of tags.
//! sh:66
//! sh:67  comptags "-N$prev"
//! ```
//!
//! Calls the real `bin_comptags` / `bin_comptry` / `bin_compgroups` in
//! `src/ported/zle/computil.rs` and the real `lookupstyle` in
//! `src/ported/modules/zutil.rs`. `$_sort_tags` hook fires via
//! `crate::ported::exec_hooks::dispatch_function_call` when set.

use crate::ported::exec_hooks::dispatch_function_call;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::getsparam;
use crate::ported::zle::computil::{bin_compgroups, bin_comptags, bin_comptry};
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `_tags` — register / iterate completion tag sets for the current
/// context. Returns the underlying `comptags` exit code:
///   * with args (registration mode): `comptags -T$prev` (0 → "at
///     least one tag set should be tried").
///   * without args (next-set mode): `comptags -N$prev`.
pub fn _tags(args: &[String]) -> i32 {
    // sh:3  local prev
    // sh:10-13 — `--` as first arg sets `prev = "-"` (the suffix that
    //   becomes `-i-` / `-T-` / `-N-`, telling comptags to use the
    //   preceding nesting level).
    let (prev, mut argv): (&str, Vec<String>) = if !args.is_empty() && args[0] == "--" {
        ("-", args[1..].to_vec())
    } else {
        ("", args.to_vec())
    };

    // sh:15  if (( $# ))
    if argv.is_empty() {
        // sh:67  comptags "-N$prev"
        return bin_comptags("comptags", &[format!("-N{}", prev)], &make_ops(), 0);
    }

    // sh:19  local curcontext="$curcontext"
    //   Snapshot the shell-side curcontext. `-C` flag overrides for the
    //   duration of this call without leaking out (only argv-based
    //   passthrough; shell-side `$curcontext` is not mutated).
    let mut curcontext = getsparam("curcontext").unwrap_or_default();

    // sh:21-27 — `-C foo` / `-Cfoo` flag rewrites curcontext's last
    //   `:`-separated field.
    if !argv.is_empty() {
        let a0 = &argv[0];
        if a0.starts_with("-C") && a0.len() > 2 {
            // sh:22  curcontext="${curcontext%:*}:${1[3,-1]}"
            let head = match curcontext.rfind(':') {
                Some(i) => &curcontext[..i],
                None => &curcontext[..],
            };
            curcontext = format!("{}:{}", head, &a0[2..]);
            argv.remove(0);
        } else if a0 == "-C" && argv.len() >= 2 {
            // sh:25  curcontext="${curcontext%:*}:${2}"
            let head = match curcontext.rfind(':') {
                Some(i) => &curcontext[..i],
                None => &curcontext[..],
            };
            curcontext = format!("{}:{}", head, &argv[1]);
            argv.drain(..2);
        }
    }

    // sh:29  [[ "$1" = -(|-) ]] && shift
    //   Strip a leading `-` or `--` marker (end-of-flags sentinel).
    if !argv.is_empty() && (argv[0] == "-" || argv[0] == "--") {
        argv.remove(0);
    }

    // sh:31  zstyle -a ":completion:${curcontext}:" group-order order
    // sh:32      compgroups "$order[@]"
    let ctx = format!(":completion:{}:", curcontext);
    let order = lookupstyle(&ctx, "group-order");
    if !order.is_empty() {
        let _ = bin_compgroups("compgroups", &order, &make_ops(), 0);
    }

    // sh:36  comptags "-i$prev" "$curcontext" "$@"
    let mut comptags_i: Vec<String> = vec![format!("-i{}", prev), curcontext.clone()];
    comptags_i.extend(argv.iter().cloned());
    let _ = bin_comptags("comptags", &comptags_i, &make_ops(), 0);

    // sh:40  if [[ -n "$_sort_tags" ]]; then
    let sort_hook = getsparam("_sort_tags").unwrap_or_default();
    if !sort_hook.is_empty() {
        // sh:41  "$_sort_tags" "$@"
        //   User-defined sort hook; dispatch via the exec hook
        //   (returns None when no executor context is wired — in that
        //   case fall through to the default sort path so comptags
        //   state stays consistent).
        if dispatch_function_call(&sort_hook, &argv).is_none() {
            run_default_sort(&ctx, &argv);
        }
    } else {
        run_default_sort(&ctx, &argv);
    }

    // sh:60  comptags "-T$prev"
    bin_comptags("comptags", &[format!("-T{}", prev)], &make_ops(), 0)
}

/// sh:43-55 — default sort path: query `tag-order` zstyle, fall back to
/// the `options`-aware default when none, then iterate each tag spec
/// dispatching `comptry` per case.
fn run_default_sort(ctx: &str, argv: &[String]) {
    // sh:43  zstyle -a ":completion:${curcontext}:" tag-order order
    // sh:44      (( ! ${@[(I)options]} ))
    // sh:45      order=('(|*-)argument-* (|*-)option[-+]* values' options)
    let mut order = lookupstyle(ctx, "tag-order");
    if order.is_empty() {
        // tag-order unset: if argv contains "options", install the
        // default group order — otherwise leave empty (no default
        // sorting; the trailing `comptry "$@"` handles emission).
        if argv.iter().any(|a| a == "options") {
            order = vec![
                "(|*-)argument-* (|*-)option[-+]* values".to_string(),
                "options".to_string(),
            ];
        }
    }

    // sh:46  for tag in $order; do
    //   `$order` is iterated as WORDS (zsh splits on IFS), so a single
    //   element like "(|*-)argument-* (|*-)option[-+]* values" yields
    //   three iterations.
    let mut nodef = false;
    for entry in &order {
        for tag in entry.split_whitespace() {
            // sh:48-52
            if tag == "-" {
                // sh:49  -)     nodef=yes
                nodef = true;
            } else if let Some(rest) = tag.strip_prefix('!') {
                // sh:50  \!*)   comptry "${(@)argv:#(${(j:|:)~${=~tag[2,-1]}})}"
                //   Split `rest` on whitespace into patterns; filter
                //   argv keeping ONLY entries that don't match ANY of
                //   the patterns; pass filtered list to comptry.
                let pats: Vec<&str> = rest.split_whitespace().collect();
                let filtered: Vec<String> = argv
                    .iter()
                    .filter(|a| !pats.iter().any(|p| zsh_glob_match(p, a)))
                    .cloned()
                    .collect();
                let _ = bin_comptry("comptry", &filtered, &make_ops(), 0);
            } else if !tag.is_empty() {
                // sh:51  ?*)    comptry -m "$tag"
                let _ = bin_comptry(
                    "comptry",
                    &["-m".to_string(), tag.to_string()],
                    &make_ops(),
                    0,
                );
            }
        }
    }

    // sh:55  [[ -z "$nodef" ]] && comptry "$@"
    if !nodef {
        let _ = bin_comptry("comptry", argv, &make_ops(), 0);
    }
}

/// Thin wrapper for zsh-style glob match used in `sh:50` argv
/// filtering. Compiles `pat` via `patcompile` and tests `s` with
/// `pattry`, mirroring the C `xpandredir` / `pattry` flow.
fn zsh_glob_match(pat: &str, s: &str) -> bool {
    // PAT_HEAPDUP=0; permanent prog OK for a one-shot match.
    if let Some(prog) = crate::ported::pattern::patcompile(&{ let mut __pat_tok = (pat).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, 0, None) {
        crate::ported::pattern::pattry(&prog, s)
    } else {
        // Invalid pattern: fall back to literal compare so a broken
        // tag-order entry doesn't silently match everything.
        pat == s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    /// All cluster ports require `INCOMPFUNC == 1` because every
    /// underlying builtin (comptags/comptry/compgroups) guards on it.
    fn with_incompfunc<F: FnOnce() -> i32>(f: F) -> i32 {
        let _g = crate::test_util::global_state_lock();
        let prev = INCOMPFUNC.load(Ordering::Relaxed);
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = f();
        INCOMPFUNC.store(prev, Ordering::Relaxed);
        r
    }

    #[test]
    fn no_args_dispatches_minus_n() {
        // sh:67 — empty argv routes to `comptags -N` (next-set mode).
        //   comptags `-N` returns 0/1 depending on whether more sets
        //   exist; we just verify the call doesn't panic + returns an
        //   integer.
        let r = with_incompfunc(|| _tags(&[]));
        // No assertion on the specific value — depends on comptags
        // internal state. Just ensure the call path is reached.
        let _ = r;
    }

    #[test]
    fn double_dash_only_routes_to_minus_n() {
        // sh:10-13 — single `--` arg → prev = "-", shift, argv empty →
        // sh:67 path with `-N-`.
        let r = with_incompfunc(|| _tags(&["--".to_string()]));
        let _ = r;
    }

    #[test]
    fn c_flag_long_form_rewrites_curcontext_tail() {
        // sh:21-23 — `-Cfoo` rewrites the trailing `:`-field of
        // `$curcontext`. Test by setting a known curcontext, calling
        // `_tags -Cnewfield mytag`, and verifying the comptags call
        // succeeds without panic. Internal curcontext mutation is
        // observable indirectly through tag registration (which we
        // can't easily inspect from here without deeper comptags
        // hooks); the test just ensures the code path doesn't panic.
        let _r = with_incompfunc(|| {
            crate::ported::params::setsparam("curcontext", ":completion::complete:command:");
            _tags(&["-Cnewfield".to_string(), "mytag".to_string()])
        });
    }

    #[test]
    fn c_flag_short_form_consumes_two_args() {
        // sh:24-26 — `-C newfield` consumes 2 argv entries.
        let _r = with_incompfunc(|| {
            crate::ported::params::setsparam("curcontext", ":completion::complete:command:");
            _tags(&[
                "-C".to_string(),
                "newfield".to_string(),
                "mytag".to_string(),
            ])
        });
    }

    #[test]
    fn end_of_flags_marker_is_stripped() {
        // sh:29 — single `-` or `--` between -C and tags is consumed.
        let _r = with_incompfunc(|| {
            crate::ported::params::setsparam("curcontext", ":completion::complete:command:");
            _tags(&["-".to_string(), "mytag".to_string()])
        });
    }

    #[test]
    fn sort_hook_returns_none_falls_through_to_default() {
        // sh:40-41 — when `$_sort_tags` names an unregistered fn,
        //   dispatch_function_call returns None and we fall through to
        //   the default sort path. Verify no panic.
        let _r = with_incompfunc(|| {
            crate::ported::params::setsparam("curcontext", ":completion::complete:command:");
            crate::ported::params::setsparam("_sort_tags", "nonexistent_hook");
            let r = _tags(&["options".to_string(), "values".to_string()]);
            crate::ported::params::setsparam("_sort_tags", "");
            r
        });
    }
}
