//! Regressions for divergences found by replaying zsh's own D- and E-series
//! `.ztst` chunks (expansion, subscripts, options) against `zsh` 5.9.2 and
//! zshrs side by side. Each expected value is what `/usr/local/bin/zsh -fc`
//! printed for the same script.

use std::path::PathBuf;
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_zshrs"))
}

/// Run `zshrs --zsh -f -c script` → (exit code, stdout, stderr).
fn run(script: &str) -> (i32, String, String) {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .env_remove("ZDOTDIR")
        .output()
        .expect("spawn zshrs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// c:Src/lex.c:1229-1247 — a command-position word whose text before `=` is
/// only a `[...]` subscript (or only `+`) is still ENVSTRING; the assignment
/// then fails isident in assignsparam (c:Src/params.c:3203-3204), which is a
/// fatal error that aborts the rest of the line. zshrs used to lex the word
/// as a glob and report `no matches found`. D04parameter.ztst chunk 193.
#[test]
fn subscript_only_lhs_is_not_an_identifier() {
    for (script, msg) in [
        ("[$key]=$val; echo after", "zsh:1: not an identifier: [$key]\n"),
        ("[a]=b echo hi", "zsh:1: not an identifier: [a]\n"),
        ("[a]+=x", "zsh:1: not an identifier: [a]\n"),
        ("[]=x", "zsh:1: not an identifier: []\n"),
        ("true; +=x; echo after", "zsh:1: not an identifier: \n"),
    ] {
        let (_ec, out, err) = run(script);
        assert_eq!(out, "", "{script:?}: the error must abort the line");
        assert_eq!(err.replacen("zshrs:", "zsh:", 1), msg, "{script:?}");
    }
    // Real identifiers with subscripts are untouched.
    let (_ec, out, _err) = run("a[2]=x; typeset -A h; h[k]=v; echo $a[2] $h[k]");
    assert_eq!(out, "x v\n");
    // As an argument (not command position) the word is still a glob.
    let (_ec, _out, err) = run("echo [a]=b");
    assert!(err.contains("no matches found: [a]=b"), "{err}");
}

/// c:Src/pattern.c:4179 `zpc_disables[]` lives in the forked child, so a
/// `disable -p` / `enable -p` inside `( … )` or `$( … )` never reaches the
/// parent. zshrs runs subshells in-process and did not roll the array back,
/// so D02glob.ztst's `disable -p '*'` chunk broke every later glob.
#[test]
fn pattern_disables_do_not_leak_out_of_subshells() {
    for (script, want) in [
        ("( disable -p '*' ); disable -p; echo end", "end\n"),
        ("x=$(disable -p '('); disable -p; echo end", "end\n"),
        ("( disable -p '*'; disable -p ); disable -p; echo end", "'*'\nend\n"),
        ("disable -p '*'; ( enable -p '*' ); disable -p", "'*'\n"),
    ] {
        let (_ec, out, _err) = run(script);
        assert_eq!(out, want, "{script:?}");
    }
}

/// c:Src/glob.c:1230-1233 — zglob returns before globbing when EXECOPT is
/// unset, so under NO_EXEC a glob in the COMMAND word never reports
/// NOMATCH. zshrs gated the argument words but not the command word
/// (E01options.ztst: `setopt noexec; typeset -A hash; hash['…']`).
#[test]
fn noexec_never_globs_the_command_word() {
    for script in [
        "setopt noexec\nx[a] foo",
        "setopt noexec\n/nonexistent-dir-zshrs/*",
        "(setopt noexec\ntypeset -A hash\nhash['this is a string'])",
    ] {
        let (ec, out, err) = run(script);
        assert_eq!((ec, out.as_str(), err.as_str()), (0, "", ""), "{script:?}");
    }
    // With EXECOPT on the command word still globs and fails.
    let (_ec, _out, err) = run("/nonexistent-dir-zshrs/*");
    assert!(err.contains("no matches found"), "{err}");
}

/// c:Src/exec.c:5384-5388 — a function defined inside `eval` records
/// `funcstack->flineno + lineno`, the FILE line of its definition, which
/// `$funcsourcetrace` and `%I` (c:Src/prompt.c:901-919) then report. zshrs
/// stored the eval-relative line (E02xtrace.ztst `functions -T` chunk).
#[test]
fn eval_defined_function_records_file_line() {
    let dir = std::env::temp_dir().join(format!("zshrs-evalline-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let script = dir.join("s.zsh");
    std::fs::write(
        &script,
        ":\n:\n:\n\
         eval ':\n\
         h() { print -r -- \"h ${funcsourcetrace[1]#*:}\"; }\n\
         h\n\
         k() {\n\
         \x20 print -rP -- \"k ${funcsourcetrace[1]#*:} %I %i\"\n\
         }\n\
         k'\n",
    )
    .unwrap();
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-f"])
        .arg(&script)
        .env_remove("ZSHRS_CACHE")
        .env_remove("ZDOTDIR")
        .output()
        .expect("spawn zshrs");
    let _ = std::fs::remove_dir_all(&dir);
    // eval runs at file line 4 (flineno 4); h is eval line 2, k eval line 4.
    // Output verified against zsh 5.9.2 on the same script.
    assert_eq!(String::from_utf8_lossy(&out.stdout), "h 6\nk 8 9 1\n");
}

/// c:Src/exec.c:1471-1473 / 2056 — every pipeline sets `lineno` from its
/// WC_PIPE_LINENO, which inside a function body is function-relative.
/// zshrs compiled pipeline stages, `&` jobs, `time`, `select` and `coproc`
/// into sub-chunks that dropped the function's line context, so `$LINENO`
/// there was the absolute file line.
#[test]
fn lineno_in_function_sub_chunks_is_function_relative() {
    let script = "f() {\n\
                  \x20 :\n\
                  \x20 { print c $LINENO } | cat\n\
                  \x20 ( print d $LINENO ) | cat\n\
                  \x20 { print j $LINENO } &\n\
                  \x20 wait\n\
                  \x20 select x in a; do print s $LINENO; break; done <<<1 2>/dev/null\n\
                  \x20 { print m $LINENO\n\
                  \x20   print n $LINENO } | cat\n\
                  }\n\
                  f";
    let (_ec, out, _err) = run(script);
    // Verified against zsh 5.9.2 on the same script.
    assert_eq!(out, "c 2\nd 3\nj 4\ns 6\nm 7\nn 8\n");
}

/// c:Src/subst.c:3811-3819 — a style above plain `(q)` untokenizes the value,
/// so the Star in a `${(qq):-b*}` default word is plain text once quoted and
/// must not be filename-generated. The default-word glob flag was set from
/// the SOURCE word regardless of the quote flag, so a literal suffix next to
/// the expansion made the quoted `'b*',` a pattern ("no matches found").
/// D04parameter.ztst chunk 178; plain `(q)` keeps its tokens and still globs.
#[test]
fn quoted_default_word_with_a_glob_char_is_not_globbed() {
    let (_ec, out, err) = run("print -r - ${(qq):-b*}, ${(qqq):-a= {b,c}*}, x${(qqqq):-b?}");
    assert_eq!(err, "");
    assert_eq!(out, "'b*', \"a= b*\" \"a= c*\", x$'b?'\n");
    let (_ec, out, err) = run("print -r - ${(q):-b*},");
    assert_eq!(out, "");
    assert!(err.contains("no matches found"), "{err:?}");
}

/// c:Src/exec.c — redirections after an anonymous function's body belong to
/// the command, so their targets expand in the CALLER's scope. zshrs wrapped
/// them around the body, where `$1` was the anonymous function's own (empty)
/// positional, and the output went to the terminal instead of the file.
#[test]
fn anonymous_function_redirect_target_expands_in_the_callers_scope() {
    let dir = std::env::temp_dir().join(format!("zshrs-anon-redir-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("out.txt");
    let script = format!("set -- {}; () {{ print \"in:$1\" }} a > $1", file.display());
    let (_ec, out, _err) = run(&script);
    assert_eq!(out, "");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "in:a\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// c:Src/builtin.c:5738 — `if (stopmsg || (zexit(0, ZEXIT_DEFERRED), !stopmsg))`.
/// `jobs` leaves stopmsg at 2 (c:2486), and zshrs tested `stopmsg == 0` first,
/// so `exit` inside a function body then did nothing and `(exit 3)` in a
/// function returned 0. A05execution.ztst chunk 37.
#[test]
fn exit_in_a_function_still_defers_after_jobs_set_stopmsg() {
    let (_ec, out, _err) = run("jobs; g() { (exit 3); print $?; }; g");
    assert_eq!(out, "3\n");
    let (ec, out, _err) = run("jobs; g() { exit 4 }; g; print not-reached");
    assert_eq!((ec, out.as_str()), (4, ""));
}

/// Run as a drop-in `zsh` (argv[0] = zsh), which selects 5.9.2 behaviour where
/// the development tree differs. Expected values are what Homebrew zsh 5.9.2
/// printed for the same script.
fn run_as_zsh(script: &str) -> String {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "zshrs-asz-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let link = dir.join("zsh");
    std::os::unix::fs::symlink(zshrs_bin(), &link).unwrap();
    let out = Command::new(&link)
        .args(["-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .env_remove("ZDOTDIR")
        .output()
        .expect("spawn zsh");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// c:3906 vs c:3937 — `(j:SEP:)` joins before the case fold, so with an empty
/// separator `C` capitalises across the former element boundary.
#[test]
fn join_precedes_case_modification() {
    let (_ec, out, _err) = run("a=('x y' 'z w'); print -r -- ${(j::C)a} ${(j:a:C)a} ${(C)a}");
    assert_eq!(out, "X Yz W X Yaz W X Y Z W\n");
}

/// 5.9.2 c:1481-1486 — an out-of-range word index is clamped into
/// [1, wordcount] (the development tree returns nothing, workers/54773).
#[test]
fn word_subscript_clamps_out_of_range_on_592() {
    let out = run_as_zsh("ws='a b c d'; print -r -- \"[${ws[(w)5]}][${ws[(w)-9]}][${ws[(w)0]}]\"");
    assert_eq!(out, "[d][a][a]\n");
}

/// 5.9.2 pcre.c:322-329 / c:408 — the capture count handed to
/// zpcre_get_substrings is pcre2_match's return value, so a trailing group
/// that did not participate is dropped rather than reported empty.
#[test]
fn pcre_trailing_unset_group_is_dropped_on_592() {
    let out = run_as_zsh(
        "zmodload zsh/pcre; pcre_compile 'x(y)?z'; pcre_match -a arr xz; print -r -- n=${#arr}",
    );
    assert_eq!(out, "n=0\n");
}

/// 5.9.2 c:2043 — usepm has no PM_DECLARED term, so re-declaring a
/// TYPESET_TO_UNSET parameter with the option off creates it empty.
#[test]
fn redeclared_unset_param_becomes_empty_on_592() {
    let out = run_as_zsh(
        "f() { setopt typesettounset; typeset u; unsetopt typesettounset; typeset u; print \"[${+u}]\" }; f",
    );
    assert_eq!(out, "[1]\n");
}

/// 5.9.2 exectry (c:Src/loop.c:716-805) never sets `this_noerrexit`, so the
/// ERR trap fires once more for the construct's own non-zero status.
#[test]
fn err_trap_fires_for_the_always_construct_on_592() {
    let out = run_as_zsh(
        "trap 'print -r -- \"err rc=$?\"' ERR; f() { { false } always { print -r -- always }; print -r -- after }; f",
    );
    assert_eq!(out, "err rc=1\nalways\nerr rc=1\nafter\n");
}

/// 5.9.2 c:Src/exec.c:1645-1649 — the `-c` string runs with `exiting`, so its
/// top-level execlist calls dotrap(SIGEXIT) while errflag is set (dotrapargs
/// returns at once) and then clears the trap. A drop-in `zsh` therefore loses
/// the EXIT trap when the string ended on an error abort; the development
/// tree clears errflag around that call and runs it (0c33d1694c).
#[test]
fn a_drop_in_zsh_loses_the_exit_trap_after_an_error_abort_in_dash_c() {
    use std::os::unix::process::CommandExt;
    let script = "trap 'print p' EXIT; typeset -r ro=1; ro=2; print reached";
    let out = Command::new(zshrs_bin())
        .arg0("zsh")
        .args(["-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("spawn zshrs");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    let (_ec, dev, _err) = run(script);
    assert_eq!(dev, "p\n");
}

/// zsh 5.9.2's `%f %k %b %u %s` record the attribute as changed whether or not
/// it was on (Src/prompt.c:551-600 at the zsh-5.9.2 tag), and its output never
/// takes the clear-everything shortcut for a colour or underline. A drop-in
/// `zsh` therefore repeats a reset and ends colours with `\e[39m` / `\e[49m`
/// and underline with `\e[24m` even after `%B…%b`.
#[test]
fn a_drop_in_zsh_prompt_repeats_resets_and_ends_attributes_individually() {
    use std::os::unix::process::CommandExt;
    let cases = [
        ("%F{red}a%f%f", "\x1b[31ma\x1b[39m\x1b[39m"),
        ("%B%b%b", "\x1b[1m\x1b[0m\x1b[0m"),
        ("%F{300}o%f", "\x1b[39mo\x1b[39m"),
        ("%B%b%F{red}x%f", "\x1b[1m\x1b[0m\x1b[31mx\x1b[39m"),
        ("%B b%b|%U u%u", "\x1b[1m b\x1b[0m|\x1b[4m u\x1b[24m"),
    ];
    for (fmt, want) in cases {
        let out = Command::new(zshrs_bin())
            .arg0("zsh")
            .args(["-f", "-c", &format!("print -Pn -- '{fmt}'")])
            .env_remove("ZSHRS_CACHE")
            .env("TERM", "xterm-256color")
            .output()
            .expect("spawn zshrs");
        assert_eq!(String::from_utf8_lossy(&out.stdout), want, "{fmt}");
    }
}

/// c:Src/subst.c:3777-3880 — positive `(q)` quoting runs before the `mods` stage,
/// whose `(D)` is `substnamedir`: it backslash-quotes every string it returns. So
/// `${(Dq)empty}` is the quoted `''` quoted again (`\'\'`); zshrs contracted first
/// and quoted after, which only agrees when the value is non-empty.
#[test]
fn quote_then_namedir_quotes_the_quoted_form() {
    let (_ec, out, err) = run(
        "empty=; x='a b*'; print -r -- \"[${(Dq)empty}][${(qD)empty}][${(Dq)x}][${(Dqq)x}][${(D)empty}]\"",
    );
    assert_eq!(err, "");
    assert_eq!(out, "[\\'\\'][\\'\\'][a\\\\\\ b\\\\\\*][\\'a\\ b\\*\\'][]\n");
}

/// 5.9.2 c:2043 — a TYPESET_TO_UNSET declaration is not `usepm`, so a later
/// `typeset -U h` builds a new parameter that has only the `-U` attribute; the
/// array type of the earlier `typeset -a h` is gone.
#[test]
fn typeset_on_a_declared_unset_array_drops_its_type_on_592() {
    let out = run_as_zsh("setopt typesettounset; typeset -a h; typeset -U h; typeset -p h");
    assert_eq!(out, "typeset -U h\n");
}

/// c:Src/subst.c:3033 — a double-quoted `[*]` slice joins on IFS[0]:
/// `IFS=-; x=(a b c); "${x[*]:0:2}"` is `a-b`, while `"${x[@]:0:2}"` keeps two words.
#[test]
fn quoted_star_slice_joins_on_ifs() {
    let (_ec, out, _err) = run("x=(a b c); IFS=-; print \"${x[*]:0:2}\"; print -l \"${x[@]:0:2}\"; set -- p q r; print \"${*:1}\"");
    assert_eq!(out, "a-b\na\nb\np-q-r\n");
}

/// A cached compiled pattern skips the compile that leaves its flags in the
/// global `patglobflags`, and the matcher gates `$MATCH` on that global (c:2526).
/// After a `case` pattern compiled in between, the second pass over the same
/// `(#m)` cond no longer set `$MATCH` (the stock `zmv '(#m)…'` renamed every
/// file to the first one's name).
#[test]
fn cached_match_pattern_still_sets_match_after_another_pattern_compiled() {
    let (_ec, out, err) = run(
        "setopt extendedglob; for x in f1 f2; do [[ $x = (#m)f* ]]; case q in a) ;; esac; print -r -- \"$x [$MATCH]\"; done",
    );
    assert_eq!(err, "");
    assert_eq!(out, "f1 [f1]\nf2 [f2]\n");
}

/// zsh/files' `mv`, `ln`, `chmod`, … and zsh/stat's `zstat` live in `builtintab`
/// only once their module is loaded. A command name that is only known at run
/// time (`$cmd`, an array head — how the stock `zmv` runs `$action`) used the
/// static table regardless, so `$cmd -v` hit the module's option parser
/// ("bad option: -v") instead of /bin/mv, and `$cmd` for `zstat` ran without
/// `zmodload`.
#[test]
fn run_time_command_names_ignore_builtins_of_unloaded_modules() {
    let dir = std::env::temp_dir().join(format!("zshrs-dyn-mv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.txt"), "x").unwrap();
    let script = format!(
        "cd {}; cmd=mv; $cmd -v a.txt b.txt >/dev/null; print $?; cmd=zstat; $cmd +size b.txt 2>&1; print $?",
        dir.display()
    );
    let (_ec, out, _err) = run(&script);
    assert!(dir.join("b.txt").exists(), "{out:?}");
    assert!(out.starts_with("0\n"), "{out:?}");
    assert!(out.contains("command not found: zstat") && out.ends_with("127\n"), "{out:?}");
    std::fs::remove_dir_all(&dir).ok();
}

/// `functions -t NAME` on a not-yet-loaded autoload stub: C loads the body in
/// place (c:5751 clears only PM_UNDEFINED), so the trace tag survives the load.
/// zshrs re-registered the body on a fresh node and dropped it, so the function
/// ran silently from its first call.
#[test]
fn trace_tag_survives_autoload() {
    let dir = std::env::temp_dir().join(format!("zshrs-tagfn-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("tf"), "print in-body\n").unwrap();
    let script = format!(
        "PS4='+> '; fpath=({} $fpath); autoload -U tf; functions -t tf; tf 2>&1; tf 2>&1",
        dir.display()
    );
    let (_ec, out, _err) = run(&script);
    assert_eq!(out, "+> print in-body\nin-body\n+> print in-body\nin-body\n");
    std::fs::remove_dir_all(&dir).ok();
}

/// c:Src/options.c:555-570 — inside a traced (`functions -t`) function,
/// `emulate -R` keeps XTRACE on instead of resetting it with the other options.
#[test]
fn emulate_reset_keeps_xtrace_in_a_traced_function() {
    let (_ec, out, _err) = run(
        "PS4='+> '; f() { emulate -RL zsh; print a }; functions -t f; f 2>&1",
    );
    assert_eq!(out, "+> emulate -RL zsh\n+> print a\na\n");
}

/// c:5419 passes the format fragment to zwarnnam as a `%s`, which zerrmsg prints
/// through nicezputs: the newline the format escape pass made shows as `\n`.
#[test]
fn printf_invalid_directive_message_shows_control_chars_visibly() {
    let (_ec, out, _err) = run("printf '%\\n' 2>&1");
    assert_eq!(out, "zsh:printf:1: %\\n: invalid directive\n");
}

/// c:1717 `while (tok == SEPER) zshlex();` — `function` may be followed by
/// separators before the brace, which makes it an anonymous function.
#[test]
fn function_keyword_separator_then_brace_is_anonymous() {
    let (_ec, out, err) = run("function; { print a }; function\n{ print b }");
    assert_eq!((out.as_str(), err.as_str()), ("a\nb\n", ""));
}

/// c:2113 — a bodyless `()` is a parse error naming the `()` token, with no
/// line number (`lineno = 0` is still set from c:2072).
#[test]
fn bodyless_anonymous_function_is_a_parse_error() {
    let (ec, out, err) = run("print a; ()");
    assert_eq!((ec, out.as_str()), (1, ""));
    assert_eq!(err, "zsh: parse error near `()'\n");
}
