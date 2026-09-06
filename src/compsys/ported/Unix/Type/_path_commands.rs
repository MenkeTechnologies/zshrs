//! Port of `_path_commands` from
//! `Completion/Unix/Type/_path_commands`.
//!
//! ```text
//! sh:  3  (( $+functions[_path_commands_caching_policy] )) ||
//! sh:  4  _path_commands_caching_policy() { … cache age check … }
//! sh: 26  _call_whatis() { … whatis -s 1,6,8 OS-dependent … }
//! sh: 57  _path_commands() {
//! sh: 58    local need_desc expl ret=1
//! sh: 60    if zstyle -t ":completion:${curcontext}:commands" extra-verbose; then
//! sh: 61      local update_policy first
//! sh: 62      if [[ $+_command_descriptions -eq 0 ]]; then
//! sh: 63        first=yes; typeset -A -g _command_descriptions
//! sh: 66      zstyle -s ":completion:${curcontext}:" cache-policy update_policy
//! sh: 67      [[ -z "$update_policy" ]] && zstyle … cache-policy _path_commands_caching_policy
//! sh: 69      if ( [[ -n $first ]] || _cache_invalid command-descriptions ) && \
//! sh: 70        ! _retrieve_cache command-descriptions; then
//! sh: 72        for line in "${(f)$(_call_program command-descriptions _call_whatis)}"; do
//! sh: 73          … whatis-line pattern match, $match[1] → $match[3] …
//! sh: 77        _store_cache command-descriptions _command_descriptions
//! sh: 80      (( $#_command_descriptions )) && need_desc=yes
//! sh: 83    if [[ -n $need_desc ]]; then
//! sh: 86      compadd "$@" -O matches -k commands
//! sh: 87-95   split $matches into described (dcmds/descs) and plain (cmds)
//! sh: 96      zstyle -s ":completion:${curcontext}:" list-separator sep || sep=--
//! sh: 97      zformat -a descs " $sep " $descs
//! sh: 98      descs=("${(@r:COLUMNS-1:)descs}")
//! sh: 99      _wanted commands expl 'external command' compadd "$@" -ld descs -a dcmds
//! sh:101      _wanted commands expl 'external command' compadd "$@" -a cmds
//! sh:103    else _wanted commands expl 'external command' compadd "$@" -k commands
//! sh:108    if [[ -o path_dirs ]]; then
//! sh:111      if [[ $PREFIX$SUFFIX = */* ]]; then
//! sh:112        path_dirs=( ${path:#.} )
//! sh:114        _wanted commands expl 'external command' _path_files -W path_dirs -g '*(-*)'
//! sh:116      else path_dirs=(${^path}/*(/N:t))
//! sh:118        _wanted path-dirs expl 'directory in path' compadd "$@" -S / -a path_dirs
//! sh:122    return ret
//! sh:123  }
//! sh:125  _path_commands "$@"
//! ```
//!
//! Two upstream constructs live at FILE scope, outside the
//! `_path_commands` function, and therefore run once when the file is
//! autoloaded: the `_path_commands_caching_policy` definition (sh:3-24,
//! guarded by `$+functions`) and the `_call_whatis` definition
//! (sh:26-55, unguarded). [`install_file_scope_functions`] reproduces
//! both, verbatim, so they are present in `$functions` exactly as they
//! are under zsh — `_which` and anything else enumerating `${(k)functions}`
//! sees the same table.
//!
//! Deviation from the shell, deliberate: sh:69's `( … )` is a SUBSHELL
//! whose only job is to group the `||`. Its two branches (`[[ -n $first ]]`
//! and `_cache_invalid`) assign nothing outside their own locals, so the
//! fork is unobservable and this port evaluates the group in-process.

use crate::compsys::ported::_wanted::_wanted;
use crate::compsys::ported::shared::{declare_locals, PM_ARRAY};
use crate::ported::exec::{dispatch_function_call, getoutput};
use crate::ported::hashtable::shfunctab_lock;
use crate::ported::modules::parameter::setfunction;
use crate::ported::modules::zutil::{bin_zformat, bin_zstyle, lookupstyle};
use crate::ported::params::{
    getaparam, gethkparam, gethparam, getiparam, getsparam, issetvar, setaparam, sethparam,
};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{isset, options, Nularg, MAX_OPS, PATHDIRS};
use indexmap::IndexMap;
use std::sync::Once;

