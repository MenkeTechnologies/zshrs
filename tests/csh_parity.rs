//! `zshrs --csh` against the reference tcsh (macOS `/bin/csh`).
//!
//! Each script is written to a file and run by both shells with `-f` (no
//! startup files); stdout must match byte for byte. A script FILE, not
//! `-c`: tcsh's `-c` executes only the first line of a multi-line string.
//! Skipped where `/bin/tcsh` and `tcsh` are both absent.

use std::path::PathBuf;
use std::process::Command;

fn tcsh() -> Option<String> {
    ["/bin/tcsh", "/usr/bin/tcsh", "/opt/homebrew/bin/tcsh"]
        .iter()
        .find(|p| std::path::Path::new(p).exists())
        .map(|p| p.to_string())
}

fn run(bin: &str, extra: &[&str], file: &PathBuf) -> String {
    let out = Command::new(bin)
        .args(extra)
        .arg("-f")
        .arg(file)
        .output()
        .expect("spawn shell");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn csh_scripts_match_tcsh() {
    let Some(reference) = tcsh() else { return };
    let cases: &[(&str, &str)] = &[
        ("array count, subscript, modifiers", "set a = (1 2 3)\necho $#a $a[2]\nset f = /a/b/c.txt\necho $f:h $f:t $f:r $f:e\n"),
        ("setenv, isset, unsetenv", "echo ${?X}\nsetenv X 1\necho $X ${?X}\nunsetenv X\necho ${?X}\n"),
        ("foreach with continue", "foreach i (1 2 3)\nif ($i == 2) continue\necho $i\nend\n"),
        ("while with @ increment", "set i = 0\nwhile ($i < 3)\n@ i++\necho $i\nend\n"),
        ("if / else, && in expression", "set s = 10\nif ($s == 10 && 3 > 2) then\necho big\nelse\necho small\nendif\n"),
        ("switch with glob case and breaksw", "foreach x (a b c)\nswitch ($x)\ncase a:\necho first\nbreaksw\ncase [bc]:\necho later $x\nbreaksw\nendsw\nend\n"),
        ("one-line if ends at the list operator", "if (0) echo a && echo c\necho z\n"),
        ("alias with history args", "alias ll 'echo first \\!:1 last \\!$ all \\!*'\nll x y z\n"),
        ("backslash-newline is a blank", "echo a\\\nb\n"),
        ("$# is a count, not a comment", "set l = (a b c d)\necho $#l # trailing comment\n"),
    ];
    let dir = std::env::temp_dir().join(format!("zshrs-csh-parity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut bad = Vec::new();
    for (i, (name, script)) in cases.iter().enumerate() {
        let file = dir.join(format!("case{i}.csh"));
        std::fs::write(&file, script).unwrap();
        let want = run(&reference, &[], &file);
        let got = run(env!("CARGO_BIN_EXE_zshrs"), &["--csh"], &file);
        if want != got {
            bad.push(format!("{name}\n  tcsh : {want:?}\n  zshrs: {got:?}"));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(bad.is_empty(), "csh divergences:\n{}", bad.join("\n"));
}

/// Input read from stdin (what the prompt loop reads too) is csh text and
/// must be translated line by line, a `foreach` block held until `end`.
#[test]
fn csh_stdin_matches_tcsh() {
    use std::io::Write;
    use std::process::Stdio;
    let Some(reference) = tcsh() else { return };
let script = "set x = 5\necho $x $?x\nforeach i (a b)\necho $i\nend\nif ($x == 5) echo yes\nrepeat 2 echo r\nset n = 0\nagain:\n@ n++\nif ($n < 3) goto again\necho n=$n\nif (1) then\necho open-block\n";
    let run_stdin = |bin: &str, extra: &[&str]| {
        let mut child = Command::new(bin)
            .args(extra)
            .arg("-f")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn shell");
        child.stdin.take().unwrap().write_all(script.as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let want = run_stdin(&reference, &[]);
    let got = run_stdin(env!("CARGO_BIN_EXE_zshrs"), &["--csh"]);
    assert_eq!(got, want, "stdin csh input diverges from tcsh");
}

/// stdout and exit status of a script FILE, for the cases below where the
/// interesting part is where the script ends.
fn run_status(bin: &str, extra: &[&str], file: &PathBuf) -> (String, i32) {
    let out = Command::new(bin)
        .args(extra)
        .arg("-f")
        .arg(file)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/tmp")
        .output()
        .expect("spawn shell");
    (String::from_utf8_lossy(&out.stdout).into_owned(), out.status.code().unwrap_or(-1))
}

/// Scripts whose behaviour was wrong before: an error on one line must not
/// swallow the output of the lines before it, here-documents are literal
/// text, `^`/`#` are plain characters, and the run-time errors end the
/// script at the same place as tcsh.
#[test]
fn csh_error_and_literal_text_paths_match_tcsh() {
    let Some(reference) = tcsh() else { return };
    let cases: &[(&str, &str)] = &[
        ("error keeps earlier output", "echo one\necho two\n@ x = 1 +\necho never\n"),
        ("heredoc expands variables, keeps # and blank lines", "set v = val\ncat << EOF\nv is $v\n# not a comment\n\nlast\nEOF\necho after\n"),
        ("heredoc inside a block", "if (1) then\n  cat << EOF\ninside\nEOF\n  echo after\nendif\n"),
        ("heredoc terminator keeps its quotes", "cat << \"EOF\"\n$HOME\nEOF\n"),
        ("heredoc trailing backslash", "cat << EOF\na \\\nb\nEOF\n"),
        ("caret and hash are plain text", "echo ^a b#c\n"),
        ("subscript zero is empty", "set a = (1 2 3)\necho x$a[0]y\necho $a[2-4]\necho never\n"),
        ("subscript out of range ends the script", "set a = (1 2 3)\necho $a[4]\necho never\n"),
        ("undefined variable in if", "echo before\nif ($nope == 1) echo x\necho never\n"),
        ("undefined variable in foreach", "echo before\nforeach i ($nope)\nend\necho never\n"),
        ("root directory is a file name", "if (-d /) echo dir\nif (-e / && -r /) echo both\n"),
        ("exit with an expression", "echo before\nexit (2 + 3)\n"),
        ("substitution modifier with a blank", "set s = a:b:c\necho x$s:s/:/ /y\necho x$s:as/:/ /y\n"),
        ("nonomatch keeps the pattern", "set nonomatch\necho /nonexistent_zz*\necho done\n"),
        ("no match ends the script", "echo /nonexistent_zz*\necho never\n"),
        ("unclosed brace", "echo before\necho {a,b\necho never\n"),
        ("eval of a value with a pipe", "set c = 'echo a | cat'\neval $c\neval \"$c\"\n"),
        ("newline in double quotes", "echo \"a\\\nb\"\n"),
        ("foreach with no match", "echo before\nforeach i (/nonexistent_zz*)\necho $i\nend\necho never\n"),
        ("prompt is unset in a script", "echo $?prompt\n"),
        ("background job notice shape", "sleep 0 &\nwait\necho b\n"),
        ("hashstat needs a command hash", "hashstat\nrehash\nhashstat\nunhash\nhashstat\n"),
        ("alias of a builtin name is called", "alias echo 'echo [pre]'\necho hi\necho never\n"),
        ("backslash does not escape a double quote", "echo before\necho \"He said \\\"hi\\\"\"\necho never\n"),
        ("| and & after @ are operators", "set i = 1\n@ i |= 8\necho $i\n@ i ^= 5\necho $i\n"),
        ("echo of a variable holding a glob", "mkdir -p /tmp/_cp_g && cd /tmp/_cp_g && touch a1 a2\nset pat = 'a*'\necho $pat\ncd /tmp && rm -rf /tmp/_cp_g\n"),
        ("modifiers on a foreach variable", "foreach f (/a/b/c.txt /d/e.csh)\necho $f:t $f:h/x $f:r\nend\n"),
        ("variable value with a lone pipe", "set c = 'echo a | cat'\n/bin/echo $c\neval $c\n"),
        ("$shell is set", "echo $?shell\n"),
        ("repeat counts", "set n = 2\nrepeat $n echo x\nrepeat 0 echo never\n"),
        ("eval ends the script on an undefined variable", "echo before\neval \"echo $nope\"\necho never\n"),
        ("if with an unbalanced paren", "echo before\nif 1) echo x\necho never\n"),
        ("alias whose body is an if", "alias t 'if (1) echo yes'\nt\n"),
        ("alias ending in a redirect", "alias to 'echo hi >'\nto /tmp/_cp_ar\ncat /tmp/_cp_ar\nrm -f /tmp/_cp_ar\n"),
        ("exit inside a sourced file", "echo 'exit 3' > /tmp/_cp_se\nsource /tmp/_cp_se\necho after\nrm -f /tmp/_cp_se\n"),
        ("source of a missing file", "echo before\nsource /nonexistent_zz\necho never\n"),
        ("set status", "set status = 5\necho $status\ntrue\necho $status\n"),
        ("positional past the end", "set argv = (a b c)\necho $1 $2 $3 $4\n"),
        ("shlvl and tty exist", "echo $shlvl\necho $?tty\n"),
        ("if followed by a group", "if (1) (echo in-group)\n"),
        ("dot glob includes . and ..", "mkdir -p /tmp/_cp_dg && cd /tmp/_cp_dg && touch .h v\necho .*\ncd /tmp && rm -rf /tmp/_cp_dg\n"),
        ("set listing shows the script's variables", "set zz = (a b)\nset one = x\nset | grep -E '^(one|zz)'\n"),
        ("filetest", "filetest -e /tmp /nonexistent_zz\nfiletest -d /tmp\n"),
        ("readonly variable in a goto loop", "set -r MAX = 3\nset c = 0\nloop:\n@ c++\necho $c\nif ($c < $MAX) goto loop\n"),
    ];
    let dir = std::env::temp_dir().join(format!("zshrs-csh-status-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut bad = Vec::new();
    for (i, (name, script)) in cases.iter().enumerate() {
        let file = dir.join(format!("case{i}.csh"));
        std::fs::write(&file, script).unwrap();
        let want = run_status(&reference, &[], &file);
        let got = run_status(env!("CARGO_BIN_EXE_zshrs"), &["--csh"], &file);
        // `[N] pid` carries a process id
        let norm = |(out, rc): (String, i32)| {
            let out: Vec<String> = out
                .lines()
                .map(|l| if l.starts_with("[1] ") { "[1] PID".to_string() } else { l.to_string() })
                .collect();
            (out, rc)
        };
        let (want, got) = (norm(want), norm(got));
        if want != got {
            bad.push(format!("{name}\n  tcsh : {want:?}\n  zshrs: {got:?}"));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(bad.is_empty(), "csh divergences:\n{}", bad.join("\n"));
}

/// tcsh exits 1 when it reaches the script's last line — a block closer with
/// no newline after it — by SKIPPING forward to it: the rest of a chain after
/// a taken branch, the body of a false test with no `else`, the remainder of
/// a `switch` after `breaksw` or with no matching label, a `break` out of a
/// loop. Reached normally, or followed by a newline, the status is 0.
#[test]
fn csh_exit_status_after_skipping_to_an_unterminated_closer_matches_tcsh() {
    let Some(reference) = tcsh() else { return };
    let cases: &[(&str, &str)] = &[
        ("taken branch skips the else", "if (1) then\n  echo a\nelse\n  echo b\nendif"),
        ("else branch reached normally", "if (0) then\n  echo a\nelse\n  echo b\nendif"),
        ("false test, no else", "if (0) then\n  echo a\nendif"),
        ("true test, no else", "if (1) then\n  echo a\nendif"),
        ("else-if chain, first taken", "if (1) then\n echo a\nelse if (0) then\n echo b\nelse\n echo c\nendif"),
        ("else-if chain, second taken", "if (0) then\n echo a\nelse if (1) then\n echo b\nelse\n echo c\nendif"),
        ("else-if chain, last else", "if (0) then\n echo a\nelse if (0) then\n echo b\nelse\n echo c\nendif"),
        ("else-if chain, all false, no else", "if (0) then\n echo a\nelse if (0) then\n echo b\nendif"),
        ("closer followed by a newline", "if (1) then\n  echo a\nelse\n  echo b\nendif\n"),
        ("closer is not the last line", "if (1) then\n  echo a\nelse\n  echo b\nendif\necho z"),
        ("inner skip lands before the closer", "if (1) then\n if (1) then\n  echo a\n else\n  echo b\n endif\nendif"),
        ("outer skip over a nested block", "if (1) then\n echo a\nelse\n if (1) then\n  echo b\n endif\nendif"),
        ("breaksw skips to endsw", "switch (a)\ncase a:\n echo a\n breaksw\ncase b:\n echo b\n breaksw\nendsw"),
        ("no label matches", "switch (z)\ncase a:\n echo a\n breaksw\nendsw"),
        ("default reached normally", "switch (z)\ncase a:\n echo a\n breaksw\ndefault:\n echo d\nendsw"),
        ("last case falls into endsw", "switch (a)\ncase a:\n echo a\nendsw"),
        ("break skips to end", "foreach i (1 2)\n echo $i\n break\nend"),
        ("loop runs out", "foreach i (1 2)\n echo $i\nend"),
        ("only the last top-level block counts", "if (1) then\n echo a\nelse\n echo b\nendif\nif (0) then\n echo c\nelse\n echo d\nendif"),
        ("an earlier block skips, the last is plain", "if (0) then\n echo a\nelse\n echo b\nendif\nif (1) then\n echo c\nendif"),
        ("closer with trailing blanks", "if (1) then\n echo a\nelse if (1) then\n echo b\nendif  "),
        ("status kept when nothing skipped", "if (0) then\n echo a\nelse\n false\nendif"),
    ];
    let dir = std::env::temp_dir().join(format!("zshrs-csh-eofskip-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut bad = Vec::new();
    for (i, (name, script)) in cases.iter().enumerate() {
        let file = dir.join(format!("case{i}.csh"));
        std::fs::write(&file, script).unwrap();
        let want = run_status(&reference, &[], &file);
        let got = run_status(env!("CARGO_BIN_EXE_zshrs"), &["--csh"], &file);
        if want != got {
            bad.push(format!("{name}\n  tcsh : {want:?}\n  zshrs: {got:?}"));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(bad.is_empty(), "csh divergences:\n{}", bad.join("\n"));
}
