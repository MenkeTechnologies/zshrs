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
pub mod complete;
pub mod emit;
pub mod parse;
pub mod setcmd;

/// zsh-side helpers the translated output relies on: the options fish's
/// semantics need, `status`, `type`, `functions`, `set_color`, `source`/`eval`
/// (which translate their input), the command-not-found message and the prompt
/// hook. Installed once, ahead of the first translated input.
pub const PREAMBLE: &str = include_str!("preamble.zsh");

/// Set for `zshrs --fish-translate --source`: the text is a file that `source` runs, so a
/// top-level `exit` in it leaves only that file (`return` in the `source` function).
static SOURCE_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_source_mode(on: bool) {
    SOURCE_MODE.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn source_mode() -> bool {
    SOURCE_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

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
        assert_eq!(t("echo $x[2..3]"), "{ __fish_slice __fc1 x 2 3; echo \"${__fc1[@]}\"; }");
        // a list inside a word is a loop, so an unset variable leaves no word at all
        assert_eq!(
            t("echo $x.txt"),
            "{ __fc1=(); for __fi1_0 in ${x+\"${x[@]}\"}; do __fc1+=(\"${__fi1_0}\".txt); done; echo \"${__fc1[@]}\"; }"
        );
        assert_eq!(t("echo $HOME/x"), "echo ${^HOME}/x");
    }

    #[test]
    fn scopes_map_to_typeset() {
        assert_eq!(t("set -g y a"), "y=(a)");
        assert_eq!(t("function f\nset -l z 1\nset -g w 2\nend"), "function f {\nlocal -a _fl1_z=(1)\ntypeset -ga w=(2)\n}");
        assert_eq!(t("set -gx E vim"), "{ [[ ${(t)E} == array* ]] && unset E; typeset -gx E=vim; }");
        assert_eq!(t("set -x GOPATH a b"), "{ _fish_x=(a b); [[ ${(t)GOPATH} == array* ]] && unset GOPATH; typeset -gx GOPATH=\"${(j.:.)_fish_x}\"; }");
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
    fn literals_that_zsh_would_glob_are_quoted() {
        assert_eq!(t("echo ?x [a]"), "echo '?x' '[a]'");
        assert_eq!(t("echo ~/a"), "echo ~/a");
        assert_eq!(t("echo {a,b}c"), "echo {a,b}c");
    }

    #[test]
    fn a_wildcard_command_is_isolated_so_no_match_ends_only_that_command() {
        let out = t("echo *.txt ?x");
        assert!(out.starts_with("{ __fish_g=; builtin eval 'echo *.txt '\\''?x'\\''; __fish_cs=$?; __fish_g=1'"), "{out}");
        assert!(out.contains("|| __fish_cs=124"), "{out}");
    }

    #[test]
    fn set_l_in_a_block_is_scoped_to_the_block() {
        let out = t("set -l a 1\nif true\nset -l a 2\necho $a\nend\necho $a");
        // top level stays plain; the block's `a` lives under its own name
        assert!(out.starts_with("typeset -ga _fl_top_a=(1)\nif true\nthen\ntypeset -ga _fl1_a=(2)\necho "), "{out}");
        assert!(out.contains("${_fl1_a+\"${_fl1_a[@]}\"}"), "{out}");
        assert!(out.ends_with("fi\necho ${_fl_top_a+\"${_fl_top_a[@]}\"}"), "{out}");
    }

    #[test]
    fn two_list_variables_in_a_word_vary_the_first_fastest() {
        let out = t("echo $a$b");
        assert!(out.contains("for __fi1_1 in ${b+\"${b[@]}\"}; do for __fi1_0 in ${a+\"${a[@]}\"}; do"), "{out}");
    }

    #[test]
    fn syntax_errors_are_reported_before_anything_runs() {
        assert!(translate("echo hi\nif true").is_err());
        assert!(translate("echo 'open").is_err());
    }
}
