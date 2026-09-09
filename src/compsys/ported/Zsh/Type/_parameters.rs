//! Port of `_parameters` from `Completion/Zsh/Type/_parameters`.
//!
//! Full upstream body (58 lines verbatim, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 9  local i pfilt
//! sh:10  local -i nm=$compstate[nmatches]
//! sh:11  local -a expl pattern=( -g \* ) normal described verbose faked fakes tmp
//! sh:14  zstyle -t ":completion:${curcontext}:parameters" prefix-needed &&
//! sh:15      [[ $PREFIX != [_.]* ]] &&
//! sh:16          pfilt='[_.]*'
//! sh:18  [[ $IPREFIX = *\$ ]] && pfilt+='|*.*'
//! sh:20  _description parameters expl parameter
//! sh:21  zparseopts -D -K -E g:=pattern
//! sh:23  if zstyle -t ":completion:${curcontext}:parameters" extra-verbose; then
//! sh:24    described=(
//! sh:25        ${(k)parameters[(R)$~pattern[2]~*(hideval|local|special)*]:#$~pfilt}
//! sh:26    )
//! sh:27    compadd "$@" "$expl[@]" -D described -a - described
//! sh:28    if (( $#described )); then
//! sh:33      verbose=(
//! sh:34          ${${${(f@)"$( typeset -m ${(@b)described} )"}/=/:}[@]//'\'/'\\'}
//! sh:35      )
//! sh:36      _describe -t parameters parameter verbose "$@" "$expl[@]"
//! sh:37    fi
//! sh:39    normal=(
//! sh:40        ${(k)parameters[(R)$~pattern[2]~^(*(hideval|special)*)~*local*]:#$~pfilt}
//! sh:41    )
//! sh:42  else
//! sh:43    normal=( ${(k)parameters[(R)${~pattern[2]}~*local*]:#$~pfilt} )
//! sh:44  fi
//! sh:46  if zstyle -a ":completion:${curcontext}:" fake-parameters tmp; then …
//! sh:55  compadd "$@" "$expl[@]" - "$normal[@]" "${(@)fakes:|described}" \
//! sh:56      "${(@)${(@)${(@M)faked:#${~pattern[2]}}%%:*}:|described}"
//! sh:58  (( compstate[nmatches] > nm ))
//! ```
//!
//! `$parameters` is the shell-side assoc-array mapping param name
//! to its zsh-type string ("integer", "array", "scalar-export", …).
//! We enumerate it through `scanpmparameters`, the scan that backs
//! the `parameter` module's own `$parameters` (`Src/Modules/parameter.c:124`),
//! so the keys are the keys zsh has and the values are the REAL
//! `paramtypestr` (c:43) — meaning the `(R)pattern`
//! glob sees the full modifier suffix chain (`-local`, `-readonly`,
//! `-export`, `-hideval`, `-special`, …). A previous revision hand-rolled a
//! bare `PM_TYPE`-only string; every upstream filter that keys on a
//! modifier (`~*(hideval|local|special)*` at sh:25, `~^(*(hideval|special)*)`
//! at sh:40, `_command_names` sh:39's `^*(readonly|association)*`) silently
//! matched the wrong set against it.
//!
//! `$~pfilt` excludes names matching that pattern.
//!
//! BOTH arms of sh:23 are ported. Under `extra-verbose` the parameter set is
//! split in two: sh:25 keeps the ones whose value may be shown and hands them
//! to `_describe` at sh:36 with `name:value` descriptions built from a real
//! `typeset -m` listing, and sh:40 keeps the `hideval`/`special` remainder for
//! the plain `compadd` at sh:55. The two filters partition the same non-local
//! set the `else` arm at sh:43 offers as one flat group, which is what
//! `described_and_normal_partition_the_non_local_set` pins.
//!
//! zsh 5.9 spells the same two filters with `${(@M)…:#$~pfilt*}` and
//! `pfilt='[^_.]'`; the `(M)` inversion makes that the identical filter. The
//! port follows the newer `:#$~pfilt` / `pfilt='[_.]*'` spelling transcribed
//! above, because sh:18's `pfilt+='|*.*'` only composes with that one.

