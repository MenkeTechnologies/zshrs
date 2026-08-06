//! Port of `_next_label` from `Completion/Base/Core/_next_label`.
//!
//! Full upstream body (25 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local __gopt __descr __spec
//! sh: 4
//! sh: 5  __gopt=()
//! sh: 6  zparseopts -D -a __gopt 1 2 V J x
//! sh: 7
//! sh: 8  if comptags -A "$1" curtag __spec; then
//! sh: 9    (( $#funcstack > _tags_level )) && _comp_tags="${_comp_tags% * }"
//! sh:10    _tags_level=$#funcstack
//! sh:11    _comp_tags="$_comp_tags $__spec "
//! sh:12    if [[ "$curtag" = *[^\\]:* ]]; then
//! sh:13      zformat -f __descr "${curtag#*:}" "d:$3"
//! sh:14      _description "$__gopt[@]" "${curtag%:*}" "$2" "$__descr"
//! sh:15      curtag="${curtag%:*}"
//! sh:16      set -A $2 "${(P@)2}" "${(@)argv[4,-1]}"
//! sh:17    else
//! sh:18      _description "$__gopt[@]" "$curtag" "$2" "$3"
//! sh:19      set -A $2 "${(@)argv[4,-1]}" "${(P@)2}"
//! sh:20    fi
//! sh:22    return 0
//! sh:23  fi
//! sh:25  return 1
//! ```
//!
//! Calls real `bin_comptags` + `bin_zformat` + `bin_zparseopts` and
//! reads `FUNCSTACK` for the `(( $#funcstack > _tags_level ))` guard.
//! Reaches `_description` BY NAME (`description_byname` →
//! [`crate::compsys::ported::shared::call_compfn`]) for the per-spec format
//! step, matching the sh body's bare `_description …` command word: a
//! user's own copy earlier on `$fpath` wins, and the call gets its own
//! `doshfunc` frame.
//!
//! The self-call at the `_next_tags` tag-filter branch below stays a DIRECT
//! Rust call — see the comment there.

use super::_description::description_byname;
use crate::ported::modules::parameter::FUNCSTACK;
use crate::ported::modules::zutil::{bin_zformat, bin_zparseopts};
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use crate::ported::zle::computil::bin_comptags;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:6 — bridge to real `bin_zparseopts -D -a __gopt 1 2 V J x` via
/// the `-v <name>` flag (named array source) so we don't depend on
/// the pparams hook being installed.
fn run_gopt(args: &[String]) -> (Vec<String>, Vec<String>) {
    let src = "__compsys_argv";
    setaparam(src, args.to_vec());
    setaparam("__gopt", Vec::new());
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-D".to_string(),
            "-v".to_string(),
            src.to_string(),
            "-a".to_string(),
            "__gopt".to_string(),
            "1".to_string(),
            "2".to_string(),
            "V".to_string(),
            "J".to_string(),
            "x".to_string(),
        ],
        &make_ops(),
        0,
    );
    let gopt = getaparam("__gopt").unwrap_or_default();
    let remaining = getaparam(src).unwrap_or_default();
    // Tear down the `__compsys_argv` zparseopts-bridge scratch global (not a
    // real zsh identifier; zsh operates on positional $argv). Bug #657.
    crate::ported::params::unsetparam(src);
    (remaining, gopt)
}

/// `$#funcstack` — the call-depth counter. Reads the real
/// `crate::ported::modules::parameter::FUNCSTACK` mutex (the same
/// global the C `funcstack` linked list maps to).
fn funcstack_depth() -> usize {
    FUNCSTACK.lock().map(|s| s.len()).unwrap_or(0)
}

/// Reach `_next_label` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `while _next_label hosts expl host; do`
/// (Completion/Unix/Type/_urls sh:130) — so the normal function lookup runs.
///
/// A plain Rust call to the sibling port skips both of
/// [`crate::compsys::ported::shared::call_compfn`]'s effects: `$fpath` /
/// shfunc arbitration (the user's own copy of the function is inert) and
/// the `doshfunc` frame (no `FUNCSTACK` entry, and the callee's
/// `declare_locals` land in the CALLER's param scope instead of its own).
///
/// The direct call stays as the fallback: it runs only when neither a shell
/// function nor a registered port claims the name — i.e. in unit tests with
/// no executor installed.
pub fn next_label_byname(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_next_label", args, || _next_label(args))
}

