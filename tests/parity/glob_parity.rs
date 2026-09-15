//! File-glob parity tests — each test builds its own tempdir, runs the
//! glob in both shells from inside, sorts output, and compares. Sort
//! is needed because directory enumeration order is filesystem-dependent.

#![allow(non_snake_case)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

fn zsh_path() -> &'static str {
    if Path::new("/opt/homebrew/bin/zsh").exists() {
        "/opt/homebrew/bin/zsh"
    } else if Path::new("/usr/local/bin/zsh").exists() {
        "/usr/local/bin/zsh"
    } else {
        "/bin/zsh"
    }
}

fn zsh_available() -> bool {
    Command::new(zsh_path())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct R {
    stdout: String,
    exit: i32,
}

fn run_zsh_in(dir: &Path, s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .current_dir(dir)
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

fn run_zshrs_in(dir: &Path, s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .current_dir(dir)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}

fn assert_parity_in(dir: &Path, script: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh_in(dir, script);
    let r = run_zshrs_in(dir, script);
    let z_sorted: Vec<&str> = {
        let mut v: Vec<&str> = z.stdout.lines().collect();
        v.sort();
        v
    };
    let r_sorted: Vec<&str> = {
        let mut v: Vec<&str> = r.stdout.lines().collect();
        v.sort();
        v
    };
    assert_eq!(
        z_sorted, r_sorted,
        "glob output divergence on:\n{script}\n--- zsh sorted ---\n{:?}\n--- zshrs sorted ---\n{:?}",
        z_sorted, r_sorted
    );
}

/// Build a tempdir populated with the given files (subpaths). Directories
/// are auto-created from path separators.
fn mkdir_with_files(files: &[&str]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    for f in files {
        let p = dir.path().join(f);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).expect("mkdir -p");
        }
        std::fs::write(&p, b"").expect("touch");
    }
    dir
}

mod basic_globs {
    use super::*;

    #[test]
    fn star_dot_txt_matches_txt_files_only() {
        let d = mkdir_with_files(&["a.txt", "b.txt", "c.rs", "d.md"]);
        assert_parity_in(d.path(), "print -l *.txt");
    }

    #[test]
    fn star_matches_all_non_hidden_files() {
        let d = mkdir_with_files(&["a", "b", "c"]);
        assert_parity_in(d.path(), "print -l *");
    }

    #[test]
    fn star_skips_hidden_by_default() {
        let d = mkdir_with_files(&[".hidden", "visible"]);
        assert_parity_in(d.path(), "print -l *");
    }

    #[test]
    fn question_matches_single_char() {
        let d = mkdir_with_files(&["a", "bb", "ccc"]);
        assert_parity_in(d.path(), "print -l ?");
    }

    #[test]
    fn three_questions_match_three_chars() {
        let d = mkdir_with_files(&["a", "bb", "ccc", "dddd"]);
        assert_parity_in(d.path(), "print -l ???");
    }
}

mod char_classes {
    use super::*;

    #[test]
    fn bracket_digit_range() {
        let d = mkdir_with_files(&["file1", "file2", "file3", "fileA"]);
        assert_parity_in(d.path(), "print -l file[0-9]");
    }

    #[test]
    fn bracket_specific_chars() {
        let d = mkdir_with_files(&["fa", "fb", "fc", "fd"]);
        assert_parity_in(d.path(), "print -l f[abc]");
    }

    #[test]
    fn bracket_negation() {
        let d = mkdir_with_files(&["fa", "fb", "fc", "fd"]);
        assert_parity_in(d.path(), "print -l f[^ab]");
    }

    #[test]
    fn bracket_multiple_ranges() {
        let d = mkdir_with_files(&["aX", "bY", "zZ", "1Q", "9P"]);
        assert_parity_in(d.path(), "print -l [a-z]?");
    }
}

mod nested_dirs {
    use super::*;

    #[test]
    fn glob_in_subdir() {
        let d = mkdir_with_files(&["sub/a.txt", "sub/b.txt", "sub/c.rs"]);
        assert_parity_in(d.path(), "print -l sub/*.txt");
    }

