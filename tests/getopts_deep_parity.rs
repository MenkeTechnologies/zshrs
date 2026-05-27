//! getopts deep parity tests — flag-only, flag-with-arg, OPTARG/OPTIND,
//! leading-colon silent-error mode, unknown opt, missing arg, --.

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
fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn run_zshrs(s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn assert_parity(s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(s);
    let r = run_zshrs(s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit);
}

mod single_flag {
    use super::*;

    #[test]
    fn single_a_flag() {
        assert_parity(
            r#"
set -- -a
while getopts a opt; do echo opt=$opt; done
"#,
        );
    }

    #[test]
    fn flag_with_arg_via_colon() {
        assert_parity(
            r#"
set -- -f file.txt
while getopts "f:" opt; do echo opt=$opt arg=$OPTARG; done
"#,
        );
    }

    #[test]
    fn flag_with_no_value_provided() {
        // `getopts "f:"` expects arg; with `-f` and no arg, errors.
        assert_parity(
            r#"
set -- -f
while getopts "f:" opt 2>/dev/null; do echo opt=$opt; done
echo done
"#,
        );
    }
}

mod multiple_flags {
    use super::*;

    #[test]
    fn three_flags_in_sequence() {
        assert_parity(
            r#"
set -- -a -b -c
while getopts abc opt; do echo opt=$opt; done
"#,
        );
    }

    #[test]
    fn combined_short_flags() {
        // POSIX: -abc is same as -a -b -c when all flags take no arg.
        assert_parity(
            r#"
set -- -abc
while getopts abc opt; do echo opt=$opt; done
"#,
        );
    }

    #[test]
    fn mixed_flags_and_arg() {
        assert_parity(
            r#"
set -- -a -f filename -b
while getopts "abf:" opt; do
  case $opt in
    f) echo "f=$OPTARG";;
    *) echo "flag=$opt";;
  esac
done
"#,
        );
    }
}

mod unknown_opt {
    use super::*;

    /// Default mode: unknown opt → emits error + sets opt to '?'.
    #[test]
    fn unknown_opt_default_emits_error() {
        assert_parity(
            r#"
set -- -X
while getopts abc opt 2>/dev/null; do echo opt=$opt; done
"#,
        );
    }

    /// Leading-colon optstring → silent mode, opt='?', OPTARG=name.
    #[test]
    fn unknown_opt_silent_mode() {
        assert_parity(
            r#"
set -- -X
while getopts ":abc" opt; do echo "opt=$opt arg=$OPTARG"; done
"#,
        );
    }
}

mod missing_arg {
    use super::*;

    /// `-f` without value at end. Default mode: error + opt='?'.
    #[test]
    fn missing_arg_default_mode() {
        assert_parity(
            r#"
set -- -f
while getopts "f:" opt 2>/dev/null; do echo opt=$opt; done
echo done
"#,
        );
    }

    /// Silent mode: opt=':' and OPTARG=letter.
    #[test]
    fn missing_arg_silent_mode() {
        assert_parity(
            r#"
set -- -f
while getopts ":f:" opt; do echo "opt=$opt arg=$OPTARG"; done
echo done
"#,
        );
    }
}

mod optind {
    use super::*;

    /// OPTIND after consuming -a -b is 3 (next-to-process is positional 3).
    #[test]
    fn optind_after_two_flags() {
        assert_parity(
            r#"
set -- -a -b extra
while getopts ab opt; do :; done
echo OPTIND=$OPTIND
"#,
        );
    }

    /// OPTIND after consuming flag-with-arg.
    #[test]
    fn optind_after_flag_with_arg() {
        assert_parity(
            r#"
set -- -f file extra
while getopts "f:" opt; do :; done
echo OPTIND=$OPTIND
"#,
        );
    }

    /// OPTIND with `--` terminator.
    #[test]
    fn optind_after_double_dash() {
        assert_parity(
            r#"
set -- -a -- extra
while getopts ab opt; do echo opt=$opt; done
echo OPTIND=$OPTIND
"#,
        );
    }
}

mod end_of_options {
    use super::*;

    /// `--` ends option parsing; remaining args are positional.
    #[test]
    fn double_dash_ends_options() {
        assert_parity(
            r#"
set -- -a -- -b extra
while getopts ab opt; do echo opt=$opt; done
"#,
        );
    }

    /// Non-flag arg ends parsing.
    #[test]
    fn non_flag_ends_parsing() {
        assert_parity(
            r#"
set -- -a normal_arg -b
while getopts ab opt; do echo opt=$opt; done
"#,
        );
    }
}

mod multi_letter {
    use super::*;

    /// getopts handles long optstring.
    #[test]
    fn long_optstring_with_many_flags() {
        assert_parity(
            r#"
set -- -h -V -d -q
while getopts hVdqv opt; do echo opt=$opt; done
"#,
        );
    }
}

mod arg_inside_word {
    use super::*;

    /// `-fFILE` (no space) — arg attached to flag.
    #[test]
    fn arg_attached_to_flag() {
        assert_parity(
            r#"
set -- -ffilename
while getopts "f:" opt; do echo "opt=$opt arg=$OPTARG"; done
"#,
        );
    }
}

mod reset_optind {
    use super::*;

    /// Re-running getopts after OPTIND=1 reset works.
    #[test]
    fn reset_optind_and_re_parse() {
        assert_parity(
            r#"
set -- -a
while getopts a opt; do echo "1: $opt"; done
OPTIND=1
while getopts a opt; do echo "2: $opt"; done
"#,
        );
    }
}
