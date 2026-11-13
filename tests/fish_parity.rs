//! `zshrs --fish` against the reference fish.
//!
//! Every `tests/data/fish_parity/*.fish` script is run by both shells with startup
//! files off (`fish --no-config`, `zshrs --fish -f`) in an empty scratch directory
//! under an empty `$HOME`; stdout and the exit status must match. Scripts are
//! measured on fish first and left out when fish itself rejects them. Skipped where
//! `fish` is not installed.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn fish() -> Option<PathBuf> {
    ["/opt/homebrew/bin/fish", "/usr/local/bin/fish", "/usr/bin/fish", "/bin/fish"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

struct Run {
    stdout: String,
    status: Option<i32>,
}

fn run(bin: &Path, args: &[&str], script: &Path, home: &Path, cwd: &Path) -> Run {
    let out = Command::new(bin)
        .args(args)
        .arg(script)
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .expect("spawn shell");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        status: out.status.code(),
    }
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("zshrs-fish-parity-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn fish_scripts_match_fish() {
    let Some(reference) = fish() else { return };
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/fish_parity");
    let mut scripts: Vec<PathBuf> = std::fs::read_dir(&corpus)
        .expect("corpus directory")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "fish"))
        .collect();
    scripts.sort();
    assert!(!scripts.is_empty(), "no fish scripts in {}", corpus.display());

    let home = scratch("home");
    let cwd = scratch("cwd");
    let mut bad = Vec::new();
    for script in &scripts {
        let want = run(&reference, &["--no-config"], script, &home, &cwd);
        let got = run(Path::new(env!("CARGO_BIN_EXE_zshrs")), &["--fish", "-f"], script, &home, &cwd);
        if want.stdout != got.stdout || want.status != got.status {
            bad.push(format!(
                "{}\n  fish : {:?} {:?}\n  zshrs: {:?} {:?}",
                script.file_name().unwrap().to_string_lossy(),
                want.status,
                want.stdout,
                got.status,
                got.stdout
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&cwd);
    assert!(bad.is_empty(), "fish divergences:\n{}", bad.join("\n"));
}

/// Startup files: `conf.d/*.fish` in name order, then `config.fish`, for a `-c` string.
#[test]
fn startup_files_are_read_in_fish_order() {
    let home = scratch("rc");
    let dir = home.join(".config/fish");
    std::fs::create_dir_all(dir.join("conf.d")).unwrap();
    std::fs::write(dir.join("conf.d/b.fish"), "set -g ORDER $ORDER b\n").unwrap();
    std::fs::write(dir.join("conf.d/a.fish"), "set -g ORDER $ORDER a\n").unwrap();
    std::fs::write(
        dir.join("config.fish"),
        "set -g ORDER $ORDER config\nfunction from_config\n    echo $ORDER\nend\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_zshrs"))
        .args(["--fish", "-c", "from_config"])
        .env("HOME", &home)
        .env_remove("XDG_CONFIG_HOME")
        .stdin(Stdio::null())
        .output()
        .expect("spawn zshrs");
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "a b config\n");
}

/// A syntax error anywhere stops the script before its first command, status 127.
#[test]
fn a_syntax_error_runs_nothing() {
    let home = scratch("syntax");
    let out = Command::new(env!("CARGO_BIN_EXE_zshrs"))
        .args(["--fish", "-f", "-c", "echo before\nif true\necho x"])
        .env("HOME", &home)
        .stdin(Stdio::null())
        .output()
        .expect("spawn zshrs");
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert_eq!(out.status.code(), Some(127));
}

/// Script text on stdin is translated like a file, a block spanning several lines included.
#[test]
fn stdin_input_is_translated() {
    use std::io::Write;
    let home = scratch("stdin");
    let mut child = Command::new(env!("CARGO_BIN_EXE_zshrs"))
        .args(["--fish", "-f"])
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn zshrs");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"set x 1 2\nif test (count $x) -eq 2\n    echo two\nend\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "two\n");
}