    /// Star at top level doesn't recurse into subdirs.
    #[test]
    fn star_does_not_recurse() {
        let d = mkdir_with_files(&["top1", "top2", "sub/inner1", "sub/inner2"]);
        assert_parity_in(d.path(), "print -l *");
    }
}

mod qualifiers {
    use super::*;

    /// `*(/)` — directories only.
    #[test]
    fn slash_qualifier_directories_only() {
        let d = mkdir_with_files(&["regular_file", "subdir/.placeholder"]);
        assert_parity_in(d.path(), "print -l *(/)");
    }

    /// `*(.)` — regular files only.
    #[test]
    fn dot_qualifier_regular_files_only() {
        let d = mkdir_with_files(&["regular_file", "subdir/.placeholder"]);
        assert_parity_in(d.path(), "print -l *(.)");
    }

    /// `*(N)` — null-glob (no error on no-match).
    #[test]
    fn N_qualifier_null_glob_no_match() {
        let d = mkdir_with_files(&["a.txt"]);
        // Without (N), `*.nonexistent` errors. With (N), produces no output, exits 0.
        assert_parity_in(d.path(), "print -l -- *.nonexistent_xyz(N)");
    }

    /// `*(.)` regular files combined with a glob — picks .txt files only.
    #[test]
    fn dot_qualifier_with_glob_pattern() {
        let d = mkdir_with_files(&["a.txt", "b.txt", "subdir/.placeholder"]);
        assert_parity_in(d.path(), "print -l *.txt(.)");
    }
}

mod no_match {
    use super::*;

    /// Without NULL_GLOB, no-match glob errors. zsh exits non-zero; zshrs
    /// may or may not — pin the contract and let the test flag divergence.
    #[test]
    fn unmatched_glob_errors_by_default() {
        let d = mkdir_with_files(&["only.txt"]);
        if !zsh_available() {
            return;
        }
        let z = run_zsh_in(d.path(), "echo *.nonexistent_xyz_42");
        let r = run_zshrs_in(d.path(), "echo *.nonexistent_xyz_42");
        assert_eq!(z.exit != 0, r.exit != 0, "exit-nonzero-ness must match");
    }
}

mod hidden_files {
    use super::*;

    /// `.*` matches hidden files explicitly.
    #[test]
    fn dot_star_matches_hidden_files() {
        let d = mkdir_with_files(&[".hidden_a", ".hidden_b", "visible"]);
        assert_parity_in(d.path(), "print -l .*");
    }
}

mod multibyte_text_is_not_glob {
    use super::*;

    /// UTF-8 continuation bytes that collide with token byte values
    /// (Hat = 0x86, Inang = 0x94, Star = 0x87 as u8) must not mark a
    /// word as a glob. Pre-fix, the dispatcher's pre-untokenize gate
    /// and pattern.rs::haswilds scanned BYTES, so `↔` (E2 86 94) and
    /// `⇇` (E2 87 87) fired "no matches found" from nested parameter
    /// substitutions — the zinit.zsh:251 `col-↔` load failure.
    #[test]
    fn nested_default_arm_with_u2194_arrow() {
        let d = mkdir_with_files(&["only.txt"]);
        assert_parity_in(d.path(), "echo ${${X}:-↔}");
    }

    #[test]
    fn nested_plus_arm_with_u2194_arrow() {
        let d = mkdir_with_files(&["only.txt"]);
        assert_parity_in(d.path(), "X=1; echo ${${X}:+↔}");
    }

    /// `⇇` carries 0x87 (Star as u8) twice — Star fires with no option
    /// gate, so this caught the bug even with extendedglob unset.
    #[test]
    fn nested_default_arm_with_u21c7_arrows() {
        let d = mkdir_with_files(&["only.txt"]);
        assert_parity_in(d.path(), "echo ${${X}:-⇇}");
    }

    /// Hat (0x86) is EXTENDEDGLOB-gated — pin the option-on path too.
    #[test]
    fn nested_default_arm_with_u2194_under_extendedglob() {
        let d = mkdir_with_files(&["only.txt"]);
        assert_parity_in(d.path(), "setopt extendedglob; echo ${${X}:-↔}");
    }

