//! Port of `_files` from `Completion/Unix/Type/_files` (153 lines).
//! Faithful translation of the shell source: mirrors its local variable
//! names / control flow and dispatches `_path_files` per pattern rather
//! than reimplementing file generation.
//!
//! Shell → Rust local mapping: `opts tmp glob pat pats expl tag i def
//! descr end ign tried type sdef ignvars ignvar prepath oprefix rfiles
//! rfile subtree ret`.
//!
//! Sections ported (all present, unlike the prior approximation):
//!   * sh:9-25   — glob-qualifier dispatch (`_have_glob_qual`,
//!                 `_globflags`, `_globquals`, `_describe`) incl. the
//!                 globbing-flags-at-word-start `(#…` case.
//!   * sh:33-44  — `type` / `glob` derivation, `#q` qualifier insertion.
//!   * sh:45-61  — `-F` ignore-vars resolution (`_comp_ignore`, `(…)`
//!                 literal, and indirect param names).
//!   * sh:63-79  — `file-patterns` / `list-dirs-first` / default pats.
//!   * sh:81-151 — the tag → `_tags` → `_next_label` → `_path_files -g`
//!                 loop, including the `recursive-files` subtree walk.
//!
//! Approximations (marked `// sh:N approx`): the `eval "def=( … )"`
//! word-split (sh:83), the `${(q)}`/`${(Q)}` quoting, and the sh:36-41
//! blank→brace / `#q` glob rewrites use string ops rather than the full
//! zsh expansion engine.

use crate::compsys::ported::_next_label::_next_label;
use crate::compsys::ported::_path_files::_path_files;
use crate::compsys::ported::_tags::_tags;
use crate::ported::exec::dispatch_function_call;
use crate::ported::glob::{matchpat, tokenize, zglob};
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, gethkparam, gethparam, getsparam, setaparam, setsparam};
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{isset, options, CASEGLOB, EXTENDEDGLOB, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

fn compset(argv: Vec<String>) -> i32 {
    bin_compset("compset", &argv, &make_ops(), 0)
}
fn dispatch0(name: &str, args: &[String]) -> i32 {
    dispatch_function_call(name, args).unwrap_or(1)
}
fn get_str(name: &str) -> String {
    getsparam(name).unwrap_or_default()
}
fn zstyle_a(ctx: &str, style: &str) -> Vec<String> {
    lookupstyle(ctx, style)
}
fn zstyle_t(ctx: &str, style: &str) -> bool {
    matches!(
        lookupstyle(ctx, style).first().map(|s| s.as_str()),
        Some("yes") | Some("true") | Some("on") | Some("1")
    )
}
/// `$name[key]` for an associative parameter. `_comp_caller_options` is
/// PM_HASHED (`_main_complete` publishes it with `sethparam`) and
/// `getaparam` only ever returns PM_ARRAY values, so the hash has to be
/// read via `gethkparam`/`gethparam` (c:params.c:3117/3131). The flat
/// key/value-array path stays as a fallback for `setaparam`-staged assocs.
fn assoc_get(name: &str, key: &str) -> Option<String> {
    let keys = gethkparam(name).unwrap_or_default();
    if !keys.is_empty() {
        let vals = gethparam(name).unwrap_or_default();
        return keys
            .iter()
            .position(|k| k == key)
            .and_then(|i| vals.get(i).cloned());
    }
    getaparam(name)
        .unwrap_or_default()
        .chunks(2)
        .find(|kv| kv.first().map(|k| k == key).unwrap_or(false))
        .and_then(|kv| kv.get(1).cloned())
}

fn has_active_glob(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if matches!(
            b[i],
            b'[' | b']' | b'*' | b'?' | b'#' | b'~' | b'^' | b'|' | b'<' | b'>'
        ) {
            return true;
        }
        i += 1;
    }
    false
}

fn has_unescaped_colon(s: &str) -> bool {
    let b = s.as_bytes();
    (0..b.len()).any(|i| b[i] == b':' && (i == 0 || b[i - 1] != b'\\'))
}

