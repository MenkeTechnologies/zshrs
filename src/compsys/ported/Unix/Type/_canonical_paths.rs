//! Port of `_canonical_paths` from
//! `Completion/Unix/Type/_canonical_paths`.
//!
//! Full upstream body (123 lines, faithful):
//! ```text
//! sh: 17  _canonical_paths_add_paths () {          # recursive helper
//! sh: 23    local origpref=$1 expref rltrim curpref canpref subdir
//! sh: 24    [[ $2 != add ]] && matches=()
//! sh: 25    expref=${~origpref}
//! sh: 26-31 build canpref (:P canonicalization) + rltrim fix-ups
//! sh: 34    if [[ $canpref == $origpref ]]; then     # -M matchspec path
//! sh: 38      compadd -A tmp_buffer "$__gopts[@]" -a files
//! sh: 39      matches+=( "${(@)tmp_buffer/$canpref/$origpref}" )
//! sh: 45    else matches+=(${${(M)files:#$canpref*}/$canpref/$origpref})
//! sh: 48    for subdir in $expref?*(@); do … recurse …
//! sh: 53  _canonical_paths() {
//! sh: 63    zparseopts -D -a __gopts M+: J+: V+: o+: 1 2 n F: x+: X+: A:=__opts N=__opts
//! sh: 65    : ${1:=canonical-paths} ${2:=path}
//! sh: 67-68 -A var → append ${(P)var} to positionals
//! sh: 74    if ! zmodload -F zsh/stat b:zstat; then _wanted … compadd; return
//! sh: 82-86 files=($@) (-N) else files+=($@:P)
//! sh: 91    _canonical_paths_add_paths $base            # base=$PREFIX
//! sh: 93-115 empty base → "/"; ..-only base → back-limit ../ chase
//! sh:117    _wanted "$tag" expl "$desc" compadd $__gopts -Q -U -a matches
//! ```
//!
//! Approximations (available-primitive limits, never faked):
//!  * `:P` (sh:28,85) — zsh's realpath modifier resolves symlinks and
//!    normalizes `.`/`..` WITHOUT requiring the path to exist. Rust's
//!    `std::fs::canonicalize` requires existence, so for
//!    non-existent tails we fall back to a lexical `.`/`..` collapse
//!    (`canon_p`). Symlink resolution therefore only happens for the
//!    existing portion of a path.
//!  * `${~origpref}` (sh:25) — filename generation. We perform tilde
//!    expansion only; glob expansion of a (typically partial) prefix
//!    word is treated as identity (`expand_tilde`).
//!  * `$expref?*(@)` (sh:48) — the `(@)` glob qualifier selects
//!    symlinks; reproduced by scanning the directory and filtering on
//!    `symlink_metadata` (`glob_symlinks`).
//!  * `-ef` (sh:110) — same-file test via `(dev, ino)` comparison
//!    (`same_file`).

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::module::bin_zmodload;
use crate::ported::modules::zutil::{bin_zparseopts, lookupstyle};
use crate::ported::params::{getaparam, getsparam, setaparam, unsetparam};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};
use std::fs;

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `${~origpref}` (sh:25) — tilde expansion only; see module note.
fn expand_tilde(s: &str) -> String {
    if let Some(rest) = s.strip_prefix('~') {
        if rest.is_empty() || rest.starts_with('/') {
            if let Ok(home) = std::env::var("HOME") {
                return format!("{}{}", home, rest);
            }
        }
        // `~user` not resolved (approx); leave the word untouched.
    }
    s.to_string()
}

/// Lexical `.`/`..` collapse for the `:P` fallback (path does not
/// exist so `canonicalize` cannot run). Relative results are made
/// absolute against `$PWD`, matching `:P`'s absolute output.
fn lexical_normalize(path: &str) -> String {
    let abs = if path.starts_with('/') {
        path.to_string()
    } else {
        let pwd = getsparam("PWD")
            .or_else(|| std::env::current_dir().ok().map(|p| p.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "/".to_string());
        format!("{}/{}", pwd.trim_end_matches('/'), path)
    };
    let mut stack: Vec<&str> = Vec::new();
    for comp in abs.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            c => stack.push(c),
        }
    }
    format!("/{}", stack.join("/"))
}