/// Every function and completion script fish itself ships must parse and translate
/// (the files are read out of the installed fish with `status get-file`). A construct the
/// translator rejects here is a gap in the grammar, not in the script.
#[test]
fn fish_own_scripts_translate() {
    let Some(reference) = fish() else { return };
    let listing = Command::new(&reference)
        .args(["--no-config", "-c", "status list-files"])
        .output()
        .expect("spawn fish");
    let names: Vec<String> = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .filter(|l| (l.starts_with("functions/") || l.starts_with("completions/")) && l.ends_with(".fish"))
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        return; // a fish built without embedded files
    }
    let mut bad = Vec::new();
    let mut translated: Vec<(String, String)> = Vec::new();
    for name in &names {
        let out = Command::new(&reference)
            .args(["--no-config", "-c", &format!("status get-file {name}")])
            .output()
            .expect("spawn fish");
        let src = String::from_utf8_lossy(&out.stdout);
        match zsh::fish::translate(&src) {
            Ok(text) => translated.push((name.clone(), text)),
            Err(e) => bad.push(format!("{name}: {e}")),
        }
    }
    assert!(bad.is_empty(), "{} of {} fish scripts do not translate:\n{}", bad.len(), names.len(), bad.join("\n"));

    // ...and what comes out has to be zsh the parser accepts
    let dir = scratch("zsyntax");
    let files: Vec<(String, PathBuf)> = translated
        .iter()
        .enumerate()
        .map(|(i, (name, text))| {
            let f = dir.join(format!("{i}.zsh"));
            std::fs::write(&f, text).unwrap();
            (name.clone(), f)
        })
        .collect();
    let failures = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|sc| {
        for chunk in files.chunks(files.len().div_ceil(8).max(1)) {
            let failures = &failures;
            sc.spawn(move || {
                for (name, f) in chunk {
                    let out = Command::new(env!("CARGO_BIN_EXE_zshrs"))
                        .args(["--zsh", "-f", "-n"])
                        .arg(f)
                        .output()
                        .expect("spawn zshrs");
                    if !out.status.success() {
                        let why = String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("").to_string();
                        failures.lock().unwrap().push(format!("{name}: {why}"));
                    }
                }
            });
        }
    });
    let _ = std::fs::remove_dir_all(&dir);
    let failures = failures.into_inner().unwrap();
    assert!(failures.is_empty(), "{} translations are not valid zsh:\n{}", failures.len(), failures.join("\n"));
}

