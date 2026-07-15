//! Port of `_path_files` from `Completion/Unix/Type/_path_files`
//! (upstream 895 lines). This is a faithful translation of the shell
//! source: it mirrors the shell's local variable names and control
//! flow and drives the ported C builtins `compfiles`, `compadd`,
//! `compset` and `compquote` instead of reimplementing file
//! generation.
//!
//! Shell → Rust local mapping (names kept from the source):
//!   linepath realpath donepath prepath testpath exppath skips skipped
//!   tmp1 tmp2 tmp3 tmp4 i orig eorig pre suf tpre tsuf opre osuf cpre
//!   pats haspats ignore pfx pfxsfx sopt gopt sdirs ignpar cfopt listsfx
//!   nm menu matcher mopts sort mid accex fake Uopt accept_exact_dirs
//!   path_completion npathcheck Mopts prepaths exppaths
//! Because Rust is typed, where the shell reuses one name for both a
//! scalar and an array we keep the base name for the dominant use and
//! suffix the other (e.g. `tmp1` = the match array, `tmp1s` = scalar
//! `tmp1`, `tmp2` = array, `tmp2s` = scalar `tmp2`).
//!
//! `compfiles` subcommands driven (via `bin_compfiles`):
//!   * `-p$cfopt` / `-P$cfopt`  — cf_pats file generation (sh:463-470)
//!   * `-i`                     — cf_ignore ignore-parents  (sh:580)
//!   * `-r`                     — cf_remove_other ambiguity  (sh:634)
//! The transient parameters `tmp1`, `accex`, `fake`, `ignore` and
//! `_comp_ignore` are materialised in `paramtab` around each builtin
//! call (compfiles/compadd/compquote read/write params by name) and
//! read back into the corresponding Rust locals.
//!
//! Approximations (marked inline with `// sh:N approx`): the gnarliest
//! zsh parameter-expansion idioms — `(e)`-eval of a parameter-expansion
//! prefix, `(z)` word tokenisation, `(b)`/`(q)` pattern quoting, the
//! `(#b)` backreference substitutions and the sh:201 dir-detect glob —
//! are implemented with the closest available primitive and commented.
//! `compfiles -p$cfopt` emits the shell's exact option token (`-p` or
//! `-p-`); note the current `bin_compfiles` expects `-p--` for the
//! noopt form (discrepancy vs. C, which accepts `-p-`).

