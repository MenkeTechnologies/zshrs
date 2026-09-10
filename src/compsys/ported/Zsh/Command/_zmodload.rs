//! Port of `_zmodload` from `Completion/Zsh/Command/_zmodload`.
//!
//! Full upstream body (78 lines), branch map:
//! ```text
//! sh: 1  #compdef zmodload
//! sh: 3  local suf comp state line expl curcontext="$curcontext" ret=1 NORMARG
//! sh: 4  typeset -A opt_args
//! sh: 5  suf=()
//! sh: 7  _arguments -n -C -S -s  <20 option specs>  '(-)*:param:->params' && ret=0
//! sh:28  [[ $state = params ]] || return ret
//! sh:30  $+opt_args[-A]  → compset -S '=*' || compset -P '*=' || suf=( -S '=' )
//! sh:34  $+opt_args[-F] && CURRENT > NORMARG  → per-module FEATURE completion
//! sh:54  else                                 → the four-tag loop
//! sh:65    _tags "$comp[@]"; while _tags; do  builtins / loadedmodules /
//! sh:66                                       files / aliases  done
//! ```
//!
//! ## Why this file exists at all
//!
//! Three of the four tags need no help: `builtins`, `loadedmodules` and
//! `aliases` all read live shell tables (`$builtins`, `$modules`) that
//! zshrs populates exactly as C does, and the `-F` feature branch reads
//! `zmodload -lFP`. They were byte-identical to zsh through the shell
//! function and stay byte-identical through this port.
//!
//! The `files` tag is the one that cannot work here. sh:72 asks the
//! FILESYSTEM which modules exist —
//! `_files -W module_path -g '*.(dll|s[ol]|bundle)(:r)'` — because in C
//! every loadable module is a `.so`/`.bundle` under `$module_path`. zshrs
//! links its modules in (`register_builtin_modules`,
//! `src/ported/module.rs:1480`) and points `$module_path` at its own
//! `~/.zshrs/modules` (`src/ported/init.rs:997`), which normally does not
//! exist. So the scan matched nothing and `zmodload zsh/<TAB>` offered
//! NOTHING where zsh offers its 36 bundle names.
//!
//! This port keeps sh:72's scan — `zmodload -R` loads `znative` cdylibs
//! from a real path, and a user who puts one under `$module_path` must
//! still see it — and adds the in-memory registry beside it, under the
//! same `files` tag and the same `module file` description. The registry
//! is [`loadable_module_names`] below: the names `zmodload NAME` on THIS
//! binary can resolve. Nothing is fabricated on disk and `$modules` is not
//! padded with names that are not loaded; the tag simply asks the right
//! table.
//!
//! Set difference against zsh 5.9.2 on this host is real and expected:
//! zshrs registers `zsh/db/gdbm`, which that zsh does not ship, and does
//! not register `zsh/deltochar` or `zsh/newuser`, which it does. Offering
//! zsh's list verbatim would name modules this shell cannot load.

use crate::compsys::ported::_arguments::_arguments;
use crate::compsys::ported::_files::_files;
use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_multi_parts::_multi_parts;
use crate::compsys::ported::_requested::_requested;
use crate::compsys::ported::_tags::_tags;
use crate::compsys::ported::_wanted::_wanted;
use crate::compsys::ported::shared::{assoc_get, FnScope, LocalScope};
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{options, MAX_OPS, PM_ARRAY, PM_HASHED, PM_SCALAR};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

fn s(v: &str) -> String {
    v.to_string()
}

/// `(( $+name[key] ))` — is `key` a key of the associative parameter
/// `name`? True even when the value is empty, which is the normal case
/// for a flag in `$opt_args` (`_arguments` stores `-u` → `""`).
fn has_key(name: &str, key: &str) -> bool {
    assoc_get(name, key).is_some()
}

