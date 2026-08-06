//! Port of `_arch_namespace` from `Completion/Unix/Type/_arch_namespace`.
//!
//! Full upstream body (105 lines, abridged) — four mutually-recursive
//! shell functions that walk the arch/tla namespace one component at a
//! time (category → branch → version → revision):
//! ```text
//! sh:  3  _arch_namespace()          ARCHCMD, $1 = #components (1..4)
//! sh:  9    flags: --trailing-dashes  --library  --exclude-library-revisions
//! sh: 15    suffix=(-q -S --) when more components wanted / trailing dashes
//! sh: 18    $PREFIX = */*  → compset -P '*/'; archive=${IPREFIX%/*}; compadd categories
//! sh: 24    else            find archive from -A/--archive word; compadd categories;
//! sh:          _arch_archives "$ARCHCMD" -S / --library?
//! sh: 40    archive && >1 comp && *--* → _arch_namespace_branches
//! sh: 45  _arch_namespace_branches() compset -P 1 '*--'; compadd branches; recurse versions
//! sh: 66  _arch_namespace_versions() compset -P 1 '*--'; compadd versions; recurse revisions
//! sh: 84  _arch_namespace_revisions() compset -P 1 '*--'; compadd revisions (minus lib revs)
//! sh:105  _arch_namespace "$@"
//! ```
//!
//! The shell runs `$ARCHCMD <sub>` via backticks; the port captures that
//! stdout with a `sh -c` runner (backtick-equivalent, whitespace-split).

use crate::compsys::ported::_arch_archives::arch_archives_byname;
use crate::compsys::ported::_description::description_byname;
use crate::ported::params::{getaparam, getsparam};
use crate::ported::zle::complete::{bin_compadd, bin_compset};
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}
fn compset(argv: &[&str]) -> i32 {
    let v: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    bin_compset("compset", &v, &make_ops(), 0)
}
fn compadd(argv: Vec<String>) -> i32 {
    bin_compadd("compadd", &argv, &make_ops(), 0)
}

/// `` `$ARCHCMD sub…` `` — run the arch command and capture whitespace-split
/// stdout (backtick semantics).
fn arch_run(archcmd: &str, sub: &[&str]) -> Vec<String> {
    if archcmd.is_empty() {
        return Vec::new();
    }
    let mut cmdline = archcmd.to_string();
    for s in sub {
        cmdline.push(' ');
        cmdline.push_str(s);
    }
    std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmdline)
        .output()
        .ok()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// State threaded through the (dynamically-scoped in shell) recursion.
struct Ctx {
    archcmd: String,
    trailing_dashes: bool,
    library: String, // "" or "library-"
    exclude_library_revisions: bool,
}

/// sh:15 — `suffix=(-q -S --)` when more components are wanted.
fn suffix_for(more: bool) -> Vec<String> {
    if more {
        vec!["-q".to_string(), "-S".to_string(), "--".to_string()]
    } else {
        Vec::new()
    }
}
fn expl() -> Vec<String> {
    getaparam("expl").unwrap_or_default()
}
/// Strip a trailing `*--`-delimited component tail (`##*--`).
fn strip_before_dashes(s: &str) -> &str {
    match s.rfind("--") {
        Some(i) => &s[i + 2..],
        None => s,
    }
}

/// `_arch_namespace` — entry (sh:105 `_arch_namespace "$@"`).
pub fn _arch_namespace(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_arch_namespace");
    let Some(archcmd) = args.first().cloned() else {
        return 1;
    };
    let rest: Vec<String> = args[1..].to_vec();
    // sh:12 — `$1` after shift is the component count.
    let count: i64 = rest.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let ctx = Ctx {
        archcmd: archcmd.clone(),
        trailing_dashes: rest.iter().any(|a| a == "--trailing-dashes"),
        library: if rest.iter().any(|a| a == "--library") {
            "library-".to_string()
        } else {
            String::new()
        },
        exclude_library_revisions: rest.iter().any(|a| a == "--exclude-library-revisions"),
    };
    arch_namespace(&ctx, count)
}

