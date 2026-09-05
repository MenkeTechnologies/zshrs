//! Port of `_history_modifiers` from
//! `Completion/Zsh/Type/_history_modifiers`.
//!
//! Full upstream body (90 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh:10  local -a list
//! sh:12  local type=$1 delim expl
//! sh:13  integer global
//! sh:15  while true; do
//! sh:16    if [[ -n $PREFIX ]]; then
//! sh:17      local char=$PREFIX[1]
//! sh:19      global=0
//! sh:20      compset -p 1
//! sh:21      case $char in
//! sh:22        ([hretpqQxlu\&]) ;;                 # single character modifiers
//! sh:26        (s) ... _delimiters modifier-s / _message replacement|original ...
//! sh:44        (g) global=1; continue ;;
//! sh:48      esac
//! sh:51      compset -P : && continue
//! sh:53      [[ -n $PREFIX ]] && return 1
//! sh:55      list=("\::modifier"); [[ $type = q ]] && list+=("):end of qualifiers")
//! sh:58      _describe -t delimiters "delimiter" list -Q -S ''
//! sh:59      return
//! sh:60    else
//! sh:61      list=("s:..." "S:..." "&:..."); (( ! global )) && list+=( a A c g h t r e Q P l u ... )
//! sh:87      _describe -t modifiers "modifier" list -Q -S ''
//! sh:88      return
//! sh:89    fi
//! sh:90  done
//! ```
//!
//! Completes history-style word modifiers. `$1` is the context type:
//! `h`=history, `q`=glob qualifier, `p`=parameter. Each `while true`
//! iteration inspects the FIRST char of `$PREFIX`, strips it with
//! `compset -p 1`, and either loops (`g`/colon) or terminates by
//! offering the next modifier / delimiter / substitution list.

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getsparam, setaparam};
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `compset FLAG PAT` → true when the strip matched (shell `compset`
/// returns 0). Mutates the global PREFIX/SUFFIX completion state.
fn compset(argv: &[&str]) -> bool {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    bin_compset("compset", &owned, &make_ops(), 0) == 0
}

/// Reach `_history_modifiers` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_history_modifiers p` (Completion/Zsh/Context/_brace_parameter sh:210) — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_history_modifiers_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _history_modifiers(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_history_modifiers", args, || {
        _history_modifiers_impl(args)
    })
}