/// `complete` definitions answer `complete -C` as fish answers it: options with
/// descriptions, an option that takes the next word, `--opt=` arguments,
/// conditions that read `commandline`, `--wraps`, a definition autoloaded from
/// `~/.config/fish/completions/<cmd>.fish`, and fish's own scripts (string, set_color)
/// read out of the installed fish. Files: fuzzy paths, separators, wildcards,
/// a redirection's target, `.`/`..`, brace expansions and variables in a word.
#[test]
fn completions_match_fish() {
    let Some(reference) = fish() else { return };
    let home = scratch("complete");
    let cwd = scratch("complete-cwd");
    let comp_dir = home.join(".config/fish/completions");
    std::fs::create_dir_all(&comp_dir).unwrap();
    std::fs::create_dir_all(home.join("zqdir")).unwrap();
    std::fs::create_dir_all(cwd.join("zqsrc/zqext")).unwrap();
    std::fs::write(cwd.join("zqsrc/zqext/zqfish.rs"), "").unwrap();
    std::fs::write(cwd.join("zqfile"), "").unwrap();
    std::fs::write(comp_dir.join("zqauto.fish"), "complete -c zqauto -f -l from-file -d 'Autoloaded'\n").unwrap();
    // a command that is only an autoloadable function still gets its completions
    let fn_dir = home.join(".config/fish/functions");
    std::fs::create_dir_all(&fn_dir).unwrap();
    std::fs::write(fn_dir.join("zqfn.fish"), "function zqfn; end\n").unwrap();
    std::fs::write(comp_dir.join("zqfn.fish"), "complete -c zqfn -f -l from-fn-file\n").unwrap();
    let script = home.join("complete.fish");
    std::fs::write(
        &script,
        r#"function zq; end
function zqauto; end
function zw; end
complete -c zq -f -s v -l verbose -d 'Be loud'
complete -c zq -f -l color -xa 'auto never always'
complete -c zq -f -n 'test (count (commandline -xpc)) -eq 1' -a 'first second' -d 'Subcommand'
complete -c zq -f -n 'contains -- first (commandline -xpc)' -s q -l quiet -d 'Only after first'
complete -c zw -w zq
complete -C 'zq --'
complete -C 'zq --color '
complete -C 'zq --color=a'
complete -C 'zq '
complete -C 'zq first -'
complete -C 'zq -v'
complete -C 'zw f'
function zqw --wraps zq; end
complete -C 'zqw --c'
complete -c zqw
complete -C 'string re'
complete -C 'set_color br'
complete -c zsub -x -a '(__fish_complete_subcommand)'
complete -C 'zsub zq --v'
complete -C 'zsub zq --c'
complete -C 'which --'
complete -C 'not -'
abbr -a zqab echo x
complete -C 'zqa'
complete -C 'set_co'
complete -C 'echo $HOM'
complete -C 'echo foo$HOM'
complete -C 'echo ~roo'
complete -C 'ls $HOME/zqd'
complete -C 'ls zqs/x/zqf'
complete -C 'ls ZQS/E/'
complete -C 'cd zq'
complete -C 'ls --zqopt=zq'
complete -C 'ls a:zq'
complete -C 'ls zq*/'
complete -C 'ls ZQ*'
complete -C 'ls *ile'
complete -C 'echo >'
complete -C 'echo 2>&1 zqf'
complete -C 'ls .'
complete -C 'ls zqsrc/..'
complete -C 'ls {zqs,zqf}'
complete -C 'ls {zqs'
complete -C 'ls zqsrc/{zqe'
complete -C 'ls {x,zqs}/{zqe,y}'
complete -C 'ls "$HOME"/zqd'
complete -C 'ls "$HOME/zq"'
complete -C 'echo $__fish_sysconf'
complete -C 'functions __fish_print_hostn'
complete -C 'set -e __fish_sysc'
complete -C 'set LANG C'
complete -C 'echo "a b$HOM'
complete -C 'echo a\ $HOM'
set -g zqva 1
set -g zqVb 2
complete -C 'echo $zqv'
complete -C 'echo $__fish_ls'
complete -C 'echo $_'
function zqp; end
complete -c zqp -f -xa plain
complete -p '*zqp*' -f -xa wild
complete -C 'zqp '
"#,
    )
    .unwrap();
    let want = run(&reference, &["--no-config"], &script, &home, &cwd);
    let got = run(Path::new(env!("CARGO_BIN_EXE_zshrs")), &["--fish", "-f"], &script, &home, &cwd);
    // with configuration on, `<cmd>.fish` autoloads from the user's completions dir
    let auto = home.join("auto.fish");
    std::fs::write(&auto, "function zqauto; end\ncomplete -C 'zqauto --'\ncomplete -C 'zqauto --fr'\ncomplete -C 'zqfn --'\n").unwrap();
    let want_auto = run(&reference, &[], &auto, &home, &cwd);
    let got_auto = run(Path::new(env!("CARGO_BIN_EXE_zshrs")), &["--fish"], &auto, &home, &cwd);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&cwd);
    assert!(!want.stdout.is_empty(), "fish produced nothing");
    assert_eq!(got.stdout, want.stdout);
    assert_eq!(want_auto.stdout, "--from-file\tAutoloaded\n--from-file\tAutoloaded\n--from-fn-file\n");
    assert_eq!(got_auto.stdout, want_auto.stdout);
}

/// The fish preamble runs at startup, ahead of the first prompt line: the first
/// line no longer carries it through history expansion (its `!` produced
/// "no such word in event"), and the prompt is fish's from the start.
#[test]
fn preamble_is_installed_before_the_first_line() {
    use std::io::Write;
    let home = scratch("preamble");
    let mut child = Command::new(env!("CARGO_BIN_EXE_zshrs"))
        .args(["--fish", "-f", "-i"])
        .env("HOME", &home)
        .env("ZSHRS_HISTORY", "0")
        .env("ZSHRS_NATIVE_ZLE_FX", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn zshrs");
    child.stdin.take().unwrap().write_all(b"echo first\n").unwrap();
    let out = child.wait_with_output().unwrap();
    let _ = std::fs::remove_dir_all(&home);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(String::from_utf8_lossy(&out.stdout).contains("first"), "stdout: {:?}", out.stdout);
    assert!(!err.contains("event") && !err.contains("setopt"), "stderr: {err}");
}

/// `return` / `exit` inside a redirected group in `$( … )` leave its redirect
/// scope open; the substitution must still see end of file (it hung, holding a
/// copy of its own capture pipe). zsh: `[] 1`, `[] 3`.
#[test]
fn command_substitution_ends_when_the_body_returns_from_a_redirected_group() {
    for (body, want) in [
        ("{ return 1 } >/dev/null 2>&1", "[] 1\n"),
        ("{ exit 3 } >/dev/null; print x", "[] 3\n"),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_zshrs"))
            .args(["-f", "-c", &format!("x=$( {body} ); echo \"[$x] $?\"")])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn zshrs");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                panic!("`{body}` in $( ) never returned");
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let out = child.wait_with_output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), want, "{body}");
    }
}

