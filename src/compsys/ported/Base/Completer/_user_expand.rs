//! Port of `_user_expand` from
//! `Completion/Base/Completer/_user_expand`.
//!
//! Full upstream body (147 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh:13  [[ _matcher_num -gt 1 ]] && return 1
//! sh:18  if [[ "$funcstack[2]" = _prefix ]]; then
//! sh:19    word="$IPREFIX$PREFIX$SUFFIX"
//! sh:20  else
//! sh:21    word="$IPREFIX$PREFIX$SUFFIX$ISUFFIX"
//! sh:22  fi
//! sh:26  exp=("$word")
//! sh:30  zstyle -a … user-expand specs || return 1
//! sh:32  for spec in $specs; do
//! sh:34    case $spec in
//! sh:36    ($IDENT) eval tmp='${'$spec[2,-1]'[$word]}' …  # assoc lookup
//! sh:42    (_*) reply=(); $spec $word; if reply nonempty: exp=("$reply[@]"); break
//! sh:54    esac
//! sh:55  done
//! sh:57  [[ $#exp -eq 1 && "$exp[1]" = "$word" ]] && return 1
//! sh:94  compadd "$expl[@]" -UQ -qS "$suf" -a exp
//! ```

use crate::compsys::ported::_description::_description;
use crate::compsys::ported::shared::caller_is_prefix;
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
    // sh:18-22 — `_prefix` has already moved the whole `$SUFFIX` into
    // `$ISUFFIX` (`_prefix` sh:18-23) before re-running the completer list,
    // so appending `$ISUFFIX` under that caller hands the word back the
    // suffix `_prefix` just removed. `_expand` sh:22 and `_expand_alias`
    // sh:9 carry the same branch; this port had only the else arm.
    let word = if caller_is_prefix() {
        format!("{}{}{}", iprefix, prefix, suffix) // sh:19
    } else {
        format!("{}{}{}{}", iprefix, prefix, suffix, isuffix) // sh:21
    };

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

    // sh:57
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
    use crate::ported::modules::zutil::bin_zstyle;
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

    /// sh:18-22 — under `_prefix` the word is `$IPREFIX$PREFIX$SUFFIX` with
    /// NO `$ISUFFIX`, because `_prefix` moved the suffix there itself
    /// (`_prefix` sh:18-23) before re-running the completer list. The port
    /// always appended `$ISUFFIX`, so it looked up the wrong key and the
    /// expansion silently did not fire.
    ///
    /// Measured on a PTY with `setopt complete_in_word`,
    /// `completer _user_expand _prefix` and the cursor three left of the end
    /// of `echo foobar`, logging `$1` from the `user-expand` spec function:
    ///
    /// ```text
    ///   pass 2 (funcstack _user_expand,_prefix,_main_complete)
    ///     zsh    word=<foo>
    ///     zshrs  word=<foobar>     <- before
    ///     zshrs  word=<foo>        <- after
    /// ```
    #[test]
    fn prefix_caller_drops_isuffix_from_the_word() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("IPREFIX", "");
        let _ = setsparam("PREFIX", "foo");
        let _ = setsparam("SUFFIX", "");
        let _ = setsparam("ISUFFIX", "bar");
        let _ = setsparam("curcontext", "");
        crate::ported::params::setiparam("_matcher_num", 1);
        // sh:36's `$IDENT` arm reads the named parameter and pairs it up as
        // key/value, so a flat array stands in for the assoc here.
        setaparam(
            "ZZQ_UE_MAP",
            vec!["foo".to_string(), "ZZQ_HIT".to_string()],
        );
        setaparam("exp", Vec::new());
        let _ = bin_zstyle(
            "zstyle",
            &[
                ":completion:*".to_string(),
                "user-expand".to_string(),
                "$ZZQ_UE_MAP".to_string(),
            ],
            &make_ops(),
            0,
        );

        // No `_prefix` frame: word is `foo` + `bar`, the map misses, `exp`
        // still holds the word and sh:57 bails.
        assert_eq!(_user_expand(), 1, "sh:21 arm must look up `foobar`");

        // Under `_prefix`: word is `foo`, the map hits, and sh:26's `exp`
        // reaches the compadd at sh:94 carrying the expansion.
        // `funcstackgetfn` (c:Src/Modules/parameter.c:627) hands the frames
        // back innermost-first, so `funcstack[2]` is index 1 — the CALLER of
        // the completer. Reproduce the live shape the PTY run showed,
        // `_user_expand,_prefix,_main_complete`, with the two frames that
        // matter; only `doshfunc` pushes these, and no executor runs here.
        {
            let mut stack = crate::ported::modules::parameter::FUNCSTACK.lock().unwrap();
            for name in ["_prefix", "_user_expand"] {
                stack.push(crate::ported::zsh_h::funcstack {
                    prev: None,
                    name: name.to_string(),
                    filename: None,
                    caller: None,
                    flineno: 0,
                    lineno: 0,
                    tp: 0,
                });
            }
        }
        let _ = _user_expand();
        {
            let mut stack = crate::ported::modules::parameter::FUNCSTACK.lock().unwrap();
            stack.pop();
            stack.pop();
        }
        assert_eq!(
            getaparam("exp").unwrap_or_default(),
            vec!["ZZQ_HIT".to_string()],
            "sh:19 arm must look up `foo`, not `foo`+$ISUFFIX"
        );
    }
}
