//! Port of `_combination` from
//! `Completion/Base/Utility/_combination`.
//!
//! Faithful port of `Completion/Base/Utility/_combination` (102 lines).
//!
//! ```text
//! sh: 53  local sep tag style keys pats key num tmp
//! sh: 55  -s SEP / -sSEP parse
//! sh: 69  keys=( ${(s/-/)style} );  pats=( "${(@)keys/*/*}" )
//! sh: 72  while [[ "$1" = *=* ]]: pats[$keys[(in:num:)$key]]="${1#*\=}"
//! sh: 84  key / num of the terminating Kj[:Nj]
//! sh: 92  zstyle -a ":completion:${curcontext}:$tag" "$style" tmp
//! sh: 93  tmp=( "${(@M)tmp:#${(j($sep))~pats}}" )
//! sh: 95  strip the leading fields before key's field
//! sh: 97  tmp=( ${tmp%%${~sep}*} )
//! sh: 99  compadd "$@" -a tmp || { (( $+functions[_$key] )) && "_$key" "$@" }
//! sh:101  (( $+functions[_$key] )) && "_$key" "$@"
//! ```
//!
//! `$sep` is used as a glob pattern at sh:93/95/97 (`_telnet` passes
//! `-s '[@:]'`), so the filtering goes through `patcompile`/`pattry`.

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getsparam, setaparam};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// Reach `_combination` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_combination -s '[@:]' '' users-hosts-ports \`
/// (Completion/Unix/Command/_telnet sh:69) — so the normal function lookup
/// runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_combination_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _combination(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_combination", args, || _combination_impl(args))
}

/// Compile `pat` as a glob (the `~` in `${~sep}` / `${(j($sep))~pats}`).
fn glob_prog(pat: &str) -> Option<crate::ported::pattern::Patprog> {
    let mut tok = pat.to_string();
    crate::ported::glob::tokenize(&mut tok);
    patcompile(&tok, 0, None)
}

/// `keys[(in:num:)key]` — 1-based index of the NUM'th element matching the
/// pattern `key`; `keys.len() + 1` when there is none (the `(i)` flag).
fn nth_index(keys: &[String], key: &str, num: i64) -> usize {
    let prog = glob_prog(key);
    let mut seen = 0i64;
    for (i, k) in keys.iter().enumerate() {
        let hit = match prog.as_ref() {
            Some(p) => pattry(p, k),
            None => k == key,
        };
        if hit {
            seen += 1;
            if seen == num {
                return i + 1;
            }
        }
    }
    keys.len() + 1
}

/// sh:74-79 / sh:84-89 — split `Kj[:Nj]` (with `=Pi` already removed from
/// `tmp` by the caller) into the key and its ordinal. `whole` is `$1`: the
/// `[[ $1 = *:* ]]` test looks at the whole argument, and `${tmp##*:}` then
/// yields the text after the last colon of `tmp` (all of `tmp` when it has
/// none, which `(in:num:)` evaluates arithmetically — a bare word is 0).
fn key_and_num(tmp: &str, whole: &str) -> (String, i64) {
    let key = match tmp.rfind(':') {
        Some(i) => &tmp[..i], // sh:74 `${tmp%:*}`
        None => tmp,
    };
    let num = if whole.contains(':') {
        let n = tmp.rsplit(':').next().unwrap_or(tmp); // sh:76 `${tmp##*:}`
        n.trim().parse::<i64>().unwrap_or(0)
    } else {
        1
    };
    (key.to_string(), num)
}

