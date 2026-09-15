//! Parity for a `${name:offset}` substring inside a script FILE.
//!
//! A script file is lexed one event at a time (c:Src/init.c:155-220), so
//! when an event runs, the lexer is parked partway through the file.
//! `${x:2}` runs `parse_subscript` on the offset (c:Src/subst.c:1580),
//! which pushes the operand as its own input (c:Src/lex.c:1750). In C
//! nothing past that string is reachable. zshrs's `hgetc` fell through
//! to the outer file window once the operand drained, so the subscript
//! scan consumed the NEXT event's text up to its first `:`. Real-world
//! shape: fzf-tab's `${@:2}` destroyed `fzf-tab-lscolors::from-name`
//! several lines later ("command not found: from-name").

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

/// Run both shells on the same script file; return (zsh, zshrs) as
/// stdout followed by stderr, with the file path normalised away.
fn run_file(name: &str, body: &str) -> (String, String) {
    let dir = std::env::temp_dir().join(format!(
        "zshrs_subscript_window_{}_{}",
        std::process::id(),
        name
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("s.zsh");
    std::fs::write(&path, body).expect("write script");
    let render = |o: std::process::Output| {
        let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
        s.push_str(&String::from_utf8_lossy(&o.stderr));
        s.replace(path.to_str().unwrap(), "SCRIPT")
    };
    let z = Command::new(zsh_path())
        .arg("-f")
        .arg(&path)
        .current_dir(&dir)
        .output()
        .expect("zsh");
    let r = Command::new(zshrs_bin())
        .args(["--zsh", "-f"])
        .arg(&path)
        .current_dir(&dir)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    let _ = std::fs::remove_dir_all(&dir);
    (render(z), render(r))
}

#[test]
fn substring_offset_does_not_consume_the_next_event() {
    if !zsh_available() {
        return;
    }
    // The comment lines carry the `:` the runaway scan stopped at.
    let body = "x=abcdef\n\
                print -r -- ${x:2}\n\
                # a::b ${(s::)x}\n\
                print -r -- one::two\n";
    let (z, r) = run_file("offset", body);
    assert_eq!(z, r);
}

#[test]
fn substring_offset_and_length_do_not_consume_the_next_event() {
    if !zsh_available() {
        return;
    }
    // Two parse_subscript scans (offset, then length) ate two runs.
    let body = "set -- a b c d\n\
                print -r -- ${@:2} ${x::1} ${@:1:2}\n\
                # c::d\n\
                # e::f\n\
                print -r -- after::it\n";
    let (z, r) = run_file("offset_length", body);
    assert_eq!(z, r);
}

#[test]
fn substring_inside_a_running_function_keeps_the_file_lexer_in_place() {
    if !zsh_available() {
        return;
    }
    // fzf-tab.plugin.zsh's shape: an anonymous function whose helper
    // evaluates `${@:2}`, then `name::word` commands further down.
    let body = "() {\n\
                  f() { print -r -- ${@:2} }\n\
                  f a b c\n\
                }\n\
                # fzf-tab-lscolors::match-by $1 lstat follow\n\
                g-h::from-name() { print -r -- called $1 }\n\
                [[ -z $REPLY ]] && g-h::from-name x\n";
    let (z, r) = run_file("anon_fn", body);
    assert_eq!(z, r);
}
