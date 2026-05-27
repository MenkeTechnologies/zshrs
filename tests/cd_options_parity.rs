//! cd builtin option parity: -, -L, -P, -s, env CDPATH, AUTO_CD, etc.

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
fn run_zsh_in(d: &Path, s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .current_dir(d)
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn run_zshrs_in(d: &Path, s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .current_dir(d)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn assert_parity_in(d: &Path, s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh_in(d, s);
    let r = run_zshrs_in(d, s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit);
}
fn tdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

mod basic_cd {
    use super::*;

    #[test]
    fn cd_to_subdir_then_pwd() {
        let d = tdir();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        assert_parity_in(d.path(), "cd sub && basename $PWD");
    }

    #[test]
    fn cd_no_arg_goes_home() {
        let d = tdir();
        assert_parity_in(d.path(), "cd && [[ $PWD == $HOME ]]; echo $?");
    }

    #[test]
    fn cd_dash_goes_to_oldpwd() {
        let d = tdir();
        std::fs::create_dir(d.path().join("a")).unwrap();
        std::fs::create_dir(d.path().join("b")).unwrap();
        assert_parity_in(d.path(), "cd a; cd ../b; cd -; basename $PWD");
    }

    #[test]
    fn cd_to_nonexistent_errors() {
        let d = tdir();
        assert_parity_in(d.path(), "cd nonexistent_xyz_42 2>/dev/null; echo exit=$?");
    }
}

mod cd_L_P_flags {
    use super::*;

    /// cd -L (default): keep symlink in PWD.
    /// cd -P: resolve symlink to physical path.
    #[test]
    fn cd_dash_L_keeps_logical_path() {
        let d = tdir();
        std::fs::create_dir(d.path().join("real")).unwrap();
        std::os::unix::fs::symlink(d.path().join("real"), d.path().join("link")).unwrap();
        assert_parity_in(d.path(), "cd -L link 2>/dev/null && basename $PWD");
    }

    #[test]
    fn cd_dash_P_resolves_physical() {
        let d = tdir();
        std::fs::create_dir(d.path().join("real")).unwrap();
        std::os::unix::fs::symlink(d.path().join("real"), d.path().join("link")).unwrap();
        assert_parity_in(d.path(), "cd -P link 2>/dev/null && basename $PWD");
    }
}

mod cd_s_safe {
    use super::*;

    /// `cd -s` refuses symlinks (zsh-specific).
    #[test]
    fn cd_dash_s_rejects_symlink() {
        let d = tdir();
        std::fs::create_dir(d.path().join("real")).unwrap();
        std::os::unix::fs::symlink(d.path().join("real"), d.path().join("link")).unwrap();
        assert_parity_in(d.path(), "cd -s link 2>/dev/null; echo exit=$?");
    }
}

mod cd_two_args {
    use super::*;

    /// `cd old new` — substitute `old` with `new` in PWD.
    #[test]
    fn cd_two_arg_substitutes() {
        let d = tdir();
        std::fs::create_dir(d.path().join("apple")).unwrap();
        std::fs::create_dir(d.path().join("banana")).unwrap();
        assert_parity_in(d.path(), "cd apple; cd apple banana; basename $PWD");
    }
}

mod cdpath {
    use super::*;

    /// CDPATH allows `cd name` to find `name` in CDPATH dirs.
    #[test]
    fn cdpath_search_finds_subdir() {
        let d = tdir();
        std::fs::create_dir(d.path().join("project")).unwrap();
        std::fs::create_dir(d.path().join("project").join("src")).unwrap();
        let cdpath = d.path().join("project").display().to_string();
        let script = format!("CDPATH={cdpath} cd src && basename $PWD");
        assert_parity_in(d.path(), &script);
    }
}

mod auto_cd {
    use super::*;

    /// With AUTO_CD set, typing a dir name alone cd's there.
    #[test]
    fn auto_cd_bare_dirname() {
        let d = tdir();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        assert_parity_in(d.path(), "setopt auto_cd; sub; basename $PWD");
    }
}

mod oldpwd {
    use super::*;

    /// $OLDPWD set after cd.
    #[test]
    fn oldpwd_set_after_cd() {
        let d = tdir();
        std::fs::create_dir(d.path().join("a")).unwrap();
        assert_parity_in(d.path(), "cd a; basename $OLDPWD");
    }

    /// $PWD updates on each cd.
    #[test]
    fn pwd_var_updates_on_cd() {
        let d = tdir();
        std::fs::create_dir(d.path().join("a")).unwrap();
        assert_parity_in(
            d.path(),
            "P1=$PWD; cd a; P2=$PWD; [[ $P1 != $P2 ]]; echo $?",
        );
    }
}

mod chase_links {
    use super::*;

    /// CHASE_LINKS option = always resolve symlinks on cd.
    #[test]
    fn chase_links_resolves_symlink() {
        let d = tdir();
        std::fs::create_dir(d.path().join("real")).unwrap();
        std::os::unix::fs::symlink(d.path().join("real"), d.path().join("link")).unwrap();
        assert_parity_in(
            d.path(),
            "setopt chase_links; cd link 2>/dev/null; basename $PWD",
        );
    }
}