/// `$curpref:P` (sh:28) — realpath-style canonicalization.
fn canon_p(path: &str) -> String {
    match fs::canonicalize(path) {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(_) => lexical_normalize(path),
    }
}

/// `${s/from/to}` — replace first literal occurrence of `from`.
fn replace_first(s: &str, from: &str, to: &str) -> String {
    if from.is_empty() {
        return s.to_string();
    }
    match s.find(from) {
        Some(i) => format!("{}{}{}", &s[..i], to, &s[i + from.len()..]),
        None => s.to_string(),
    }
}

/// `-ef` (sh:110) — do `a` and `b` name the same file?
fn same_file(a: &str, b: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(ma), Ok(mb)) => ma.dev() == mb.dev() && ma.ino() == mb.ino(),
        _ => false,
    }
}

/// `[[ $base == ..(/.(|.))#(|/) ]]` (sh:95) — base is composed only
/// of `.`/`..` components and starts with `..`.
fn is_dotdot_path(base: &str) -> bool {
    if !base.starts_with("..") {
        return false;
    }
    let b = base.strip_suffix('/').unwrap_or(base);
    for (i, comp) in b.split('/').enumerate() {
        if i == 0 {
            if comp != ".." {
                return false;
            }
        } else if comp != "." && comp != ".." {
            return false;
        }
    }
    true
}

/// `$expref?*(@)` (sh:48) — entries whose name extends `expref`'s
/// basename by ≥1 char and which are symlinks.
fn glob_symlinks(expref: &str) -> Vec<String> {
    let (dir, base) = match expref.rfind('/') {
        Some(i) => (&expref[..=i], &expref[i + 1..]),
        None => ("", expref),
    };
    let readdir_path = if dir.is_empty() { "." } else { dir };
    let mut out: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(readdir_path) {
        for ent in entries.flatten() {
            let name = ent.file_name().to_string_lossy().to_string();
            // Default zsh globbing hides leading-dot names unless a
            // literal `.` is in the pattern prefix.
            if base.is_empty() && name.starts_with('.') {
                continue;
            }
            // `?*` after the literal `base` prefix ⇒ ≥1 extra char.
            if name.starts_with(base) && name.len() > base.len() {
                let full = format!("{}{}", dir, name);
                if let Ok(md) = fs::symlink_metadata(&full) {
                    if md.file_type().is_symlink() {
                        out.push(full);
                    }
                }
            }
        }
    }
    out.sort();
    out
}