/// fish's own functions, sourced from the files fish ships and run by both shells. They
/// exercise the translator on real code (dynamic variable names, `argparse` validators,
/// `{ }` blocks, `string` corner cases) rather than on scripts written for this suite.
/// `@FN@` in a script stands for the directory the needed function files are extracted to.
#[test]
fn fish_own_functions_behave_like_fish() {
    let Some(reference) = fish() else { return };
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/fish_functions");
    let functions = scratch("functions");
    let home = scratch("fn-home");
    let cwd = scratch("fn-cwd");
    let mut bad = Vec::new();
    let mut scripts: Vec<PathBuf> = std::fs::read_dir(&corpus)
        .expect("corpus directory")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "fish"))
        .collect();
    scripts.sort();
    for script in &scripts {
        let text = std::fs::read_to_string(script).unwrap();
        for line in text.lines() {
            let Some(rest) = line.strip_prefix("source @FN@/") else { continue };
            let file = rest.trim();
            let out = Command::new(&reference)
                .args(["--no-config", "-c", &format!("status get-file functions/{file}")])
                .output()
                .expect("spawn fish");
            if out.stdout.is_empty() {
                continue; // this fish does not embed that function
            }
            std::fs::write(functions.join(file), &out.stdout).unwrap();
        }
        let resolved = functions.join(format!("case-{}", script.file_name().unwrap().to_string_lossy()));
        std::fs::write(&resolved, text.replace("@FN@", &functions.to_string_lossy())).unwrap();
        let want = run(&reference, &["--no-config"], &resolved, &home, &cwd);
        let got = run(Path::new(env!("CARGO_BIN_EXE_zshrs")), &["--fish", "-f"], &resolved, &home, &cwd);
        if want.stdout != got.stdout || want.status != got.status {
            bad.push(format!(
                "{}\n  fish : {:?} {:?}\n  zshrs: {:?} {:?}",
                script.file_name().unwrap().to_string_lossy(),
                want.status,
                want.stdout,
                got.status,
                got.stdout
            ));
        }
    }
    for d in [&functions, &home, &cwd] {
        let _ = std::fs::remove_dir_all(d);
    }
    assert!(bad.is_empty(), "fish function divergences:\n{}", bad.join("\n"));
}

/// fish's builtins that zshrs runs as in-process native commands (`string`,
/// `math`, `count`, `contains`, `path`) classify as builtins, as in fish: they
/// sit in the builtin dispatch slot, so `whence`/`type` must say so.
#[test]
fn native_commands_are_builtins_to_type() {
    let Some(reference) = fish() else { return };
    let home = scratch("type");
    let script = home.join("type.fish");
    std::fs::write(&script, "type -t string math count contains path\ntype -q string; echo $status\n").unwrap();
    let want = run(&reference, &["--no-config"], &script, &home, &home);
    let got = run(Path::new(env!("CARGO_BIN_EXE_zshrs")), &["--fish", "-f"], &script, &home, &home);
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(want.stdout, "builtin\nbuiltin\nbuiltin\nbuiltin\nbuiltin\n0\n");
    assert_eq!(got.stdout, want.stdout);
}