use crate::compsys::ported::_describe::_describe;
use crate::compsys::ported::_description::_description;
use crate::compsys::ported::shared::{capture_builtin_stdout, zstyle_t, LocalScope};
use crate::ported::builtin::{bin_typeset, BIN_TYPESET};
use crate::ported::modules::parameter::scanpmparameters;
use crate::ported::modules::zutil::{bin_zparseopts, lookupstyle};
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::utils::quotestring;
use crate::ported::zle::compcore::get_compstate_str;
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{
    options, MAX_OPS, PM_ARRAY, PM_READONLY, PM_SCALAR, QT_BACKSLASH_PATTERN, SCANPM_MATCHVAL,
};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// C's `ScanFunc` closes over nothing — it is a bare function pointer
/// (`Src/zsh.h`, `typedef void (*ScanFunc) (HashNode, int)`), and every C
/// caller that needs the scan's OUTPUT parks it in a file-scope static
/// while the scan runs (`Src/params.c:4225` `scancount`,
/// `Src/params.c:4340` `paramvalarr` → `mflags`/`keyfoo`). This is that
/// static, made thread-local because zshrs runs completion off the main
/// thread; `scanparamtypes` below is the `ScanFunc`.
thread_local! {
    static SCANNED_PARAMS: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The `ScanFunc` [`enumerate_params`] hands to `scanpmparameters` —
/// c:145's `func(&pm.node, flags)` callee. Collects `(name, value)`, which
/// for `$parameters` is `(name, paramtypestr(pm))` (c:144).
fn scanparamtypes(pm: &crate::ported::zsh_h::param, _flags: i32) {
    let name = pm.node.nam.clone();
    let ty = pm.u_str.clone().unwrap_or_default();
    SCANNED_PARAMS.with(|c| c.borrow_mut().push((name, ty)));
}

/// The `$parameters` assoc sh:25, sh:40 and sh:43 read, as
/// (name, zsh-type-string, flags) triples.
///
/// This is a SCAN of `$parameters`, so it runs the real
/// `scanpmparameters` (`Src/Modules/parameter.c:124`) — the `scanfn` the
/// `parameters` special parameter is declared with at c:2306 — rather than
/// re-deriving what that scan reports. `SCANPM_MATCHVAL` is the flag the
/// shell's `[(R)…]` value subscript scans with, and it is what makes
/// c:140-144 compute each value at all.
///
/// That value is c:43 `paramtypestr`. The modifier suffixes it appends
/// (`-local` c:63, `-readonly` c:75, `-export` c:81, `-hideval` c:87,
/// `-special` c:89, …) are load-bearing: sh:25 and sh:40 partition the
/// parameter set on exactly those substrings. The third element is c:129's
/// `pm.node.flags`, the flags C gives the SCAN's node (`PM_SCALAR |
/// PM_READONLY`), not the scanned param's own — no caller here reads it.
///
/// **Why not `paramtab` directly.** It was, and that dropped c:138's
/// `if (((Param)hn)->node.flags & PM_UNSET) continue;`. A PM_UNSET param
/// keeps its `paramtab` node — `unset RANDOM` does not remove `RANDOM`,
/// it flags it — and c:48 renders such a node's type as the empty string,
/// which matches sh:43's default `$pattern[2]` of `*` and contains no
/// `local`, so the name was offered. zsh, whose scan never reaches the
/// node, offers nothing: after `unset RANDOM`, `echo $RAND<TAB>` completed
/// to `$RANDOM` here and stayed `$RAND` there. It also dropped c:49-50's
/// `"undefined"` for a still-untouched `zsh/parameter` autoload stub, which
/// `scanpmparameters` tracks separately. Reading a table that a builtin or
/// a special parameter FILTERS, without reproducing the filter, is the same
/// defect class as `_limits` offering a `resident` that `limit` never prints.
fn enumerate_params() -> Vec<(String, String, i32)> {
    SCANNED_PARAMS.with(|c| c.borrow_mut().clear());
    scanpmparameters(
        std::ptr::null_mut(),
        Some(scanparamtypes),
        SCANPM_MATCHVAL as i32,
    );
    // c:129 — the flags C stamps on the scan node, for every entry alike.
    let node_flags = (PM_SCALAR | PM_READONLY) as i32;
    SCANNED_PARAMS.with(|c| {
        std::mem::take(&mut *c.borrow_mut())
            .into_iter()
            .map(|(name, ty)| (name, ty, node_flags))
            .collect()
    })
}

/// sh:25's `~*(hideval|local|special)*` — the type strings that reach
/// `_describe` at sh:36 and so have their VALUE shown.
///
/// `~PAT` is EXTENDED_GLOB's pattern exclusion, and the three words are
/// substrings of the modifier chain `paramtypestr` appends
/// (`Src/Modules/parameter.c:63-90`), not flag bits: `-hideval` c:87,
/// `-special` c:89, `-local` c:63. Each is excluded for its own reason —
/// `hideval` because the user asked for the value to stay hidden, `special`
/// because reading one can have a side effect, `local` because sh:4's
/// contract is "completes only non-local parameters".
fn is_described_type(ty: &str) -> bool {
    !(ty.contains("hideval") || ty.contains("local") || ty.contains("special"))
}

/// sh:40's `~^(*(hideval|special)*)~*local*` — the type strings that reach
/// the plain `compadd` at sh:55 while the `extra-verbose` branch is running.
///
/// `~^(…)` excludes what does NOT match, i.e. it KEEPS only the type strings
/// that do contain `hideval` or `special`; `~*local*` then drops the locals.
/// Within one `$pattern[2]` set this is the exact complement of
/// [`is_described_type`] minus the locals both arms already drop — see
/// `described_and_normal_partition_the_non_local_set`.
fn is_extra_verbose_normal_type(ty: &str) -> bool {
    (ty.contains("hideval") || ty.contains("special")) && !ty.contains("local")
}

/// One element of sh:33's `verbose` array, from one `typeset -m` line.
///
/// `${…/=/:}` rewrites the FIRST `=` — the one `printparamnode` puts between
/// name and value (`Src/params.c:6290`) — so `_describe` reads the line as
/// `name:description`; a later `=` inside the value is left alone.
/// `${…//'\'/'\\'}` then doubles every backslash, because the description
/// reaches `compadd` as a `-d` display string.
fn verbose_line(line: &str) -> String {
    let colonised = match line.find('=') {
        Some(i) => format!("{}:{}", &line[..i], &line[i + 1..]),
        None => line.to_string(),
    };
    colonised.replace('\\', "\\\\")
}

/// sh:34's `$( typeset -m ${(@b)described} )` — the `name=value` listing
/// `_describe` turns into descriptions at sh:36.
///
/// The `typeset -m` half is kept VERBATIM: this really does run
/// [`bin_typeset`] with `-m` set in `ops` and the `(b)`-quoted names as its
/// arguments, so every line is rendered by the same
/// [`crate::ported::params::printparamnode`] the builtin would reach
/// (`Src/builtin.c:3090-3095` → `Src/params.c:6123`) and nothing about the
/// value formatting — the single-quoting of values with spaces, `array=( … )`,
/// the `PM_UNSET` skip at c:3078-3079, the `hnamcmp` ordering at c:3083 — is
/// hand-rolled here. A previous revision of this file hand-rolled a
/// `PM_TYPE`-only type string for a different filter and every upstream test
/// that keyed on a modifier silently matched the wrong set; the same mistake
/// on the VALUE side would silently mis-describe every parameter.
///
/// Only the `$( … )` plumbing is replaced. Upstream needs a command
/// substitution because a shell has no other way to get a builtin's stdout
/// into an array (and needs `-m` because a bare `typeset NAME` inside a
/// function DECLARES instead of printing — sh:29-32's own comment).
/// [`capture_builtin_stdout`] reaches the identical bytes in-process; see
/// there for why that is not a pipe and what an empty return means. Here an
/// empty return leaves `verbose` empty and costs sh:36 its descriptions,
/// adding nothing wrong.
fn typeset_m_capture(names: &[String]) -> String {
    // `${(@b)described}` — c:Src/subst.c:2259 sets `QT_BACKSLASH_PATTERN`,
    // whose body (c:Src/utils.c:6242-6248) backslash-escapes the pattern
    // metacharacters. Without it a parameter whose name holds one (`*`, `?`,
    // `#`, `[`) would reach `typeset -m` as a PATTERN and list its neighbours.
    let quoted: Vec<String> = names
        .iter()
        .map(|n| quotestring(n, QT_BACKSLASH_PATTERN))
        .collect();

    capture_builtin_stdout(false, || {
        // `typeset -m` — the `-m` bit is `OPT_MINUS`, i.e. `ind[c] & 1`
        // (`Src/zsh.h:1402`). `m` is not in `TYPESET_OPTSTR`
        // (`Src/zsh.h:1947`, "aiEFALRZlurtxUhHT"), so it contributes no
        // `PM_*` bit to `on`/`off`: the call lands on c:3090-3095's pure
        // listing arm with `PRINT_INCLUDEVALUE | PRINT_WITH_NAMESPACE`.
        let mut ops = make_ops();
        ops.ind[b'm' as usize] = 1;
        let _ = bin_typeset("typeset", &quoted, &ops, BIN_TYPESET);
    })
}

/// Call `_parameters` by NAME, the way the upstream shell code does.
///
/// The shell contexts that end in `_parameters` (`_brace_parameter`,
/// `_subscript`, `_parameter`, …) write a plain command word, so `$fpath`
/// arbitration applies: a user's or plugin's own `_parameters` file is
/// autoloaded instead of the stock one. `dispatch_function_call` runs that
/// arbitration (`compsys::router::try_rust_dispatch` → `has_fpath_override`);
/// calling [`_parameters`] as a Rust fn skips it and pins the port, which
/// silently kills the override. Falls back to the port when there is no
/// executor in scope (unit tests).
pub fn call_parameters(args: &[String]) -> i32 {
    crate::ported::exec::dispatch_function_call("_parameters", args)
        .unwrap_or_else(|| _parameters(args))
}

/// `_parameters` — complete non-local parameter names. `-g <pat>`
/// filters by parameter type-string.
///
/// Callers inside other ported completers must use [`call_parameters`], not
/// this fn, so an `$fpath` override still wins.
pub fn _parameters(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_parameters");
    // sh:10  local -i nm=$compstate[nmatches]
    let nm: i64 = get_compstate_str("nmatches")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    // sh:11
    let mut pattern_seed: Vec<String> = vec!["-g".to_string(), "*".to_string()];

    // sh:14-16  prefix-needed handling
    let curcontext = getsparam("curcontext").unwrap_or_default();
    // sh:18 — `zstyle -t … prefix-needed`, a VALUE test; see [`zstyle_t`].
    let prefix_needed = zstyle_t(
        &format!(":completion:{}:parameters", curcontext),
        "prefix-needed",
    ) == 0;
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let mut pfilt = String::new();
    if prefix_needed && !prefix.starts_with('_') && !prefix.starts_with('.') {
        pfilt = "[_.]*".to_string();
    }
    // sh:18
    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    if iprefix.ends_with('$') {
        if pfilt.is_empty() {
            pfilt = "*.*".to_string();
        } else {
            pfilt.push_str("|*.*");
        }
    }

    // sh:20
    let _ = _description(&[
        "parameters".to_string(),
        "expl".to_string(),
        "parameter".to_string(),
    ]);

    // sh:21  zparseopts -D -K -E g:=pattern
    let src = "__compsys_argv";
    crate::compsys::ported::shared::set_bridge_argv(src, args);
    setaparam("pattern", pattern_seed.clone());
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-D".to_string(),
            "-K".to_string(),
            "-E".to_string(),
            "-v".to_string(),
            src.to_string(),
            "g:=pattern".to_string(),
        ],
        &make_ops(),
        0,
    );
    pattern_seed = getaparam("pattern").unwrap_or_default();
    let pattern_val = pattern_seed
        .get(1)
        .cloned()
        .unwrap_or_else(|| "*".to_string());
    let argv = getaparam(src).unwrap_or_default();
    // Tear down `__compsys_argv` — the zparseopts-bridge scratch array, not a
    // real zsh identifier (zsh operates on positional $argv). It is declared
    // FUNCTION-LOCAL by `shared::set_bridge_argv`; this unset is what clears it
    // when the port runs outside any function scope. Bug #657.
    crate::ported::params::unsetparam(src);

    // Build the filter against (R)pattern + excl PM_LOCAL + pfilt.
    let pat_prog = patcompile(
        &{
            let mut __pat_tok = (&pattern_val).to_string();
            crate::ported::glob::tokenize(&mut __pat_tok);
            __pat_tok
        },
        0,
        None,
    );
    let pfilt_prog = if pfilt.is_empty() {
        None
    } else {
        patcompile(
            &{
                let mut __pat_tok = (&pfilt).to_string();
                crate::ported::glob::tokenize(&mut __pat_tok);
                __pat_tok
            },
            0,
            None,
        )
    };

    let all_params = enumerate_params();
    let expl = getaparam("expl").unwrap_or_default();

    // `(R)$~pattern[2]` — the assoc subscript matches the VALUE, i.e. the
    //   `paramtypestr` string, not the parameter name. Shared by sh:25, sh:40
    //   and sh:43.
    let val_matches = |ty: &str| match pat_prog.as_ref() {
        Some(p) => pattry(p, ty),
        None => ty == pattern_val,
    };
    // `:#$~pfilt` — drop the names that MATCH pfilt. Shared by all three.
    let pfilt_drops = |name: &str| match pfilt_prog.as_ref() {
        Some(prog) => pattry(prog, name),
        None => false,
    };

    let mut normal: Vec<String> = Vec::new();
    // sh:11's `described`. Empty unless the sh:23 branch runs; sh:55-56's two
    // `${…:|described}` set-differences read it back.
    let mut described_final: Vec<String> = Vec::new();

    // sh:23  if zstyle -t ":completion:${curcontext}:parameters" extra-verbose
    let extra_verbose = zstyle_t(
        &format!(":completion:{}:parameters", curcontext),
        "extra-verbose",
    ) == 0;

    if extra_verbose {
        // Two of sh:11's `local -a` names have to exist as REAL parameters,
        // because the builtins they are handed to read their arrays out of
        // `paramtab` BY NAME: `described` for sh:27's `compadd -D … -a -`,
        // `verbose` for sh:36's `_describe`. The rest of sh:11's list stays as
        // Rust locals, which is what the `else` arm already does. `LocalScope`
        // is what puts the caller's values back on the way out, standing in
        // for `endparamscope` (`Src/params.c:5867-5933`) — without it a
        // completion would leave two arrays behind in the user's shell.
        let _locals = LocalScope::declare(&["described", "verbose"], PM_ARRAY);

        // sh:24-26
        //   described=(
        //       ${(k)parameters[(R)$~pattern[2]~*(hideval|local|special)*]:#$~pfilt}
        //   )
        // `~PAT` is a pattern EXCLUSION under EXTENDED_GLOB: keep the type
        // strings that match `$pattern[2]` and contain NONE of the three
        // modifier words. Those three are exactly the ones whose value must
        // not be shown — `hideval` by request, `special` because reading one
        // can have side effects, `local` because sh:4's contract is
        // "completes only non-local parameters".
        // (zsh 5.9 spells sh:25 as `"${(@M)${(@k)parameters[…]}:#$~pfilt*}"`
        // with `pfilt='[^_.]'`; the `(M)` inversion makes that the same
        // filter, and the port follows the newer `:#$~pfilt` spelling because
        // sh:18's `pfilt+='|*.*'` only composes with that one.)
        let mut described: Vec<String> = Vec::new();
        for (name, ty, _flags) in &all_params {
            if !val_matches(ty) {
                continue;
            }
            if pfilt_drops(name) {
                continue;
            }
            if !is_described_type(ty) {
                continue;
            }
            described.push(name.clone());
        }
        described.sort();
        setaparam("described", described);

        // sh:27  compadd "$@" "$expl[@]" -D described -a - described
        //
        // This call adds NO MATCHES. c:Src/Zle/compcore.c:2178 computes
        //     doadd = (!dat->apar && !dat->opar && !dat->dpar);
        // so with `-D` set the walk only decides which words WOULD have
        // matched: c:2519-2523 and c:2540-2543 advance the dpar cursor past a
        // rejected word, c:2571-2578 collect the element belonging to a kept
        // one, and c:2606-2607 writes the survivors back over the named array.
        // Its entire job is to filter `described` down to the names that match
        // the current word before sh:33 renders their values.
        //
        // Getting this wrong in the "it adds matches too" direction
        // double-lists every described parameter; getting it wrong in the
        // "it empties the array" direction makes sh:28 false and silently
        // drops the whole described group.
        let mut dcmd: Vec<String> = argv.clone();
        dcmd.extend(expl.iter().cloned());
        dcmd.push("-D".to_string());
        dcmd.push("described".to_string());
        dcmd.push("-a".to_string());
        dcmd.push("-".to_string());
        dcmd.push("described".to_string());
        let _ = bin_compadd("compadd", &dcmd, &make_ops(), 0);
        described_final = getaparam("described").unwrap_or_default();

        // sh:28  if (( $#described )); then
        if !described_final.is_empty() {
            // sh:33-35
            //   verbose=(
            //       ${${${(f@)"$( typeset -m ${(@b)described} )"}/=/:}[@]//'\'/'\\'}
            //   )
            // `(f@)` splits the listing on newlines — one element per
            // parameter, since `printparamnode` ends every entry with one.
            let verbose: Vec<String> = typeset_m_capture(&described_final)
                .lines()
                .map(verbose_line)
                .collect();
            setaparam("verbose", verbose);

            // sh:36  _describe -t parameters parameter verbose "$@" "$expl[@]"
            let mut dsc: Vec<String> = vec![
                "-t".to_string(),
                "parameters".to_string(),
                "parameter".to_string(),
                "verbose".to_string(),
            ];
            dsc.extend(argv.iter().cloned());
            dsc.extend(expl.iter().cloned());
            let _ = _describe(&dsc);
        }

        // sh:39-41
        //   normal=(
        //       ${(k)parameters[(R)$~pattern[2]~^(*(hideval|special)*)~*local*]:#$~pfilt}
        //   )
        // `~^(…)` excludes what does NOT match, i.e. keeps ONLY the type
        // strings that DO contain `hideval` or `special`; `~*local*` then
        // drops the locals. Within the `$pattern[2]` set this is the exact
        // complement of sh:25, so the two arms partition the same set the
        // `else` arm at sh:43 would have offered in one flat group — the
        // described half through `_describe`, the rest through sh:55.
        for (name, ty, _flags) in &all_params {
            if !val_matches(ty) {
                continue;
            }
            if pfilt_drops(name) {
                continue;
            }
            if !is_extra_verbose_normal_type(ty) {
                continue;
            }
            normal.push(name.clone());
        }
    } else {
        // sh:43  normal=( ${(k)parameters[(R)${~pattern[2]}~*local*]:#$~pfilt} )
        for (name, ty, _flags) in &all_params {
            if !val_matches(ty) {
                continue;
            }
            if pfilt_drops(name) {
                continue;
            }
            // sh:43's `~*local*` is a plain substring test against the type
            //   string. The previous port tested `flags & PM_LOCAL` instead —
            //   which never matches, because `createparam` CLEARS that bit once
            //   it has used it to stamp `pm->level` (`Src/params.c:1155`). Locals
            //   were therefore never filtered at all.
            if ty.contains("local") {
                continue;
            }
            normal.push(name.clone());
        }
    }
    normal.sort();

    // sh:46-54  fake-parameters
    let fake_vals = lookupstyle(&format!(":completion:{}:", curcontext), "fake-parameters");
    let mut faked: Vec<String> = Vec::new();
    let mut fakes: Vec<String> = Vec::new();
    for v in fake_vals {
        if v.contains(':') {
            faked.push(v);
        } else {
            fakes.push(v);
        }
    }
    // Faked names whose declared type (after `:`) matches `pattern_val`
    let faked_matching: Vec<String> = faked
        .iter()
        .filter_map(|s| {
            let mut parts = s.splitn(2, ':');
            let name = parts.next()?.to_string();
            let ty = parts.next()?;
            let matches = match pat_prog.as_ref() {
                Some(p) => pattry(p, ty),
                None => ty == pattern_val,
            };
            if matches {
                Some(name)
            } else {
                None
            }
        })
        .collect();

    // sh:55  compadd "$@" "$expl[@]" - "$normal[@]" "${(@)fakes:|described}" \
    // sh:56      "${(@)${(@)${(@M)faked:#${~pattern[2]}}%%:*}:|described}"
    //   `${a:|b}` is the set DIFFERENCE — elements of `a` that are not in
    //   `b`. Under extra-verbose the described names were already offered by
    //   `_describe` at sh:36, so re-adding a fake by the same name would
    //   double-list it.
    let mut combined: Vec<String> = normal;
    combined.extend(fakes.into_iter().filter(|f| !described_final.contains(f)));
    combined.extend(
        faked_matching
            .into_iter()
            .filter(|f| !described_final.contains(f)),
    );
    combined.sort();
    combined.dedup();

    let mut compadd_argv = argv;
    compadd_argv.extend(expl);
    compadd_argv.push("-".to_string());
    compadd_argv.extend(combined);
    let _ = bin_compadd("compadd", &compadd_argv, &make_ops(), 0);

    // sh:58  (( compstate[nmatches] > nm )) — the real counter, not "did
    //   this call pass a non-empty list". `_describe` at sh:36 can be the
    //   only thing that added, and an empty `normal` at sh:55 does not make
    //   the function fail in that case.
    let nm_after = get_compstate_str("nmatches")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    if nm_after > nm {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn enumerate_params_returns_some_entries() {
        // Param table should never be empty at runtime (zshrs init
        //   creates many special params).
        let _g = crate::test_util::global_state_lock();
        let entries = enumerate_params();
        assert!(!entries.is_empty(), "paramtab unexpectedly empty");
    }

    /// c:Src/Modules/parameter.c:138 — `scanpmparameters` SKIPS a PM_UNSET
    /// node, so an unset parameter is not a key of `$parameters` and sh:43
    /// cannot offer it. `unset` on a param whose node survives (every
    /// PM_SPECIAL one) is exactly that case, and it is what made
    /// `echo $RAND<TAB>` complete to `$RANDOM` after `unset RANDOM` while
    /// zsh left the word alone.
    #[test]
    fn unset_parameter_is_not_enumerated() {
        let _g = crate::test_util::global_state_lock();
        const NAME: &str = "_ZSHRS_PARAMS_UNSET_PROBE";

        crate::ported::params::setsparam(NAME, "x");
        assert!(
            enumerate_params().iter().any(|(n, _, _)| n == NAME),
            "a set parameter must be enumerated"
        );

        // Flag the live node PM_UNSET without removing it — the state
        // `unset` leaves a special parameter in (c:48 then renders its type
        // as the empty string, which matches sh:43's default `*` pattern).
        {
            let mut tab = crate::ported::params::paramtab().write().unwrap();
            let pm = tab.get_mut(NAME).expect("probe param vanished");
            pm.node.flags |= crate::ported::zsh_h::PM_UNSET as i32;
        }
        let entries = enumerate_params();
        assert!(
            !entries.iter().any(|(n, _, _)| n == NAME),
            "PM_UNSET parameter must not be enumerated (c:138)"
        );

        let mut tab = crate::ported::params::paramtab().write().unwrap();
        tab.remove(NAME);
    }

    #[test]
    fn returns_one_or_zero_no_panic() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let _r = _parameters(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
    }

    /// sh:25 and sh:40 must PARTITION the non-local parameters: every type
    /// string that is not `-local` belongs to exactly one of the two arms.
    ///
    /// This is the invariant the `extra-verbose` branch trades on. Under it
    /// the described half is offered by `_describe` at sh:36 and the rest by
    /// `compadd` at sh:55, so the user must still see exactly the set the
    /// `else` arm at sh:43 would have offered in one flat group. A filter
    /// that overlaps double-lists a parameter; one that leaves a gap drops it
    /// silently, which is what "the described group came out short" looked
    /// like the first time this branch was attempted.
    ///
    /// Driven over every string `paramtypestr`
    /// (`Src/Modules/parameter.c:43`) can produce for the relevant flags,
    /// not over a hand-written list, so a new modifier suffix cannot slip
    /// past both filters unnoticed.
    #[test]
    fn described_and_normal_partition_the_non_local_set() {
        use crate::ported::zsh_h::{
            PM_ARRAY, PM_EXPORTED, PM_HASHED, PM_HIDE, PM_HIDEVAL, PM_INTEGER, PM_READONLY,
            PM_SCALAR, PM_SPECIAL, PM_TIED, PM_UNIQUE,
        };
        let _g = crate::test_util::global_state_lock();
        let types = [PM_SCALAR, PM_ARRAY, PM_INTEGER, PM_HASHED];
        // `hide` is in the list on purpose: it SHARES the prefix `hide` with
        // `hideval`, so a filter written as a `contains("hide")` substring
        // test would wrongly route every `typeset -h` parameter into the
        // plain group. Only `-hideval` may.
        let mods = [
            0,
            PM_READONLY,
            PM_EXPORTED,
            PM_UNIQUE,
            PM_TIED,
            PM_HIDE,
            PM_HIDEVAL,
            PM_SPECIAL,
            PM_HIDEVAL | PM_SPECIAL,
            PM_HIDE | PM_READONLY,
            PM_SPECIAL | PM_EXPORTED,
        ];
        for level in [0u32, 1u32] {
            for t in types {
                for m in mods {
                    let pm = make_probe_param(t | m, level);
                    let ty = crate::ported::modules::parameter::paramtypestr(&pm);
                    let d = is_described_type(&ty);
                    let n = is_extra_verbose_normal_type(&ty);
                    if level != 0 {
                        // sh:25 and sh:40 BOTH carry `~*local*`: a local is in
                        // neither arm, exactly as sh:43 also excludes it.
                        assert!(
                            !d && !n,
                            "local type {:?} must be in neither arm (described={}, normal={})",
                            ty,
                            d,
                            n
                        );
                        continue;
                    }
                    assert!(
                        d != n,
                        "type {:?} must be in EXACTLY one arm (described={}, normal={})",
                        ty,
                        d,
                        n
                    );
                }
            }
        }
    }

    /// The two arms route on the modifier SUFFIX, not on the base type. Pins
    /// the three words sh:25 names, plus the `hide`/`hideval` near-miss.
    #[test]
    fn type_filters_route_on_the_modifier_suffix() {
        // Plain values are described…
        assert!(is_described_type("scalar"));
        assert!(is_described_type("array"));
        assert!(is_described_type("scalar-readonly-export"));
        // `-hide` is NOT `-hideval`: `typeset -h` hides the parameter from a
        // bare listing, it does not ask for its value to be withheld.
        assert!(is_described_type("scalar-hide"));
        assert!(!is_extra_verbose_normal_type("scalar-hide"));
        // …and hideval / special are not.
        assert!(!is_described_type("scalar-hideval"));
        assert!(is_extra_verbose_normal_type("scalar-hideval"));
        assert!(!is_described_type("array-special"));
        assert!(is_extra_verbose_normal_type("array-special"));
        // A local is excluded from both, whatever else it carries.
        assert!(!is_described_type("scalar-local"));
        assert!(!is_extra_verbose_normal_type("scalar-local-special"));
    }

    /// sh:33's `${${…/=/:}[@]//'\'/'\\'}` on a `typeset -m` line.
    #[test]
    fn verbose_line_rewrites_first_equals_and_doubles_backslashes() {
        // The `=` printparamnode writes becomes the `_describe` separator.
        assert_eq!(verbose_line("HOME=/root"), "HOME:/root");
        // Only the FIRST one — a `=` inside the value is part of the
        // description and must survive.
        assert_eq!(
            verbose_line("LS_COLORS=di=34:ln=35"),
            "LS_COLORS:di=34:ln=35"
        );
        // Every backslash doubled.
        assert_eq!(verbose_line(r"WORDCHARS=a\b"), r"WORDCHARS:a\\b");
        // A line with no `=` at all (nothing upstream produces one, but the
        // `(f@)` split can hand us a stray) passes through but is still
        // backslash-doubled.
        assert_eq!(verbose_line(r"lone\line"), r"lone\\line");
    }

    /// sh:34's capture must leave fd 1 exactly where it found it and take its
    /// scratch file with it.
    ///
    /// The failure this guards is loud and permanent: a `dup2` that is not
    /// undone leaves the user's shell writing every subsequent line of output
    /// into a deleted file in `$TMPPREFIX` instead of onto the terminal, from
    /// the first `$<TAB>` onward. `fstat` on fd 1 identifies the open file
    /// description, so a leaked redirect shows up as a changed `(dev, ino)`.
    ///
    /// What this test does NOT assert is the CONTENT of the capture, because
    /// libtest replaces `println!`'s destination with a per-thread buffer
    /// (`std::io::set_output_capture`, inherited by spawned threads) — under
    /// `cargo test`, `printparamnode`'s bytes never reach fd 1 at all, so
    /// they never reach the redirect either, whatever the redirect does. The
    /// rendering is measured where it is actually observable, on the PTY:
    /// `scripts/comptab_parity.py` and the `screen.py` probe compare the
    /// described group's `NAME -- value` column against real zsh.
    #[test]
    fn typeset_m_capture_restores_fd1_and_removes_its_scratch_file() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::setsparam("_ZSHRS_PV_SCALAR", "a value with spaces");

        let ident = || -> (u64, u64) {
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe { libc::fstat(1, &mut st) } != 0 {
                return (0, 0);
            }
            (st.st_dev as u64, st.st_ino as u64)
        };
        let before = ident();
        let tmpdir = crate::ported::params::getsparam("TMPPREFIX")
            .unwrap_or_else(|| crate::ported::config_h::DEFAULT_TMPPREFIX.to_string());
        let count_scratch = || -> usize {
            let (dir, prefix) = match tmpdir.rfind('/') {
                Some(i) => (tmpdir[..i].to_string(), tmpdir[i + 1..].to_string()),
                None => (".".to_string(), tmpdir.clone()),
            };
            std::fs::read_dir(&dir)
                .map(|it| {
                    it.filter_map(|e| e.ok())
                        .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
                        .count()
                })
                .unwrap_or(0)
        };
        let files_before = count_scratch();

        let _ = typeset_m_capture(&["_ZSHRS_PV_SCALAR".to_string()]);
        crate::ported::params::unsetparam("_ZSHRS_PV_SCALAR");

        assert_eq!(
            before,
            ident(),
            "fd 1 must point at the same open file after the capture"
        );
        assert_eq!(
            files_before,
            count_scratch(),
            "the capture's scratch file under {:?} must be removed",
            tmpdir
        );
    }

    /// A name holding a glob metacharacter must reach `typeset -m` as a
    /// LITERAL — that is what sh:34's `${(@b)described}` is for. Without it
    /// `typeset -m` treats the name as a PATTERN and the capture carries
    /// every parameter it happened to match, so `_describe` describes the
    /// wrong names.
    ///
    /// Pins the QUOTE TYPE, which is the part that can silently be wrong:
    /// `QT_BACKSLASH` (`${(q)…}`) escapes shell metacharacters and leaves
    /// `*`/`?`/`[` alone, so picking it here would look right and glob
    /// anyway. Only `QT_BACKSLASH_PATTERN` (`${(b)…}`,
    /// `Src/utils.c:6242-6248`) escapes the pattern set.
    #[test]
    fn described_names_are_pattern_quoted_for_typeset_m() {
        assert_eq!(
            quotestring("_ZSHRS_PVQ_*", QT_BACKSLASH_PATTERN),
            r"_ZSHRS_PVQ_\*"
        );
        assert_eq!(quotestring("a?b[c]", QT_BACKSLASH_PATTERN), r"a\?b\[c\]");
        // Ordinary names are untouched, so the common case still reaches
        // `typeset -m` byte-for-byte.
        assert_eq!(quotestring("HISTFILE", QT_BACKSLASH_PATTERN), "HISTFILE");
    }

    /// Bare `param` value for the filter matrix above. Only `flags` and
    /// `level` are read by `paramtypestr` (`Src/Modules/parameter.c:46`).
    fn make_probe_param(flags: u32, level: u32) -> crate::ported::zsh_h::param {
        crate::ported::zsh_h::param {
            node: crate::ported::zsh_h::hashnode {
                next: None,
                nam: "probe".to_string(),
                flags: flags as i32,
            },
            u_data: 0,
            u_tied: None,
            u_arr: None,
            u_str: None,
            u_val: 0,
            u_dval: 0.0,
            u_hash: None,
            gsu_s: None,
            gsu_i: None,
            gsu_f: None,
            gsu_a: None,
            gsu_h: None,
            base: 0,
            width: 0,
            env: None,
            ename: None,
            old: None,
            level: level as i32,
        }
    }
}