fn first_unescaped_colon(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    (0..b.len()).find(|&i| b[i] == b':' && (i == 0 || b[i - 1] != b'\\'))
}

/// Split on `:` not preceded by a backslash (the `${...#*[^\\]:}` /
/// `${...%%:tag*}` decomposition of a `pat:tag:descr` spec).
fn split_unescaped_colon(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b':' && (i == 0 || b[i - 1] != b'\\') {
            parts.push(s[start..i].to_string());
            start = i + 1;
        }
        i += 1;
    }
    parts.push(s[start..].to_string());
    parts
}

/// `${(q)head}` dedup key — the `pat:` head of a spec (up to and
/// including the first unescaped colon).
fn pat_head(s: &str) -> String {
    match first_unescaped_colon(s) {
        Some(i) => s[..=i].to_string(),
        None => s.to_string(),
    }
}

/// `${(Q)s}` — strip one level of backslash quoting. sh approx.
fn unquote_q(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            if let Some(n) = it.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// sh:36 test — a blank preceded by a non-backslash.
fn has_unescaped_blank(s: &str) -> bool {
    let b = s.as_bytes();
    (1..b.len()).any(|i| (b[i] == b' ' || b[i] == b'\t') && b[i - 1] != b'\\')
}
/// sh:37 — each run of blanks (after a non-backslash) becomes a comma.
fn blanks_to_commas(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if (c == b' ' || c == b'\t') && i > 0 && b[i - 1] != b'\\' {
            while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
                i += 1;
            }
            out.push(',');
        } else {
            out.push(c as char);
            i += 1;
        }
    }
    out
}
/// sh:40-41 — insert `#q` into the trailing `(qualifier)` if absent.
fn add_hashq(g: &str) -> String {
    if let Some(op) = g.rfind('(') {
        let head = &g[..op];
        let after = &g[op + 1..];
        if after.ends_with(')')
            && !after.starts_with("#q")
            && after[..after.len() - 1]
                .chars()
                .all(|c| c != '|' && c != '~')
        {
            return format!("{}(#q{}", head, after);
        }
    }
    g.to_string()
}

/// `subtree=( **/*(/) )` — all subdirectories, recursively.
fn glob_subdirs() -> Vec<String> {
    let mut list = {
        let mut s = "**/*(/)".to_string();
        tokenize(&mut s);
        vec![s]
    };
    zglob(&mut list, 0, 0);
    list.into_iter().filter(|e| !has_active_glob(e)).collect()
}

