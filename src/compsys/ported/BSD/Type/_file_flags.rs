//! Port of `_file_flags` from `Completion/BSD/Type/_file_flags`.
//!
//! Full upstream body (71 lines, abridged):
//! ```text
//! sh:1  #autoload
//! sh:5  local curcontext=$curcontext su=$(( ! EUID || $+_comp_priv_prefix ))
//! sh:6  local -a … copts=( "${@}" ) flags flag_descs
//! sh:9  flag_descs+=( nodump nodump  uappnd 'user append-only'
//! sh:        uchg 'user immutable' )
//! sh:15  if (( su )); then
//! sh:16    flag_descs+=( arch archived  sappnd 'system append-only'
//! sh:          schg 'system immutable' )
//! sh:21  fi
//! sh:23  if [[ $OSTYPE = (darwin|dragonfly|freebsd|netbsd)* ]]; then
//! sh:24    flag_descs+=(opaque opaque)
//! sh:26    if [[ $OSTYPE = darwin* ]]; then
//! sh:27      flag_descs+=(hidden hidden)
//! sh:28    fi
//! sh:30    if [[ $OSTYPE = (dragonfly|freebsd)* ]]; then
//! sh:31      flag_descs+=(uunlnk 'user undeletable')
//! sh:33      (( su )) && flag_descs+=(sunlnk 'system undeletable')
//! sh:35    fi
//! sh:37    if [[ $OSTYPE = dragonfly* ]]; then
//! sh:38      flag_descs+=( cache cache  nouhistory 'user nohistory' )
//! sh:43      (( su )) && flag_descs+=( noscache noscache
//! sh:            noshistory 'system nohistory' )
//! sh:47    fi
//! sh:49    [[ $OSTYPE = freebsd* ]] && flag_descs+=(
//! sh:50      uarch archive  uhidden hidden  uoffline offline
//! sh:53      urdonly 'DOS, Windows and CIFS readonly'
//! sh:54      ureparse 'Windows reparse point'
//! sh:55      usparse 'sparse file'
//! sh:56      usystem 'DOS, Windows and CIFS system' )
//! sh:58  fi
//! sh:60  for 1 2 in $flag_descs; do
//! sh:61    if [[ $1 = no* ]]; then
//! sh:62      flags+=("(${1#no})$1[set the $2 flag]"
//! sh:63               "($1)${1#no}[unset the $2 flag]")
//! sh:64    else
//! sh:65      flags+=("(no$1)$1[set the $2 flag]"
//! sh:66               "($1)no$1[unset the $2 flag]")
//! sh:67    fi
//! sh:68  done
//! sh:70  _values -O copts -s , 'file flags' $flags
//! ```

use crate::compsys::ported::_values::_values;

/// sh:23 — `[[ $OSTYPE = (darwin|dragonfly|freebsd|netbsd)* ]]`.
fn is_bsd_like(ostype: &str) -> bool {
    ["darwin", "dragonfly", "freebsd", "netbsd"]
        .iter()
        .any(|p| ostype.starts_with(p))
}

/// sh:9-58 — build the `flag_descs` pair list (flag name, description).
fn build_flag_descs(su: bool, ostype: &str) -> Vec<(&'static str, &'static str)> {
    // sh:9-13
    let mut d: Vec<(&'static str, &'static str)> = vec![
        ("nodump", "nodump"),
        ("uappnd", "user append-only"),
        ("uchg", "user immutable"),
    ];

    // sh:15-21
    if su {
        d.push(("arch", "archived"));
        d.push(("sappnd", "system append-only"));
        d.push(("schg", "system immutable"));
    }

    // sh:23-58
    if is_bsd_like(ostype) {
        // sh:24
        d.push(("opaque", "opaque"));

        // sh:26-29
        if ostype.starts_with("darwin") {
            d.push(("hidden", "hidden"));
        }

        // sh:30-35
        if ostype.starts_with("dragonfly") || ostype.starts_with("freebsd") {
            d.push(("uunlnk", "user undeletable"));
            if su {
                d.push(("sunlnk", "system undeletable"));
            }
        }

        // sh:37-47
        if ostype.starts_with("dragonfly") {
            d.push(("cache", "cache"));
            d.push(("nouhistory", "user nohistory"));
            if su {
                d.push(("noscache", "noscache"));
                d.push(("noshistory", "system nohistory"));
            }
        }

        // sh:49-57
        if ostype.starts_with("freebsd") {
            d.push(("uarch", "archive"));
            d.push(("uhidden", "hidden"));
            d.push(("uoffline", "offline"));
            d.push(("urdonly", "DOS, Windows and CIFS readonly"));
            d.push(("ureparse", "Windows reparse point"));
            d.push(("usparse", "sparse file"));
            d.push(("usystem", "DOS, Windows and CIFS system"));
        }
    }

    d
}

