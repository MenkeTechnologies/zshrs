//! Port of `_user_expand` from
//! `Completion/Base/Completer/_user_expand`.
//!
//! Full upstream body (147 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh:14  [[ _matcher_num -gt 1 ]] && return 1
//! sh:18  if [[ "$funcstack[2]" = _prefix ]]; then word="$IPREFIX$PREFIX$SUFFIX"
//! sh:21    else word="$IPREFIX$PREFIX$SUFFIX$ISUFFIX"
//! sh:25  exp=("$word")
//! sh:30  zstyle -a … user-expand specs || return 1
//! sh:32  for spec in $specs; do
//! sh:34    case $spec in
//! sh:36    ($IDENT) eval tmp='${'$spec[2,-1]'[$word]}' …  # assoc lookup
//! sh:42    (_*) reply=(); $spec $word; if reply nonempty: exp=("$reply[@]"); break
//! sh:50    esac
//! sh:51  done
//! sh:54  [[ $#exp -eq 1 && "$exp[1]" = "$word" ]] && return 1
//! sh:80  compadd "$expl[@]" -UQ -qS "$suf" -a exp
//! ```

use crate::compsys::ported::_description::_description;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getiparam, getsparam, setaparam};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `_user_expand` — completer that applies user-defined expansions.
pub fn _user_expand() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_user_expand");
    if getiparam("_matcher_num") > 1 {
        return 1;
    }
    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    let isuffix = getsparam("ISUFFIX").unwrap_or_default();
    let word = format!("{}{}{}{}", iprefix, prefix, suffix, isuffix);

    let curcontext = getsparam("curcontext").unwrap_or_default();
    let specs = lookupstyle(&format!(":completion:{}:", curcontext), "user-expand");
    if specs.is_empty() {
        return 1;
    }

    let mut exp: Vec<String> = vec![word.clone()];
    for spec in &specs {
        if let Some(name) = spec.strip_prefix('$') {
            // sh:36  assoc-lookup
            let arr = getaparam(name).unwrap_or_default();
            let val = arr
                .chunks(2)
                .find(|kv| kv.first().map(|k| k == &word).unwrap_or(false))
                .and_then(|kv| kv.get(1).cloned())
                .unwrap_or_default();
            if !val.is_empty() {
                exp = vec![val];
                break;
            }
        } else if spec.starts_with('_') {
            // sh:42  shell-fn dispatch — fn writes into $reply
            setaparam("reply", Vec::new());
            let _ = dispatch_function_call(spec, &[word.clone()]);
            let reply = getaparam("reply").unwrap_or_default();
            if !reply.is_empty() {
                exp = reply;
                break;
            }
        }
    }

    // sh:54
    if exp.len() == 1 && exp[0] == word {
        return 1;
    }

    // Emit matches via compadd
    setaparam("exp", exp);
    let _ = _description(&[
        "-V".to_string(),
        "expansions".to_string(),
        "expl".to_string(),
        "expansions".to_string(),
        format!("o:{}", word),
    ]);
    let expl = getaparam("expl").unwrap_or_default();
    let mut compadd_argv: Vec<String> = expl;
    compadd_argv.push("-UQ".to_string());
    compadd_argv.push("-a".to_string());
    compadd_argv.push("exp".to_string());
    bin_compadd("compadd", &compadd_argv, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setsparam;

    #[test]
    fn returns_one_when_user_expand_unset() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "");
        let _ = setsparam("SUFFIX", "");
        let _ = setsparam("IPREFIX", "");
        let _ = setsparam("ISUFFIX", "");
        crate::ported::params::setiparam("_matcher_num", 1);
        assert_eq!(_user_expand(), 1);
    }
}