/// sh:17-51 — recursive helper. `matches` accumulates across
/// recursion (`&mut Vec`); `files_vec` mirrors the `files` param that
/// `compadd -a files` reads; `gopts` is `$__gopts`.
fn _canonical_paths_add_paths(
    origpref_in: &str,
    add: bool,
    gopts: &[String],
    files_vec: &[String],
    matches: &mut Vec<String>,
) {
    // sh:23
    let mut origpref = origpref_in.to_string();
    // sh:24
    if !add {
        matches.clear();
    }
    // sh:25
    let expref = expand_tilde(&origpref);
    // sh:26  [[ $origpref == (|*/). ]] && rltrim=.
    let mut rltrim = String::new();
    if origpref == "." || origpref.ends_with("/.") {
        rltrim = ".".to_string();
    }
    // sh:27  curpref=${${expref%$rltrim}:-./}
    let stripped = if !rltrim.is_empty() && expref.ends_with(&rltrim) {
        expref[..expref.len() - rltrim.len()].to_string()
    } else {
        expref.clone()
    };
    let curpref = if stripped.is_empty() {
        "./".to_string()
    } else {
        stripped
    };
    // sh:28  canpref=$curpref:P
    let mut canpref = canon_p(&curpref);
    // sh:29  [[ $curpref == */ && $canpref == *[^/] ]] && canpref+=/
    if curpref.ends_with('/') && canpref.chars().last().is_some_and(|c| c != '/') {
        canpref.push('/');
    }
    // sh:30  canpref+=$rltrim
    canpref.push_str(&rltrim);
    // sh:31  [[ $expref == *[^/] && $canpref == */ ]] && origpref+=/
    if expref.chars().last().is_some_and(|c| c != '/') && canpref.ends_with('/') {
        origpref.push('/');
    }

    // sh:34-46  append the subset of $files that matches $canpref.
    if canpref == origpref {
        // sh:36-40  matchspec-honouring path via `compadd -A tmp_buffer`.
        setaparam("tmp_buffer", Vec::new());
        let mut cargv: Vec<String> = vec!["-A".to_string(), "tmp_buffer".to_string()];
        cargv.extend(gopts.iter().cloned());
        cargv.push("-a".to_string());
        cargv.push("files".to_string());
        let _ = bin_compadd("compadd", &cargv, &make_ops(), 0);
        let tmp = getaparam("tmp_buffer").unwrap_or_default();
        for e in &tmp {
            // sh:39  ${(@)tmp_buffer/$canpref/$origpref}
            matches.push(replace_first(e, &canpref, &origpref));
        }
        unsetparam("tmp_buffer");
    } else {
        // sh:45  ${${(M)files:#$canpref*}/$canpref/$origpref}
        for f in files_vec {
            if f.starts_with(&canpref) {
                matches.push(replace_first(f, &canpref, &origpref));
            }
        }
    }

    // sh:48-50  for subdir in $expref?*(@); do recurse
    for subdir in glob_symlinks(&expref) {
        let recur = replace_first(&subdir, &expref, &origpref);
        _canonical_paths_add_paths(&recur, true, gopts, files_vec, matches);
    }
}