/// `_next_label` — advance to the next tag-spec in the current
/// `_tags` round, format its description, and call `_description`
/// to emit the per-tag option array. Returns 0 on advance, 1 when
/// no specs remain.
pub fn _next_label(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_next_label");
    // sh:3  local __gopt __descr __spec
    crate::compsys::ported::shared::declare_locals(&["__gopt", "__descr", "__spec"], 0);
    // sh:3-6
    let (argv, gopt) = run_gopt(args);

    // sh:8  comptags -A "$1" curtag __spec
    let arg1 = argv.first().cloned().unwrap_or_default();
    let comptags_argv = vec![
        "-A".to_string(),
        arg1,
        "curtag".to_string(),
        "__spec".to_string(),
    ];
    if bin_comptags("comptags", &comptags_argv, &make_ops(), 0) != 0 {
        return 1; // sh:25
    }

    // sh:9  (( $#funcstack > _tags_level )) && _comp_tags="${_comp_tags% * }"
    //   When we're deeper in the call stack than the recorded level,
    //   strip the trailing tag-spec word and re-add the new spec.
    let cur_depth = funcstack_depth();
    let prev_level = getsparam("_tags_level")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0);
    if cur_depth > prev_level {
        let comp_tags = getsparam("_comp_tags").unwrap_or_default();
        // `${_comp_tags% * }` — strip the longest suffix matching
        //   " <something> " (the trailing tag-spec word + trailing
        //   space). Implemented as: trim trailing space, drop last
        //   space-separated token, re-pad with trailing space.
        let trimmed = comp_tags.trim_end_matches(' ');
        let last_sp = trimmed.rfind(' ');
        let kept = match last_sp {
            Some(i) => &trimmed[..i],
            None => "",
        };
        let mut rebuilt = String::from(kept);
        if !rebuilt.is_empty() {
            rebuilt.push(' ');
        }
        let _ = setsparam("_comp_tags", &rebuilt);
    }

    // sh:10  _tags_level=$#funcstack
    let _ = setsparam("_tags_level", &cur_depth.to_string());

    // sh:11
    let spec = getsparam("__spec").unwrap_or_default();
    // sh:_next_tags:66 — `[[ "$_next_tags_not" = *\ ${__spec}\ * ]] &&
    //   continue`. Skip any spec already shown in a previous cycle. This
    //   natively reproduces the tag-filtering `_next_label` body that the
    //   `_next_tags` widget installs via
    //   `unfunction _next_label; _next_label() { … }` at sh:10-82 — a Rust
    //   port can't inject a shell-function body, so it consults
    //   `$_next_tags_not` directly instead. The sh `continue` (inside the
    //   caller's `while`) is expressed here by recursing to advance to the
    //   next spec, preserving the 0/1 return contract. Empty
    //   `_next_tags_not` makes this branch dead, matching the unshadowed
    //   body exactly.
    //
    //   DELIBERATELY a direct Rust self-call, NOT `next_label_byname`. This
    //   recursion stands in for sh `continue`, which is control flow, not a
    //   function call — zsh pushes no frame for it. `next_label_byname`
    //   would route through `doshfunc`, whose `inc_locallevel()`
    //   (exec.rs:6131) and `FUNCSTACK` push both feed the sh:9 guard
    //   `(( $#funcstack > _tags_level ))` two lines above: each skipped spec
    //   would deepen `$#funcstack`, so the guard would fire on every
    //   iteration and strip a word off `$_comp_tags` that zsh keeps.
    //   Arbitration is not lost either — the outer call already resolved
    //   `_next_label` (a user's own copy would be running its own body here,
    //   not this port), so a by-name re-entry could only land back on this
    //   same fn.
    if spec_in_not_list(&spec, &getsparam("_next_tags_not").unwrap_or_default()) {
        return _next_label(args);
    }
    let mut comp_tags = getsparam("_comp_tags").unwrap_or_default();
    comp_tags.push_str(&format!(" {} ", spec));
    let _ = setsparam("_comp_tags", &comp_tags);

    // sh:12
    let curtag = getsparam("curtag").unwrap_or_default();
    let name = argv.get(1).cloned().unwrap_or_default();
    let descr_arg = argv.get(2).cloned().unwrap_or_default();
    let extras: Vec<String> = if argv.len() > 3 {
        argv[3..].to_vec()
    } else {
        Vec::new()
    };

    if has_unescaped_colon(&curtag) {
        // sh:13  zformat -f __descr "${curtag#*:}" "d:$3"
        let after_colon = curtag.splitn(2, ':').nth(1).unwrap_or("").to_string();
        let _ = setsparam("__descr", "");
        let zfmt_argv = vec![
            "-f".to_string(),
            "__descr".to_string(),
            after_colon,
            format!("d:{}", descr_arg),
        ];
        let _ = bin_zformat("zformat", &zfmt_argv, &make_ops(), 0);
        let descr = getsparam("__descr").unwrap_or_default();

        // sh:14  _description "$__gopt[@]" "${curtag%:*}" "$2" "$__descr"
        let tag_lhs = curtag
            .rsplitn(2, ':')
            .nth(1)
            .map(|s| s.to_string())
            .unwrap_or_else(|| curtag.clone());
        let mut desc_argv = gopt.clone();
        desc_argv.push(tag_lhs);
        desc_argv.push(name.clone());
        desc_argv.push(descr);
        let _ = description_byname(&desc_argv);

        // sh:16  set -A $2 "${(P@)2}" "${(@)argv[4,-1]}"
        let mut prev = getaparam(&name).unwrap_or_default();
        prev.extend(extras);
        setaparam(&name, prev);
    } else {
        // sh:18  _description "$__gopt[@]" "$curtag" "$2" "$3"
        let mut desc_argv = gopt.clone();
        desc_argv.push(curtag);
        desc_argv.push(name.clone());
        desc_argv.push(descr_arg);
        let _ = description_byname(&desc_argv);

        // sh:19  set -A $2 "${(@)argv[4,-1]}" "${(P@)2}"
        let mut new_arr = extras;
        new_arr.extend(getaparam(&name).unwrap_or_default());
        setaparam(&name, new_arr);
    }

    0 // sh:22
}

