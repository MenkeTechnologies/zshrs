//! Port of `_command_names` from `Completion/Zsh/Type/_command_names`.
//!
//! Full upstream body (~60 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  # The option `-e' if given as the first argument says that we should
//! sh: 4  # complete only external commands and executable files. This and a
//! sh: 5  # `-' as the first argument is then removed from the arguments.
//! sh: 7  local args defs expl ffilt verbose
//! sh: 9  zstyle -t ":completion:${curcontext}:commands" rehash && rehash
//! sh:11  zstyle -t ":completion:${curcontext}:functions" prefix-needed && \
//! sh:12   [[ $PREFIX != [_.]* ]] && \
//! sh:13   ffilt='[(I)[^_.]*]'
//! sh:15  defs=(
//! sh:16    'commands:external command:_path_commands'
//! sh:17  )
//! sh:19  if [[ -n "$path[(r).]" || $PREFIX = */* ]]; then
//! sh:20    defs+=( 'executables:executable file:_files -g \*\(-\*\)' )
//! sh:21  else
//! sh:23    _description executables expl 'executable file'
//! sh:24  fi
//! sh:26  if [[ "$1" = -e ]]; then
//! sh:27    shift
//! sh:28  elif (( ${#precommands:|builtin_precommands} )); then
//! sh:29    # precommand excludes internal options below
//! sh:30  else
//! sh:31    [[ "$1" = - ]] && shift
//! sh:33    defs=( "$defs[@]"
//! sh:34      'builtins:builtin command:compadd -Qk builtins'
//! sh:35      "functions:shell function:compadd -k 'functions$ffilt'"
//! sh:36      'suffix-aliases:suffix alias:_suffix_alias_files'
//! sh:37      'reserved-words:reserved word:compadd -Qk reswords'
//! sh:38      'jobs:: _jobs -t'
//! sh:39      'parameters:: _parameters …'
//! sh:40      'parameters:: _parameters …'
//! sh:41    )
//! sh:42-47  aliases (verbose / non-verbose)
//! sh:50  args=( "$@" )
//! sh:52-71  cmdpath / PATH shadowing
//! sh:73  _alternative -O args "$defs[@]"
//! ```
//!
//! Calls real `testforstyle`/`lookupstyle`; dispatches `_description`
//! + `_alternative` via `exec accessors`. The cmdpath PATH-shadow dance
//! at sh:62-71 left as TODO (only fires under `_comp_priv_prefix`
//! ≠ empty, rare).

use crate::compsys::ported::_description::_description;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::{lookupstyle, testforstyle};
use crate::ported::params::{getaparam, getsparam, setaparam};

/// `_command_names` — complete a command name. `-e` (first arg)
/// restricts to externals only.
pub fn _command_names(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_command_names");
    // sh:7
    let mut ffilt = String::new();
    let curcontext = getsparam("curcontext").unwrap_or_default();

    // sh:9 — rehash style (TODO: dispatch `rehash` builtin)
    let _ = testforstyle(&format!(":completion:{}:commands", curcontext), "rehash");

    // sh:11-13
    let style_ctx = format!(":completion:{}:functions", curcontext);
    let prefix_needed = testforstyle(&style_ctx, "prefix-needed") == 0;
    let prefix = getsparam("PREFIX").unwrap_or_default();
    if prefix_needed && !prefix.starts_with('_') && !prefix.starts_with('.') {
        ffilt = "[(I)[^_.]*]".to_string();
    }

    // sh:15-17
    let mut defs: Vec<String> = vec!["commands:external command:_path_commands".to_string()];

    // sh:19-24
    let path = getaparam("path").unwrap_or_default();
    let path_has_dot = path.iter().any(|p| p == ".");
    if path_has_dot || prefix.contains('/') {
        defs.push("executables:executable file:_files -g \\*\\(-\\*\\)".to_string());
    } else {
        let _ = _description(&[
            "executables".to_string(),
            "expl".to_string(),
            "executable file".to_string(),
        ]);
    }

    // sh:26-48
    let (mut argv, dash_e): (Vec<String>, bool) =
        if args.first().map(|s| s == "-e").unwrap_or(false) {
            (args[1..].to_vec(), true)
        } else {
            (args.to_vec(), false)
        };

    let precommands = getaparam("precommands").unwrap_or_default();
    let builtin_precommands = getaparam("builtin_precommands").unwrap_or_default();
    // sh:28 — `(( ${#precommands:|builtin_precommands} ))`. `${a:|b}` is the
    // set difference (elements of `a` NOT in `b`); the test is true when that
    // difference is NON-EMPTY, i.e. at least one precommand is NOT a builtin
    // precommand. The previous port tested the INTERSECTION (any precommand
    // that IS a builtin precommand) — the inverted condition, so the defs
    // block was included/excluded backwards. Bug #657.
    let precmd_diff_nonempty = precommands.iter().any(|p| !builtin_precommands.contains(p));

    if !dash_e && !precmd_diff_nonempty {
        // sh:31
        if argv.first().map(|s| s == "-").unwrap_or(false) {
            argv.remove(0);
        }
        // sh:33-41
        defs.push("builtins:builtin command:compadd -Qk builtins".to_string());
        defs.push(format!(
            "functions:shell function:compadd -k 'functions{}'",
            ffilt
        ));
        defs.push("suffix-aliases:suffix alias:_suffix_alias_files".to_string());
        defs.push("reserved-words:reserved word:compadd -Qk reswords".to_string());
        defs.push("jobs:: _jobs -t".to_string());
        defs.push(
            "parameters:: _parameters -g \"^*(readonly|association)*\" -qS= -r \"\\n\\t\\- =[+\""
                .to_string(),
        );
        defs.push(
            "parameters:: _parameters -g \"*association*~*readonly*\" -qS\\[ -r \"\\n\\t\\- =[+\""
                .to_string(),
        );

        // sh:42-47 — aliases (verbose mode emission TODO sh:43)
        defs.push("aliases:alias:compadd -Qk aliases".to_string());
    }

    // sh:50  args=( "$@" )
    setaparam("args", argv);

    // sh:52-53
    let _ = lookupstyle(&format!(":completion:{}", curcontext), "command-path");

    // sh:73
    let mut alt_argv: Vec<String> = vec!["-O".to_string(), "args".to_string()];
    alt_argv.extend(defs);
    dispatch_function_call("_alternative", &alt_argv).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_command_names(&[]), 1);
    }

    #[test]
    fn dash_e_excludes_e_from_args_publication() {
        // sh:26-27 — `-e` consumed; downstream `args` array doesn't
        //   contain it.
        let _g = crate::test_util::global_state_lock();
        let _ = _command_names(&["-e".to_string()]);
        let args = getaparam("args").unwrap_or_default();
        assert!(!args.contains(&"-e".to_string()));
    }
}