use crate::ported::exec::dispatch_function_call;
use crate::ported::glob::{tokenize, zglob};
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use crate::ported::subst::{filesubstr, singsub};
use crate::ported::zle::compcore::get_compstate_str;
use crate::ported::zle::complete::{bin_compadd, bin_compset};
use crate::ported::zle::computil::{bin_compfiles, bin_compquote};
use crate::ported::zsh_h::{isset, options, CASEGLOB, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

// ---- small helpers -------------------------------------------------

fn compadd(argv: Vec<String>) -> i32 {
    bin_compadd("compadd", &argv, &make_ops(), 0)
}
fn compset(argv: Vec<String>) -> i32 {
    bin_compset("compset", &argv, &make_ops(), 0)
}
/// `compquote [-p] name...` — sync each named local into paramtab is
/// the caller's job; this just fires the builtin.
fn compquote(argv: Vec<String>) {
    bin_compquote("compquote", &argv, &make_ops(), 0);
}
fn compfiles(argv: Vec<String>) -> i32 {
    bin_compfiles("compfiles", &argv, &make_ops(), 0)
}

fn dispatch0(name: &str, args: &[String]) -> i32 {
    dispatch_function_call(name, args).unwrap_or(1)
}

fn get_arr(name: &str) -> Vec<String> {
    getaparam(name).unwrap_or_default()
}
fn get_str(name: &str) -> String {
    getsparam(name).unwrap_or_default()
}

fn cs_i(key: &str) -> i64 {
    get_compstate_str(key)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
fn cs_s(key: &str) -> String {
    get_compstate_str(key).unwrap_or_default()
}

/// zstyle -s: first value (None when style unset).
fn zstyle_s(ctx: &str, style: &str) -> Option<String> {
    lookupstyle(ctx, style).into_iter().next()
}
/// zstyle -a: all values.
fn zstyle_a(ctx: &str, style: &str) -> Vec<String> {
    lookupstyle(ctx, style)
}
/// zstyle -t: present and true-ish.
fn zstyle_t(ctx: &str, style: &str) -> bool {
    match lookupstyle(ctx, style).first() {
        Some(w) => matches!(w.as_str(), "yes" | "true" | "on" | "1"),
        None => false,
    }
}
/// zstyle -T: default-true (true unless explicitly false-ish).
fn zstyle_t_default(ctx: &str, style: &str) -> bool {
    match lookupstyle(ctx, style).first() {
        Some(w) => !matches!(w.as_str(), "no" | "false" | "off" | "0"),
        None => true,
    }
}

/// Flat-assoc lookup for `_comp_caller_options[key]` style access.
fn assoc_get(name: &str, key: &str) -> Option<String> {
    get_arr(name)
        .chunks(2)
        .find(|kv| kv.first().map(|k| k == key).unwrap_or(false))
        .and_then(|kv| kv.get(1).cloned())
}

/// `${(b)s}` — backslash-quote pattern metacharacters so `s` matches
/// literally when used as a pattern. sh approx.
fn quote_b(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '('
                | ')'
                | '['
                | ']'
                | '|'
                | '*'
                | '?'
                | '#'
                | '^'
                | '~'
                | '<'
                | '>'
                | '{'
                | '}'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// True if `s` contains an unescaped glob metacharacter (the shell's
/// `(|*[^\\])[][*?#~^\|\<\>]*` test). sh approx.
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

/// `${(M)tpre##${~skips}}` — longest leading run of `./`, `../` (and,
/// when squeeze, bare `/`) components. Returns that prefix.
fn match_skips_prefix(s: &str, squeeze: bool) -> String {
    let b = s.as_bytes();
    let mut i = 0;
    loop {
        if b[i..].starts_with(b"./") {
            i += 2;
        } else if b[i..].starts_with(b"../") {
            i += 3;
        } else if squeeze && b.get(i) == Some(&b'/') {
            i += 1;
        } else {
            break;
        }
    }
    s[..i].to_string()
}

/// `tmp1=( $~tmp1 )` — tokenise + glob-expand each element. Elements
/// that still carry wildcards after expansion (no match) are dropped,
/// approximating completion's nullglob-style file generation.
fn tilde_glob(pats: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for p in pats {
        let mut list = {
            let mut s = p.clone();
            tokenize(&mut s);
            vec![s]
        };
        zglob(&mut list, 0, 0);
        for e in list {
            if !crate::ported::glob::hasbraces(&e, true) && !has_active_glob(&e) {
                out.push(e);
            }
        }
    }
    out
}

/// basename of each element (`${(@)arr:t}`).
fn tails(arr: &[String]) -> Vec<String> {
    arr.iter()
        .map(|s| {
            let t = s.trim_end_matches('/');
            match t.rfind('/') {
                Some(i) => t[i + 1..].to_string(),
                None => t.to_string(),
            }
        })
        .collect()
}

// ---- zparseopts ----------------------------------------------------

/// Result of the sh:59-62 `zparseopts -a mopts ...` parse. Each field
/// is the array the shell binds via `=name`; `mopts` is the `-a`
/// default (everything without an explicit `=name`).
#[derive(Default, Debug)]
pub struct Parsed {
    pub mopts: Vec<String>,    // -a mopts (J V x X 1 2 o n)
    pub pfx: Vec<String>,      // P:=pfx
    pub pfxsfx: Vec<String>,   // S: q r: R: => pfxsfx
    pub prepaths: Vec<String>, // W:=prepaths
    pub ignore: Vec<String>,   // F:=ignore
    pub matcher: Vec<String>,  // M+:=matcher
    pub tmp1: Vec<String>,     // f= /= g+:-= tmp1
}

// (takes_arg, dest, concat) for each single-char option.
enum Dest {
    Mopts,
    Pfx,
    Pfxsfx,
    Prepaths,
    Ignore,
    Matcher,
    Tmp1,
}

fn opt_spec(c: u8) -> Option<(bool, Dest, bool)> {
    // concat=true only for `g` (the `:-` ZOF_SAME form).
    Some(match c {
        b'P' => (true, Dest::Pfx, false),
        b'S' => (true, Dest::Pfxsfx, false),
        b'q' => (false, Dest::Pfxsfx, false),
        b'r' => (true, Dest::Pfxsfx, false),
        b'R' => (true, Dest::Pfxsfx, false),
        b'W' => (true, Dest::Prepaths, false),
        b'F' => (true, Dest::Ignore, false),
        b'M' => (true, Dest::Matcher, false),
        b'J' | b'V' | b'x' | b'X' | b'o' => (true, Dest::Mopts, false),
        b'1' | b'2' | b'n' => (false, Dest::Mopts, false),
        b'f' | b'/' => (false, Dest::Tmp1, false),
        b'g' => (true, Dest::Tmp1, true),
        _ => return None,
    })
}

/// Faithful port of the sh:59-62 `zparseopts` invocation for this exact
/// spec. Follows the zsh short-option scan (`bin_zparseopts`,
/// `add_opt_val`): options bundle; a value can be attached or taken
/// from the next argv; `g` stores option+value concatenated
/// (ZOF_SAME), every other value-taking option stores option and value
/// as two array elements. Parsing stops at the first non-option, `-`
/// or `--`.
pub fn zparse_pathfiles(args: &[String]) -> Parsed {
    let mut p = Parsed::default();
    let mut i = 0;
    while i < args.len() {
        let tok = &args[i];
        // Not an option / bare `-` / `--` ends the parse.
        if !tok.starts_with('-') || tok == "-" {
            break;
        }
        if tok == "--" {
            i += 1;
            break;
        }
        let rest = &tok[1..];
        let rb = rest.as_bytes();
        let mut j = 0;
        let mut consumed_next = false;
        while j < rb.len() {
            let c = rb[j];
            let Some((takes_arg, dest, concat)) = opt_spec(c) else {
                // bad option — stop (zparseopts default aborts; we halt).
                j = rb.len();
                break;
            };
            let optname = format!("-{}", c as char);
            let push = |p: &mut Parsed, dest: &Dest, vals: Vec<String>| {
                let d = match dest {
                    Dest::Mopts => &mut p.mopts,
                    Dest::Pfx => &mut p.pfx,
                    Dest::Pfxsfx => &mut p.pfxsfx,
                    Dest::Prepaths => &mut p.prepaths,
                    Dest::Ignore => &mut p.ignore,
                    Dest::Matcher => &mut p.matcher,
                    Dest::Tmp1 => &mut p.tmp1,
                };
                d.extend(vals);
            };
            if takes_arg {
                let value = if j + 1 < rb.len() {
                    let v = rest[j + 1..].to_string();
                    j = rb.len();
                    v
                } else if i + 1 < args.len() {
                    consumed_next = true;
                    args[i + 1].clone()
                } else {
                    // missing mandatory arg — bind empty.
                    String::new()
                };
                if concat {
                    push(&mut p, &dest, vec![format!("{}{}", optname, value)]);
                } else {
                    push(&mut p, &dest, vec![optname, value]);
                }
                break;
            } else {
                push(&mut p, &dest, vec![optname]);
                j += 1;
            }
        }
        i += 1;
        if consumed_next {
            i += 1;
        }
    }
    p
}

// ---- main ----------------------------------------------------------

/// `_path_files` — file/directory completion entry point.
pub fn _path_files(argv: &[String]) -> i32 {
    // sh:3 — match/mbegin/mend are populated by _have_glob_qual.
    let curcontext = get_str("curcontext");
    let ctx = format!(":completion:{}:", curcontext);
    let paths_ctx = format!(":completion:{}:paths", curcontext);

    // sh:5-8 — file-split-chars.
    if let Some(splitchars) = zstyle_s(&ctx, "file-split-chars") {
        // sh:7 approx: (q)-quote each char for the char class.
        let quoted: String = splitchars.chars().flat_map(|c| ['\\', c]).collect();
        compset(vec!["-P".into(), format!("*[{}]", quoted)]);
    }

    // sh:22-39 — glob-qualifier dispatch.
    let prefix = get_str("PREFIX");
    if dispatch_function_call("_have_glob_qual", &[prefix.clone()]) == Some(0) {
        let mut ret = 1;
        let mtch = get_arr("match");
        let m1len = mtch.first().map(|s| s.chars().count()).unwrap_or(0);
        compset(vec!["-p".into(), m1len.to_string()]);
        compset(vec!["-S".into(), r"[^\)\|\~]#(|\))".into()]);
        let eg_on = assoc_get("_comp_caller_options", "extendedglob").as_deref() == Some("on");
        if eg_on && compset(vec!["-P".into(), r"\#".into()]) == 0 {
            if dispatch0("_globflags", &[]) == 0 {
                ret = 0;
            }
        } else {
            if eg_on {
                // sh:31-34 — flags=( '#:introduce glob flag' ); _describe...
                setaparam("flags", vec!["#:introduce glob flag".into()]);
                if dispatch0(
                    "_describe",
                    &[
                        "-t".into(),
                        "globflags".into(),
                        "glob flag".into(),
                        "flags".into(),
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
    }

    // sh:59-62 — option parse.
    let parsed = zparse_pathfiles(argv);
    let mut mopts = parsed.mopts;
    let pfx = parsed.pfx;
    let mut pfxsfx = parsed.pfxsfx;
    let mut prepaths = parsed.prepaths;
    let mut ignore = parsed.ignore;
    let mut matcher = parsed.matcher;
    let topt = parsed.tmp1; // sh `tmp1` (the -f/-/-g flag array)

    // sh:64 — sopt = "-" + first char of each topt element.
    let mut sopt: Option<String> = {
        let mut s = String::from("-");
        for e in &topt {
            let stripped = e.strip_prefix('-').unwrap_or(e);
            if let Some(c) = stripped.chars().next() {
                s.push(c);
            }
        }
        Some(s)
    };
    // sh:65-66
    let haspats_flags = topt
        .iter()
        .any(|e| e.starts_with("-/") || e.starts_with("-g"));
    let gopt = topt.iter().any(|e| e.starts_with("-g"));

    // sh:67-74 — build pats.
    let g_pats: Vec<String> = topt
        .iter()
        .filter(|e| e.starts_with("-g"))
        .map(|e| e[2..].to_string())
        .collect();
    let mut pats: Vec<String> = {
        // sh:69/72 approx: (z) word-split the joined -g patterns.
        let split: Vec<String> = g_pats
            .join(" ")
            .split_whitespace()
            .map(String::from)
            .collect();
        if topt.iter().any(|e| e == "-/") {
            let mut v = vec!["*(-/)".to_string()];
            v.extend(split);
            v
        } else {
            split
        }
    };
    // sh:74 — drop empty/blank elements.
    pats.retain(|p| !p.trim().is_empty());
    let haspats = haspats_flags;

    // sh:76-78 — leading literal prefix.
    if !pfx.is_empty() {
        let pfx2 = pfx.get(1).cloned().unwrap_or_default();
        if compset(vec!["-P".into(), quote_b(&pfx2)]) != 0 {
            let mut np = pfx.clone();
            np.extend(pfxsfx.clone());
            pfxsfx = np;
        }
    }

    // sh:80-93 — resolve -W into prepaths.
    if !prepaths.is_empty() {
        let tmp1s = prepaths.get(1).cloned().unwrap_or_default();
        if tmp1s.starts_with('(') {
            // sh:83 — ${^=tmp1[2,-2]%/}/
            let inner = &tmp1s[1..tmp1s.len().saturating_sub(1)];
            prepaths = inner
                .split_whitespace()
                .map(|w| format!("{}/", w.trim_end_matches('/')))
                .collect();
        } else if tmp1s.starts_with('/') {
            prepaths = vec![format!("{}/", tmp1s.trim_end_matches('/'))];
        } else {
            // sh:87 — ${(P)^tmp1%/}/ (indirect through named param).
            let vals = getaparam(&tmp1s)
                .or_else(|| getsparam(&tmp1s).map(|s| vec![s]))
                .unwrap_or_default();
            prepaths = vals
                .iter()
                .filter(|v| !v.is_empty())
                .map(|v| format!("{}/", v.trim_end_matches('/')))
                .collect();
            if prepaths.is_empty() {
                prepaths = vec![format!("{}/", tmp1s.trim_end_matches('/'))];
            }
        }
        if prepaths.is_empty() {
            prepaths = vec![String::new()];
        }
    } else {
        prepaths = vec![String::new()];
    }

    // sh:95-101 — resolve -F ignore.
    if !ignore.is_empty() {
        let ig2 = ignore.get(1).cloned().unwrap_or_default();
        if ig2.starts_with('(') {
            ignore = ig2[1..ig2.len().saturating_sub(1)]
                .split_whitespace()
                .map(String::from)
                .collect();
        } else {
            ignore = getaparam(&ig2)
                .or_else(|| getsparam(&ig2).map(|s| vec![s]))
                .unwrap_or_default();
        }
    }

    // sh:106-113 — default file selection.
    if matches!(sopt.as_deref(), Some("-f") | Some("-")) {
        if !gopt {
            sopt = Some("-f".into());
            pats = vec!["*".into()];
        } else {
            sopt = None; // unset sopt
        }
    }

    // sh:115-132 — description / matcher from _description.
    let has_jvx = mopts.iter().any(|e| e == "-J" || e == "-V" || e == "-X");
    if !has_jvx {
        if !gopt && sopt.as_deref() == Some("-/") {
            dispatch0(
                "_description",
                &["directories".into(), "expl".into(), "directory".into()],
            );
        } else {
            dispatch0(
                "_description",
                &["files".into(), "expl".into(), "file".into()],
            );
        }
        let expl = get_arr("expl");
        // sh:123 — highest index of a -M* element.
        if let Some(pos) = expl.iter().rposition(|e| e.starts_with("-M")) {
            let spec = if expl[pos] == "-M" {
                expl.get(pos + 1).cloned().unwrap_or_default()
            } else {
                expl[pos][2..].to_string()
            };
            if !matcher.is_empty() {
                let m2 = matcher.get(1).cloned().unwrap_or_default();
                if matcher.len() >= 2 {
                    matcher[1] = format!("{} {}", m2, spec);
                } else {
                    matcher = vec!["-M".into(), spec];
                }
            } else {
                matcher = vec!["-M".into(), spec];
            }
        }
        mopts.extend(expl);
    }

    // sh:136-138 — fold $fignore into ignore patterns.
    let fignore = get_arr("fignore");
    let comp_no_ignore = get_str("_comp_no_ignore");
    let fignore_env = get_str("FIGNORE");
    let pats_is_star = pats.join(" ").trim() == "*"; // sh:137 approx
    if comp_no_ignore.is_empty()
        && ignore.is_empty()
        && (!gopt || pats_is_star)
        && !fignore_env.is_empty()
    {
        ignore = fignore.iter().map(|f| format!("?*{}", f)).collect();
    }

    // sh:140-143 — install ignore into _comp_ignore + mopts -F.
    if !ignore.is_empty() {
        let mut ci = get_arr("_comp_ignore");
        ci.extend(ignore.clone());
        setaparam("_comp_ignore", ci);
        if !mopts.iter().any(|e| e == "-F") {
            mopts.push("-F".into());
            mopts.push("_comp_ignore".into());
        }
    }

    // sh:145-149 — case-insensitive matcher under nocaseglob.
    if matcher.is_empty() && !isset(CASEGLOB) {
        matcher = vec!["-M".into(), "m:{a-zA-Z}={A-Za-z}".into()];
    }

    // sh:151-154 — add matcher to mopts.
    if !matcher.is_empty() {
        mopts.extend(matcher.clone());
    }

    // sh:156-185 — file-sort.
    if let Some(fs) = zstyle_s(&ctx, "file-sort") {
        let mut sort = if fs.contains("size") {
            "oL".to_string()
        } else if fs.contains("links") {
            "ol".to_string()
        } else if fs.contains("time") || fs.contains("date") || fs.contains("modi") {
            "om".to_string()
        } else if fs.contains("access") {
            "oa".to_string()
        } else if fs.contains("inode") || fs.contains("change") {
            "oc".to_string()
        } else {
            "on".to_string()
        };
        if fs.contains("rev") {
            // sort[1]=O — replace first char.
            let mut c: Vec<char> = sort.chars().collect();
            c[0] = 'O';
            sort = c.into_iter().collect();
        }
        if fs.contains("follow") {
            sort = format!("-{}-", sort);
        }
        if sort == "on" {
            sort.clear();
        } else {
            let mut nm = vec!["-o".to_string(), "nosort".to_string()];
            nm.extend(mopts.clone());
            mopts = nm;
            let mut tmp2v = Vec::new();
            for t in &pats {
                if dispatch_function_call("_have_glob_qual", &[t.clone(), "complete".into()])
                    == Some(0)
                {
                    let m = get_arr("match");
                    let m1 = m.first().cloned().unwrap_or_default();
                    let m5 = m.get(4).cloned().unwrap_or_default();
                    tmp2v.push(format!("{}#q{})({})", m1, sort, m5));
                } else {
                    tmp2v.push(format!("{}({})", t, sort));
                }
            }
            pats = tmp2v;
        }
    }

    // sh:191-195 — squeeze-slashes.
    let squeeze = zstyle_t(&paths_ctx, "squeeze-slashes");

    // sh:197-212 — assorted styles.
    let sdirs = zstyle_s(&paths_ctx, "special-dirs").unwrap_or_default();
    let listsfx = zstyle_t(&paths_ctx, "list-suffixes");
    // sh:201 approx — bump sopt to include `/` when pats look dir-ish.
    if sopt.is_some()
        && (pats.iter().any(|p| p.contains("(-/)")) || pats.iter().any(|p| p.trim() == "*"))
    {
        sopt = Some(format!("{}/", sopt.clone().unwrap_or_default()));
    }
    let accex = zstyle_a(&paths_ctx, "accept-exact");
    let fake = zstyle_a(&ctx, "fake-files");
    let ignpar = zstyle_s(&paths_ctx, "ignore-parents").unwrap_or_default();
    let accept_exact_dirs = zstyle_t(&paths_ctx, "accept-exact-dirs");
    let path_completion = zstyle_t_default(&paths_ctx, "path-completion");

    // sh:214-237 — copy glob qualifiers from the line into the patterns.
    if !cs_s("pattern_match").is_empty() {
        let suffix0 = get_str("SUFFIX");
        let prefix0 = get_str("PREFIX");
        let hit = (suffix0.is_empty()
            && dispatch_function_call("_have_glob_qual", &[prefix0.clone(), "complete".into()])
                == Some(0))
            || dispatch_function_call("_have_glob_qual", &[suffix0.clone(), "complete".into()])
                == Some(0);
        if hit {
            let m = get_arr("match");
            let tmp3 = m.get(4).cloned().unwrap_or_default(); // match[5]
            if !suffix0.is_empty() {
                setsparam("SUFFIX", &m.get(1).cloned().unwrap_or_default()); // match[2]
            } else {
                setsparam("PREFIX", &m.get(1).cloned().unwrap_or_default());
            }
            let mut tmp2v = Vec::new();
            for t in &pats {
                if dispatch_function_call("_have_glob_qual", &[t.clone(), "complete".into()])
                    == Some(0)
                {
                    let mm = get_arr("match");
                    let m1 = mm.first().cloned().unwrap_or_default();
                    let m5 = mm.get(4).cloned().unwrap_or_default();
                    tmp2v.push(format!("{}{}{})", m1, tmp3, m5));
                } else {
                    tmp2v.push(format!("{}({})", t, tmp3));
                }
            }
            pats = tmp2v;
        }
    }

    // sh:242-247 — snapshot prefix/suffix/orig.
    let mut pre = get_str("PREFIX");
    let mut suf = get_str("SUFFIX");
    let opre = get_str("PREFIX");
    let osuf = get_str("SUFFIX");
    let mut orig = format!("{}{}", pre, suf);
    let eorig = orig.clone();

    // sh:249-257 — menu? correction options?
    let comp_correct = get_str("_comp_correct");
    let insert = cs_s("insert");
    let pattern_match = cs_s("pattern_match");
    let orig_no_tilde = orig.strip_prefix('~').unwrap_or(&orig);
    let menu = insert.ends_with("menu")
        || insert
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        || !comp_correct.is_empty()
        || (!pattern_match.is_empty() && !has_active_glob(orig_no_tilde));
    let _ = menu;
    let mut cfopt = String::new();
    let mut uopt = String::new();
    let mut mopts_r: Vec<String> = Vec::new(); // Mopts
    if !comp_correct.is_empty() {
        cfopt = "-".into();
        uopt = "-U".into();
    } else {
        mopts_r = vec!["-M".into(), "r:|/=* r:|=*".into()];
    }

    // sh:259-359 — split line into linepath + working prefix.
    let mut linepath = String::new();
    let mut realpath = String::new();
    let mut donepath;
    let quote = cs_s("quote");

    // sh:261 — parameter-expansion prefix branch (approx: only take the
    // branch when pre contains `$` before a slash and isn't single-quoted).
    if quote != "'" && pre.contains('$') && {
        // matches [^glob]#(`...`|$)*/*  — roughly: a $… followed later by /
        pre.find('$')
            .map(|d| pre[d..].contains('/'))
            .unwrap_or(false)
    } {
        // sh:269 linepath = ${(M)pre##*$[^/]##/}  — through the first
        // slash after the parameter expansion.
        let dollar = pre.find('$').unwrap();
        let after = &pre[dollar..];
        let slash_rel = after.find('/').unwrap();
        linepath = pre[..dollar + slash_rel + 1].to_string();
        // sh:273 realpath = eval ${(e)~linepath} — expand params via
        // singsub (PREFORK_SINGLE). sh approx: env/param expansion only.
        realpath = singsub(&linepath);
        if realpath.is_empty() || realpath == linepath {
            return 1;
        }
        pre = pre[linepath.len()..].to_string();
        // sh:277-279 orig truncated after the same slash count.
        let nslash = linepath.matches('/').count();
        orig = truncate_after_nth_slash(&orig, nslash);
        donepath = String::new();
        prepaths = vec![String::new()];
    } else if pre.starts_with('~') && (quote.is_empty() || quote == "`") {
        // sh:282-327 — ~ prefix.
        let after_tilde = &pre[1..];
        let lp = after_tilde.split('/').next().unwrap_or("").to_string();
        if lp.is_empty() {
            let home = get_str("HOME");
            realpath = format!("{}/", home.trim_end_matches('/'));
        } else if is_numeric_dirstack(&lp) {
            // sh:294-312 — directory stack index.
            let dirstack = get_arr("dirstack");
            let mut tmp1n: i64;
            if !lp.starts_with(['-', '+']) {
                tmp1n = lp.parse().unwrap_or(0);
            } else if lp.starts_with('-') {
                tmp1n = dirstack.len() as i64 + lp.parse::<i64>().unwrap_or(0);
            } else {
                tmp1n = lp[1..].parse().unwrap_or(0);
            }
            if isset(crate::ported::zsh_h::PUSHDMINUS) {
                tmp1n = dirstack.len() as i64 - tmp1n;
            }
            if tmp1n == 0 {
                realpath = format!("{}/", get_str("PWD"));
            } else if tmp1n <= dirstack.len() as i64 && tmp1n >= 1 {
                realpath = format!("{}/", dirstack[(tmp1n - 1) as usize]);
            } else {
                dispatch0("_message", &["not enough directory stack entries".into()]);
                return 1;
            }
        } else if lp == "-" || lp == "+" {
            realpath = format!("{}/", expand_tilde(&format!("~{}", lp)).unwrap_or_default());
        } else {
            // sh:316 — eval "realpath=~user/"
            realpath = expand_tilde(&format!("~{}/", lp)).unwrap_or_default();
            if realpath.is_empty() {
                dispatch0("_message", &[format!("unknown user `{}'", lp)]);
                return 1;
            }
        }
        linepath = format!("~{}/", lp);
        if realpath == linepath {
            return 1;
        }
        pre = pre.splitn(2, '/').nth(1).unwrap_or("").to_string();
        orig = orig.splitn(2, '/').nth(1).unwrap_or("").to_string();
        donepath = String::new();
        prepaths = vec![String::new()];
    } else {
        // sh:328-358 — no ~ prefix.
        linepath.clear();
        realpath.clear();
        let preserve = zstyle_s(&ctx, "preserve-prefix");
        if let Some(pp) = preserve.filter(|s| !s.is_empty()).and_then(|pp| {
            // pre = (#b)(${~pp})*  — leading match of pp.
            match_leading_pattern(&pre, &pp).map(|m1| m1)
        }) {
            pre = pre[pp.len()..].to_string();
            orig = orig[pp.len().min(orig.len())..].to_string();
            donepath = pp;
            prepaths = vec![String::new()];
        } else if pre.starts_with('/') {
            pre = pre[1..].to_string();
            orig = orig[1..].to_string();
            donepath = "/".into();
            prepaths = vec![String::new()];
        } else {
            if pre.starts_with("./") || pre.starts_with("../") {
                prepaths = vec![String::new()];
            }
            donepath = String::new();
        }
    }

    // sh:361-877 — generate matches, looping over prepaths.
    let mut exppaths: Vec<String> = Vec::new();
    let nm = cs_i("nmatches");
    let skips_squeeze = squeeze;

    for prepath in prepaths.clone() {
        let mut skipped = String::new();
        let mut cpre = String::new();

        // sh:373-410 — accept an exact directory prefix immediately.
        if (accept_exact_dirs || !path_completion) && pre.contains('/') {
            // (#b)(*)/([^/]#)
            if let Some(cut) = pre.rfind('/') {
                let mut tmp1s = pre[..cut].to_string(); // match[1]
                let mut tpre = pre[cut + 1..].to_string(); // match[2]
                loop {
                    let candidate = format!("{}{}{}{}", prepath, realpath, donepath, tmp1s);
                    if !path_completion || is_dir(&candidate) {
                        donepath = format!("{}{}/", donepath, tmp1s);
                        pre = tpre.clone();
                        break;
                    } else if let Some(cut2) = tmp1s.rfind('/') {
                        let nt = tmp1s[cut2 + 1..].to_string();
                        tmp1s = tmp1s[..cut2].to_string();
                        tpre = format!("{}/{}", nt, tpre);
                    } else {
                        break;
                    }
                }
            }
        }

        let mut tpre = pre.clone();
        let mut tsuf = suf.clone();
        // sh:421 — testpath from donepath (unquoted).
        let mut testpath = donepath.clone();

        // sh:423-426 — strip leading skips.
        let mut tmp2s = match_skips_prefix(&tpre, skips_squeeze);
        tpre = tpre[tmp2s.len()..].to_string();
        let mut tmp1: Vec<String> = vec![format!("{}{}{}{}", prepath, realpath, donepath, tmp2s)];

        let mut npathcheck: i32 = 0;
        let mut hit_continue_outer = false;

        // sh:430-610 — walk path components generating matches.
        loop {
            let origtmp1 = tmp1.clone();

            // sh:435-441 — prefix/suffix for this component.
            if tpre.contains('/') {
                setsparam("PREFIX", tpre.split('/').next().unwrap_or(""));
                setsparam("SUFFIX", "");
            } else {
                setsparam("PREFIX", &tpre);
                setsparam("SUFFIX", tsuf.split('/').next().unwrap_or(""));
            }

            let tmp2: Vec<String> = tmp1.clone(); // sh:452

            let matcher_str = format!(
                "{} {}",
                get_str("_matcher"),
                matcher.get(1).cloned().unwrap_or_default()
            );

            // sh:454-471 — drive compfiles.
            setaparam("tmp1", tmp1.clone());
            setaparam("accex", accex.clone());
            setaparam("fake", fake.clone());
            let concat = format!("{}{}", tpre, tsuf);
            if concat.contains('/') {
                let tail = concat.rsplit('/').next().unwrap_or("");
                let use_sdirs = if !fake.is_empty() || !tail.is_empty() {
                    sdirs.clone()
                } else {
                    String::new()
                };
                compfiles(vec![
                    format!("-P{}", cfopt),
                    "tmp1".into(),
                    "accex".into(),
                    skipped.clone(),
                    matcher_str.clone(),
                    use_sdirs,
                    "fake".into(),
                ]);
            } else if sopt
                .as_deref()
                .map(|s| s.contains('/') || s.contains('f'))
                .unwrap_or(false)
            {
                let mut a = vec![
                    format!("-p{}", cfopt),
                    "tmp1".into(),
                    "accex".into(),
                    skipped.clone(),
                    matcher_str.clone(),
                    sdirs.clone(),
                    "fake".into(),
                ];
                a.extend(pats.clone());
                compfiles(a);
            } else {
                let mut a = vec![
                    format!("-p{}", cfopt),
                    "tmp1".into(),
                    "accex".into(),
                    skipped.clone(),
                    matcher_str.clone(),
                    "".into(),
                    "fake".into(),
                ];
                a.extend(pats.clone());
                compfiles(a);
            }
            // sh:472 — tmp1=( $~tmp1 )
            tmp1 = tilde_glob(&get_arr("tmp1"));

            let cur_prefix = get_str("PREFIX");
            let cur_suffix = get_str("SUFFIX");
            if !format!("{}{}", cur_prefix, cur_suffix).is_empty() {
                // sh:487-502 — pws non-canonical hack.
                if tmp1.is_empty() && npathcheck == 0 {
                    npathcheck = 1;
                    for tmp3 in &tmp2 {
                        let mut base = tmp3.clone();
                        if !base.is_empty() && !base.ends_with('/') {
                            base.push('/');
                        }
                        let probe =
                            format!("{}{}{}", base, unquote(&cur_prefix), unquote(&cur_suffix));
                        if path_exists(&probe) {
                            npathcheck = 2;
                        }
                    }
                    if npathcheck == 2 {
                        tmp1 = origtmp1.clone();
                        continue;
                    }
                }

                let tmp2b: Vec<String>;
                if tmp1.is_empty() {
                    // sh:505 — tmp2=( ${^${tmp2:#/}}/$PREFIX$SUFFIX )
                    tmp2b = tmp2
                        .iter()
                        .filter(|e| e.as_str() != "/")
                        .map(|e| format!("{}/{}{}", e, cur_prefix, cur_suffix))
                        .collect();
                } else if tmp1.first().map(|s| s.contains('/')).unwrap_or(false) {
                    // sh:506-518 — reduce to basenames via compadd -D.
                    tmp2b = tmp1.clone();
                    setaparam("tmp1", tmp1.clone());
                    let mut a: Vec<String> = vec!["-D".into(), "tmp1".into()];
                    a.extend(matcher.clone());
                    a.push("-".into());
                    a.extend(tails(&tmp1));
                    compadd(a);
                    tmp1 = get_arr("tmp1");
                    if !comp_correct.is_empty() && tmp1.is_empty() {
                        tmp1 = tmp2b.clone();
                        setaparam("tmp1", tmp1.clone());
                        let mut a2: Vec<String> = vec!["-D".into(), "tmp1".into()];
                        a2.extend(matcher.clone());
                        a2.push("-".into());
                        a2.extend(tails(&tmp2b));
                        compadd(a2);
                        tmp1 = get_arr("tmp1");
                    }
                } else {
                    // sh:519-522
                    tmp2b = vec![String::new()];
                    setaparam("tmp1", tmp1.clone());
                    let mut a: Vec<String> = vec!["-D".into(), "tmp1".into()];
                    a.extend(matcher.clone());
                    a.push("-a".into());
                    a.push("tmp1".into());
                    compadd(a);
                    tmp1 = get_arr("tmp1");
                }

                // sh:527-544 — no file matched: save expanded path.
                if tmp1.is_empty() {
                    if tmp2b.first().map(|s| s.contains('/')).unwrap_or(false) {
                        let pr = format!("{}{}", prepath, realpath);
                        let mut tt: Vec<String> = tmp2b
                            .iter()
                            .map(|s| s.strip_prefix(&pr).unwrap_or(s).to_string())
                            .collect();
                        if tt.first().map(|s| s.contains('/')).unwrap_or(false) {
                            // ${(@)tmp2:h}
                            tt = tt.iter().map(|s| head_dir(s)).collect();
                            setaparam("tmp2", tt.clone());
                            compquote(vec!["tmp2".into()]);
                            tt = get_arr("tmp2");
                            for t in &tt {
                                if t.ends_with('/') {
                                    exppaths.push(format!("{}{}{}", t, tpre, tsuf));
                                } else {
                                    exppaths.push(format!("{}/{}{}", t, tpre, tsuf));
                                }
                            }
                        } else if concat.contains('/') {
                            exppaths.push(format!("{}{}", tpre, tsuf));
                        }
                    }
                    hit_continue_outer = true;
                    break;
                }
            } else if tmp1.is_empty() {
                // sh:546-573 — empty dir hacks.
                if concat.is_empty() && !format!("{}{}", pre, suf).is_empty() {
                    let mut np = vec!["-S".to_string(), "".to_string()];
                    np.extend(pfxsfx.clone());
                    pfxsfx = np;
                } else if haspats
                    && format!("{}{}{}", tpre, tsuf, suf).is_empty()
                    && pre.ends_with('/')
                {
                    setsparam("PREFIX", &opre);
                    setsparam("SUFFIX", &osuf);
                    compadd(vec![
                        "-nQS".into(),
                        "".into(),
                        "-".into(),
                        format!("{}{}{}", linepath, donepath, orig),
                    ]);
                }
                hit_continue_outer = true;
                break;
            }

            // sh:575-585 — ignore-parents.
            if !ignpar.is_empty()
                && comp_no_ignore.is_empty()
                && !concat.contains('/')
                && !tmp1.is_empty()
                && (!ignpar.contains("dir") || pats.first().map(|s| s == "*(-/)").unwrap_or(false))
                && (!ignpar.contains("..")
                    || tmp1.first().map(|s| s.contains("../")).unwrap_or(false))
            {
                let base = format!("{}{}{}", prepath, realpath, donepath);
                setaparam("tmp1", tmp1.clone());
                setaparam("ignore", ignore.clone());
                compfiles(vec![
                    "-i".into(),
                    "tmp1".into(),
                    "ignore".into(),
                    ignpar.clone(),
                    base.clone(),
                ]);
                ignore = get_arr("ignore");
                let mut ci = get_arr("_comp_ignore");
                ci.extend(
                    ignore
                        .iter()
                        .map(|e| e.strip_prefix(&base).unwrap_or(e).to_string()),
                );
                setaparam("_comp_ignore", ci.clone());
                if !ci.is_empty() && !mopts.iter().any(|e| e == "-F") {
                    mopts.push("-F".into());
                    mopts.push("_comp_ignore".into());
                }
            }

            // sh:589-596 — advance to next component.
            if tpre.contains('/') {
                tpre = tpre.splitn(2, '/').nth(1).unwrap_or("").to_string();
            } else if tsuf.contains('/') {
                tpre = tsuf.splitn(2, '/').nth(1).unwrap_or("").to_string();
                tsuf.clear();
            } else {
                break;
            }

            // sh:602-608 — skip over next components.
            tmp2s = match_skips_prefix(&tpre, skips_squeeze);
            if !tmp2s.is_empty() {
                skipped = format!("/{}", tmp2s);
                tpre = tpre[tmp2s.len()..].to_string();
            } else {
                skipped = "/".into();
            }
            npathcheck = 0;
        }

        if hit_continue_outer {
            continue; // continue 2
        }

        // sh:612-625 — the first-ambiguous-component search.
        let mut tmp3 = format!("{}{}", pre, suf);
        tpre = pre.clone();
        tsuf = suf.clone();
        let anchor = format!("{}{}{}", prepath, realpath, testpath);
        if !anchor.is_empty() {
            tmp1 = tmp1
                .iter()
                .map(|s| s.strip_prefix(&anchor).unwrap_or(s).to_string())
                .collect();
        }

        let mut tmp4 = String::new();
        loop {
            // sh:634-635 — compfiles -r.
            setaparam("tmp1", tmp1.clone());
            let amb = compfiles(vec!["-r".into(), "tmp1".into(), unquote(&tmp3)]);
            tmp1 = get_arr("tmp1");
            tmp4 = amb.to_string();

            let tmp2s2;
            if tpre.contains('/') {
                tmp2s2 = format!("{}{}", cpre, tpre.split('/').next().unwrap_or(""));
                setsparam("PREFIX", &format!("{}{}{}", linepath, donepath, tmp2s2));
                setsparam(
                    "SUFFIX",
                    &format!(
                        "/{}{}",
                        tpre.splitn(2, '/').nth(1).unwrap_or(""),
                        tsuf.splitn(2, '/').nth(1).unwrap_or("")
                    ),
                );
            } else {
                tmp2s2 = format!("{}{}", cpre, tpre);
                setsparam("PREFIX", &format!("{}{}{}", linepath, donepath, tmp2s2));
                setsparam("SUFFIX", &tsuf);
            }

            if amb != 0 {
                // sh:651-757 — ambiguous component: add candidates.
                let mut tmp2s3 = testpath.clone();
                if !linepath.is_empty() {
                    setaparam("tmp2", vec![tmp2s3.clone()]);
                    setaparam("tmp1", tmp1.clone());
                    compquote(vec!["-p".into(), "tmp2".into(), "tmp1".into()]);
                    tmp2s3 = get_arr("tmp2").into_iter().next().unwrap_or_default();
                    tmp1 = get_arr("tmp1");
                } else if !tmp2s3.is_empty() {
                    setaparam("tmp1", tmp1.clone());
                    compquote(vec!["-p".into(), "tmp1".into()]);
                    tmp1 = get_arr("tmp1");
                    setaparam("tmp2", vec![tmp2s3.clone()]);
                    compquote(vec!["tmp2".into()]);
                    tmp2s3 = get_arr("tmp2").into_iter().next().unwrap_or_default();
                } else {
                    setaparam("tmp1", tmp1.clone());
                    setaparam("tmp2", vec![tmp2s3.clone()]);
                    compquote(vec!["tmp1".into(), "tmp2".into()]);
                    tmp1 = get_arr("tmp1");
                    tmp2s3 = get_arr("tmp2").into_iter().next().unwrap_or_default();
                }

                if comp_correct.is_empty()
                    && pattern_match == "*"
                    && listsfx
                    && has_active_glob(&tmp2s3)
                {
                    setsparam("PREFIX", &opre);
                    setsparam("SUFFIX", &osuf);
                }

                let ipx = get_str("IPREFIX");
                let isx = get_str("ISUFFIX");
                let anchor2 = format!("{}{}{}", prepath, realpath, testpath);
                let listing = cs_s("insert").is_empty()
                    || (!zstyle_t(&paths_ctx, "expand")
                        && !listsfx
                        && (!comp_correct.is_empty()
                            || pattern_match.is_empty()
                            || !get_str("SUFFIX").contains('/')
                            || has_active_glob(
                                get_str("SUFFIX").splitn(2, '/').nth(1).unwrap_or(""),
                            )));

                if listing {
                    if amb != 0 && zstyle_t(&paths_ctx, "ambiguous") {
                        crate::ported::zle::compcore::set_compstate_str("to_end", "");
                    }
                    if tmp3.contains('/') {
                        if !listsfx
                            || !tmp3
                                .split('/')
                                .nth(1)
                                .map(|s| !s.is_empty())
                                .unwrap_or(false)
                        {
                            // sh:694-702
                            tmp1 = tmp1
                                .iter()
                                .map(|s| s.split('/').next().unwrap_or("").to_string())
                                .collect();
                            setaparam("tmp1", tmp1.clone());
                            dispatch0("_list_files", &["tmp1".into(), anchor2.clone()]);
                            let listopts = get_arr("listopts");
                            let mut a = vec![uopt.clone()];
                            a.retain(|s| !s.is_empty());
                            a.push("-Qf".into());
                            a.extend(mopts.clone());
                            a.push("-p".into());
                            a.push(format!(
                                "{}{}{}",
                                if uopt.is_empty() { "" } else { ipx.as_str() },
                                linepath,
                                tmp2s3
                            ));
                            a.push("-s".into());
                            a.push(format!(
                                "/{}{}",
                                tmp3.splitn(2, '/').nth(1).unwrap_or(""),
                                if uopt.is_empty() {
                                    String::new()
                                } else {
                                    isx.clone()
                                }
                            ));
                            a.push("-W".into());
                            a.push(anchor2.clone());
                            a.extend(pfxsfx.clone());
                            a.extend(mopts_r.clone());
                            a.extend(listopts.clone());
                            a.push("-a".into());
                            a.push("tmp1".into());
                            compadd(a);
                        } else {
                            // sh:704-713
                            tmp1 = tmp1
                                .iter()
                                .map(|s| {
                                    format!(
                                        "{}/{}",
                                        s.split('/').next().unwrap_or(""),
                                        tmp3.splitn(2, '/').nth(1).unwrap_or("")
                                    )
                                })
                                .collect();
                            setaparam("tmp1", tmp1.clone());
                            dispatch0("_list_files", &["tmp1".into(), anchor2.clone()]);
                            let listopts = get_arr("listopts");
                            let mut a = vec![uopt.clone()];
                            a.retain(|s| !s.is_empty());
                            a.push("-Qf".into());
                            a.extend(mopts.clone());
                            a.push("-p".into());
                            a.push(format!(
                                "{}{}{}",
                                if uopt.is_empty() { "" } else { ipx.as_str() },
                                linepath,
                                tmp2s3
                            ));
                            a.push("-s".into());
                            a.push(if uopt.is_empty() {
                                String::new()
                            } else {
                                isx.clone()
                            });
                            a.push("-W".into());
                            a.push(anchor2.clone());
                            a.extend(pfxsfx.clone());
                            a.extend(mopts_r.clone());
                            a.extend(listopts.clone());
                            a.push("-a".into());
                            a.push("tmp1".into());
                            compadd(a);
                        }
                    } else {
                        // sh:716-722
                        setaparam("tmp1", tmp1.clone());
                        dispatch0("_list_files", &["tmp1".into(), anchor2.clone()]);
                        let listopts = get_arr("listopts");
                        let mut a = vec![uopt.clone()];
                        a.retain(|s| !s.is_empty());
                        a.push("-Qf".into());
                        a.extend(mopts.clone());
                        a.push("-p".into());
                        a.push(format!(
                            "{}{}{}",
                            if uopt.is_empty() { "" } else { ipx.as_str() },
                            linepath,
                            tmp2s3
                        ));
                        a.push("-s".into());
                        a.push(if uopt.is_empty() {
                            String::new()
                        } else {
                            isx.clone()
                        });
                        a.push("-W".into());
                        a.push(anchor2.clone());
                        a.extend(pfxsfx.clone());
                        a.extend(mopts_r.clone());
                        a.extend(listopts.clone());
                        a.push("-a".into());
                        a.push("tmp1".into());
                        compadd(a);
                    }
                } else {
                    // sh:724-753 — inserting the match.
                    if tmp3.contains('/') {
                        let mut base = vec![uopt.clone()];
                        base.retain(|s| !s.is_empty());
                        base.push("-Qf".into());
                        base.extend(mopts.clone());
                        base.push("-p".into());
                        base.push(format!(
                            "{}{}{}",
                            if uopt.is_empty() { "" } else { ipx.as_str() },
                            linepath,
                            tmp2s3
                        ));
                        base.push("-W".into());
                        base.push(anchor2.clone());
                        base.extend(pfxsfx.clone());
                        base.extend(mopts_r.clone());
                        if !listsfx {
                            for it in tmp1.clone() {
                                setaparam("tmpdisp", vec![it.clone()]);
                                dispatch0("_list_files", &["tmpdisp".into(), anchor2.clone()]);
                                let disp = get_arr("tmpdisp").into_iter().next().unwrap_or(it);
                                let listopts = get_arr("listopts");
                                let mut a = base.clone();
                                a.push("-s".into());
                                a.push(if uopt.is_empty() {
                                    String::new()
                                } else {
                                    isx.clone()
                                });
                                a.extend(listopts);
                                a.push("-".into());
                                a.push(disp);
                                compadd(a);
                            }
                        } else {
                            if !pattern_match.is_empty() {
                                // SUFFIX gs./.*/ + '*'
                                let cs = get_str("SUFFIX").replace('/', "/*/") + "*";
                                setsparam("SUFFIX", &cs);
                            }
                            for it in tmp1.clone() {
                                setaparam("i", vec![it.clone()]);
                                dispatch0("_list_files", &["i".into(), anchor2.clone()]);
                                let disp = get_arr("i").into_iter().next().unwrap_or(it);
                                let listopts = get_arr("listopts");
                                let mut a = base.clone();
                                a.extend(listopts);
                                a.push("-".into());
                                a.push(disp);
                                compadd(a);
                            }
                        }
                    } else {
                        setaparam("tmp1", tmp1.clone());
                        dispatch0("_list_files", &["tmp1".into(), anchor2.clone()]);
                        let listopts = get_arr("listopts");
                        let mut a = vec![uopt.clone()];
                        a.retain(|s| !s.is_empty());
                        a.push("-Qf".into());
                        a.extend(mopts.clone());
                        a.push("-p".into());
                        a.push(format!(
                            "{}{}{}",
                            if uopt.is_empty() { "" } else { ipx.as_str() },
                            linepath,
                            tmp2s3
                        ));
                        a.push("-s".into());
                        a.push(if uopt.is_empty() {
                            String::new()
                        } else {
                            isx.clone()
                        });
                        a.push("-W".into());
                        a.push(anchor2.clone());
                        a.extend(pfxsfx.clone());
                        a.extend(mopts.clone());
                        a.extend(mopts_r.clone());
                        a.extend(listopts);
                        a.push("-a".into());
                        a.push("tmp1".into());
                        compadd(a);
                    }
                }
                tmp4 = "-".into();
                break;
            }

            // sh:762-765 — all components checked.
            if !tmp3.contains('/') {
                tmp4.clear();
                break;
            }

            // sh:770-797 — commit the unambiguous component.
            let head = tmp1
                .first()
                .map(|s| s.split('/').next().unwrap_or(""))
                .unwrap_or("");
            testpath = format!("{}{}/", testpath, head);
            tmp3 = tmp3.splitn(2, '/').nth(1).unwrap_or("").to_string();

            let use_line_head =
                comp_correct.is_empty() && !pattern_match.is_empty() && has_active_glob(&tmp2s2);
            if tpre.contains('/') {
                if use_line_head {
                    cpre = format!(
                        "{}{}/",
                        cpre,
                        tmp1.first()
                            .map(|s| s.split('/').next().unwrap_or(""))
                            .unwrap_or("")
                    );
                } else {
                    cpre = format!("{}{}/", cpre, tpre.split('/').next().unwrap_or(""));
                }
                tpre = tpre.splitn(2, '/').nth(1).unwrap_or("").to_string();
            } else if tsuf.contains('/') {
                // mid handling folded below via testpath
                if use_line_head {
                    cpre = format!(
                        "{}{}/",
                        cpre,
                        tmp1.first()
                            .map(|s| s.split('/').next().unwrap_or(""))
                            .unwrap_or("")
                    );
                } else {
                    cpre = format!("{}{}/", cpre, tpre);
                }
                tpre = tsuf.splitn(2, '/').nth(1).unwrap_or("").to_string();
                tsuf.clear();
            } else {
                tpre.clear();
                tsuf.clear();
            }

            tmp1 = tmp1
                .iter()
                .map(|s| s.splitn(2, '/').nth(1).unwrap_or("").to_string())
                .collect();
        }

        // sh:800-876 — final add of collected matches (non-ambiguous).
        if tmp4.is_empty() {
            // The `mid` middle-of-line branch (sh:803-840) is folded into
            // the common last-component add below; testpath already
            // carries the committed directory prefix. sh approx.
            if osuf.contains('/') {
                setsparam("PREFIX", &format!("{}{}", opre, osuf));
                setsparam("SUFFIX", "");
            } else {
                setsparam("PREFIX", &opre);
                setsparam("SUFFIX", &osuf);
            }
            let mut tmp4s = testpath.clone();
            if !linepath.is_empty() {
                setaparam("tmp4", vec![tmp4s.clone()]);
                setaparam("tmp1", tmp1.clone());
                compquote(vec!["-p".into(), "tmp4".into(), "tmp1".into()]);
                tmp4s = get_arr("tmp4").into_iter().next().unwrap_or_default();
                tmp1 = get_arr("tmp1");
            } else if !tmp4s.is_empty() {
                setaparam("tmp1", tmp1.clone());
                compquote(vec!["-p".into(), "tmp1".into()]);
                tmp1 = get_arr("tmp1");
                setaparam("tmp4", vec![tmp4s.clone()]);
                compquote(vec!["tmp4".into()]);
                tmp4s = get_arr("tmp4").into_iter().next().unwrap_or_default();
            } else {
                setaparam("tmp4", vec![tmp4s.clone()]);
                setaparam("tmp1", tmp1.clone());
                compquote(vec!["tmp4".into(), "tmp1".into()]);
                tmp4s = get_arr("tmp4").into_iter().next().unwrap_or_default();
                tmp1 = get_arr("tmp1");
            }

            let prefix_now = get_str("PREFIX");
            let suffix_now = get_str("SUFFIX");
            let px = format!(
                "{}{}",
                prefix_now.strip_prefix('~').unwrap_or(&prefix_now),
                suffix_now
            );
            let ipx = get_str("IPREFIX");
            let isx = get_str("ISUFFIX");
            let anchor3 = format!("{}{}{}", prepath, realpath, testpath);
            if comp_correct.is_empty() && !pattern_match.is_empty() && has_active_glob(&px) {
                // sh:862-866 — pattern match.
                tmp1 = tmp1
                    .iter()
                    .map(|s| format!("{}{}{}", linepath, tmp4s, s))
                    .collect();
                setaparam("tmp1", tmp1.clone());
                dispatch0(
                    "_list_files",
                    &["tmp1".into(), format!("{}{}", prepath, realpath)],
                );
                let listopts = get_arr("listopts");
                let mut a = vec![
                    "-Qf".to_string(),
                    "-W".into(),
                    format!("{}{}", prepath, realpath),
                ];
                a.extend(pfxsfx.clone());
                a.extend(mopts.clone());
                a.push("-M".into());
                a.push("r:|/=* r:|=*".into());
                a.extend(listopts);
                a.push("-a".into());
                a.push("tmp1".into());
                compadd(a);
            } else {
                // sh:868-873 — normal add.
                setaparam("tmp1", tmp1.clone());
                dispatch0("_list_files", &["tmp1".into(), anchor3.clone()]);
                let listopts = get_arr("listopts");
                let mut a = vec![uopt.clone()];
                a.retain(|s| !s.is_empty());
                a.push("-Qf".into());
                a.push("-p".into());
                a.push(format!(
                    "{}{}{}",
                    if uopt.is_empty() { "" } else { ipx.as_str() },
                    linepath,
                    tmp4s
                ));
                a.push("-s".into());
                a.push(if uopt.is_empty() {
                    String::new()
                } else {
                    isx.clone()
                });
                a.push("-W".into());
                a.push(anchor3.clone());
                a.extend(pfxsfx.clone());
                a.extend(mopts.clone());
                a.extend(mopts_r.clone());
                a.extend(listopts);
                a.push("-a".into());
                a.push("tmp1".into());
                compadd(a);
            }
        }
    }

    // sh:886-893 — expand-paths.
    let matcher_num = getsparam("_matcher_num")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let matchers = get_arr("_matchers").len() as i64;
    if matcher_num == matchers
        && zstyle_t(&paths_ctx, "expand")
        && nm == cs_i("nmatches")
        && !exppaths.is_empty()
        && format!("{}{}", linepath, exppaths.join(" ")) != eorig
    {
        setsparam("PREFIX", &opre);
        setsparam("SUFFIX", &osuf);
        setaparam("exppaths", dedup(exppaths.clone()));
        let mut a = vec!["-Q".to_string()];
        a.extend(mopts.clone());
        a.push("-S".into());
        a.push("".into());
        a.push("-M".into());
        a.push("r:|/=* r:|=*".into());
        a.push("-p".into());
        a.push(linepath.clone());
        a.push("-a".into());
        a.push("exppaths".into());
        compadd(a);
    }

    // sh:895 — return status.
    if nm != cs_i("nmatches") {
        0
    } else {
        1
    }
}

// ---- misc string helpers ------------------------------------------

fn dedup(v: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    v.into_iter().filter(|e| seen.insert(e.clone())).collect()
}

fn unquote(s: &str) -> String {
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

/// `eval "x=~spec"` — tilde expansion (`~`, `~user`, `~-`, `~+`). Routes
/// through the ported `filesubstr` by converting the leading ASCII `~`
/// to the Tilde token it expects.
fn expand_tilde(spec: &str) -> Option<String> {
    let rest = spec.strip_prefix('~')?;
    filesubstr(&format!("\u{98}{}", rest), false)
}

fn is_dir(p: &str) -> bool {
    std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}
fn path_exists(p: &str) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}
fn is_numeric_dirstack(s: &str) -> bool {
    // ([-+]|)[0-9]##
    let body = s.strip_prefix(['-', '+']).unwrap_or(s);
    !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit())
}

/// `${(@)s:h}` — dirname.
fn head_dir(s: &str) -> String {
    let t = s.trim_end_matches('/');
    match t.rfind('/') {
        Some(i) if i > 0 => t[..i].to_string(),
        Some(_) => "/".to_string(),
        None => ".".to_string(),
    }
}

/// `${orig[1,(in:i:)/][1,-2]}` — keep everything up to and including the
/// i-th slash, then drop the final char. sh approx.
fn truncate_after_nth_slash(s: &str, n: usize) -> String {
    let mut count = 0;
    for (idx, c) in s.char_indices() {
        if c == '/' {
            count += 1;
            if count == n {
                return s[..idx].to_string();
            }
        }
    }
    s.to_string()
}

/// `pre = (#b)(${~pp})*` — return the leading match of pattern `pp`
/// against `pre` (the matched prefix), if any. sh approx: literal or
/// simple leading match.
fn match_leading_pattern(pre: &str, pp: &str) -> Option<String> {
    if let Some(prog) = crate::ported::pattern::patcompile(
        &{
            let mut s = pp.to_string();
            tokenize(&mut s);
            s
        },
        0,
        None,
    ) {
        // Longest leading prefix of `pre` that matches `pp`.
        let mut best: Option<String> = None;
        for (i, _) in pre.char_indices().chain(std::iter::once((pre.len(), ' '))) {
            if crate::ported::pattern::pattry(&prog, &pre[..i]) {
                best = Some(pre[..i].to_string());
            }
        }
        best
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zparse_dirs_only() {
        let p = zparse_pathfiles(&["-/".to_string()]);
        assert_eq!(p.tmp1, vec!["-/".to_string()]);
        assert!(p.mopts.is_empty());
    }

    #[test]
    fn zparse_glob_separate_concats() {
        // -g <pat> stores option+value concatenated (ZOF_SAME).
        let p = zparse_pathfiles(&["-g".to_string(), "*.rs".to_string()]);
        assert_eq!(p.tmp1, vec!["-g*.rs".to_string()]);
    }

    #[test]
    fn zparse_glob_attached() {
        let p = zparse_pathfiles(&["-g*.txt".to_string()]);
        assert_eq!(p.tmp1, vec!["-g*.txt".to_string()]);
    }

    #[test]
    fn zparse_value_options_split_into_two_elements() {
        // -W stores option and value as two elements (not concatenated).
        let p = zparse_pathfiles(&["-W".to_string(), "/tmp".to_string()]);
        assert_eq!(p.prepaths, vec!["-W".to_string(), "/tmp".to_string()]);
        // -P → pfx, -M → matcher.
        let p2 = zparse_pathfiles(&[
            "-P".to_string(),
            "pre".to_string(),
            "-M".to_string(),
            "m:{a-z}={A-Z}".to_string(),
        ]);
        assert_eq!(p2.pfx, vec!["-P".to_string(), "pre".to_string()]);
        assert_eq!(
            p2.matcher,
            vec!["-M".to_string(), "m:{a-z}={A-Z}".to_string()]
        );
    }

    #[test]
    fn zparse_flags_go_to_mopts() {
        let p = zparse_pathfiles(&[
            "-J".to_string(),
            "grp".to_string(),
            "-1".to_string(),
            "-n".to_string(),
        ]);
        assert_eq!(
            p.mopts,
            vec![
                "-J".to_string(),
                "grp".to_string(),
                "-1".to_string(),
                "-n".to_string()
            ]
        );
    }

    #[test]
    fn zparse_stops_at_bare_dash() {
        // A bare `-` (compadd terminator) ends option parsing.
        let p = zparse_pathfiles(&["-f".to_string(), "-".to_string(), "x".to_string()]);
        assert_eq!(p.tmp1, vec!["-f".to_string()]);
    }

    #[test]
    fn empty_line_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "/nonexistent/path/here_");
        let _ = setsparam("SUFFIX", "");
        // No active completion => nmatches unchanged => rc 1.
        assert_eq!(_path_files(&[]), 1);
    }
}
