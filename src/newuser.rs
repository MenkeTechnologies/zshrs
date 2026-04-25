//! Newuser module - port of Modules/newuser.c
//!
//! Provides first-run setup for new zsh users.

use std::path::Path;

/// Check if user needs first-run setup
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

/// Generate default .zshrc content
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

/// Generate minimal .zshrc content
pub fn minimal_zshrc() -> String {
    r#"# Minimal zsh configuration
HISTFILE=~/.zsh_history
HISTSIZE=1000
SAVEHIST=1000
bindkey -e
"#
    .to_string()
}

/// First-run setup options
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupChoice {
    Recommended,
    Minimal,
    Exit,
    Manual,
}

/// Run newuser setup
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