/// Universal variables live in fish's own `fish_variables`: what zshrs writes, fish reads,
/// and the other way round.
#[test]
fn universal_variables_share_fish_variables_with_fish() {
    let Some(reference) = fish() else { return };
    let cfg = scratch("uvar");
    let home = scratch("uvar-home");
    let zshrs = env!("CARGO_BIN_EXE_zshrs");
    let run_zshrs = |script: &str| {
        let mut child = Command::new(zshrs)
            .args(["--fish", "-f"])
            .env_clear()
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", &cfg)
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn zshrs");
        std::io::Write::write_all(&mut child.stdin.take().unwrap(), script.as_bytes()).unwrap();
        String::from_utf8_lossy(&child.wait_with_output().unwrap().stdout).into_owned()
    };
    let probe = "echo (count $zu_list) $zu_list[2] $zu_list[3] $zu_list[4]; echo (count $zu_exp) $zu_exp; echo (count $zu_empty) $zu_one";
    // zshrs writes, fish reads
    run_zshrs("set -U zu_list a 'b c' 'd\\e' é\nset -Ux zu_exp 1 2\nset -U zu_empty\nset -U zu_one hello\n");
    // (`--no-config` makes fish ignore the file, so it reads it with config enabled)
    let by_fish = {
        let out = Command::new(&reference)
            .args(["-c", probe])
            .env_clear()
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", &cfg)
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .stdin(Stdio::null())
            .output()
            .expect("spawn fish");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let by_zshrs = run_zshrs(&format!("{probe}\n"));
    assert_eq!(by_zshrs, by_fish, "zshrs and fish disagree about the variables zshrs wrote");
    assert_eq!(by_fish, "4 b c d\\e é\n2 1 2\n0 hello\n");
    // fish writes, zshrs reads
    let setter = "set -U zu_list x 'y z'; set -Ux zu_exp 7";
    let _ = Command::new(&reference)
        .args(["-c", setter])
        .env_clear()
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &cfg)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .stdin(Stdio::null())
        .output();
    let back = run_zshrs("echo (count $zu_list) $zu_list[2]; echo $zu_exp\n");
    assert_eq!(back, "2 y z\n7\n");
    for d in [&cfg, &home] {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// fish runs the FIRST file on $PATH with the command's name; when that file
/// cannot be executed (its `#!` names a missing interpreter) the command
/// fails there, where zsh goes on to the next directory. Both the plain and
/// the redirected (forked) exec paths are checked.
#[test]
fn a_failing_first_path_match_is_not_retried_further_down_path() {
    use std::os::unix::fs::PermissionsExt;
    let Some(reference) = fish() else { return };
    let home = scratch("exec");
    let (broken, good) = (home.join("a"), home.join("b"));
    for (dir, body) in [(&broken, "#!/nonexistent/zz-interp\n"), (&good, "#!/bin/sh\necho ran-second\n")] {
        std::fs::create_dir_all(dir).unwrap();
        let f = dir.join("zzexe");
        std::fs::write(&f, body).unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!("{}:{}:/usr/bin:/bin", broken.display(), good.display());
    let script = home.join("exec.fish");
    std::fs::write(&script, "zzexe; echo st=$status\nzzexe >/dev/null; echo st=$status\n").unwrap();
    let go = |bin: &std::path::Path, args: &[&str]| {
        let out = Command::new(bin)
            .args(args)
            .arg(&script)
            .env_clear()
            .env("HOME", &home)
            .env("PATH", &path)
            .stdin(Stdio::null())
            .output()
            .expect("spawn shell");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let want = go(&reference, &["--no-config"]);
    let got = go(Path::new(env!("CARGO_BIN_EXE_zshrs")), &["--fish", "-f"]);
    let _ = std::fs::remove_dir_all(&home);
    assert!(!want.contains("ran-second"), "fish: {want}");
    assert_eq!(got, want);
}

/// `functions NAME`, `type NAME`, `functions --details`, `funcsave` and `status line-number`
/// show a function as written, with the file and line it was defined at. Both shells run
/// the same script from the same directory with their own configuration directory.
#[test]
fn function_definitions_are_shown_as_written() {
    let Some(reference) = fish() else { return };
    let dir = scratch("fndef");
    std::fs::write(
        dir.join("fn.fish"),
        "function f --description \"d\"\n    echo hi\n  if true\n  echo   x\n  end\nend\nfunctions f\ntype f\nfunctions --details f\nfuncsave f\nstatus line-number\n\necho (status line-number)\nfunction g\n    status line-number\nend\ng\n",
    )
    .unwrap();
    let run_in = |bin: &Path, args: &[&str], cfg: &Path| {
        let out = Command::new(bin)
            .args(args)
            .arg("fn.fish")
            .env_clear()
            .env("HOME", &dir)
            .env("XDG_CONFIG_HOME", cfg)
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .current_dir(&dir)
            .stdin(Stdio::null())
            .output()
            .expect("spawn shell");
        String::from_utf8_lossy(&out.stdout).replace(&*cfg.to_string_lossy(), "CFG")
    };
    let (cfg_a, cfg_b) = (dir.join("cfg-a"), dir.join("cfg-b"));
    let want = run_in(&reference, &["--no-config"], &cfg_a);
    let got = run_in(Path::new(env!("CARGO_BIN_EXE_zshrs")), &["--fish", "-f"], &cfg_b);
    assert_eq!(got, want);
    assert_eq!(
        std::fs::read_to_string(cfg_a.join("fish/functions/f.fish")).unwrap(),
        std::fs::read_to_string(cfg_b.join("fish/functions/f.fish")).unwrap(),
        "funcsave wrote different files"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