/// `_history_modifiers` — complete history modifier letters.
/// `$1` is the context (`h`=history, `q`=glob qualifier,
/// `p`=parameter).
pub fn _history_modifiers_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_history_modifiers");
    // sh:10 — `local -a list`.
    //
    // `list` is the modifier catalogue the port publishes by name for
    // `_describe`. `type`/`delim` stay Rust-side and `expl` (sh:12) is
    // never written by the port, so neither is declared. Without this,
    // `echo !!:<TAB>` left the table standing in the user's shell:
    //
    //   zsh  : list=[][0]        zshrs: list=[array][17]
    crate::compsys::ported::shared::declare_locals(
        &["list"],
        crate::compsys::ported::shared::PM_ARRAY,
    );
    // sh:10-13  locals. `type` is a Rust keyword → raw identifier
    //   keeps the source name verbatim.
    let mut list: Vec<String> = Vec::new();
    let r#type = args.first().cloned().unwrap_or_default();
    let mut delim: String = String::new();
    let mut global: i64 = 0;

    // sh:15
    loop {
        let prefix = getsparam("PREFIX").unwrap_or_default();
        // sh:16  if [[ -n $PREFIX ]]; then
        if !prefix.is_empty() {
            // sh:17  local char=$PREFIX[1]
            let char = prefix.chars().next().unwrap();

            // sh:19  global=0
            global = 0;
            // sh:20  compset -p 1
            compset(&["-p", "1"]);
            // sh:21  case $char in
            match char {
                // sh:22-25  single character modifiers — nothing to do
                'h' | 'r' | 'e' | 't' | 'p' | 'q' | 'Q' | 'x' | 'l' | 'u' | '&' => {}

                // sh:26-42  (s) substitution: delimiter string delimiter
                //   string delimiter
                's' => {
                    let prefix = getsparam("PREFIX").unwrap_or_default();
                    // sh:28  if [[ -z $PREFIX ]]; then
                    if prefix.is_empty() {
                        // sh:29-30  _delimiters modifier-s; return
                        return dispatch_function_call("_delimiters", &["modifier-s".to_string()])
                            .unwrap_or(1);
                    }
                    // sh:32  delim=$PREFIX[1]
                    delim = prefix.chars().next().unwrap().to_string();
                    // sh:33  compset -p 1
                    compset(&["-p", "1"]);
                    // sh:34  if ! compset -P "[^${delim}]#${delim}[^${delim}]#${delim}"
                    let full = format!("[^{d}]#{d}[^{d}]#{d}", d = delim);
                    if !compset(&["-P", full.as_str()]) {
                        // sh:35  if compset -P "[^${delim}]#${delim}"
                        let half = format!("[^{d}]#{d}", d = delim);
                        return if compset(&["-P", half.as_str()]) {
                            // sh:36  _message "replacement string"
                            dispatch_function_call("_message", &["replacement string".to_string()])
                                .unwrap_or(1)
                        } else {
                            // sh:38  _message "original string"
                            dispatch_function_call("_message", &["original string".to_string()])
                                .unwrap_or(1)
                        };
                    }
                }

                // sh:44-47  (g) global flag, restart the loop
                'g' => {
                    global = 1;
                    continue;
                }

                _ => {}
            }

            // sh:50  # modifier completely matched, see what's next.
            // sh:51  compset -P : && continue
            if compset(&["-P", ":"]) {
                continue;
            }
            // sh:52-53  something other than colon next → bummer
            if !getsparam("PREFIX").unwrap_or_default().is_empty() {
                return 1;
            }

            // sh:55  list=("\::modifier")
            list = vec!["\\::modifier".to_string()];
            // sh:56  [[ $type = q ]] && list+=("):end of qualifiers")
            if r#type == "q" {
                list.push("):end of qualifiers".to_string());
            }
            // sh:57-58  _describe -t delimiters "delimiter" list -Q -S ''
            setaparam("list", list);
            return dispatch_function_call(
                "_describe",
                &[
                    "-t".to_string(),
                    "delimiters".to_string(),
                    "delimiter".to_string(),
                    "list".to_string(),
                    "-Q".to_string(),
                    "-S".to_string(),
                    "".to_string(),
                ],
            )
            .unwrap_or(1);
        } else {
            // sh:61-65  top-level modifier list
            list = vec![
                "s:substitute string".to_string(),
                "&:repeat substitution".to_string(),
            ];
            // sh:66  if (( ! global )); then
            if global == 0 {
                // sh:67-80
                list.extend(
                    [
                        "a:absolute path, resolve '..' lexically",
                        "A:as ':a', then resolve symlinks",
                        "c:PATH search for command",
                        "g:globally apply s or &",
                        "h:head - strip trailing path element",
                        "t:tail - strip directories",
                        "r:root - strip suffix",
                        "e:leave only extension",
                        "Q:strip quotes",
                        "P:realpath, resolve '..' physically",
                        "l:lower case all words",
                        "u:upper case all words",
                    ]
                    .into_iter()
                    .map(String::from),
                );
                // sh:81-84  [[ $type = h ]] && list+=( p x )
                if r#type == "h" {
                    list.push("p:print without executing".to_string());
                    list.push("x:quote words, breaking on whitespace".to_string());
                }
                // sh:85  [[ $type = [hp] ]] && list+=("q:...")
                if r#type == "h" || r#type == "p" {
                    list.push("q:quote to escape further substitutions".to_string());
                }
            }
            // sh:87  _describe -t modifiers "modifier" list -Q -S ''
            setaparam("list", list);
            return dispatch_function_call(
                "_describe",
                &[
                    "-t".to_string(),
                    "modifiers".to_string(),
                    "modifier".to_string(),
                    "list".to_string(),
                    "-Q".to_string(),
                    "-S".to_string(),
                    "".to_string(),
                ],
            )
            .unwrap_or(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        // PREFIX empty → sh:60 else branch → `_describe` dispatch,
        //   which returns None (no executor) → `.unwrap_or(1)`.
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PREFIX", "");
        assert_eq!(_history_modifiers_impl(&["h".to_string()]), 1);
    }

    #[test]
    fn publishes_top_level_list_for_describe() {
        // sh:61-85 — the top-level catalog is published into the
        //   `list` shell array for `_describe -t modifiers`.
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PREFIX", "");
        let _ = _history_modifiers_impl(&["h".to_string()]);
        let list = crate::ported::params::getaparam("list").unwrap_or_default();
        // s, & (always) + 12 non-global + p, x (type=h) + q (type=[hp]).
        assert_eq!(list.len(), 17);
        assert_eq!(list[0], "s:substitute string");
        assert_eq!(list[1], "&:repeat substitution");
        assert!(list.contains(&"p:print without executing".to_string()));
        assert!(list.contains(&"q:quote to escape further substitutions".to_string()));
    }

    #[test]
    fn glob_qualifier_type_omits_history_only_entries() {
        // sh:81-85 — the `p`/`x`/`q` entries are gated on the context
        //   type; a `q` (glob-qualifier) context must not see them.
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PREFIX", "");
        let _ = _history_modifiers_impl(&["q".to_string()]);
        let list = crate::ported::params::getaparam("list").unwrap_or_default();
        assert_eq!(list.len(), 14);
        assert!(!list.contains(&"p:print without executing".to_string()));
        assert!(!list.contains(&"q:quote to escape further substitutions".to_string()));
    }
}
