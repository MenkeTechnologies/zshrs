//! fish source → zsh source translator for `zshrs --fish`.
//!
//! zshrs-original — no zsh C counterpart. fish syntax is not zsh syntax
//! (`set x 1 2`, `end` instead of `fi`/`done`/`esac`, `(cmd)` substitution,
//! `$x[2..3]`, list-valued variables), so a `--fish` input is rewritten here
//! into zsh text with the same meaning and run by the ordinary engine, the way
//! `--csh` is. The reference is fish 4.x; every rule here is checked against it.
//!
//! Layering (each file owns one concern):
//!   * [`parse`]    — fish source → AST (quoting, substitutions, blocks)
//!   * [`emit`]     — AST → zsh (variables as arrays, words, control flow)
//!   * [`setcmd`]   — `set`, `read`, `alias`, `abbr`: the builtins that name variables
//!   * [`builtins`] — `string`, `math`, `count`, `contains` as in-process commands
//!
//! A script is parsed whole before anything runs, as fish does: a syntax error
//! anywhere prevents the first command from executing.

pub mod builtins;
pub mod emit;
pub mod parse;
pub mod setcmd;

/// zsh-side helpers the translated output relies on: the options fish's
/// semantics need, `status`, `type`, `functions`, `set_color`, `source`/`eval`
/// (which translate their input), the command-not-found message and the prompt
/// hook. Installed once, ahead of the first translated input.
pub const PREAMBLE: &str = include_str!("preamble.zsh");

/// Translate a whole fish script to zsh.
pub fn translate(src: &str) -> Result<String, String> {
    let script = parse::parse(src)?;
    let mut text = emit::Emitter::default().script(&script);
    text.push('\n');
    Ok(text)
}

/// True for the errors that mean "the input is not finished yet" (an open block
/// or an unterminated quote), so a prompt keeps reading lines.
pub fn is_incomplete(msg: &str) -> bool {
    msg.contains("Missing end") || msg.contains("quotes are not balanced") || msg.contains("parentheses do not match")
}

/// The preamble with the path of this binary spliced into `source` and `eval`.
pub fn preamble(exe: &str) -> String {
    PREAMBLE.replace("@ZSHRS@", &exe.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::translate;

    fn t(src: &str) -> String {
        translate(src).unwrap().trim_end().to_string()
    }

    #[test]
    fn set_assigns_a_list_and_expansion_keeps_it_a_list() {
        assert_eq!(t("set x 1 2 3"), "x=(1 2 3)");
        assert_eq!(t("echo $x"), "echo ${x+\"${x[@]}\"}");
        assert_eq!(t("echo \"$x\""), "echo \"${(j: :)x}\"");
        assert_eq!(t("echo $x[2]"), "echo \"${(@)x[2,2]}\"");
        assert_eq!(t("echo $x[2..3]"), "echo \"${(@)x[2,3]}\"");
        assert_eq!(t("echo $x.txt"), "echo ${^x}.txt");
    }

    #[test]
    fn scopes_map_to_typeset() {
        assert_eq!(t("set -g y a"), "y=(a)");
        assert_eq!(t("function f\nset -l z 1\nset -g w 2\nend"), "function f {\nlocal -a z=(1)\ntypeset -ga w=(2)\n}");
        assert_eq!(t("set -gx E vim"), "typeset -gx E=vim");
        assert_eq!(t("set -x GOPATH a b"), "_fish_x=(a b); typeset -gx GOPATH=\"${(j.:.)_fish_x}\"");
    }

    #[test]
    fn colliding_names_do_not_touch_zsh_specials() {
        assert_eq!(t("for path in a b\nend"), "for _fish_v_path in a b\ndo\n:\ndone");
        assert_eq!(t("set PATH $PATH /x"), "path=(${path+\"${path[@]}\"} /x)");
    }

    #[test]
    fn control_flow() {
        assert_eq!(t("if test -f a\necho y\nelse if true\necho z\nelse\necho n\nend"),
                   "if test -f a\nthen\necho y\nelif true\nthen\necho z\nelse\necho n\nfi");
        assert_eq!(t("a; and b; or c"), "a && b || c");
        assert_eq!(t("not a"), "! a");
        assert_eq!(t("while true\nbreak\nend"), "while true\ndo\nbreak\ndone");
    }

    #[test]
    fn command_substitution_splits_on_newlines() {
        assert_eq!(t("echo (ls)"), "{ __fc1=\"$( ls )\"; echo ${__fc1:+\"${(@f)__fc1}\"}; }");
        assert_eq!(t("echo \"$(ls)\""), "echo \"$( ls )\"");
    }

    #[test]
    fn globs_and_literals() {
        assert_eq!(t("echo *.txt ?x [a]"), "echo *.txt '?x' '[a]'");
        assert_eq!(t("echo ~/a"), "echo ~/a");
        assert_eq!(t("echo {a,b}c"), "echo {a,b}c");
    }

    #[test]
    fn syntax_errors_are_reported_before_anything_runs() {
        assert!(translate("echo hi\nif true").is_err());
        assert!(translate("echo 'open").is_err());
    }
}
