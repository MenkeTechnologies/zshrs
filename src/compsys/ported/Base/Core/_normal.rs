//! Port of `_normal` from `Completion/Base/Core/_normal`.
//!
//! Full upstream body (40 lines verbatim):
//! ```text
//! sh: 1  #compdef -command-line-
//! sh: 3  local _comp_command1 _comp_command2 _comp_command precommand
//! sh: 4  local -A opts
//! sh: 5
//! sh: 6  zparseopts -A opts -D - P p+:-=precommand s
//! sh: 7  (( $+opts[-s] )) || _compskip=
//! sh: 8  (( $+opts[-P] )) && precommands=()
//! sh: 9  (( $#precommand )) && precommands+=(${precommand#-p})
//! sh:14  if [[ -o BANGHIST &&
//! sh:15       ( ( $words[CURRENT] = \!*: && -z $compstate[quote] ) ||
//! sh:16         ( $words[CURRENT] = \"\!*: && $compstate[all_quotes] = \" ) ) ]]; then
//! sh:19    PREFIX=${PREFIX//\\!/!}
//! sh:20    compset -P '*:'
//! sh:21    _history_modifiers h
//! sh:22    return
//! sh:23  fi
//! sh:27  if [[ CURRENT -eq 1 ]]; then
//! sh:28    curcontext="${curcontext%:*:*}:-command-:"
//! sh:30    comp="$_comps[-command-]"
//! sh:31    [[ -n "$comp" ]] && eval "$comp" && return
//! sh:33    return 1
//! sh:34  fi
//! sh:36  _set_command
//! sh:38  _dispatch ${(k)opts[-s]} "$_comp_command" \
//! sh:39            "$_comp_command1" "$_comp_command2" -default-
//! ```

use crate::compsys::ported::_set_command::_set_command;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::bin_zparseopts;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use crate::ported::zle::compcore::get_compstate_str;
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{isset, options, BANGHIST, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `_normal` — `-command-line-` context entry. Strips precommands,
/// handles history-modifier completion, dispatches command-position
/// vs arg-position completion.
pub fn _normal(args: &[String]) -> i32 {
    // sh:6  zparseopts -A opts -D - P p+:-=precommand s
    //   The `-A opts` flag makes opts an assoc; we approximate with
    //   a flat array `opts_flat` of [flag, value, ...] where the
    //   value is empty for boolean flags.
    let src = "__compsys_argv";
    setaparam(src, args.to_vec());
    setaparam("opts_flat", Vec::new());
    setaparam("precommand", Vec::new());
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-D".to_string(),
            "-v".to_string(),
            src.to_string(),
            "-a".to_string(),
            "opts_flat".to_string(),
            "-".to_string(),
            "P".to_string(),
            "p+:-=precommand".to_string(),
            "s".to_string(),
        ],
        &make_ops(),
        0,
    );
    let opts_flat = getaparam("opts_flat").unwrap_or_default();
    let precommand = getaparam("precommand").unwrap_or_default();
    let saw_s = opts_flat.contains(&"-s".to_string());
    let saw_p_cap = opts_flat.contains(&"-P".to_string());

    // sh:7
    if !saw_s {
        let _ = setsparam("_compskip", "");
    }
    // sh:8
    if saw_p_cap {
        setaparam("precommands", Vec::new());
    }
    // sh:9 — push remaining precommand entries (strip "-p" prefix)
    if !precommand.is_empty() {
        let mut current = getaparam("precommands").unwrap_or_default();
        current.extend(
            precommand
                .iter()
                .map(|s| s.trim_start_matches("-p").to_string()),
        );
        setaparam("precommands", current);
    }

    // sh:14-23  history-modifier completion
    let bang_hist = isset(BANGHIST);
    let words = getaparam("words").unwrap_or_default();
    let current: usize = getsparam("CURRENT")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let curword = if current >= 1 && current <= words.len() {
        words[current - 1].clone()
    } else {
        String::new()
    };
    let quote = get_compstate_str("quote").unwrap_or_default();
    let all_quotes = get_compstate_str("all_quotes").unwrap_or_default();
    let bare_bang = curword.starts_with('!') && curword.ends_with(':') && quote.is_empty();
    let quoted_bang = curword.starts_with("\"!") && curword.ends_with(':') && all_quotes == "\"";
    if bang_hist && (bare_bang || quoted_bang) {
        // sh:19
        let prefix = getsparam("PREFIX").unwrap_or_default();
        let _ = setsparam("PREFIX", &prefix.replace("\\!", "!"));
        // sh:20
        let _ = bin_compset(
            "compset",
            &["-P".to_string(), "*:".to_string()],
            &make_ops(),
            0,
        );
        // sh:21
        return dispatch_function_call("_history_modifiers", &["h".to_string()]).unwrap_or(1);
    }

    // sh:27  CURRENT == 1: command-position completion
    if current == 1 {
        let mut curcontext = getsparam("curcontext").unwrap_or_default();
        // sh:28 — strip last two `:` fields and append `:-command-:`
        if let Some(i) = curcontext.rfind(':') {
            if let Some(j) = curcontext[..i].rfind(':') {
                curcontext.truncate(j);
            }
        }
        curcontext.push_str(":-command-:");
        let _ = setsparam("curcontext", &curcontext);

        // sh:30 — look up `$_comps[-command-]`
        let comps = getaparam("_comps").unwrap_or_default();
        let comp = comps
            .chunks(2)
            .find(|kv| kv.first().map(|k| k == "-command-").unwrap_or(false))
            .and_then(|kv| kv.get(1).cloned())
            .unwrap_or_default();
        if !comp.is_empty() {
            // sh:31  eval "$comp" — dispatch via exec_hook
            if dispatch_function_call(&comp, &[]).unwrap_or(1) == 0 {
                return 0;
            }
        }
        // sh:33
        return 1;
    }

    // sh:36
    let _ = _set_command();

    // sh:38-39
    let mut dispatch_argv: Vec<String> = Vec::new();
    if saw_s {
        dispatch_argv.push("-s".to_string());
    }
    dispatch_argv.push(getsparam("_comp_command").unwrap_or_default());
    dispatch_argv.push(getsparam("_comp_command1").unwrap_or_default());
    dispatch_argv.push(getsparam("_comp_command2").unwrap_or_default());
    dispatch_argv.push("-default-".to_string());
    dispatch_function_call("_dispatch", &dispatch_argv).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_in_command_position_without_comps() {
        // sh:27-33 — CURRENT=1, no $_comps[-command-] → return 1.
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("CURRENT", "1");
        setaparam("_comps", Vec::new());
        assert_eq!(_normal(&[]), 1);
    }
}