/// sh:4-24 body, verbatim. Installed as the `cache-policy` style by
/// sh:67-68 when the user has not set one, and then called from
/// `_cache_invalid` with the cache file path as `$1`.
const CACHING_POLICY_BODY: &str = r##"
local file
local -a oldp dbfiles

# rebuild if cache is more than a week old
oldp=( "$1"(Nmw+1) )
(( $#oldp )) && return 0

dbfiles=(/usr/share/man/index.(bt|db|dir|pag)(N) \
  /usr/man/index.(bt|db|dir|pag)(N) \
  /var/cache/man/index.(bt|db|dir|pag)(N) \
  /var/catman/index.(bt|db|dir|pag)(N) \
  /usr/share/man/*/whatis(N))

for file in $dbfiles; do
  [[ $file -nt $1 ]] && return 0
done

return 1
"##;

/// sh:27-54 body, verbatim. Called only from sh:72
/// (`_call_program command-descriptions _call_whatis`). Note that on
/// darwin the `case $OSTYPE` at sh:28-41 matches no arm, so `$impl`
/// stays empty, the second `case` at sh:42-54 matches nothing, and the
/// function produces no output — `$_command_descriptions` therefore
/// stays empty there and sh:103 is the branch that runs.
const CALL_WHATIS_BODY: &str = r##"
  local sec impl variant sections=( 1 6 8 )
  case "$OSTYPE" in
    (#i)dragonfly|(free|open)bsd*) impl=mandoc ;;
    netbsd*) impl=apropos ;;
    linux-gnu*)
      sections=( 1 8 )
      # The same test as for man so has a good chance of being cached
      _pick_variant -c man -r variant \
        freebsd='-S mansect' \
        openbsd='-S subsection' \
        $OSTYPE \
        ---
      [[ $variant = $OSTYPE ]] && impl=man-db || impl=mandoc
    ;;
  esac
  case $impl in
    mandoc)
      for sec in $sections; do
        whatis -s $sec .\*
      done
    ;;
    man-db)
      whatis -s ${(j.,.)sections} -r .\*
    ;;
    apropos)
      apropos -l ''|grep "([${(j..)sections}])"
    ;;
  esac
"##;

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `zstyle -t ctx style` — the no-extra-args half of the shared t/T arm
/// quoted at zutil.rs:1244-1259 (`Src/Modules/zutil.c:700-724`): true
/// only when the style is set AND its first value is one of zsh's
/// boolean-true words. `testforstyle` (zutil.rs:976) is the presence
/// check backing `zstyle -q`; it does NOT look at the value, so it is
/// the wrong primitive for `-t`.
fn zstyle_t(ctx: &str, style: &str) -> bool {
    matches!(
        lookupstyle(ctx, style).first().map(String::as_str),
        Some("true") | Some("yes") | Some("on") | Some("1")
    )
}

/// `zstyle -s ctx style name` — Src/Modules/zutil.c:643-658: all values
/// joined with a space; `None` is the "style unset" (`val = 1`) arm.
fn zstyle_s(ctx: &str, style: &str) -> Option<String> {
    let vals = lookupstyle(ctx, style);
    if vals.is_empty() {
        None
    } else {
        Some(crate::ported::utils::sepjoin(&vals, Some(" ")))
    }
}

/// `$_command_descriptions` read back as key→value pairs. `gethkparam`
/// and `gethparam` walk the same `IndexMap` backing in the same order
/// (params.rs:5723 / params.rs:5753), so zipping them is the assoc.
fn command_descriptions() -> IndexMap<String, String> {
    let keys = gethkparam("_command_descriptions").unwrap_or_default();
    let vals = gethparam("_command_descriptions").unwrap_or_default();
    keys.into_iter().zip(vals).collect()
}

/// sh:73-74 — the whatis-line parse, as one function.
///
/// sh:73 keeps a line only when it matches, in full,
/// `(#b)([^ ]#) #\([^ ]#\)( #\[[^ ]#\]|)[ -]#(*)`:
///   `$match[1]` a run of non-blanks (the command name), optional
///   blanks, a parenthesised non-blank run (the man section), then
///   `$match[2]` either blanks + a bracketed non-blank run or nothing,
///   then a run of blanks/hyphens, then `$match[3]` the rest.
/// sh:74 then drops the line when `$match[1]` or `$match[3]` is empty,
/// or when `${match[1]:#*:*}` is empty — i.e. when the name contains a
/// `:`, which would corrupt the `name:description` cache format.
///
/// Returns `Some((name, description))` for a line that survives both.
fn parse_whatis_line(line: &str) -> Option<(String, String)> {
    let b = line.as_bytes();
    let mut i = 0usize;

    // `([^ ]#)` — $match[1]. `[^ ]` admits `(`, so a purely greedy run would
    // swallow `ls(1)` whole; zsh's matcher backtracks until the following
    // ` #\(` can match, which lands the name on `ls`. Stopping at `(` as well
    // as at a blank reproduces that without a backtracking engine: the name is
    // the blank-free run up to whichever comes first, so `ls(1) - …` yields
    // `ls` and `pr (1) [print] - …` yields `pr`.
    let start = i;
    while i < b.len() && b[i] != b' ' && b[i] != b'(' {
        i += 1;
    }
    let name = &line[start..i];

    // ` #` then `\([^ ]#\)`.
    while i < b.len() && b[i] == b' ' {
        i += 1;
    }
    if i >= b.len() || b[i] != b'(' {
        return None;
    }
    i += 1;
    while i < b.len() && b[i] != b' ' && b[i] != b')' {
        i += 1;
    }
    if i >= b.len() || b[i] != b')' {
        return None;
    }
    i += 1;

    // `( #\[[^ ]#\]|)` — $match[2], the optional bracketed field.
    let after_section = i;
    let mut j = i;
    while j < b.len() && b[j] == b' ' {
        j += 1;
    }
    if j < b.len() && b[j] == b'[' {
        j += 1;
        while j < b.len() && b[j] != b' ' && b[j] != b']' {
            j += 1;
        }
        if j < b.len() && b[j] == b']' {
            i = j + 1;
        } else {
            i = after_section;
        }
    }

    // `[ -]#` then `(*)` — $match[3].
    while i < b.len() && (b[i] == b' ' || b[i] == b'-') {
        i += 1;
    }
    let desc = &line[i..];

    // sh:74 — reject empty name/description and names holding a `:`.
    if name.is_empty() || desc.is_empty() || name.contains(':') {
        return None;
    }
    Some((name.to_string(), desc.to_string()))
}

/// Run the file-scope statements upstream executes when
/// `Completion/Unix/Type/_path_commands` is autoloaded. `Once` mirrors
/// that: zsh reads the file the first time `_path_commands` is called
/// and never again, so a later `unfunction _call_whatis` is not undone
/// by the next completion. The Rust router dispatches `_path_commands`
/// directly on every call, so the guard has to live here.
fn install_file_scope_functions() {
    static FILE_SCOPE: Once = Once::new();
    FILE_SCOPE.call_once(|| {
        // sh:3 — `(( $+functions[_path_commands_caching_policy] )) ||`
        let already_defined = shfunctab_lock()
            .read()
            .map(|t| t.get("_path_commands_caching_policy").is_some())
            .unwrap_or(false);
        if !already_defined {
            // sh:4-24
            setfunction(
                "_path_commands_caching_policy",
                CACHING_POLICY_BODY.to_string(),
                0,
            );
        }
        // sh:26-55 — defined unconditionally upstream.
        setfunction("_call_whatis", CALL_WHATIS_BODY.to_string(), 0);
    });
}

/// sh:60-81 — the `extra-verbose` pass: make sure `$_command_descriptions`
/// is populated (from the completion cache, else from `whatis`), which is
/// what sh:80 tests to decide whether the listing carries descriptions.
fn extra_verbose_pass(curcontext: &str) {
    // sh:61 — local update_policy first
    declare_locals(&["update_policy", "first"], 0);

    // sh:62-65 — first call in this shell? `_command_descriptions` is
    // `-g`, so it survives to the next completion and `first` is empty
    // from then on.
    let first = issetvar("_command_descriptions") == 0;
    if first {
        sethparam("_command_descriptions", Vec::new());
    }

    // sh:66-68 — leave a user-set cache-policy alone; otherwise install
    // the file-scope default from sh:4.
    let ctx = format!(":completion:{}:", curcontext);
    let update_policy = zstyle_s(&ctx, "cache-policy").unwrap_or_default();
    if update_policy.is_empty() {
        bin_zstyle(
            "zstyle",
            &[
                ctx.clone(),
                "cache-policy".to_string(),
                "_path_commands_caching_policy".to_string(),
            ],
            &make_ops(),
            0,
        );
    }

    // sh:69-70 — rebuild only when the cache is stale AND cannot be read
    // back. `_retrieve_cache` itself calls `_cache_invalid` (its sh:21),
    // which is where a broken `cache-policy` reports `command not found`.
    let ident = ["command-descriptions".to_string()];
    let stale = first || dispatch_function_call("_cache_invalid", &ident) == Some(0);
    if stale && dispatch_function_call("_retrieve_cache", &ident) != Some(0) {
        // sh:71-76 — parse `whatis` output into the assoc.
        declare_locals(&["line"], 0);
        let out = getoutput("_call_program command-descriptions _call_whatis", 1);
        // qt=1 yields one element: the whole output with trailing
        // newlines stripped, or `Nularg` when the command printed
        // nothing (exec.rs:613-618).
        let nul = Nularg.to_string();
        let text = match out.first() {
            Some(s) if *s != nul => s.clone(),
            _ => String::new(),
        };
        let mut descs = command_descriptions();
        for line in text.split('\n') {
            if let Some((name, desc)) = parse_whatis_line(line) {
                // sh:75 — `_command_descriptions[$match[1]]=$match[3]`
                descs.insert(name, desc);
            }
        }
        let flat: Vec<String> = descs.into_iter().flat_map(|(k, v)| [k, v]).collect();
        sethparam("_command_descriptions", flat);
        // sh:77
        let _ = dispatch_function_call(
            "_store_cache",
            &[
                "command-descriptions".to_string(),
                "_command_descriptions".to_string(),
            ],
        );
    }
}

/// sh:83-101 — the described listing: split the candidate commands into
/// those `$_command_descriptions` knows about and those it does not, and
/// add each set with its own `compadd`.
fn add_with_descriptions(curcontext: &str, args: &[String]) -> bool {
    // sh:84-85
    declare_locals(&["dcmds", "descs", "cmds", "matches"], PM_ARRAY);
    declare_locals(&["desc", "cmd", "sep"], 0);

    // sh:86 — `compadd "$@" -O matches -k commands` collects the
    // candidates without adding them.
    let mut probe = args.to_vec();
    probe.extend([
        "-O".to_string(),
        "matches".to_string(),
        "-k".to_string(),
        "commands".to_string(),
    ]);
    bin_compadd("compadd", &probe, &make_ops(), 0);

    // sh:87-95
    let known = command_descriptions();
    let mut dcmds: Vec<String> = Vec::new();
    let mut descs: Vec<String> = Vec::new();
    let mut cmds: Vec<String> = Vec::new();
    for cmd in getaparam("matches").unwrap_or_default() {
        // sh:88-94 — `desc=$_command_descriptions[$cmd]`; an absent key
        // and a key holding "" both land in the plain `cmds` set.
        match known.get(&cmd) {
            Some(desc) if !desc.is_empty() => {
                descs.push(format!("{}:{}", cmd, desc));
                dcmds.push(cmd);
            }
            _ => cmds.push(cmd),
        }
    }

    // sh:96-97 — `zformat -a descs " $sep " $descs` column-aligns the
    // `name:description` pairs around the separator.
    let sep = zstyle_s(&format!(":completion:{}:", curcontext), "list-separator")
        .unwrap_or_else(|| "--".to_string());
    let mut zf = vec!["descs".to_string(), format!(" {} ", sep)];
    zf.extend(descs);
    let mut zf_ops = make_ops();
    zf_ops.ind[b'a' as usize] = 1;
    bin_zformat("zformat", &zf, &zf_ops, 0);

    // sh:98 — `descs=("${(@r:COLUMNS-1:)descs}")`: right-pad (and, for
    // over-long lines, truncate) every entry to one screen line.
    let width = getiparam("COLUMNS").saturating_sub(1).max(0) as usize;
    let padded: Vec<String> = getaparam("descs")
        .unwrap_or_default()
        .into_iter()
        .map(|d| {
            let n = d.chars().count();
            if n >= width {
                d.chars().take(width).collect()
            } else {
                d + &" ".repeat(width - n)
            }
        })
        .collect();
    setaparam("descs", padded);
    setaparam("dcmds", dcmds);
    setaparam("cmds", cmds);

    // sh:99-100
    let mut ret = false;
    let mut described = vec![
        "commands".to_string(),
        "expl".to_string(),
        "external command".to_string(),
        "compadd".to_string(),
    ];
    described.extend(args.iter().cloned());
    described.extend([
        "-ld".to_string(),
        "descs".to_string(),
        "-a".to_string(),
        "dcmds".to_string(),
    ]);
    if _wanted(&described) == 0 {
        ret = true;
    }

    // sh:101
    let mut plain = vec![
        "commands".to_string(),
        "expl".to_string(),
        "external command".to_string(),
        "compadd".to_string(),
    ];
    plain.extend(args.iter().cloned());
    plain.extend(["-a".to_string(), "cmds".to_string()]);
    if _wanted(&plain) == 0 {
        ret = true;
    }
    ret
}

/// sh:108-120 — with `PATH_DIRS` set, a command word may name a file
/// under a `$path` entry, so offer those too.
fn add_path_dirs(args: &[String]) -> bool {
    // sh:109
    declare_locals(&["path_dirs"], PM_ARRAY);
    let path = getaparam("path").unwrap_or_default();

    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    // sh:111
    if format!("{}{}", prefix, suffix).contains('/') {
        // sh:112 — `path_dirs=( ${path:#.} )`
        setaparam(
            "path_dirs",
            path.into_iter().filter(|p| p != ".").collect::<Vec<_>>(),
        );
        // sh:114 — note this leg does NOT forward `"$@"`.
        let argv = [
            "commands".to_string(),
            "expl".to_string(),
            "external command".to_string(),
            "_path_files".to_string(),
            "-W".to_string(),
            "path_dirs".to_string(),
            "-g".to_string(),
            "*(-*)".to_string(),
        ];
        return _wanted(&argv) == 0;
    }

    // sh:116 — `path_dirs=(${^path}/*(/N:t))`: the basename of every
    // DIRECTORY one level under each `$path` entry, in glob (sorted)
    // order per entry, with a non-matching entry contributing nothing.
    let mut dirs: Vec<String> = Vec::new();
    for p in &path {
        let mut here: Vec<String> = match std::fs::read_dir(p) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => Vec::new(),
        };
        here.sort();
        dirs.extend(here);
    }
    // sh:117
    if dirs.is_empty() {
        return false;
    }
    setaparam("path_dirs", dirs);
    // sh:118
    let mut argv = vec![
        "path-dirs".to_string(),
        "expl".to_string(),
        "directory in path".to_string(),
        "compadd".to_string(),
    ];
    argv.extend(args.iter().cloned());
    argv.extend([
        "-S".to_string(),
        "/".to_string(),
        "-a".to_string(),
        "path_dirs".to_string(),
    ]);
    _wanted(&argv) == 0
}

/// `_path_commands` — complete names of external commands from
/// `$commands` (zsh's PATH cache), optionally annotated with the
/// one-line `whatis` description of each.
pub fn _path_commands(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_path_commands");
    // sh:3-55 — file-scope helper definitions.
    install_file_scope_functions();

    // sh:58 — local need_desc expl ret=1
    declare_locals(&["need_desc", "expl", "ret"], 0);
    let mut ret = 1i32;

    let curcontext = getsparam("curcontext").unwrap_or_default();

    // sh:60-81
    let mut need_desc = false;
    if zstyle_t(
        &format!(":completion:{}:commands", curcontext),
        "extra-verbose",
    ) {
        extra_verbose_pass(&curcontext);
        // sh:80 — `(( $#_command_descriptions )) && need_desc=yes`
        need_desc = !gethparam("_command_descriptions")
            .unwrap_or_default()
            .is_empty();
    }

    // sh:83-104
    let added = if need_desc {
        add_with_descriptions(&curcontext, args)
    } else {
        // sh:103
        let mut argv = vec![
            "commands".to_string(),
            "expl".to_string(),
            "external command".to_string(),
            "compadd".to_string(),
        ];
        argv.extend(args.iter().cloned());
        argv.extend(["-k".to_string(), "commands".to_string()]);
        _wanted(&argv) == 0
    };
    if added {
        ret = 0;
    }

    // sh:108-120
    if isset(PATHDIRS) && add_path_dirs(args) {
        ret = 0;
    }

    // sh:122
    ret
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        // The lock serialises but restores nothing, and a `compadd`-driven
        // test parks a word in `$PREFIX` that makes every later candidate fail
        // to match — so this returned 0 in a full run and 1 on its own.
        crate::test_util::reset_completion_state();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        assert_eq!(_path_commands(&[]), 1);
        INCOMPFUNC.store(0, Ordering::Relaxed);
    }

    /// sh:3-55 — both file-scope helpers must land in `$functions`.
    /// `_which` lists `${(k)functions}`, so a missing name shifts every
    /// subsequent row against reference zsh. This also pins that both
    /// bodies survive `parse_string` (a body zshrs cannot parse would be
    /// dropped by `setfunction` with an `invalid function definition`
    /// warning instead of being installed).
    #[test]
    fn installs_both_file_scope_helpers() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let _ = _path_commands(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        let tab = crate::ported::hashtable::shfunctab_lock()
            .read()
            .expect("shfunctab poisoned");
        assert!(
            tab.get("_path_commands_caching_policy").is_some(),
            "sh:4 _path_commands_caching_policy not installed"
        );
        assert!(
            tab.get("_call_whatis").is_some(),
            "sh:26 _call_whatis not installed"
        );
    }

    /// sh:73-74 — the shapes real `whatis` emits, and the two rejects.
    #[test]
    fn parses_whatis_lines_like_the_shell_pattern() {
        // `name(sec) - description`
        assert_eq!(
            parse_whatis_line("ls(1) - list directory contents"),
            Some(("ls".to_string(), "list directory contents".to_string()))
        );
        // Padded name column and the optional `[alias]` field (sh:73's
        // `( #\[[^ ]#\]|)` group).
        assert_eq!(
            parse_whatis_line("pr (1) [print] - print files"),
            Some(("pr".to_string(), "print files".to_string()))
        );
        // sh:74 — no description ⇒ dropped.
        assert_eq!(parse_whatis_line("orphan(8)"), None);
        // sh:74 — `${match[1]:#*:*}` empty ⇒ a `:` in the name drops it.
        assert_eq!(parse_whatis_line("a:b(1) - nope"), None);
        // No `(section)` at all ⇒ the pattern does not match.
        assert_eq!(parse_whatis_line("just some prose"), None);
    }

    /// sh:60 — `zstyle -t` must test the VALUE, not just presence:
    /// `extra-verbose off` has to leave the cheap sh:103 branch in play.
    #[test]
    fn zstyle_t_tests_the_value_not_just_presence() {
        let _g = crate::test_util::global_state_lock();
        let ctx = ":completion:_path_commands_test:commands";
        for (val, want) in [("on", true), ("off", false), ("1", true), ("no", false)] {
            bin_zstyle(
                "zstyle",
                &[
                    ctx.to_string(),
                    "extra-verbose".to_string(),
                    val.to_string(),
                ],
                &make_ops(),
                0,
            );
            assert_eq!(zstyle_t(ctx, "extra-verbose"), want, "value {val}");
        }
        bin_zstyle(
            "zstyle",
            &["-d".to_string(), ctx.to_string()],
            &make_ops(),
            0,
        );
    }
}
