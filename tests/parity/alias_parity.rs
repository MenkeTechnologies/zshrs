//! alias / unalias parity tests.

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

mod basic {
    use super::*;

    // NOTE on entire `basic` group:
    // In real zsh `-c` mode, an alias defined on the same line as its use
    // is NOT expanded — the parser sees the invocation BEFORE the alias
    // hashtable update commits. zshrs (more bash-like) expands eagerly.
    // The expansion only fires across parse boundaries (e.g. via eval).
    // These tests are kept active+ignored to document the divergence;
    // remove the #[ignore] when zshrs adopts the C-faithful defer.

    #[test]
    fn alias_simple_expansion() {
        assert_parity(r#"alias hi='echo hello'; hi"#);
    }

    #[test]
    fn alias_with_args() {
        assert_parity(r#"alias greet='echo hi'; greet world"#);
    }

    #[test]
    fn alias_multi_word_target() {
        assert_parity(r#"alias ll='echo ls -la'; ll"#);
    }

    #[test]
    fn alias_can_chain_into_pipeline() {
        assert_parity(r#"alias src='echo line1; echo line2'; src | wc -l"#);
    }

    #[test]
    fn alias_to_other_alias() {
        assert_parity(r#"alias a='echo a'; alias b=a; b"#);
    }
}

/// Cross-parse-boundary alias tests via `eval` — both shells expand.
mod via_eval {
    use super::*;

    #[test]
    fn alias_via_eval_expands_in_both_shells() {
        assert_parity(r#"alias hi='echo hello'; eval hi"#);
    }

    #[test]
    fn alias_with_args_via_eval() {
        assert_parity(r#"alias greet='echo hi'; eval "greet world""#);
    }

    /// Pin: nested alias inside an alias frame keeps the frame's
    /// remaining words. inpoptop must restore the PUSH-TIME inbufct
    /// (input.c:686/764) — recomputing only the restored frame's
    /// remainder tripped ingetc's `!inbufct && strin` EOF gate
    /// (input.c:342) and dropped ` x` entirely.
    #[test]
    fn nested_alias_keeps_frame_remainder_via_eval() {
        assert_parity(r#"alias g='echo A'; alias t='g x'; eval t"#);
    }

    /// Pin: two-level alias chain with trailing args after the inner
    /// alias word (`tommy` → `git status`, `git` → `hub`). The alias
    /// body must not fuse with the following word (`hubstatus`) and
    /// the frame remainder must survive the inner expansion.
    #[test]
    fn two_level_alias_chain_via_eval() {
        assert_parity(
            r#"alias hub='echo HUB'; alias git=hub; alias tommy='git status'; eval tommy"#,
        );
    }

    /// Pin: trailing-space alias body marks the NEXT word
    /// alias-eligible (inalmore, input.c:775 / lex.c:1917) — the
    /// `alias sudo='sudo '` chaining pattern.
    #[test]
    fn trailing_space_alias_chains_next_word() {
        assert_parity(r#"alias sp='echo trail '; alias comp='echo comp'; eval "sp comp""#);
    }
}

mod subshell_survival {
    use super::*;

    /// Pin: ALIAS_GLOBAL must survive a subshell exit. The in-process
    /// subshell alias snapshot restored every entry with flags=0, so
    /// ANY subshell (`(true)`, zsh-z's `(zshz --add … &)` precmd)
    /// reflagged every global alias to regular in the parent —
    /// `alias -g` listed nothing one prompt after every define.
    #[test]
    fn global_alias_survives_subshell() {
        assert_parity(r#"alias -g gx=t; (true); print -r -- ${+galiases[gx]} ${+aliases[gx]}"#);
    }

    /// Pin: suffix aliases and DISABLED flags round-trip too.
    #[test]
    fn suffix_and_disabled_survive_subshell() {
        assert_parity(
            r#"alias -s txt=cat; alias dd1=x; disable -a dd1; (true); print -r -- ${+saliases[txt]} ${+dis_aliases[dd1]}"#,
        );
    }
}

mod position {
    use super::*;

    /// Aliases expand ONLY in command position by default.
    #[test]
    fn alias_in_arg_position_not_expanded() {
        assert_parity(r#"alias hi='echo HELLO'; echo hi"#);
    }

    /// Quoted command name disables alias expansion.
    #[test]
    fn quoted_command_disables_alias() {
        assert_parity(r#"alias hi='echo HELLO'; \hi 2>/dev/null; echo done"#);
    }

    /// Single-quoted command name same.
    #[test]
    fn single_quoted_command_disables_alias() {
        assert_parity(r#"alias hi='echo HELLO'; 'hi' 2>/dev/null; echo done"#);
    }
}

mod unalias {
    use super::*;

    #[test]
    fn unalias_removes_alias() {
        assert_parity(r#"alias hi='echo HELLO'; hi; unalias hi; hi 2>/dev/null; echo done"#);
    }

    #[test]
    fn unalias_nonexistent_exits_nonzero() {
        // `unalias nonexistent_xyz` errors; pin exit code propagation.
        assert_parity(r#"unalias nonexistent_xyz 2>/dev/null; echo $?"#);
    }
}

mod listing {
    use super::*;

    /// `alias` (no args) lists all defined aliases. Output format varies
    /// slightly between shells; sort and check member presence.
    #[test]
    fn alias_list_contains_defined() {
        assert_parity(r#"alias myhi='echo hi'; alias | grep myhi | head -1"#);
    }

    /// `alias name` shows one alias definition.
    #[test]
    fn alias_name_shows_single_definition() {
        assert_parity(r#"alias x='echo y'; alias x"#);
    }
}

mod global_alias {
    use super::*;

    /// `alias -g` makes a global alias (expands anywhere on line, not
    /// just command position).
    #[test]
    fn global_alias_expands_in_arg_position() {
        assert_parity(r#"alias -g HI='hello'; echo HI"#);
    }

    /// Global alias inside double quotes — NOT expanded (only outside).
    #[test]
    fn global_alias_inside_double_quotes_not_expanded() {
        assert_parity(r#"alias -g HI='hello'; echo "HI""#);
    }
}

mod suffix_alias {
    use super::*;

    /// `alias -s` makes a suffix alias (runs handler for arg with that suffix).
    /// Setup: register `txt` suffix to `cat`.
    #[test]
    fn suffix_alias_runs_handler_for_extension() {
        if !zsh_available() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("hi.txt");
        std::fs::write(&f, "from file").unwrap();
        let script = format!(r#"alias -s txt=cat; cd {}; ./hi.txt"#, dir.path().display());
        let z = Command::new(zsh_path())
            .args(["-fc", &script])
            .output()
            .unwrap();
        let r = Command::new(zshrs_bin())
            .args(["--zsh", "-f", "-c", &script])
            .env_remove("ZSHRS_CACHE")
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&z.stdout),
            String::from_utf8_lossy(&r.stdout),
            "suffix alias output should match"
        );
    }
}

mod recursive {
    use super::*;

    /// Aliases CAN be recursive: zsh resolves them iteratively but
    /// guards against infinite recursion via tracking.
    #[test]
    fn alias_pointing_to_command_with_same_name_uses_command() {
        // `alias ls='ls --color'` — when invoking `ls`, alias expands to
        // `ls --color`; the second `ls` is the actual command (not the
        // alias) to prevent infinite recursion.
        assert_parity(r#"alias true='true && true'; true; echo $?"#);
    }
}

mod definition_replacement {
    use super::*;

    /// Redefining an alias replaces the body. The use goes through
    /// `eval` because aliases never apply to commands in the SAME
    /// parse unit (`zsh -fc '...; x'` parses the whole string before
    /// the alias exists) — the old bare-`x` form never exercised the
    /// alias at all and instead executed whatever `x` was on PATH:
    /// on macOS with XQuartz, /opt/X11/bin/x is the X server, which
    /// blocks for minutes in BOTH shells and stalled the whole
    /// parity sweep. `eval` re-parses with the live alias table, so
    /// both shells print the redefined body.
    #[test]
    fn alias_redefinition_replaces_body() {
        assert_parity(
            r#"alias zr_alias_t='echo first'; alias zr_alias_t='echo second'; eval zr_alias_t"#,
        );
    }
}