    /// The exact zinit.zsh:251 shape: nested `${(M)…:#…}` match flag
    /// feeding a `:+` arm whose value is a multibyte arrow.
    #[test]
    fn zinit_col_lr_shape() {
        let d = mkdir_with_files(&["only.txt"]);
        assert_parity_in(
            d.path(),
            "LANG=en_US.UTF-8; echo ${${${(M)LANG:#*UTF-8*}:+↔}:-fallback}",
        );
    }
}

/// Default/alternate-word filename generation (#2 default-word globbing).
/// The unquoted default/alt word in `${x:-W}` / `${x-W}` / `${x:+W}` /
/// `${x+W}` is SOURCE text, so a glob metachar in it drives filename
/// generation on the ASSEMBLED word — a parameter VALUE never globs.
/// c:Src/subst.c → globlist. The paramsubst arm sets a pending flag
/// (only for a source-glob default, via pretokenize_src_pat which skips
/// nested `$..` spans), and the compile-emitted DEFAULT_WORD_GLOB op
/// globs the assembled word; gated off in DQ / scalar-assign / assign-
/// builtin-arg contexts.
mod default_word_globbing {
    use super::*;

    fn files() -> tempfile::TempDir {
        mkdir_with_files(&["afile", "bfile"])
    }

    /// Unquoted source-glob default globs (`*`, `?`, `[...]`).
    #[test]
    fn unquoted_default_globs() {
        let d = files();
        assert_parity_in(d.path(), "print -l ${x:-*file}");
        assert_parity_in(d.path(), "print -l ${x-*file}");
        assert_parity_in(d.path(), "print -l ${x:-*fil?}");
        assert_parity_in(d.path(), "print -l ${x:-[ab]file}");
        assert_parity_in(d.path(), "print -l ${x:-a*}");
    }

    /// A default inside a CONCATENATED word must bracket on every
    /// `haswilds` character (c:Src/pattern.c:4315-4390), not only `*`/`?`/`[`:
    /// `(`/`|` alternation, `<…>`, and EXTENDEDGLOB `^` / `#` came out literal.
    #[test]
    fn concatenated_default_globs_on_every_haswilds_char() {
        let d = files();
        assert_parity_in(d.path(), "p=; print -l ${p}${x:-(a|b)file}");
        assert_parity_in(d.path(), "print -l a${x:-(f|x)ile}");
        assert_parity_in(d.path(), "print -l pre${x:-(a|b)file}; echo done");
        assert_parity_in(d.path(), "setopt extendedglob; p=; print -l ${p}${x:-^afile}");
        assert_parity_in(d.path(), "setopt extendedglob; print -l b${x:-#file}");
        assert_parity_in(d.path(), "a=set; p=; print -l ${p}${a:+(a|b)file}");
    }

    /// The alternate word (`:+`/`+`) globs when the var is set.
    #[test]
    fn alternate_word_globs() {
        let d = files();
        assert_parity_in(d.path(), "a=set; print -l ${a:+*file}");
        assert_parity_in(d.path(), "a=set; print -l ${a+*file}");
    }

    /// Glob runs on the ASSEMBLED word — prefix/suffix concatenate with
    /// the default before filename generation.
    #[test]
    fn assembled_word_globs() {
        let d = files();
        assert_parity_in(d.path(), "print -l ${x:-a*}bar; echo done");
        assert_parity_in(d.path(), "print -l pre${x:-*file}; echo done");
        assert_parity_in(d.path(), "print -l ${x:-*file}suf; echo done");
        assert_parity_in(d.path(), "p=pre; print -l ${p}${x:-*file}; echo done");
    }

    /// A parameter VALUE is never globbed (no GLOB_SUBST), even when the
    /// default word also carries a glob; quoted defaults stay literal.
    #[test]
    fn values_and_quoted_stay_literal() {
        let d = files();
        assert_parity_in(d.path(), "x='*file'; print -l $x");
        assert_parity_in(d.path(), "x='*file'; print -l ${x:-*other}");
        assert_parity_in(d.path(), "d='*file'; print -l ${x:-$d}");
        assert_parity_in(d.path(), "print -r -- \"${x:-*file}\"");
    }