/// sh:7-26 — the `_arguments` spec set, verbatim.
fn arg_specs() -> Vec<String> {
    vec![
        s("-n"), // sh:7
        s("-C"),
        s("-S"),
        s("-s"),
        s("(-R -P -i -u -d -a -b -c -I -p -f -e -F -m)-A[create module aliases]"), // sh:8
        s("(-)-R[remove module aliases]"),                                        // sh:9
        s("(-A -R -F -L -m -P -l -e)-u[unload module]"),                           // sh:10
        s("(-d -e -l)-a[autoload module]"),                                        // sh:11
        s("(-c -d -I -p -f -F -P -l -m -A -R)-b[autoload module for builtins]"),   // sh:12
        s("(-b -d -I -p -f -F -P -l -m -A -R)-c[autoload module for condition codes]"), // sh:13
        s("(-A -R -F -I -P -a -b -c -e -f -i -l -m -p)-d[list or specify module dependencies]"), // sh:14
        s("(-i -u -d -a -b -c -p -f -L -R)-e[test if modules are loaded]"), // sh:15
        s("(-b -c -d -I -p -F -P -l -m -A -R)-f[autoload module for math functions]"), // sh:16
        s("(-u -b -c -d -p -f -A -R -I)-F[handle features]"),              // sh:17
        s("(-u -b -c -d -p -f -A -R -I)-m[treat feature arguments as patterns]"), // sh:18
        s("(-d -e)-i[suppress error if command would do nothing]"),        // sh:19
        s("(-d -e -L)-s[suppress error if module is not available]"),      // sh:20
        s("(-b -c -d -p -f -F -P -m)-I[define infix condition names]"),    // sh:21
        s("(-u -b -c -d -p -f -A -R)-l[list features]"),                   // sh:22
        s("(-e -u)-L[output in the form of calls to zmodload]"),           // sh:23
        s("(-b -c -d -I -f -F -P -l -m -A -R)-p[autoload module for parameters]"), // sh:24
        s("(-u -b -c -d -p -f -A -R)-P[array param for features]:array name:_parameters"), // sh:25
        s("(-)*:param:->params"),                                          // sh:26
    ]
}

