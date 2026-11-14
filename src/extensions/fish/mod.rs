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
    let mut emitter = emit::Emitter::default();
    emitter.plain_top = emit::uses_dynamic_names(&script);
    emitter.watched_vars = watched_variables(src);
    let mut text = emitter.script(&script);
    text.push('\n');
    Ok(text)
}

/// The names after `--on-variable` / `-v` / `--on-variable=` anywhere in the source.
fn watched_variables(src: &str) -> std::collections::HashSet<String> {
    let mut names = Vec::new();
    let mut words = src.split_whitespace();
    while let Some(w) = words.next() {
        if let Some(n) = w.strip_prefix("--on-variable=") {
            names.push(n.to_string());
        } else if matches!(w, "--on-variable" | "-v") {
            names.extend(words.next().map(|n| n.trim_matches(|c| matches!(c, ';' | '\'' | '"')).to_string()));
        }
    }
    // kept across translations, so a prompt line sees a handler defined on an earlier one
    WATCHED.with(|w| {
        let mut w = w.borrow_mut();
        w.extend(names);
        w.clone()
    })
}

thread_local! {
    static WATCHED: std::cell::RefCell<std::collections::HashSet<String>> = Default::default();
}

/// True for the errors that mean "the input is not finished yet" (an open block
/// or an unterminated quote), so a prompt keeps reading lines.
pub fn is_incomplete(msg: &str) -> bool {
    msg.contains("Missing end") || msg.contains("quotes are not balanced") || msg.contains("parentheses do not match")
}

/// The preamble with the path of this binary spliced into `source` and `eval`.
pub fn preamble(exe: &str) -> String {
    let mut text = PREAMBLE.replace("@ZSHRS@", &exe.replace('\'', "'\\''"));
    text.push_str(&env_fixups());
    text
}

/// zshrs exports a few variables fish does not (`FPATH`, `INFOPATH`, `MANPATH`, `LOGNAME`,
/// `OLDPWD`) and leaves `USER` unexported, where fish exports exactly `HOME PATH PWD SHLVL
/// TERM USER` on top of what it inherited. A variable the process did not inherit is
/// taken out of the environment its children see.
/// The login name of the real user, from the passwd database.
fn passwd_user() -> Option<String> {
    // SAFETY: getpwuid returns a pointer to static storage (or null); the name is copied
    // out before anything else can call into the passwd database.
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() {
            return None;
        }
        Some(std::ffi::CStr::from_ptr((*pw).pw_name).to_string_lossy().into_owned())
    }
}

/// fish's `__fish_*_dir` variables: the user's directories from the XDG settings, and the
/// install directories relative to the running binary's prefix.
fn install_dirs() -> String {
    let q = |s: &str| s.replace('\'', "'\\''");
    let prefix = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().and_then(|b| b.parent()).map(|p| p.to_path_buf()))
        .map_or_else(|| "/usr/local".to_string(), |p| p.to_string_lossy().into_owned());
    let (mut bin, mut data) = (format!("{prefix}/bin"), format!("{prefix}/share/fish"));
    let (mut help, mut man) = (format!("{prefix}/share/doc/fish"), format!("{data}/man"));
    let mut sysconf = format!("{prefix}/etc/fish");
    // with no fish data installed beside zshrs, the installed fish's own
    // (its man pages are what `man` completes fish commands from)
    if !std::path::Path::new(&data).is_dir() {
        if let Some(d) = complete::fish_data() {
            [data, help, man, bin] = d.dirs.clone();
            sysconf = d.sysconf.clone();
        }
    }
    format!(
        "__fish_bin_dir=('{}') __fish_data_dir=('{}') __fish_help_dir=('{}') __fish_man_dir=('{}') __fish_sysconf_dir=('{}')\n\
         __fish_config_dir=(${{XDG_CONFIG_HOME:-$HOME/.config}}/fish) __fish_cache_dir=(${{XDG_CACHE_HOME:-$HOME/.cache}}/fish) \
         __fish_user_data_dir=(${{XDG_DATA_HOME:-$HOME/.local/share}}/fish)\n",
        q(&bin), q(&data), q(&help), q(&man), q(&sysconf)
    )
}

/// The environment the shell started with, for `set --show`'s "originally inherited as".
static INHERITED: std::sync::OnceLock<std::collections::HashMap<String, String>> = std::sync::OnceLock::new();

/// What `name` was in the environment at startup.
pub(crate) fn inherited_value(name: &str) -> Option<String> {
    INHERITED.get()?.get(name).cloned()
}