    /// A `*` inside a NESTED strip/filter pattern within the default word
    /// is not a filename glob (`${x:-${p#a*}}`); pretokenize_src_pat skips
    /// the nested `${..}` span.
    #[test]
    fn nested_pattern_star_is_not_glob() {
        let d = files();
        assert_parity_in(d.path(), "p=abc; print -- ${x:-${p#a*}}");
        assert_parity_in(
            d.path(),
            "profile='user:custom'; print -- \"[${${${(M)profile:#*:*}:+${profile#*:}}:-default}]\"",
        );
    }

    /// Assignment contexts: scalar `v=` and assignment-builtin
    /// (typeset/export/...) NAME=value args do NOT glob; an array-literal
    /// element DOES glob; a regular command's `E=...` arg DOES glob.
    #[test]
    fn assignment_context_gating() {
        let d = files();
        assert_parity_in(d.path(), "v=${x:-*file}; print -l $v");
        assert_parity_in(d.path(), "typeset T=${x:-*file}; print $T");
        assert_parity_in(d.path(), "export E=${x:-*file}; print $E");
        assert_parity_in(d.path(), "a=(${x:-*file}); print -l $a");
        assert_parity_in(d.path(), "print -l ${x:=*file}"); // := assign-default never globs
    }

    /// No-match honours nomatch/nullglob; flag doesn't leak across words.
    #[test]
    fn no_match_and_no_leak() {
        let d = files();
        assert_parity_in(d.path(), "print -l ${x:-zzz*}; echo done");
        assert_parity_in(d.path(), "setopt nullglob; print -l ${x:-zzz*}; echo after");
        assert_parity_in(
            d.path(),
            "y=Y; print -l ${y:-*nomatch}; print -l ${x:-*file}",
        );
    }
}

/// c:Src/exec.c:2554-2567 — a scalar assignment value is expanded with
/// PREFORK_SINGLE and never globbed unless GLOB_ASSIGN is set. zshrs globbed
/// or pattern-parsed a value holding glob syntax its DQ-wrap did not list
/// (`(`, `|`, `#`) or starting with a quote: `x=q(` → "bad pattern: q(",
/// `x=q(a)` → "number expected", `x=q(|b)` → "no matches found".
mod scalar_assignment_never_globs {
    use super::*;

    #[test]
    fn a_scalar_value_with_glob_syntax_is_assigned_as_text() {
        let d = mkdir_with_files(&["qa", "qb", "ab", "aXb"]);
        for script in [
            r#"x=q(; print -r -- "[$x]" rc=$?"#,
            r#"x="q"(; print -r -- "[$x]" rc=$?"#,
            r#"x='q'(; print -r -- "[$x]" rc=$?"#,
            r#"x=q(a); print -r -- "[$x]" rc=$?"#,
            r#"x=q(|b); print -r -- "[$x]" rc=$?"#,
            r#"x=a(#i)b; print -r -- "[$x]" rc=$?"#,
            r#"x="q"(a); print -r -- "[$x]" rc=$?"#,
            // Controls: already literal, an ARRAY value globs, GLOB_ASSIGN globs.
            r#"x=q*; print -r -- "[$x]" rc=$?"#,
            r#"x=(q*); print -r -- "[$x]" rc=$?"#,
            r#"setopt globassign; x=q*; print -r -- "[$x]" rc=$?"#,
            r#"setopt globassign; x=q(; print -r -- "[$x]" rc=$?"#,
            r#"f(){ local x=q(a); print -r -- "[$x]" rc=$?; }; f"#,
        ] {
            assert_parity_in(d.path(), script);
        }
    }
}

/// c:Src/subst.c:4737 `chrealpath(&copy, 'A', 1)` — for mode 'A' chrealpath
/// runs chabspath FIRST (c:Src/hist.c:1988-1990), so `..` collapses lexically
/// before realpath(3) resolves a symlink. `:P` skips that step
/// (c:4787-4796) and still resolves the link before climbing.
mod modifier_a_collapses_dotdot_before_symlinks {
    use super::*;

