//! Port of `_deb_architectures` from `Completion/Debian/Type/_deb_architectures`.
//!
//! Full upstream body (9 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local extra expl
//! sh: 4  zparseopts -E -D -a extra a:
//! sh: 6  _description architectures expl 'architecture'
//! sh: 7  compadd "$@" "$expl[@]" alpha amd64 arm arm64 armel armhf hppa hurd-i386 i386 \
//! sh: 8      ia64 kfreebsd-amd64 loong64 loongarch6 m68k mips mips64el mipsel powerpc \
//! sh: 9      ppc64 ppc64el riscv64 s390x sh4 sparc sparc64 x32 ${=extra[2]}
//! ```

use crate::compsys::ported::_description::_description;
use crate::ported::params::getaparam;
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

/// sh:7-9 — static Debian architecture list.
const ARCHITECTURES: &[&str] = &[
    "alpha",
    "amd64",
    "arm",
    "arm64",
    "armel",
    "armhf",
    "hppa",
    "hurd-i386",
    "i386",
    "ia64",
    "kfreebsd-amd64",
    "loong64",
    "loongarch6",
    "m68k",
    "mips",
    "mips64el",
    "mipsel",
    "powerpc",
    "ppc64",
    "ppc64el",
    "riscv64",
    "s390x",
    "sh4",
    "sparc",
    "sparc64",
    "x32",
];

/// sh:4 — `zparseopts -E -D -a extra a:` restricted to the single `a:` spec
/// this function declares: pull the *first* `-a VALUE` pair out of `args`
/// (removing every `-a VALUE` occurrence per `-D`, scanning the whole list
/// per `-E` rather than stopping at the first non-option word), leaving the
/// rest for pass-through to `compadd "$@"`. `extra[2]` in the shell is the
/// value of the first `-a` match (`extra[1]` is the literal `-a` flag).
fn zparse_a(args: &[String]) -> (Option<String>, Vec<String>) {
    let mut value = None;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-a" && i + 1 < args.len() {
            if value.is_none() {
                value = Some(args[i + 1].clone());
            }
            i += 2;
        } else {
            rest.push(args[i].clone());
            i += 1;
        }
    }
    (value, rest)
}

/// `_deb_architectures` — complete Debian architecture names.
pub fn _deb_architectures(args: &[String]) -> i32 {
    // sh:4
    let (extra2, rest) = zparse_a(args);

    // sh:6  _description architectures expl 'architecture'
    let _ = _description(&[
        "architectures".to_string(),
        "expl".to_string(),
        "architecture".to_string(),
    ]);

    // sh:7-9  compadd "$@" "$expl[@]" alpha amd64 ... x32 ${=extra[2]}
    let mut cadd: Vec<String> = rest;
    cadd.extend(getaparam("expl").unwrap_or_default());
    cadd.extend(ARCHITECTURES.iter().map(|s| s.to_string()));
    // ${=extra[2]} — word-split the -a VALUE onto separate compadd args.
    if let Some(v) = extra2 {
        cadd.extend(v.split_whitespace().map(|s| s.to_string()));
    }
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zparse_a_pulls_first_value_and_removes_all_occurrences() {
        let (v, rest) = zparse_a(&[
            "-P".into(),
            "foo".into(),
            "-a".into(),
            "extra1 extra2".into(),
            "-a".into(),
            "ignored".into(),
        ]);
        assert_eq!(v.as_deref(), Some("extra1 extra2"));
        assert_eq!(rest, vec!["-P".to_string(), "foo".to_string()]);
    }

    #[test]
    fn zparse_a_no_match_returns_none_and_full_rest() {
        let (v, rest) = zparse_a(&["-P".into(), "foo".into()]);
        assert_eq!(v, None);
        assert_eq!(rest, vec!["-P".to_string(), "foo".to_string()]);
    }

    #[test]
    fn architectures_list_has_no_duplicates_and_is_nonempty() {
        let mut sorted = ARCHITECTURES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), ARCHITECTURES.len());
        assert!(!ARCHITECTURES.is_empty());
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_deb_architectures(&[]), 1);
    }
}