/// sh:_next_tags:39 / :66 — the `[[ "$_next_tags_not" = *\ ${__spec}\ * ]]`
/// glob test. Returns true when `spec` occurs as a whole, space-delimited
/// entry inside `not` (a space required on BOTH sides, exactly like the zsh
/// glob). This is how the Rust port replaces the `_next_tags` widget's
/// `unfunction _all_labels _next_label; <redefine>` trick (sh:10-82): a
/// shell-function body can't be injected from Rust, so the ports read
/// `$_next_tags_not` themselves. Empty `not`/`spec` → false, keeping the
/// default (unshadowed) path unchanged.
fn spec_in_not_list(spec: &str, not: &str) -> bool {
    !not.is_empty() && !spec.is_empty() && not.contains(&format!(" {} ", spec))
}

/// sh:12 — `*[^\\]:*` test (mirrors helper in `_description`).
fn has_unescaped_colon(s: &str) -> bool {
    let b = s.as_bytes();
    for i in 0..b.len() {
        if b[i] == b':' && (i == 0 || b[i - 1] != b'\\') {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    fn with_incompfunc<F: FnOnce() -> i32>(f: F) -> i32 {
        let _g = crate::test_util::global_state_lock();
        let prev = INCOMPFUNC.load(Ordering::Relaxed);
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = f();
        INCOMPFUNC.store(prev, Ordering::Relaxed);
        r
    }

    #[test]
    fn run_gopt_extracts_boolean_flags_in_argv_order() {
        // sh:6 — `1 2 V J x` are all boolean. `bin_zparseopts` -D
        //   strips them from argv and accumulates into the gopt
        //   array preserving argv encounter order (matches C's
        //   `a->vals` linked-list semantics at zutil.c:2076).
        let _g = crate::test_util::global_state_lock();
        let (rem, gopt) = run_gopt(&[
            "-V".to_string(),
            "-1".to_string(),
            "-x".to_string(),
            "tag".to_string(),
            "name".to_string(),
            "descr".to_string(),
        ]);
        assert_eq!(gopt, vec!["-V", "-1", "-x"]);
        assert_eq!(rem, vec!["tag", "name", "descr"]);
    }

    #[test]
    fn funcstack_depth_reads_real_funcstack_global() {
        // The function should compile and read the real FUNCSTACK
        // mutex without panicking. Depth at test time (no
        // shfunc_call wrapping) is whatever the test harness has
        // already pushed; typically 0.
        let _g = crate::test_util::global_state_lock();
        let d = funcstack_depth();
        let _ = d; // just confirm no panic
    }

    #[test]
    fn no_more_specs_returns_one() {
        // sh:8 — comptags -A on an unregistered tag returns non-zero
        //   → _next_label returns 1 (sh:25).
        let r = with_incompfunc(|| {
            _next_label(&[
                "unregistered_tag".to_string(),
                "name".to_string(),
                "descr".to_string(),
            ])
        });
        assert_eq!(r, 1);
    }

    #[test]
    fn has_unescaped_colon_works() {
        assert!(has_unescaped_colon("foo:bar"));
        assert!(!has_unescaped_colon("foo\\:bar"));
        assert!(!has_unescaped_colon("plain"));
    }

    #[test]
    fn spec_in_not_list_skips_listed_spec() {
        // sh:_next_tags:66 — a spec bracketed by spaces in $_next_tags_not
        //   is on the not-list, so `_next_label` recurses past it (skips
        //   it) to advance to the next spec.
        assert!(spec_in_not_list("files", " files directories "));
        assert!(spec_in_not_list("directories", " files directories "));
    }

    #[test]
    fn spec_in_not_list_default_and_boundary() {
        // Default path: empty $_next_tags_not never filters.
        assert!(!spec_in_not_list("files", ""));
        // Empty spec never filters (guards the `${__spec}` == "" edge).
        assert!(!spec_in_not_list("", " files "));
        // An unlisted spec is not skipped.
        assert!(!spec_in_not_list("options", " files directories "));
        // sh-glob parity: the trailing entry has no following space (the
        //   widget appends `" $tags"` with no trailing space at sh:95), so
        //   `*\ ${__spec}\ *` does NOT match it — mirrored here.
        assert!(!spec_in_not_list("dirs", " files dirs"));
    }

    #[test]
    fn next_tags_not_empty_is_noop() {
        // sh:_next_tags:66 — when `_next_tags_not` is unset/empty the
        //   filter branch is dead and behavior matches the unshadowed
        //   body (here: unregistered tag → 1).
        let r = with_incompfunc(|| {
            setsparam("_next_tags_not", "").unwrap();
            _next_label(&[
                "unregistered_tag".to_string(),
                "name".to_string(),
                "descr".to_string(),
            ])
        });
        assert_eq!(r, 1);
    }
}