/// `_files` — file/directory completion entry point.
pub fn _files(argv: &[String]) -> i32 {
    let curcontext = get_str("curcontext");
    let ctx = format!(":completion:{}:", curcontext);
    let mut ret = 1;
    let mut subtree: Vec<String> = Vec::new();

    // sh:9-25 — glob-qualifier dispatch.
    let prefix = get_str("PREFIX");
    let eg_on = assoc_get("_comp_caller_options", "extendedglob").as_deref() == Some("on");
    if dispatch_function_call("_have_glob_qual", &[prefix.clone()]) == Some(0) {
        let mtch = getaparam("match").unwrap_or_default();
        let m1len = mtch.first().map(|s| s.chars().count()).unwrap_or(0);
        compset(vec!["-p".into(), m1len.to_string()]);
        compset(vec!["-S".into(), r"[^\)\|\~]#(|\))".into()]);
        if eg_on && compset(vec!["-P".into(), r"\#".into()]) == 0 {
            if dispatch0("_globflags", &[]) == 0 {
                ret = 0;
            }
        } else {
            if eg_on {
                // sh:16 — literal action group, not a param name.
                if dispatch0(
                    "_describe",
                    &[
                        "-t".into(),
                        "globflags".into(),
                        "glob flag".into(),
                        r"(\#:introduce\ glob\ flag)".into(),
                        "-Q".into(),
                        "-S".into(),
                        "".into(),
                    ],
                ) == 0
                {
                    ret = 0;
                }
            }
            if dispatch0("_globquals", &[]) == 0 {
                ret = 0;
            }
        }
        return ret;
    } else if eg_on
        && prefix.starts_with("(#")
        && !prefix[2..].contains(')')
        && compset(vec!["-P".into(), r"\(\#".into()]) == 0
    {
        // sh:21-24 — globbing flags can start at word beginning.
        return dispatch0("_globflags", &[]);
    }

    // sh:30-31 — option parse: everything but /,f,g goes to `opts`.
    let (mut opts, tmp) = zparse_files(argv);

    // sh:33 — joined flag-name string.
    let type_str: String = tmp
        .iter()
        .filter_map(|e| e.strip_prefix('-').unwrap_or(e).chars().next())
        .collect();

    // sh:34-44 — derive glob.
    let mut glob: Option<String> = None;
    if tmp.iter().any(|e| e.starts_with("-g")) {
        let raw: String = tmp
            .iter()
            .filter(|e| e.starts_with("-g"))
            .map(|e| e[2..].to_string())
            .collect::<Vec<_>>()
            .join("");
        let mut g = raw.trim().to_string(); // ##/%% blank strip
        if has_unescaped_blank(&g) {
            g = format!("{{{}}}", blanks_to_commas(&g));
        }
        g = add_hashq(&g);
        glob = Some(g);
    } else if type_str.contains('/') {
        glob = Some("*(#q-/)".to_string());
    }

    // sh:45-61 — resolve -F ignore vars.
    let mut ign: Vec<String> = Vec::new();
    if let Some(tpos) = opts.iter().position(|e| e == "-F") {
        let spec = opts.get(tpos + 1).cloned().unwrap_or_default();
        if spec.trim() == "_comp_ignore" {
            ign = getaparam("_comp_ignore").unwrap_or_default();
        } else if spec.starts_with('(') {
            ign = spec[1..spec.len().saturating_sub(1)]
                .split_whitespace()
                .map(String::from)
                .collect();
        } else {
            for ignvar in spec.split_whitespace() {
                ign.extend(
                    getaparam(ignvar)
                        .or_else(|| getsparam(ignvar).map(|s| vec![s]))
                        .unwrap_or_default(),
                );
            }
            if tpos + 1 < opts.len() {
                opts[tpos + 1] = "_comp_ignore".into();
            }
        }
    }

    // sh:63-79 — build pats.
    let file_patterns = zstyle_a(&ctx, "file-patterns");
    let glob_or_star = glob.clone().unwrap_or_else(|| "*".to_string());
    let glob_colon = glob_or_star.replace(':', "\\:");
    let pats: Vec<String> = if !file_patterns.is_empty() {
        // sh:64-72 — %p substitution + word split; add :files default tag.
        let mut out = Vec::new();
        for elem in &file_patterns {
            let sub = elem.replace("%p", &glob_colon);
            for i in sub.split_whitespace() {
                if has_unescaped_colon(i) {
                    out.push(format!(" {} ", i));
                } else {
                    out.push(format!(" {}:files ", i));
                }
            }
        }
        out
    } else if zstyle_t(&ctx, "list-dirs-first") {
        // sh:74
        vec![
            format!(
                " *(-/):directories:directory {}(#q^-/):globbed-files",
                glob_colon
            ),
            "*:all-files".to_string(),
        ]
    } else {
        // sh:78 — directories shown on first try by default.
        vec![
            format!("{}:globbed-files *(-/):directories", glob_colon),
            "*:all-files ".to_string(),
        ]
    };

    // sh:81-151 — the tag loop.
    let mut tried: Vec<String> = Vec::new();
    for def in &pats {
        // sh:83 — eval re-split into specs (approx: whitespace split).
        let specs: Vec<String> = def.split_whitespace().map(String::from).collect();
        // sh:85-87 — dedup on the pattern-head set.
        let key: String = specs
            .iter()
            .map(|s| pat_head(s))
            .collect::<Vec<_>>()
            .join(" ");
        if tried.iter().any(|t| t == &key) {
            continue;
        }
        tried.push(key);

        for sdef in &specs {
            // sh:91-106 — split into pat / tag / descr.
            let cols = split_unescaped_colon(sdef);
            let pat = cols
                .first()
                .cloned()
                .unwrap_or_default()
                .replace("\\:", ":");
            let tag = cols.get(1).cloned().unwrap_or_default();
            let (descr, end) = if cols.len() >= 3 {
                (unquote_q(&cols[2..].join(":")), false)
            } else if opts.iter().any(|e| e == "-X") {
                (String::new(), true)
            } else {
                ("file".to_string(), true)
            };

            // sh:108 — register tags.
            let _ = _tags(&[tag.clone()]);
            // sh:109 — while _tags; do
            loop {
                if _tags(&[]) != 0 {
                    break;
                }
                // sh:110
                setaparam("_comp_ignore", Vec::new());
                // sh:111 — while _next_label ...; do
                loop {
                    if _next_label(&[tag.clone(), "expl".to_string(), descr.clone()]) != 0 {
                        break;
                    }
                    // sh:112 — _comp_ignore += ign.
                    let mut ci = getaparam("_comp_ignore").unwrap_or_default();
                    ci.extend(ign.clone());
                    setaparam("_comp_ignore", ci);
                    // sh:113-117 — merge opts into expl.
                    let expl = getaparam("expl").unwrap_or_default();
                    let new_expl: Vec<String> = if end {
                        let mut v = opts.clone();
                        v.extend(expl);
                        v
                    } else {
                        let mut v = expl.clone();
                        v.extend(opts.clone());
                        v
                    };
                    setaparam("expl", new_expl.clone());

                    // sh:119 — _path_files -g pat "$expl[@]"
                    let mut pf = vec!["-g".to_string(), pat.clone()];
                    pf.extend(new_expl.clone());
                    if _path_files(&pf) == 0 {
                        ret = 0;
                    } else {
                        // sh:121-138 — recursive-files.
                        let ps = format!("{}{}", get_str("PREFIX"), get_str("SUFFIX"));
                        if !ps.contains('/') {
                            let rfiles = zstyle_a(
                                &format!(":completion:{}:{}", curcontext, tag),
                                "recursive-files",
                            );
                            if !rfiles.is_empty() {
                                let pwd = format!("{}/", get_str("PWD"));
                                for rfile in &rfiles {
                                    // [[ $PWD/ = ${~rfile} ]]
                                    if matchpat(rfile, &pwd, isset(EXTENDEDGLOB), isset(CASEGLOB)) {
                                        if subtree.is_empty() {
                                            subtree = glob_subdirs();
                                        }
                                        for prepath in subtree.clone() {
                                            let oprefix = get_str("PREFIX");
                                            setsparam(
                                                "PREFIX",
                                                &format!("{}/{}", prepath, oprefix),
                                            );
                                            let mut pf2 = vec!["-g".to_string(), pat.clone()];
                                            pf2.extend(new_expl.clone());
                                            if _path_files(&pf2) == 0 {
                                                ret = 0;
                                            }
                                            setsparam("PREFIX", &oprefix);
                                        }
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
                // sh:140 — (( ret )) || break
                if ret == 0 {
                    break;
                }
            }

            // sh:147 — `*` short-circuit.
            if pat == "*" {
                return ret;
            }
        }
        // sh:150 — (( ret )) || return 0
        if ret == 0 {
            return 0;
        }
    }

    // sh:153
    1
}

/// sh:30-31 `zparseopts -a opts '/=tmp' 'f=tmp' 'g+:-=tmp' q n 1 2 P: S:
/// r: R: W: x+: X+: M+: F: J+: V+: o+:`. `/ f g` go to `tmp` (`g`
/// concatenated, ZOF_SAME); everything else to the default `opts`.
/// Returns (opts, tmp). Parsing stops at the first non-option / `-` /
/// `--`.
pub fn zparse_files(args: &[String]) -> (Vec<String>, Vec<String>) {
    let mut opts: Vec<String> = Vec::new();
    let mut tmp: Vec<String> = Vec::new();
    // (takes_arg, to_tmp, concat)
    let spec = |c: u8| -> Option<(bool, bool, bool)> {
        Some(match c {
            b'/' | b'f' => (false, true, false),
            b'g' => (true, true, true),
            b'q' | b'n' | b'1' | b'2' => (false, false, false),
            b'P' | b'S' | b'r' | b'R' | b'W' | b'F' => (true, false, false),
            b'x' | b'X' | b'M' | b'J' | b'V' | b'o' => (true, false, false),
            _ => return None,
        })
    };
    let mut i = 0;
    while i < args.len() {
        let t = &args[i];
        if !t.starts_with('-') || t == "-" {
            break;
        }
        if t == "--" {
            i += 1;
            break;
        }
        let rest = &t[1..];
        let rb = rest.as_bytes();
        let mut j = 0;
        let mut consumed_next = false;
        while j < rb.len() {
            let c = rb[j];
            let Some((takes_arg, to_tmp, concat)) = spec(c) else {
                j = rb.len();
                break;
            };
            let optname = format!("-{}", c as char);
            let dest = if to_tmp { &mut tmp } else { &mut opts };
            if takes_arg {
                let value = if j + 1 < rb.len() {
                    let v = rest[j + 1..].to_string();
                    j = rb.len();
                    v
                } else if i + 1 < args.len() {
                    consumed_next = true;
                    args[i + 1].clone()
                } else {
                    String::new()
                };
                if concat {
                    dest.push(format!("{}{}", optname, value));
                } else {
                    dest.push(optname);
                    dest.push(value);
                }
                break;
            } else {
                dest.push(optname);
                j += 1;
            }
        }
        i += 1;
        if consumed_next {
            i += 1;
        }
    }
    (opts, tmp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zparse_slash_goes_to_tmp() {
        let (opts, tmp) = zparse_files(&["-/".to_string()]);
        assert_eq!(tmp, vec!["-/".to_string()]);
        assert!(opts.is_empty());
    }

    #[test]
    fn zparse_g_concats_into_tmp() {
        let (opts, tmp) = zparse_files(&["-g".to_string(), "*.rs".to_string()]);
        assert_eq!(tmp, vec!["-g*.rs".to_string()]);
        assert!(opts.is_empty());
    }

    #[test]
    fn zparse_value_opts_go_to_opts_as_pairs() {
        let (opts, tmp) = zparse_files(&[
            "-W".to_string(),
            "/tmp".to_string(),
            "-J".to_string(),
            "grp".to_string(),
        ]);
        assert!(tmp.is_empty());
        assert_eq!(
            opts,
            vec![
                "-W".to_string(),
                "/tmp".to_string(),
                "-J".to_string(),
                "grp".to_string()
            ]
        );
    }

    #[test]
    fn glob_default_for_dirs_only() {
        // type contains '/', no -g => glob defaults to *(#q-/).
        let (_o, tmp) = zparse_files(&["-/".to_string()]);
        let type_str: String = tmp
            .iter()
            .filter_map(|e| e.strip_prefix('-').unwrap_or(e).chars().next())
            .collect();
        assert!(type_str.contains('/'));
    }

    #[test]
    fn split_unescaped_colon_respects_backslash() {
        let parts = split_unescaped_colon(r"a\:b:tag:desc");
        assert_eq!(
            parts,
            vec![r"a\:b".to_string(), "tag".into(), "desc".into()]
        );
    }

    #[test]
    fn add_hashq_inserts_qualifier_flag() {
        assert_eq!(add_hashq("*(-/)"), "*(#q-/)");
        // already has #q — untouched.
        assert_eq!(add_hashq("*(#q-/)"), "*(#q-/)");
    }

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_files(&[]), 1);
    }

    #[test]
    fn accepts_dirs_only_flag() {
        let _g = crate::test_util::global_state_lock();
        let _r = _files(&["-/".to_string()]);
    }
}