fn env_fixups() -> String {
    INHERITED.get_or_init(|| {
        crate::ported::params::environ
            .get()
            .map(|env| env.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    });
    let inherited = |name: &str| {
        crate::ported::params::environ
            .get()
            .is_some_and(|env| env.iter().any(|(k, _)| k == name))
    };
    let extras: Vec<&str> = ["FPATH", "INFOPATH", "MANPATH", "LOGNAME", "OLDPWD"]
        .into_iter()
        .filter(|n| !inherited(n))
        .collect();
    let mut out = format!(
        "__fish_uvar_file='{}'\n",
        builtins::uvar_path().to_string_lossy().replace('\'', "'\\''")
    );
    out.push_str(&install_dirs());
    out.push_str(&complete::search_path_assignments());
    if !extras.is_empty() {
        // one statement each: a failure on one must not skip the rest
        for n in &extras {
            out.push_str(&format!("typeset +x {n} 2>/dev/null\n"));
        }
    }
    // fish sets USER from the passwd entry when the environment has none
    if let Some(user) = passwd_user() {
        out.push_str(&format!(
            "[[ -n ${{USER-}}  ]] || USER='{}'\n",
            user.replace('\'', "'\\''")
        ));
    }
    // fish counts only interactive shells in SHLVL; a script or `-c` keeps what it inherited
    let shlvl = crate::ported::params::environ
        .get()
        .and_then(|env| env.iter().find(|(k, _)| k == "SHLVL").map(|(_, v)| v.clone()));
    match shlvl {
        Some(v) => out.push_str(&format!("[[ -o interactive ]] || SHLVL='{}'\n", v.replace('\'', "'\\''"))),
        None => out.push_str("[[ -o interactive ]] || unset SHLVL\n"),
    }
    out.push_str("[[ -n ${USER-} ]] && typeset -gx USER\n:\n"); // (ends with status 0)
    out
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
        assert_eq!(t("function f\nset -l z 1\nset -g w 2\nend"), "function f {\nlocal -a _fl1_z=(1)\ntypeset -ga w=(2)\n}\n__fish_gset[f]=1\n__fish_fnsrc[f]='function f\nset -l z 1\nset -g w 2\nend'; __fish_fnline[f]=1; __fish_fnfile[f]=\"${__fish_srcfile:-${(%):-%x}}\"\n__fish_fndesc[f]=''\n__fish_fnshadow[f]=''");
        // an exported list stays a list: the exported scalar is tied to the array __fx_NAME
        assert_eq!(t("set -gx E vim"), "{ _fish_x=(vim); if (( ! ${+__fx_E} )); then unset E 2>/dev/null; typeset -gxT E __fx_E ' '; fi; __fx_E=(\"${_fish_x[@]}\"); }");
        assert_eq!(t("set -x GOPATH a b"), "{ _fish_x=(a b); if (( ! ${+__fx_GOPATH} )); then unset GOPATH 2>/dev/null; typeset -gxT GOPATH __fx_GOPATH ':'; fi; __fx_GOPATH=(\"${_fish_x[@]}\"); }");
    }

    #[test]
    fn colliding_names_do_not_touch_zsh_specials() {
        assert_eq!(t("for path in a b\nend"), "for _fish_v_path in a b\ndo\n:\ndone");
        assert_eq!(t("set PATH $PATH /x"), "path=(${path+\"${path[@]}\"} /x)");
    }

    #[test]
    fn control_flow() {
        assert_eq!(t("if test -f a\necho y\nelse if true\necho z\nelse\necho n\nend"),
                   "if __fish_test -f a\nthen\necho y\nelif true\nthen\necho z\nelse\necho n\nfi");
        assert_eq!(t("a; and b; or c"), "a && b || c");
        assert_eq!(t("not a"), "{ a; __fish_nrc=$?; if (( __fish_nrc == 127 )); then (exit 127); else (( __fish_nrc != 0 )); fi; }");
        assert_eq!(t("while true\nbreak\nend"), "while true\ndo\nbreak\ndone");
    }

    #[test]
    fn command_substitution_splits_on_newlines() {
        assert_eq!(t("echo (ls)"), "{ if __fish_inproc ls; then __fish_ipf=$(mktemp \"${TMPDIR:-/tmp}/.zrcs.XXXXXX\"); { ls\n} >$__fish_ipf; __fish_ips=$?; IFS= read -rd '' __fc1 <$__fish_ipf; __fc1+=x; command rm -f $__fish_ipf; (exit $__fish_ips); else __fc1=\"$( ls\n__fish_cs=$?; print -rn x; exit $__fish_cs )\"; fi; echo ${${__fc1%x}:+\"${(@f)${${__fc1%x}%$'\\n'}}\"}; }");
        assert_eq!(t("echo \"$(ls)\""), "echo \"$( ls )\"");
    }

    #[test]
    fn literals_that_zsh_would_glob_are_quoted() {
        assert_eq!(t("echo ?x [a]"), "echo '?x' '[a]'");
        assert_eq!(t("echo ~/a"), "echo ~/a");
        // brace groups are expanded by the translator, the first group varying fastest
        assert_eq!(t("echo {a,b}c"), "echo ac bc");
        assert_eq!(t("echo {+,-}{a,b}"), "echo +a -a +b -b");
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