/// `_combination` — multi-key zstyle-driven completer. See upstream
/// docstring for the spec language (e.g. `users-hosts-ports`).
pub fn _combination_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_combination");
    // sh:53
    crate::compsys::ported::shared::declare_locals(
        &["sep", "tag", "style", "keys", "pats", "key", "num"],
        0,
    );
    crate::compsys::ported::shared::declare_locals(&["tmp"], crate::ported::zsh_h::PM_ARRAY);
    let mut idx = 0usize;

    // sh:55-63 — `-s SEP`, `-sSEP`, default `:`
    let mut sep = ":".to_string();
    if let Some(a) = args.first() {
        if a == "-s" {
            sep = args.get(1).cloned().unwrap_or_default(); // sh:56
            idx = 2;
        } else if let Some(rest) = a.strip_prefix("-s") {
            sep = rest.to_string(); // sh:59 `${1[3,-1]}`
            idx = 1;
        }
    }

    // sh:65-67
    let arg = |i: usize| args.get(i).cloned().unwrap_or_default();
    let tag = arg(idx);
    let style = arg(idx + 1);
    idx += 2;

    // sh:69-70
    let keys: Vec<String> = style
        .split('-')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let mut pats: Vec<String> = keys.iter().map(|_| "*".to_string()).collect();

    // sh:72-82 — `Ki[:Ni]=Pi` pairs
    while idx < args.len() && args[idx].contains('=') {
        let whole = &args[idx];
        let eq = whole.find('=').unwrap();
        let (key, num) = key_and_num(&whole[..eq], whole); // sh:73-79
        let at = nth_index(&keys, &key, num); // sh:80
        if pats.len() < at {
            pats.resize(at, String::new()); // assignment past the end pads
        }
        pats[at - 1] = whole[eq + 1..].to_string(); // sh:80 `${1#*\=}`
        idx += 1;
    }

    // sh:84-90 — the terminating `Kj[:Nj]`
    let last = arg(idx);
    let (key, num) = key_and_num(&last, &last);
    idx += 1;
    let extras: &[String] = args.get(idx..).unwrap_or(&[]);

    // sh:99/sh:101 — `(( $+functions[_$key] )) && "_$key" "$@"`. BOTH
    // fallbacks are guarded on the function existing, and the `&&` yields 1
    // when it does not; calling unconditionally is not a no-op:
    // `dispatch_function_call` autoloads a `_`-name it finds in `$fpath`
    // (`src/vm_helper.rs:4544-4557`), which `(( $+functions[...] ))` never
    // does. Measured with `fpath=(...5.9.2/share/zsh/functions)` and no
    // `compinit`, `_combination mytag foo-alias alias`:
    //   zsh    rc 1, silent, `$+functions[_alias]` still 0
    //   zshrs  rc 0, `_arguments:comparguments:327: can only be called from
    //          completion function` on stderr, `$+functions[_alias]` now 1
    let call_key = || -> i32 {
        let f = format!("_{}", key);
        if !crate::compsys::ported::shared::plus_functions(&f) {
            return 1; // sh:99/101 — the `&&` short-circuits
        }
        dispatch_function_call(&f, extras).unwrap_or(1)
    };

    // sh:92 — `zstyle -a ":completion:${curcontext}:$tag" "$style" tmp`
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let style_ctx = format!(":completion:{}:{}", curcontext, tag);
    let zs = crate::ported::modules::zutil::bin_zstyle(
        "zstyle",
        &["-a".to_string(), style_ctx, style.clone(), "tmp".to_string()],
        &make_ops(),
        0,
    );
    if zs != 0 {
        return call_key(); // sh:101
    }
    let mut tmp: Vec<String> = crate::ported::params::getaparam("tmp").unwrap_or_default();

    // sh:93 — `tmp=( "${(@M)tmp:#${(j($sep))~pats}}" )`
    if let Some(p) = glob_prog(&pats.join(&sep)) {
        tmp.retain(|e| pattry(&p, e));
    } else {
        tmp.clear();
    }

    // sh:94-96 — drop the fields in front of KEY's own:
    // `tmp=( ${tmp#${(j(sep))~${(@)keys[2,(rn:num:)$key]/*/*}}${~sep}} )`
    let at = nth_index(&keys, &key, num);
    if at != 1 {
        let nstar = at.min(keys.len()).saturating_sub(1);
        let lead = format!("{}{}", vec!["*"; nstar].join(&sep), sep);
        if let Some(p) = glob_prog(&lead) {
            for e in tmp.iter_mut() {
                // `${e#pat}`: shortest matching prefix
                let cut = e
                    .char_indices()
                    .map(|(i, _)| i)
                    .chain(std::iter::once(e.len()))
                    .find(|&i| pattry(&p, &e[..i]));
                if let Some(i) = cut {
                    *e = e[i..].to_string();
                }
            }
        }
    }

    // sh:97 — `tmp=( ${tmp%%${~sep}*} )`: longest matching suffix
    if let Some(p) = glob_prog(&format!("{}*", sep)) {
        for e in tmp.iter_mut() {
            let cut = e
                .char_indices()
                .map(|(i, _)| i)
                .chain(std::iter::once(e.len()))
                .find(|&i| pattry(&p, &e[i..]));
            if let Some(i) = cut {
                e.truncate(i);
            }
        }
    }
    setaparam("tmp", tmp);

    // sh:99 — `compadd "$@" -a tmp || { (( $+functions[_$key] )) && "_$key" "$@" }`
    let mut compadd_argv: Vec<String> = extras.to_vec();
    compadd_argv.push("-a".to_string());
    compadd_argv.push("tmp".to_string());
    if bin_compadd("compadd", &compadd_argv, &make_ops(), 0) == 0 {
        0
    } else {
        call_key()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_and_num_follow_sh74_79() {
        assert_eq!(key_and_num("hosts:2", "hosts:2"), ("hosts".to_string(), 2));
        assert_eq!(key_and_num("hosts", "hosts"), ("hosts".to_string(), 1));
        // colon only in the VALUE: `[[ $1 = *:* ]]` is true, `${tmp##*:}` is `hosts`
        assert_eq!(key_and_num("hosts", "hosts=a:b"), ("hosts".to_string(), 0));
    }

    #[test]
    fn nth_index_counts_matches_and_misses_past_end() {
        let keys: Vec<String> = ["users", "hosts", "users"].iter().map(|s| s.to_string()).collect();
        assert_eq!(nth_index(&keys, "users", 1), 1);
        assert_eq!(nth_index(&keys, "users", 2), 3);
        assert_eq!(nth_index(&keys, "ports", 1), 4);
    }

    #[test]
    fn returns_one_without_style() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            _combination_impl(&[
                "mytag".to_string(),
                "users-hosts".to_string(),
                "users".to_string(),
            ]),
            1
        );
    }

    /// sh:101 — with no style set the fallback is
    /// `(( $+functions[_$key] )) && "_$key" "$@"`, so a key whose `_$key` is
    /// neither defined nor routed must return 1 having called NOTHING. The
    /// port used to reach `dispatch_function_call` unconditionally, which for
    /// a `_`-name also AUTOLOADS a matching `$fpath` file
    /// (`src/vm_helper.rs:4544-4557`) — a side effect `(( $+functions[...] ))`
    /// cannot have. Measured against zsh with
    /// `fpath=(.../5.9.2/share/zsh/functions)` and no `compinit`:
    /// `_combination mytag foo-alias alias` gave zsh rc 1 and silence, and
    /// zshrs rc 0 plus `_arguments:comparguments:327: can only be called from
    /// completion function` on stderr.
    #[test]
    fn absent_key_function_is_not_called() {
        let _g = crate::test_util::global_state_lock();
        const KEY: &str = "zzq_no_such_key";
        assert!(
            !crate::compsys::ported::shared::plus_functions(&format!("_{KEY}")),
            "premise: `_{KEY}` is neither a shell function nor a routed port"
        );
        assert_eq!(
            _combination_impl(&[
                "mytag".to_string(),
                format!("users-{KEY}"),
                KEY.to_string(),
            ]),
            1
        );
    }
}