/// sh:72's glob, kept verbatim so a real `znative` cdylib under
/// `$module_path` still completes.
const MODULE_FILE_GLOB: &str = "*.(dll|s[ol]|bundle)(:r)";

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Completion/` or in
/// `Src/Modules/zmodload.c`. C never needs one: every module it can load
/// is a `.so`/`.bundle` FILE under `$module_path`, so "which modules
/// exist" is answered by reading that directory — which is exactly what
/// sh:72 does. This is the same question asked of the in-memory registry,
/// for a shell whose modules have no files.
///
/// The store is `modulestab.linkedmodules`
/// (`src/ported/module.rs:7791`), the port of C's `LinkList
/// linkedmodules` (`Src/module.c:39`). `register_builtin_modules`
/// (`src/ported/module.rs:1643-1648`) appends every compiled-in module to
/// it at boot, and `register_module` (`src/ported/module.rs:393`) appends
/// anything registered later, so it holds exactly the names
/// `zmodload NAME` can resolve.
///
/// `zsh/main` is filtered out. It is on the list because
/// `Src/mkbltnmlst.sh:107` emits a `register_module("zsh/main", …)` for
/// the `link=static` shell core (`src/ported/module.rs:1650-1653`), not
/// because it is a module anyone loads; C's own `$module_path` holds no
/// `main.bundle`, so sh:72 never offers it either.
///
/// Order is registration order — unsorted, like C's `readdir`. `compadd`
/// sorts the matches.
fn loadable_module_names() -> Vec<String> {
    let table = match crate::ported::module::MODULESTAB.lock() {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    table
        .linkedmodules
        .iter()
        .filter(|n| n.as_str() != "zsh/main")
        .cloned()
        .collect()
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// No function of this name exists in `Completion/`. It is the action
/// word of sh:71's `_requested files expl 'module file' …`, standing in
/// for sh:72's bare `_files -W module_path -g …` so that the tag can be
/// fed from TWO sources instead of one.
///
/// `_all_labels` calls the action as `ACTION "$expl[@]" REST…`
/// (`Completion/Base/Core/_all_labels` sh:39), so `args` arrives as the
/// `expl` options and is forwarded unchanged to both sources — the tag,
/// its description and its group are still upstream's.
///
/// Source 1 is the compiled-in registry, offered through `_multi_parts`
/// with `/` as the separator. That is the shape sh:72 gets for free from
/// `_path_files`: `zsh/net/socket` and `zsh/param/private` list as the
/// directory stubs `net/` and `param/` rather than as full paths, which
/// is what zsh shows for `zmodload zsh/<TAB>`. The names go in as
/// `_multi_parts`' literal `(a b c)` array form (that function's sh:33)
/// — module names cannot contain whitespace, and it saves publishing a
/// scratch array.
///
/// Source 2 is sh:72 itself, unchanged.
///
/// Returns 0 if EITHER source added a match, matching the `&& ret=0` at
/// sh:72.
pub fn _zmodload_module_files(args: &[String]) -> i32 {
    let mut ret = 1;

    // Source 1 — the names `zmodload NAME` can resolve on this binary.
    let names = loadable_module_names();
    if !names.is_empty() {
        let mut mp: Vec<String> = args.to_vec();
        mp.push(s("/"));
        mp.push(format!("({})", names.join(" ")));
        if _multi_parts(&mp) == 0 {
            ret = 0;
        }
    }

    // Source 2 — sh:72, for cdylibs actually present under `$module_path`.
    let mut fa: Vec<String> = args.to_vec();
    fa.push(s("-W"));
    fa.push(s("module_path"));
    fa.push(s("-g"));
    fa.push(s(MODULE_FILE_GLOB));
    if _files(&fa) == 0 {
        ret = 0;
    }

    ret
}

/// sh:34-53 — the `-F` branch: complete one module's FEATURE names.
///
/// Split out of the entry only to keep the two arms of sh:34 readable;
/// it runs in the entry's parameter scope and returns what sh:39/sh:52
/// leave as the function's status.
fn feature_branch() -> i32 {
    // sh:35  local module=$words[NORMARG]
    let normarg: usize = getsparam("NORMARG")
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let words = getaparam("words").unwrap_or_default();
    let module = normarg
        .checked_sub(1)
        .and_then(|i| words.get(i).cloned())
        .unwrap_or_default();

    // sh:36  local -a features
    let mut _feat_scope = LocalScope::declare(&["features"], PM_ARRAY);

    // sh:38  if [[ $modules[$module] != loaded ]]; then
    if assoc_get("modules", &module).as_deref() != Some("loaded") {
        // sh:39  _message -e features "feature for unloaded module"
        return _message(&[
            s("-e"),
            s("features"),
            s("feature for unloaded module"),
        ]);
    }

    // sh:41  zmodload -lFP features $module
    //
    // `-P features` is the option-with-argument form, so the argument
    // goes in `ops.args` and `ind['P']` carries `(index << 2) | 1`
    // (`OPT_ARG`, `src/ported/zsh_h.rs:4911`).
    let mut ops = make_ops();
    ops.ind[b'l' as usize] = 1;
    ops.ind[b'F' as usize] = 1;
    ops.args.push(s("features"));
    ops.argscount = 1;
    ops.argsalloc = 1;
    ops.ind[b'P' as usize] = (1 << 2) | 1;
    crate::compsys::ported::shared::set_sh_lineno(41);
    let _ = crate::ported::module::bin_zmodload("zmodload", &[module], &ops, 0);
    let features = getaparam("features").unwrap_or_default();

    // sh:42-51 — offer the OPPOSITE of each feature's current state.
    // `zmodload -lFP` reports `+name` for enabled and `-name` for
    // disabled.
    let filtered: Vec<String> = if bin_compset("compset", &[s("-P"), s("-")], &make_ops(), 0) == 0 {
        // sh:43-44  only enabled features needed
        //           features=(${${features:#-*}##?})
        features
            .iter()
            .filter(|f| !f.starts_with('-'))
            .map(|f| strip_first_char(f))
            .collect()
    } else if bin_compset("compset", &[s("-P"), s("+")], &make_ops(), 0) == 0 {
        // sh:45-47  only disabled features needed
        //           features=(${${features:#+*}##?})
        features
            .iter()
            .filter(|f| !f.starts_with('+'))
            .map(|f| strip_first_char(f))
            .collect()
    } else {
        // sh:48-50  complete opposite of current feature state, + is default
        //           features=(${${features#-}/(#s)+/-})
        features
            .iter()
            .map(|f| {
                let stripped = f.strip_prefix('-').unwrap_or(f);
                match stripped.strip_prefix('+') {
                    Some(rest) => format!("-{}", rest),
                    None => stripped.to_string(),
                }
            })
            .collect()
    };
    setaparam("features", filtered);

    // sh:52  _wanted features expl feature compadd -a features
    _wanted(&[
        s("features"),
        s("expl"),
        s("feature"),
        s("compadd"),
        s("-a"),
        s("features"),
    ])
}

/// `${f##?}` — drop the first CHARACTER (not byte) of `f`.
fn strip_first_char(f: &str) -> String {
    let mut it = f.chars();
    it.next();
    it.as_str().to_string()
}

/// `_zmodload` — completion for the `zmodload` builtin.
pub fn _zmodload(args: &[String]) -> i32 {
    let _fn_scope = FnScope::enter("_zmodload");

    // sh:3  local suf comp state line expl curcontext="$curcontext" ret=1 NORMARG
    //
    // `state`, `line`, `NORMARG` and `opt_args` are written by
    // `_arguments` (`_arguments.rs:1294` — `comparguments -W line
    // opt_args` on the `->state` path) and read back below, so they must
    // be the CALLER's locals here exactly as sh:3/sh:4 make them.
    // `suf` and `comp` are assigned arrays BY THIS PORT (sh:5, sh:55), so
    // they are declared with the type they end up with. `state`, `line`,
    // `expl` and `NORMARG` are written by a CALLEE (`_arguments`,
    // `_description`) and so are declared exactly as sh:3 writes them —
    // plain `local`, retyped by whoever assigns.
    let mut _locals = LocalScope::declare(&["suf", "comp"], PM_ARRAY);
    _locals.also(&["state", "line", "expl", "NORMARG"], PM_SCALAR);
    _locals.also_keeping_value(&["curcontext"]);
    // sh:4  typeset -A opt_args
    _locals.also(&["opt_args"], PM_HASHED);
    let mut ret: i32 = 1; // sh:3  ret=1

    // sh:5  suf=()
    setaparam("suf", Vec::new());

    // sh:7-26  _arguments -n -C -S -s … '(-)*:param:->params' && ret=0
    if _arguments(&arg_specs()) == 0 {
        ret = 0; // sh:26
    }

    // sh:28  [[ $state = params ]] || return ret
    if getsparam("state").unwrap_or_default() != "params" {
        return ret;
    }

    // sh:30-32  if (( $+opt_args[-A] )); then
    //             compset -S '=*' || compset -P '*=' || suf=( -S '=' )
    if has_key("opt_args", "-A") {
        crate::compsys::ported::shared::set_sh_lineno(31);
        if bin_compset("compset", &[s("-S"), s("=*")], &make_ops(), 0) != 0
            && bin_compset("compset", &[s("-P"), s("*=")], &make_ops(), 0) != 0
        {
            setaparam("suf", vec![s("-S"), s("=")]);
        }
    }

    // sh:34  if (( $+opt_args[-F] && CURRENT > NORMARG )); then
    let current: i64 = getsparam("CURRENT")
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(0);
    let normarg: i64 = getsparam("NORMARG")
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(0);
    if has_key("opt_args", "-F") && current > normarg {
        return feature_branch(); // sh:35-53
    }

    // sh:55  comp=( files aliases )
    let mut comp: Vec<String> = vec![s("files"), s("aliases")];
    // sh:56  if (( $+opt_args[-u] )); then
    if has_key("opt_args", "-u") {
        // sh:57  if (( $+opt_args[-b] || $+opt_args[-a] )); then
        if has_key("opt_args", "-b") || has_key("opt_args", "-a") {
            comp = vec![s("builtins")]; // sh:58
        } else {
            comp = vec![s("loadedmodules"), s("aliases")]; // sh:60
        }
    }
    // sh:63  (( $+opt_args[-a] && CURRENT > 3 )) && comp=( builtins )
    if has_key("opt_args", "-a") && current > 3 {
        comp = vec![s("builtins")];
    }
    setaparam("comp", comp.clone());

    // sh:65  _tags "$comp[@]"
    let _ = _tags(&comp);

    // sh:66  while _tags; do
    loop {
        if _tags(&[]) != 0 {
            break;
        }

        // sh:67-68  _requested builtins expl 'builtin command' \
        //             compadd "$@" -k builtins && ret=0
        let mut r: Vec<String> = vec![
            s("builtins"),
            s("expl"),
            s("builtin command"),
            s("compadd"),
        ];
        r.extend_from_slice(args); // sh:68 "$@"
        r.push(s("-k"));
        r.push(s("builtins"));
        if _requested(&r) == 0 {
            ret = 0;
        }

        // sh:69-70  _requested loadedmodules expl 'loaded module' \
        //             compadd -k 'modules[(R)loaded]' && ret=0
        if _requested(&[
            s("loadedmodules"),
            s("expl"),
            s("loaded module"),
            s("compadd"),
            s("-k"),
            s("modules[(R)loaded]"),
        ]) == 0
        {
            ret = 0;
        }

        // sh:71-72  _requested files expl 'module file' \
        //             _files -W module_path -g '*.(dll|s[ol]|bundle)(:r)' && ret=0
        //
        // The action word is this port's own helper; see
        // [`_zmodload_module_files`] for why sh:72 alone finds nothing here.
        if _requested(&[
            s("files"),
            s("expl"),
            s("module file"),
            s("_zmodload_module_files"),
        ]) == 0
        {
            ret = 0;
        }

        // sh:73-74  _requested aliases expl 'module alias' \
        //             compadd "$suf[@]" -k 'modules[(R)alias*]' && ret=0
        let mut a: Vec<String> = vec![
            s("aliases"),
            s("expl"),
            s("module alias"),
            s("compadd"),
        ];
        a.extend(getaparam("suf").unwrap_or_default()); // sh:74 "$suf[@]"
        a.push(s("-k"));
        a.push(s("modules[(R)alias*]"));
        if _requested(&a) == 0 {
            ret = 0;
        }

        // sh:75  (( ret )) || return 0
        if ret == 0 {
            return 0;
        }
    }

    // sh:77  return ret
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spec set must carry every one of sh:8-26's specs plus the four
    /// leading flags of sh:7. A dropped spec silently changes which
    /// options `_arguments` recognises, which changes `$opt_args` and so
    /// which tag the four-tag loop offers.
    #[test]
    fn spec_set_is_complete_and_ordered() {
        let specs = arg_specs();
        assert_eq!(
            &specs[..4],
            &["-n".to_string(), "-C".to_string(), "-S".to_string(), "-s".to_string()],
            "sh:7 flags"
        );
        // sh:8-25 = 18 option specs, sh:26 = 1 rest spec.
        assert_eq!(specs.len(), 4 + 18 + 1);
        assert_eq!(specs.last().unwrap(), "(-)*:param:->params", "sh:26");
        // Every option spec names exactly one option letter after its
        // exclusion list.
        for letter in [
            "-A", "-R", "-u", "-a", "-b", "-c", "-d", "-e", "-f", "-F", "-m", "-i", "-s", "-I",
            "-l", "-L", "-p", "-P",
        ] {
            assert!(
                specs.iter().any(|sp| sp.contains(&format!("){}[", letter))),
                "sh:8-25 spec for {letter} is missing"
            );
        }
    }

    /// sh:50 — `${${features#-}/(#s)+/-}`: offer the OPPOSITE of each
    /// feature's current state. This is the transform a user sees on a
    /// bare `zmodload -F zsh/parameter <TAB>`, and getting it backwards
    /// offers exactly the wrong half of the list.
    #[test]
    fn bare_feature_state_is_inverted() {
        let invert = |f: &str| -> String {
            let stripped = f.strip_prefix('-').unwrap_or(f);
            match stripped.strip_prefix('+') {
                Some(rest) => format!("-{}", rest),
                None => stripped.to_string(),
            }
        };
        // enabled (`+`) → offer `-name` so the user can disable it
        assert_eq!(invert("+p:commands"), "-p:commands");
        // disabled (`-`) → offer the bare name (`+` is the default)
        assert_eq!(invert("-p:aliases"), "p:aliases");
        // no prefix at all is left alone
        assert_eq!(invert("p:aliases"), "p:aliases");
    }

    /// sh:44 / sh:47 — `${features:#-*}` / `${features:#+*}` keep the
    /// COMPLEMENT of the typed sign, then `##?` drops the sign.
    #[test]
    fn signed_feature_filters_keep_the_complement() {
        let feats = vec![
            "+p:commands".to_string(),
            "-p:aliases".to_string(),
            "+p:modules".to_string(),
        ];
        // `compset -P -` → enabled only
        let enabled: Vec<String> = feats
            .iter()
            .filter(|f| !f.starts_with('-'))
            .map(|f| strip_first_char(f))
            .collect();
        assert_eq!(enabled, vec!["p:commands", "p:modules"]);
        // `compset -P +` → disabled only
        let disabled: Vec<String> = feats
            .iter()
            .filter(|f| !f.starts_with('+'))
            .map(|f| strip_first_char(f))
            .collect();
        assert_eq!(disabled, vec!["p:aliases"]);
    }

    /// `${f##?}` drops one CHARACTER. A byte-slice would panic mid-
    /// codepoint on a non-ASCII feature name.
    #[test]
    fn strip_first_char_is_codepoint_safe() {
        assert_eq!(strip_first_char("+abc"), "abc");
        assert_eq!(strip_first_char("é!"), "!");
        assert_eq!(strip_first_char(""), "");
    }

    /// The `files` tag must be fed from the compiled-in registry, and
    /// that registry must be non-empty and free of `zsh/main` — the
    /// `link=static` shell core, which upstream's `$module_path` scan
    /// never offers because no `main.bundle` exists.
    #[test]
    fn registry_backs_the_files_tag() {
        let _g = crate::test_util::global_state_lock();
        let names = loadable_module_names();
        assert!(
            names.len() > 20,
            "registry has {} names; register_builtin_modules seeds ~36",
            names.len()
        );
        assert!(
            !names.iter().any(|n| n == "zsh/main"),
            "zsh/main is the static core, not a loadable module"
        );
        // The nested names are what make `/`-separated completion the
        // right shape for this tag (`net/`, `param/` as stubs).
        assert!(names.iter().any(|n| n.contains('/') && n != "zsh/main"));
        assert!(names.iter().all(|n| n.starts_with("zsh/")));
        assert!(
            names.iter().all(|n| !n.contains(char::is_whitespace)),
            "a name with whitespace would break _multi_parts' literal `(a b c)` form"
        );
    }

    /// sh:72's glob is kept verbatim so a real cdylib under
    /// `$module_path` still completes.
    #[test]
    fn upstream_module_file_glob_is_unchanged() {
        assert_eq!(MODULE_FILE_GLOB, "*.(dll|s[ol]|bundle)(:r)");
    }

    /// sh:3/sh:4 declare EIGHT names local. A port that only hands a
    /// name to a callee (`expl` to `_description`;
    /// `line`/`opt_args`/`NORMARG`/`state` to `_arguments`) still owes
    /// the declaration — otherwise the callee creates the parameter at
    /// level 0 and it outlives the completion. `curcontext` is the one
    /// that matters most: `_arguments -C` (sh:7) REWRITES it, so a
    /// missing `local curcontext="$curcontext"` poisons every later
    /// completion in the same shell.
    #[test]
    fn sh3_locals_do_not_leak() {
        let _g = crate::test_util::global_state_lock();
        crate::test_util::reset_completion_state();
        const NAMES: [&str; 7] = ["suf", "comp", "state", "line", "expl", "NORMARG", "opt_args"];
        for n in NAMES {
            crate::ported::params::unsetparam(n);
        }
        // The caller's `curcontext`, which sh:3 saves and `_arguments -C`
        // would otherwise overwrite for good.
        let sentinel = ":completion::complete:__caller__::";
        let _ = crate::ported::params::setsparam("curcontext", sentinel);

        crate::ported::utils::inc_locallevel();
        let inner = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = _zmodload(&[]);
        }));
        crate::ported::params::endparamscope();

        let survivors: Vec<&str> = NAMES
            .iter()
            .copied()
            .filter(|n| {
                crate::ported::params::paramtab()
                    .read()
                    .map(|t| t.get(*n).is_some())
                    .unwrap_or(false)
            })
            .collect();
        let ctx = crate::ported::params::getsparam("curcontext").unwrap_or_default();
        crate::ported::params::unsetparam("curcontext");
        crate::test_util::reset_completion_state();
        if let Err(p) = inner {
            std::panic::resume_unwind(p);
        }
        assert!(
            survivors.is_empty(),
            "sh:3/sh:4 locals survived the call: {survivors:?}"
        );
        assert_eq!(
            ctx, sentinel,
            "sh:3's `curcontext=\"$curcontext\"` must restore the caller's value"
        );
    }

    /// Both names this file publishes must be reachable through the
    /// router. `_zmodload` is what `#compdef zmodload` dispatches to;
    /// `_zmodload_module_files` is reached BY NAME from sh:71's
    /// `_requested` (through `_all_labels` sh:39), so an unrouted name
    /// would surface as `command not found` in the middle of the tag
    /// loop and the `files` tag would go silent again.
    #[test]
    fn both_entry_points_are_routed() {
        assert!(crate::compsys::router::is_intercepted("_zmodload"));
        assert!(crate::compsys::router::is_intercepted("_zmodload_module_files"));
    }

    /// The registry names must be exactly the set `zmodload NAME`
    /// resolves. Offering a name the shell cannot load is the failure
    /// this tag is supposed to stop having.
    #[test]
    fn every_offered_name_is_module_linked() {
        let _g = crate::test_util::global_state_lock();
        let names = loadable_module_names();
        let table = crate::ported::module::MODULESTAB.lock().unwrap();
        for n in &names {
            assert!(
                table.module_linked(n),
                "`{n}` is offered by the files tag but is not module_linked"
            );
        }
    }
}