/// sh:60-68 — expand each `(flag, desc)` pair into a set/unset `_values`
/// spec pair, mutually excluding one another.
fn build_flags(flag_descs: &[(&str, &str)]) -> Vec<String> {
    let mut flags = Vec::with_capacity(flag_descs.len() * 2);
    for &(f1, f2) in flag_descs {
        if let Some(stripped) = f1.strip_prefix("no") {
            // sh:62-63
            flags.push(format!("({stripped}){f1}[set the {f2} flag]"));
            flags.push(format!("({f1}){stripped}[unset the {f2} flag]"));
        } else {
            // sh:65-66
            flags.push(format!("(no{f1}){f1}[set the {f2} flag]"));
            flags.push(format!("({f1})no{f1}[unset the {f2} flag]"));
        }
    }
    flags
}

/// `_file_flags` — complete BSD `chflags`-style file flag names
/// (`nodump`, `uchg`, `opaque`, …), mutually excluding the set/unset
/// form of each flag from its counterpart.
pub fn _file_flags(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_file_flags");
    // sh:6 — `local -a context line state state_descr copts=( "${@}" ) flags
    // flag_descs`.
    //
    // `copts` is this function's saved `"$@"`, which the port publishes as
    // a shell parameter so `_values` can splat it by name. It is the only
    // name on sh:6 the port materialises: `flags`/`flag_descs` stay
    // Rust-side, and `context`/`line`/`state`/`state_descr` are already
    // declared local by `_main_complete` (sh:27-30), so a write to those
    // lands in THAT shadow and `endparamscope` unwinds it. Without the
    // declaration `chflags <TAB>` left `copts` standing:
    //
    //   zsh  : copts=[][0]        zshrs: copts=[array][2]
    crate::compsys::ported::shared::declare_locals(
        &["copts"],
        crate::compsys::ported::shared::PM_ARRAY,
    );
    // sh:5 — `local curcontext=$curcontext` is a self-assign purely to
    // shadow the caller's binding; the port has no local-param scope so
    // there is nothing to do here (curcontext is read as-is by `_values`).
    // `su = (( ! EUID || $+_comp_priv_prefix ))`.
    // `$EUID` is a libc-backed special: its value comes from `euidgetfn()`
    // (`params.rs:11271` → `geteuid(2)`), which only the SCALAR getter consults
    // (`params.rs:15049`). `getiparam` short-circuits on the paramtab node's
    // PM_INTEGER flag and returns its `u_val` (`params.rs:5720-5726`), and
    // nothing ever writes that slot — so `getiparam("EUID")` reported 0 and
    // `! EUID` was true for every user. Read it the way `$EUID` reads.
    let euid: i64 = crate::ported::params::getsparam("EUID")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    // `$+_comp_priv_prefix` is an IS-IT-SET test, not an is-it-declared test.
    // `_main_complete` (Completion/Base/Core/_main_complete:42-43) declares the
    // parameter and then unsets it —
    //     local _comp_priv_prefix
    //     unset _comp_priv_prefix
    // — purely to hide a caller-scope value, so inside every completer the
    // node exists but carries PM_UNSET and `${+_comp_priv_prefix}` is 0.
    // `getaparam` hands back `Some(vec![])` for that node (params.rs:6250-6255
    // returns `pm.u_arr` without consulting PM_UNSET), so `.is_some()` read the
    // declaration as a value and made `su` true for an ordinary user: `chflags
    // <TAB>` gained the six root-only flags `arch`/`noarch`, `sappnd`/`nosappnd`
    // and `schg`/`noschg` that zsh does not offer. `issetvar` (params.rs:1503,
    // the `[[ -v … ]]` path) is the PM_UNSET-aware test that matches `${+…}`.
    let su = euid == 0 || crate::ported::params::issetvar("_comp_priv_prefix") != 0;

    // sh:6 — `copts=( "${@}" )`; `_values -O copts` (sh:70) reads it back
    // by name, so publish it as the `copts` array param.
    crate::ported::params::setaparam("copts", args.to_vec());

    let ostype = crate::ported::params::getsparam("OSTYPE").unwrap_or_default();

    // sh:9-58
    let flag_descs = build_flag_descs(su, &ostype);

    // sh:60-68
    let flags = build_flags(&flag_descs);

    // sh:70 — `_values -O copts -s , 'file flags' $flags`.
    let mut v: Vec<String> = vec![
        "-O".to_string(),
        "copts".to_string(),
        "-s".to_string(),
        ",".to_string(),
        "file flags".to_string(),
    ];
    v.extend(flags);
    // By NAME so `_values` gets its own `comp_wrapper` frame (c:1556); without
    // one its `compstate[restore]=''` (`_values.rs:388`) leaks into this
    // function's frame and suppresses the caller's restore. `copts` is a plain
    // global param, so the extra function scope doesn't hide it from `-O`.
    crate::compsys::ported::shared::call_compfn("_values", &v, || _values(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_bsd_like_matches_known_prefixes() {
        assert!(is_bsd_like("darwin23"));
        assert!(is_bsd_like("dragonfly5"));
        assert!(is_bsd_like("freebsd13"));
        assert!(is_bsd_like("netbsd9"));
        assert!(!is_bsd_like("linux-gnu"));
        assert!(!is_bsd_like("solaris"));
    }

    #[test]
    fn build_flag_descs_base_non_su_linux() {
        let d = build_flag_descs(false, "linux-gnu");
        assert_eq!(
            d,
            vec![
                ("nodump", "nodump"),
                ("uappnd", "user append-only"),
                ("uchg", "user immutable"),
            ]
        );
    }

    #[test]
    fn build_flag_descs_su_adds_privileged_flags() {
        let d = build_flag_descs(true, "linux-gnu");
        assert!(d.contains(&("arch", "archived")));
        assert!(d.contains(&("sappnd", "system append-only")));
        assert!(d.contains(&("schg", "system immutable")));
    }

    #[test]
    fn build_flag_descs_darwin_adds_opaque_and_hidden() {
        let d = build_flag_descs(false, "darwin23.0");
        assert!(d.contains(&("opaque", "opaque")));
        assert!(d.contains(&("hidden", "hidden")));
        // dragonfly/freebsd-only flags absent on darwin.
        assert!(!d.contains(&("uunlnk", "user undeletable")));
    }

    #[test]
    fn build_flag_descs_freebsd_non_su_omits_sunlnk_but_has_uunlnk() {
        let d = build_flag_descs(false, "freebsd14.0");
        assert!(d.contains(&("uunlnk", "user undeletable")));
        assert!(!d.contains(&("sunlnk", "system undeletable")));
        assert!(d.contains(&("uarch", "archive")));
        assert!(d.contains(&("usystem", "DOS, Windows and CIFS system")));
        // dragonfly-only flags absent on freebsd.
        assert!(!d.contains(&("cache", "cache")));
    }

    #[test]
    fn build_flag_descs_freebsd_su_adds_sunlnk() {
        let d = build_flag_descs(true, "freebsd14.0");
        assert!(d.contains(&("sunlnk", "system undeletable")));
    }

    #[test]
    fn build_flag_descs_dragonfly_su_adds_noscache_and_noshistory() {
        let d = build_flag_descs(true, "dragonfly6.4");
        assert!(d.contains(&("cache", "cache")));
        assert!(d.contains(&("nouhistory", "user nohistory")));
        assert!(d.contains(&("noscache", "noscache")));
        assert!(d.contains(&("noshistory", "system nohistory")));
        // freebsd-only flags absent on dragonfly.
        assert!(!d.contains(&("uarch", "archive")));
    }

    #[test]
    fn build_flags_no_prefixed_flag_strips_no_for_the_set_spec() {
        let flags = build_flags(&[("nodump", "nodump")]);
        assert_eq!(
            flags,
            vec![
                "(dump)nodump[set the nodump flag]".to_string(),
                "(nodump)dump[unset the nodump flag]".to_string(),
            ]
        );
    }

    #[test]
    fn build_flags_plain_flag_prepends_no_for_the_unset_spec() {
        let flags = build_flags(&[("uchg", "user immutable")]);
        assert_eq!(
            flags,
            vec![
                "(nouchg)uchg[set the user immutable flag]".to_string(),
                "(uchg)nouchg[unset the user immutable flag]".to_string(),
            ]
        );
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("OSTYPE", "linux-gnu");
        assert_eq!(_file_flags(&[]), 1);
    }
}