    #[test]
    fn dotdot_after_a_symlink_is_lexical_for_a() {
        let d = mkdir_with_files(&[]);
        let setup = "mkdir -p T/dir3/subdir; touch T/hello; ln -s dir3/subdir T/link; cd T; ";
        assert_parity_in(d.path(), &format!("{setup}print -r -- ${{${{:-link/../hello}}:A}}"));
        assert_parity_in(d.path(), &format!("{setup}() {{ print -r -- ${{1:A}} }} link/../../hello"));
        assert_parity_in(d.path(), &format!("{setup}print -r -- ${{${{:-link/../hello}}:P}}"));
    }

    #[test]
    fn nonexistent_components_unchanged() {
        let d = mkdir_with_files(&[]);
        assert_parity_in(
            d.path(),
            "print -r -- ${${:-/a/b/../c}:A} ${${:-/a/b/../c}:P} ${${:-/a/./b}:A} ${${:-/a/./b}:P}",
        );
    }
}

/// c:Src/exec.c:3357-3359 prefork expands the whole argv (c:Src/subst.c:100-193)
/// and c:3755-3757 `globlist(args, 0)` globs it afterwards, stopping at the
/// first error (c:Src/subst.c:494). An `=cmd` failure in a LATER word is
/// therefore reported before an earlier word's glob runs. Redirect targets
/// (c:Src/glob.c:2161 xpandredir) and assignment values stay per word.
mod argv_prefork_before_glob {
    use super::*;

    fn run(script: &str) {
        let d = mkdir_with_files(&["a1", "a2"]);
        assert_parity_in(d.path(), &format!("{script} 2>&1; echo rc=$?"));
    }

    #[test]
    fn a_later_equals_failure_wins_over_an_earlier_glob() {
        run(r#"print 3 [[ a == *\\(* ]]"#);
        run("print [[ ==");
        run("print nomatch_zz* =nosuchcmd_zz");
    }

    /// c:Src/subst.c:165-191 — filesub (`=cmd`, `~user`) is prefork's second
    /// pass, after stringsubst has run over every word, so a later
    /// substitution runs before an earlier `=cmd` fails.
    #[test]
    fn equals_and_tilde_expand_after_every_substitution() {
        run("print =nosuchcmd_zz $(print ran >&2)");
        run("print ~nosuchuser_zz `print ran >&2`");
        run("f() { print x =nosuchcmd_zz $(print ran >&2); }; f");
        run("setopt nonomatch; print =nosuchcmd_zz $(print ran >&2)");
        run("print =ls $(print ran >&2) a{1,2} | cat");
        run("setopt noglob; print =ls ~/zz_no* a*");
    }

    #[test]
    fn orders_that_already_agreed() {
        run("print == [[");
        run("print a* =nosuchcmd_zz");
        run("print =nosuchcmd_zz a*");
        run("print [ =nosuchcmd_zz");
        run("print =ls a*");
        run("setopt nonomatch; print [[ ==");
        // c:Src/exec.c:3523-3524: a prefork error keeps a non-zero lastval,
        // and the deferred glob of `x(a)` never runs.
        run("(exit 5); print x(a) =nosuchcmd_zz");
    }
}

/// c:Src/glob.c:3633-3643 `zshtokenize` turns a backslash before a
/// metacharacter into `Bnull`, which is a token, so c:Src/pattern.c:605-610
/// sends that component through full compilation and c:650-664 hands the
/// scanner the UNQUOTED string. A backslash before any other character is
/// data and stays in the literal. The PAT_PURES fast path kept the
/// backslashes of `A\(B\)` and stat'ed a directory that does not exist, so
/// `_path_files` never descended into `A(B)/` (Y01completion #16).
mod escaped_metachar_in_a_literal_dir {
    use super::*;

    #[test]
    fn globsubst_descends_through_a_quoted_component() {
        let d = mkdir_with_files(&["A(B)/C/f", "A*B/C/f", "A[B/C/f", "Ab/C/f"]);
        for pat in [
            r"A\(B\)/*",
            r"A\*B/*",
            r"A\[B/*",
            r"A\b/*",
            r"A\(B\)/C/*",
        ] {
            assert_parity_in(
                d.path(),
                &format!("setopt nullglob; y='{pat}'; c=( $~y ); print -rl -- \"[$y]\" $c"),
            );
        }
    }
}