/// `_canonical_paths` — complete file paths, also offering
/// same-file completions (relative↔absolute, symlink-resolved).
pub fn _canonical_paths(args: &[String]) -> i32 {
    // sh:63  zparseopts -D -a __gopts M+: J+: V+: o+: 1 2 n F: x+: X+: A:=__opts N=__opts
    let src = "__compsys_argv";
    setaparam(src, args.to_vec());
    setaparam("__gopts", Vec::new());
    setaparam("__opts", Vec::new());
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-D".to_string(),
            "-v".to_string(),
            src.to_string(),
            "-a".to_string(),
            "__gopts".to_string(),
            "M+:".to_string(),
            "J+:".to_string(),
            "V+:".to_string(),
            "o+:".to_string(),
            "1".to_string(),
            "2".to_string(),
            "n".to_string(),
            "F:".to_string(),
            "x+:".to_string(),
            "X+:".to_string(),
            "A:=__opts".to_string(),
            "N=__opts".to_string(),
        ],
        &make_ops(),
        0,
    );
    let __gopts = getaparam("__gopts").unwrap_or_default();
    let __opts = getaparam("__opts").unwrap_or_default();
    let mut argv = getaparam(src).unwrap_or_default();
    unsetparam(src);

    // sh:65  : ${1:=canonical-paths} ${2:=path}
    while argv.len() < 2 {
        argv.push(String::new());
    }
    if argv[0].is_empty() {
        argv[0] = "canonical-paths".to_string();
    }
    if argv[1].is_empty() {
        argv[1] = "path".to_string();
    }

    // sh:67-68  __index=$__opts[(I)-A]; (( $__index )) && set -- $@ ${(P)__opts[__index+1]}
    if let Some(pos) = __opts.iter().rposition(|x| x == "-A") {
        if let Some(varname) = __opts.get(pos + 1) {
            let extra = getaparam(varname).unwrap_or_default();
            argv.extend(extra);
        }
    }

    // sh:70  tag=$1 desc=$2  ;  sh:72  shift 2
    let tag = argv[0].clone();
    let desc = argv[1].clone();
    let positional: Vec<String> = argv[2..].to_vec();

    // sh:74  if ! zmodload -F zsh/stat b:zstat; then … fi
    let mut ops_f = make_ops();
    ops_f.ind[b'F' as usize] = 1;
    if bin_zmodload(
        "zmodload",
        &["zsh/stat".to_string(), "b:zstat".to_string()],
        &ops_f,
        0,
    ) != 0
    {
        // sh:75  _wanted "$tag" expl "$desc" compadd $__gopts $@
        let mut wargv = vec![
            tag.clone(),
            "expl".to_string(),
            desc.clone(),
            "compadd".to_string(),
        ];
        wargv.extend(__gopts.iter().cloned());
        wargv.extend(positional.iter().cloned());
        let ret = _wanted(&wargv);
        unsetparam("__gopts");
        unsetparam("__opts");
        return ret; // sh:76
    }

    // sh:82-86  files=($@) (-N) else files+=($@:P)
    let files_vec: Vec<String> = if __opts.iter().any(|x| x == "-N") {
        positional.clone()
    } else {
        positional.iter().map(|p| canon_p(p)).collect()
    };
    setaparam("files", files_vec.clone());

    // sh:88  base=$PREFIX
    let mut base = getsparam("PREFIX").unwrap_or_default();
    let mut matches: Vec<String> = Vec::new();

    // sh:91
    _canonical_paths_add_paths(&base, false, &__gopts, &files_vec, &mut matches);

    if base.is_empty() {
        // sh:94
        _canonical_paths_add_paths("/", true, &__gopts, &files_vec, &mut matches);
    } else if is_dotdot_path(&base) {
        // sh:103-104  zstyle -s … canonical-paths-back-limit blimit || blimit=8
        let curcontext = getsparam("curcontext").unwrap_or_default();
        let mut blimit: i64 = lookupstyle(
            &format!(":completion:{}:{}", curcontext, tag),
            "canonical-paths-back-limit",
        )
        .first()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);

        // sh:106-109
        if !base.ends_with('/') {
            if !base.ends_with("..") {
                base.push('.');
            }
            base.push('/');
        }
        // sh:110-114  until [[ $base.. -ef $base || blimit -le 0 ]]
        loop {
            if same_file(&format!("{}..", base), &base) || blimit <= 0 {
                break;
            }
            base.push_str("../");
            _canonical_paths_add_paths(&base, true, &__gopts, &files_vec, &mut matches);
            blimit -= 1;
        }
    }

    setaparam("matches", matches);

    // sh:117  _wanted "$tag" expl "$desc" compadd $__gopts -Q -U -a matches
    let mut wargv = vec![
        tag.clone(),
        "expl".to_string(),
        desc.clone(),
        "compadd".to_string(),
    ];
    wargv.extend(__gopts.iter().cloned());
    wargv.push("-Q".to_string());
    wargv.push("-U".to_string());
    wargv.push("-a".to_string());
    wargv.push("matches".to_string());
    let ret = _wanted(&wargv); // sh:117-119

    // Tear down transient by-name arrays used to bridge compadd/_wanted.
    unsetparam("files");
    unsetparam("matches");
    unsetparam("__gopts");
    unsetparam("__opts");

    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_for_no_paths() {
        // sh:65 — bare tag `mytag` (desc defaults to `path`); with no
        //   paths and no registered comptags, `_wanted` fails → 1.
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_canonical_paths(&["mytag".to_string()]), 1);
    }

    #[test]
    fn dotdot_path_detection() {
        // sh:95 — `..(/.(|.))#(|/)` component matcher.
        assert!(is_dotdot_path(".."));
        assert!(is_dotdot_path("../"));
        assert!(is_dotdot_path("../.."));
        assert!(is_dotdot_path("../."));
        assert!(is_dotdot_path("../../.."));
        assert!(!is_dotdot_path("."));
        assert!(!is_dotdot_path("foo"));
        assert!(!is_dotdot_path("../foo"));
        assert!(!is_dotdot_path("..."));
    }

    #[test]
    fn replace_first_only_first_occurrence() {
        // sh:39/45 — `${x/from/to}` replaces the first match only.
        assert_eq!(replace_first("/a/a/b", "/a", "X"), "X/a/b");
        assert_eq!(replace_first("abc", "z", "Q"), "abc");
        assert_eq!(replace_first("abc", "", "Q"), "abc");
    }
}
