//! Newuser module - port of Modules/newuser.c
//!
//! Provides first-run setup for new zsh users.

use std::path::Path;

/// Check whether the user needs the first-run setup wizard.
/// Port of the dotfile-presence check inside `boot_()` from
/// Src/Modules/newuser.c:68 (which calls `check_dotfile()` at
/// line 58 once per startup file). The C source skips the wizard
/// if any of `.zshrc`/`.zshenv`/`.zprofile`/`.zlogin`/`.zlogout`
/// already exist; same predicate here.
pub fn needs_newuser_setup(home: &Path) -> bool {
    let zshrc = home.join(".zshrc");
    let zshenv = home.join(".zshenv");
    let zprofile = home.join(".zprofile");
    let zlogin = home.join(".zlogin");
    let zlogout = home.join(".zlogout");

    !zshrc.exists()
        && !zshenv.exists()
        && !zprofile.exists()
        && !zlogin.exists()
        && !zlogout.exists()
}

/// Generate default `.zshrc` content for the recommended path.
/// Port of the `Functions/Newuser/zsh-newuser-install` script the
/// `newuser` module ships (Src/Modules/newuser.mdd lists it under
/// `functions=`). The C side just dispatches into the script;
/// here we inline the same content the script's "recommended"
/// branch would write.
pub fn default_zshrc() -> String {
    r#"# Lines configured by zsh-newuser-install

# History configuration
HISTFILE=~/.zsh_history
HISTSIZE=10000
SAVEHIST=10000

# Options
setopt HIST_IGNORE_DUPS
setopt HIST_IGNORE_SPACE
setopt EXTENDED_HISTORY
setopt SHARE_HISTORY
setopt APPEND_HISTORY
setopt AUTO_CD
setopt CORRECT

# Key bindings - emacs style
bindkey -e

# Completion
autoload -Uz compinit
compinit

# Prompt
autoload -Uz promptinit
promptinit
prompt adam1

# End of lines configured by zsh-newuser-install
"#
    .to_string()
}

/// Generate a minimal `.zshrc` for the "just give me a working
/// shell" path.
/// zshrs-original convenience — the upstream wizard
/// (`Functions/Newuser/zsh-newuser-install`) only writes the full
/// recommended file. We expose a smaller alternative for the
/// `Minimal` setup choice.
pub fn minimal_zshrc() -> String {
    r#"# Minimal zsh configuration
HISTFILE=~/.zsh_history
HISTSIZE=1000
SAVEHIST=1000
bindkey -e
"#
    .to_string()
}

/// First-run setup choice.
/// Mirrors the menu items the upstream `zsh-newuser-install`
/// script (loaded by Src/Modules/newuser.c:68 `boot_()`) presents.
/// `Recommended` writes the full template, `Minimal` writes a
/// stripped-down one, `Manual` lets the user edit themselves, and
/// `Exit` skips the wizard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupChoice {
    Recommended,
    Minimal,
    Exit,
    Manual,
}

/// Run the first-run setup wizard.
/// Port of the `boot_()` dispatcher from Src/Modules/newuser.c:68
/// — the C source autoloads and runs `zsh-newuser-install` which
/// then writes the chosen template to `~/.zshrc`. This Rust path
/// inlines the file write directly.
pub fn run_newuser_setup(home: &Path, choice: SetupChoice) -> std::io::Result<()> {
    let zshrc = home.join(".zshrc");

    match choice {
        SetupChoice::Recommended => {
            std::fs::write(&zshrc, default_zshrc())?;
        }
        SetupChoice::Minimal => {
            std::fs::write(&zshrc, minimal_zshrc())?;
        }
        SetupChoice::Exit | SetupChoice::Manual => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_default_zshrc() {
        let content = default_zshrc();
        assert!(content.contains("HISTFILE"));
        assert!(content.contains("compinit"));
    }

    #[test]
    fn test_minimal_zshrc() {
        let content = minimal_zshrc();
        assert!(content.contains("HISTFILE"));
        assert!(content.len() < default_zshrc().len());
    }

    #[test]
    fn test_needs_newuser_setup_empty() {
        let temp = std::env::temp_dir().join("zsh_test_newuser_empty");
        std::fs::create_dir_all(&temp).ok();

        for f in &[".zshrc", ".zshenv", ".zprofile", ".zlogin", ".zlogout"] {
            let _ = std::fs::remove_file(temp.join(f));
        }

        assert!(needs_newuser_setup(&temp));

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_needs_newuser_setup_has_zshrc() {
        let temp = std::env::temp_dir().join("zsh_test_newuser_has");
        std::fs::create_dir_all(&temp).ok();

        std::fs::write(temp.join(".zshrc"), "# test").ok();
        assert!(!needs_newuser_setup(&temp));

        let _ = std::fs::remove_dir_all(&temp);
    }
}
