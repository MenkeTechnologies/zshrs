//! Gaps found by differential probing against the reference zsh:
//!
//! * a pipeline stage whose stdout is the pipe and whose redirect list dups
//!   that stream onto another fd (`cmd >/dev/null 2>&1 | cat`, `cmd >&2 2>&1 |
//!   cat`) hung forever: the MULTIOS splitter was joined at scope end while
//!   the dup'd fd still held its write end. Every case runs under a deadline so
//!   a regression fails instead of wedging the suite.
//! * `paramsubst`'s post-name gate (c:Src/subst.c:2994-3004) was skipped for a
//!   blank where the name belongs (`${ }`, `${|}`), for a nested substitution
//!   glued to identifier characters (`${${a}b}`), and a `(l:…:)` / `(r:…:)`
//!   width that is not a valid math expression was silently read as 0.
//! * `${(QD)v}` contracted the directory name before unquoting; C unquotes
//!   first (c:4041) and `substnamedir` then backslash-quotes (c:4149-4167).

use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

struct Run {
    stdout: String,
    stderr: String,
    exit: Option<i32>,
}

/// Run `bin args… -f -c script` with a deadline; `exit` is `None` on timeout.
fn run(bin: &str, args: &[&str], script: &str) -> Run {
    let mut child = Command::new(bin)
        .args(args)
        .args(["-f", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn shell");
    let mut out = child.stdout.take().unwrap();
    let mut err = child.stderr.take().unwrap();
    let out_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = out.read_to_string(&mut s);
        s
    });
    let err_t = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = err.read_to_string(&mut s);
        s
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    let exit = loop {
        match child.try_wait().expect("try_wait") {
            Some(st) => break Some(st.code().unwrap_or(-1)),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    Run {
        stdout: out_t.join().unwrap(),
        stderr: err_t.join().unwrap(),
        exit,
    }
}

/// Drop the leading `zsh:` / `zshrs:` shell-name tag from every line.
fn untag(s: &str) -> String {
    s.lines()
        .map(|l| {
            l.strip_prefix("zshrs:")
                .or_else(|| l.strip_prefix("zsh:"))
                .unwrap_or(l)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Assert stdout, exit status and (tag-stripped) stderr agree with the reference.
fn assert_parity(script: &str) {
    let reference = crate::oracle::zsh_path();
    if !std::path::Path::new(reference).exists() {
        return;
    }
    let z = run(reference, &[], script);
    let r = run(zshrs_bin().to_str().unwrap(), &["--zsh"], script);
    assert!(r.exit.is_some(), "zshrs timed out on:\n{script}");
    assert_eq!(z.stdout, r.stdout, "stdout divergence on:\n{script}");
    assert_eq!(z.exit, r.exit, "exit divergence on:\n{script}");
    assert_eq!(
        untag(&z.stderr),
        untag(&r.stderr),
        "stderr divergence on:\n{script}"
    );
}

#[test]
fn stage_dup_of_pipe_onto_stderr_after_null_redirect_terminates() {
    assert_parity("echo e >/dev/null 2>&1 | cat; print done");
}

#[test]
fn stage_dup_stdout_to_stderr_then_stderr_to_stdout_terminates() {
    assert_parity("echo e >&2 2>&1 | cat; print done");
}

#[test]
fn stage_dup_through_third_fd_terminates() {
    assert_parity("echo e >&2 3>&1 | cat; print done");
}

#[test]
fn stage_dup_after_file_redirect_keeps_both_copies() {
    assert_parity("f=${TMPDIR:-/tmp}/zshrs-dup-$$; echo e >$f 2>&1 | cat; cat $f; rm -f $f");
}

#[test]
fn external_stage_dup_stderr_onto_pipe_terminates() {
    assert_parity("/bin/echo e >/dev/null 2>&1 | cat; print done");
}

#[test]
fn blank_where_name_belongs_is_bad_substitution() {
    assert_parity("echo ${ }");
}

#[test]
fn blank_name_inside_double_quotes_is_bad_substitution() {
    assert_parity("echo \"${ }\"");
}

#[test]
fn blank_name_in_assignment_is_bad_substitution() {
    assert_parity("x=${ }; echo \"[$x]\"");
}

#[test]
fn tab_where_name_belongs_is_bad_substitution() {
    assert_parity("echo ${\t}");
}

#[test]
fn bare_bar_without_command_is_bad_substitution() {
    assert_parity("echo ${|}");
}

#[test]
fn nested_subst_glued_to_identifier_is_bad_substitution() {
    assert_parity("a=x; echo ${${a}b}");
}

#[test]
fn nested_subst_followed_by_blank_is_bad_substitution() {
    assert_parity("a=x; echo ${${a} }");
}

#[test]
fn nested_subst_with_valid_postmodifiers_still_expands() {
    assert_parity(
        "a=x; echo ${${a}-b} ${${a}:-b} ${${a}[1]} ${${a}#x} ${${a}/x/y} ${#${a}} ${(U)${a}} ${${a}}",
    );
}

#[test]
fn padding_width_with_trailing_garbage_is_bad_math_expression() {
    assert_parity("a=xyz; echo before ${(l:3x:)a} after; echo next");
}

#[test]
fn padding_width_missing_operand_is_bad_math_expression() {
    assert_parity("a=xyz; echo ${(l:1+:)a}");
}

#[test]
fn right_padding_width_with_two_operands_is_bad_math_expression() {
    assert_parity("a=xyz; echo ${(r:3 4:)a}");
}

#[test]
fn padding_width_arithmetic_and_unset_name_still_pad() {
    assert_parity("a=xyz; n=2; echo \"${(l:n+3::-:)a}|${(r:x:)a}|${(l:$#a+1::*:)a}\"");
}

#[test]
fn unquote_then_dir_contraction_quotes_the_result() {
    assert_parity("s='  x y  '; print -r -- \"[${(QD)s}]\"");
}

#[test]
fn unquote_dir_and_word_flags_combine_in_c_order() {
    assert_parity("s='  x y  '; print -r -- \"[${(QDw)s}]\"; a=('a b' c); print -r -- ${(QD)a}");
}

#[test]
fn dir_contraction_alone_still_quotes() {
    assert_parity("s='  x y  '; print -r -- \"[${(D)s}]\"");
}
