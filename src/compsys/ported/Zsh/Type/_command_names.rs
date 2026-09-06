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
//! sh:43  if zstyle -T ":completion:${curcontext}:aliases" verbose; then
//! sh:44    printf -v verbose %s:%s\  ${(@q+)${(kv)aliases}[@]//\:/\\:}
//! sh:45    defs+=( "aliases:alias:(( $verbose ))" )
//! sh:46  else
//! sh:47    defs+=( 'aliases:alias:compadd -Qk aliases' )
//! sh:48  fi
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
use crate::compsys::ported::shared::zstyle_t;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::utils::quotedzputs;

/// `${(kv)aliases}` — key/value pairs of the REGULAR aliases.
///
/// `aliases` is a magic assoc served by `scanpmraliases`
/// (`Src/Modules/parameter.c:1990` → `scanaliases(aliastab, …, alflags=0)`),
/// NOT a plain `PM_HASHED` parameter: it has no entry in the paramtab
/// storage that `gethkparam`/`gethparam` read, so those return nothing and
/// building the sh:44 list from them yields an EMPTY `((…))` action —
/// the `aliases` group then disappears from the completion entirely.
/// Walk the canonical `aliastab` instead, applying the same
/// `al->node.flags == alflags` filter (`parameter.c:1977`) that keeps
/// global and suffix aliases out.
fn regular_aliases() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if let Ok(tab) = crate::ported::hashtable::aliastab_lock().read() {
        for (_, alias) in tab.iter() {
            if alias.node.flags != 0 {
                continue;
            }
            out.push((alias.node.nam.clone(), alias.text.clone()));
        }
    }
    out
}

/// `zstyle -T <ctx> <style>` — true when the style is unset, or set with
/// a boolean-true first value (`Src/Modules/zutil.c:700-724`).
fn style_true_or_unset(ctx: &str, style: &str) -> bool {
    match lookupstyle(ctx, style).first() {
        Some(v) => matches!(v.as_str(), "true" | "yes" | "on" | "1"),
        None => true,
    }
}

/// `${word//\:/\\:}` (sh:44) — backslash-escape every colon so the pair
/// separator printf writes stays the only unescaped one.
fn escape_colons(s: &str) -> String {
    s.replace(':', "\\:")
}

/// Reach `_command_names` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `"(-)1: :{ $cpp; _command_names -e }" \` (Completion/BSD/Command/_mdo sh:30) — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_command_names_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _command_names(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_command_names", args, || {
        _command_names_impl(args)
    })
}

/// `_command_names` — complete a command name. `-e` (first arg)
/// restricts to externals only.
pub fn _command_names_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_command_names");
    // sh:7 `local args defs expl ffilt verbose` — `expl` is the one this
    // port publishes (sh:16 hands it to `_description`); the rest stay
    // Rust locals. Without the declaration the write lands in the GLOBAL
    // param table and survives the completion, so `expl` itself was
    // offered as a parameter name — the same leak `_subscript`'s `ind`
    // had, and `_parameters` filters candidates with `~*local*`.
    let _locals = crate::compsys::ported::shared::LocalScope::declare(
        &["expl"],
        crate::ported::zsh_h::PM_ARRAY,
    );
    let mut ffilt = String::new();
    let curcontext = getsparam("curcontext").unwrap_or_default();

    // sh:9 — `zstyle -t … rehash && rehash`, a VALUE test; see
    //   [`zstyle_t`]. (TODO: dispatch the `rehash` builtin.)
    let _ = zstyle_t(&format!(":completion:{}:commands", curcontext), "rehash");

    // sh:11 — `zstyle -t … prefix-needed`, a VALUE test; see [`zstyle_t`].
    let style_ctx = format!(":completion:{}:functions", curcontext);
    let prefix_needed = zstyle_t(&style_ctx, "prefix-needed") == 0;
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

        // sh:43 — `zstyle -T` is DEFAULT-TRUE, so an unset `verbose` style
        //   takes the verbose branch. The previous port hardcoded the sh:47
        //   `else` arm, i.e. the branch zsh only reaches when the style is
        //   explicitly set false — so every alias was offered bare, with no
        //   expansion shown as its description.
        if style_true_or_unset(&format!(":completion:{}:aliases", curcontext), "verbose") {
            // sh:44  printf -v verbose %s:%s\  ${(@q+)${(kv)aliases}[@]//\:/\\:}
            let mut verbose = String::new();
            for (k, v) in regular_aliases() {
                // `${…//\:/\\:}` — escape every colon so `_describe` splits
                // on the ONE colon printf writes between the pair.
                // `(@q+)` — QT_QUOTEDZPUTS (`Src/subst.c:2245`): quote only
                // when the word needs it, so `_alternative`'s
                // `eval ws=( … )` (sh:39) round-trips values with spaces.
                verbose.push_str(&quotedzputs(&escape_colons(&k)));
                verbose.push(':');
                verbose.push_str(&quotedzputs(&escape_colons(&v)));
                verbose.push(' ');
            }
            // sh:45  defs+=( "aliases:alias:(( $verbose ))" )
            defs.push(format!("aliases:alias:(( {} ))", verbose));
        } else {
            // sh:47  defs+=( 'aliases:alias:compadd -Qk aliases' )
            defs.push("aliases:alias:compadd -Qk aliases".to_string());
        }
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
        assert_eq!(_command_names_impl(&[]), 1);
    }

    #[test]
    fn dash_e_excludes_e_from_args_publication() {
        // sh:26-27 — `-e` consumed; downstream `args` array doesn't
        //   contain it.
        let _g = crate::test_util::global_state_lock();
        let _ = _command_names_impl(&["-e".to_string()]);
        let args = getaparam("args").unwrap_or_default();
        assert!(!args.contains(&"-e".to_string()));
    }
}