/// sh:3-43 — the top-level category level.
fn arch_namespace(ctx: &Ctx, count: i64) -> i32 {
    // sh:8 — default archive.
    let mut archive = arch_run(&ctx.archcmd, &["my-default-archive"])
        .into_iter()
        .next()
        .unwrap_or_default();
    // sh:14-16
    let suffix = suffix_for(count > 1 || ctx.trailing_dashes);

    let prefix = getsparam("PREFIX").unwrap_or_default();
    let iprefix = getsparam("IPREFIX").unwrap_or_default();

    if prefix.contains('/') {
        // sh:18-23
        let _ = compset(&["-P", "*/"]);
        let iprefix = getsparam("IPREFIX").unwrap_or_default();
        archive = iprefix
            .rsplit_once('/')
            .map(|(a, _)| a)
            .unwrap_or(&iprefix)
            .to_string();
        let _ = description_byname(&[
            "-V".to_string(),
            "categories".to_string(),
            "expl".to_string(),
            format!("{}categories in {}", ctx.library, archive),
        ]);
        let cats = arch_run(
            &ctx.archcmd,
            &[&format!("{}categories", ctx.library), &archive],
        );
        let mut cadd = suffix.clone();
        cadd.extend(expl());
        cadd.extend(cats);
        let _ = compadd(cadd);
    } else if iprefix.is_empty() {
        // sh:24-38 — find archive from an -A/--archive word.
        let words = getaparam("words").unwrap_or_default();
        let current: usize = getsparam("CURRENT")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let idx_of = |flag: &str| words.iter().position(|w| w == flag).map(|i| i + 1);
        let mut index = idx_of("-A").unwrap_or(0);
        if !(index < current && index != 0) {
            index = idx_of("--archive").unwrap_or(0);
        }
        if index != 0 && index < current {
            if let Some(w) = words.get(index) {
                archive = w.clone();
            }
        }
        if !archive.is_empty() {
            let _ = description_byname(&[
                "-V".to_string(),
                "categories".to_string(),
                "expl".to_string(),
                format!("{}categories in {}", ctx.library, archive),
            ]);
            let cats = arch_run(
                &ctx.archcmd,
                &[&format!("{}categories", ctx.library), &archive],
            );
            let mut cadd = expl();
            cadd.extend(suffix.clone());
            cadd.extend(cats);
            let _ = compadd(cadd);
        }
        // sh:38  _arch_archives "$ARCHCMD" -S / ${library:+--library}
        let mut aa = vec![ctx.archcmd.clone(), "-S".to_string(), "/".to_string()];
        if !ctx.library.is_empty() {
            aa.push("--library".to_string());
        }
        let _ = arch_archives_byname(&aa);
    }

    // sh:40-43 — descend to branches once past the category.
    if !archive.is_empty() && count > 1 && !prefix.contains('@') && prefix.contains("--") {
        arch_namespace_branches(ctx, count - 1);
    }
    1
}

/// sh:45-64.
fn arch_namespace_branches(ctx: &Ctx, count: i64) -> i32 {
    let suffix = suffix_for(count > 1 || ctx.trailing_dashes);
    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    if !iprefix.ends_with("--") {
        let _ = compset(&["-P", "1", "*--"]);
        let category = getsparam("IPREFIX")
            .unwrap_or_default()
            .trim_end_matches("--")
            .to_string();
        let _ = description_byname(&[
            "-V".to_string(),
            "branches".to_string(),
            "expl".to_string(),
            format!("{}branches", ctx.library),
        ]);
        let branches: Vec<String> = arch_run(
            &ctx.archcmd,
            &[&format!("{}branches", ctx.library), &category],
        )
        .iter()
        .map(|b| strip_before_dashes(b).to_string())
        .collect();
        let mut cadd = suffix;
        cadd.extend(expl());
        cadd.extend(branches);
        let _ = compadd(cadd);
    }
    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();
    if count > 1 && iprefix.ends_with("--") && prefix.contains("--") {
        arch_namespace_versions(ctx, count - 1);
    }
    1
}

/// sh:66-82.
fn arch_namespace_versions(ctx: &Ctx, count: i64) -> i32 {
    let suffix = suffix_for(count > 1);
    let _ = compset(&["-P", "1", "*--"]);
    let branch = getsparam("IPREFIX")
        .unwrap_or_default()
        .trim_end_matches("--")
        .to_string();
    let _ = description_byname(&[
        "-V".to_string(),
        "versions".to_string(),
        "expl".to_string(),
        format!("{}versions", ctx.library),
    ]);
    let versions: Vec<String> = arch_run(
        &ctx.archcmd,
        &[&format!("{}versions", ctx.library), &branch],
    )
    .iter()
    .map(|v| strip_before_dashes(v).to_string())
    .collect();
    let mut cadd = suffix;
    cadd.extend(expl());
    cadd.extend(versions);
    let _ = compadd(cadd);

    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();
    if count > 1 && iprefix.ends_with("--") && prefix.contains("--") {
        arch_namespace_revisions(ctx);
    }
    1
}

/// sh:84-103.
fn arch_namespace_revisions(ctx: &Ctx) -> i32 {
    let _ = compset(&["-P", "1", "*--"]);
    let version = getsparam("IPREFIX")
        .unwrap_or_default()
        .trim_end_matches("--")
        .to_string();
    let _ = description_byname(&[
        "-V".to_string(),
        "revisions".to_string(),
        "expl".to_string(),
        format!("{}revisions", ctx.library),
    ]);
    let mut completions: Vec<String> = arch_run(
        &ctx.archcmd,
        &[&format!("{}revisions", ctx.library), &version],
    )
    .iter()
    .map(|r| strip_before_dashes(r).to_string())
    .collect();
    // sh:98-99 — exclude library revisions.
    if ctx.exclude_library_revisions {
        let lib_revs = arch_run(&ctx.archcmd, &["library-revisions", &version]);
        completions.retain(|c| !lib_revs.contains(c));
    }
    let mut cadd = expl();
    cadd.push("-a".to_string());
    // publish `completions` as a named array for `compadd -a`.
    crate::ported::params::setaparam("completions", completions);
    cadd.push("completions".to_string());
    compadd(cadd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_archcmd_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_arch_namespace(&[]), 1);
    }

    #[test]
    fn strip_before_dashes_keeps_tail() {
        assert_eq!(strip_before_dashes("cat--branch--0.1"), "0.1");
        assert_eq!(strip_before_dashes("plain"), "plain");
    }
}
