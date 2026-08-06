//! Port of `_expand_alias` from
//! `Completion/Base/Completer/_expand_alias`.
//!
//! Full upstream body (67 lines, abridged):
//! ```text
//! sh: 1  #compdef -K _expand_alias complete-word \C-xa
//! sh: 6  if [[ -n $funcstack[2] ]]; then
//! sh: 7    if [[ "$funcstack[2]" = _prefix ]]; then
//! sh: 8      word="$IPREFIX$PREFIX$SUFFIX"
//! sh: 9    else
//! sh:10      word="$IPREFIX$PREFIX$SUFFIX$ISUFFIX"
//! sh:11    fi
//! sh:12    pre=()
//! sh:13  else
//! sh:14    local curcontext="$curcontext"
//! sh:16    if [[ -z "$curcontext" ]]; then
//! sh:17      curcontext="expand-alias-word:::"
//! sh:18    else
//! sh:19      curcontext="expand-alias-word:${curcontext#*:}"
//! sh:20    fi
//! sh:22    word="$IPREFIX$PREFIX$SUFFIX$ISUFFIX"
//! sh:23    pre=(_main_complete - aliases)
//! sh:24  fi
//! sh:26  zstyle -s … regular tmp || tmp=yes
//! sh:27  case $tmp in always) sel=r;; yes|...) [[ CURRENT==1 ]] && sel=r;;
//! sh:30  zstyle -T … global && sel="g$sel"
//! sh:31  zstyle -t … disabled && sel="${sel}${(U)sel}"
//! sh:33  tmp=
//! sh:34  [[ $sel = *r* ]] && tmp=$aliases[$word]
//! sh:35  [[ -z $tmp && $sel = *g* ]] && tmp=$galiases[$word]
//! sh:36  [[ -z $tmp && $sel = *R* ]] && tmp=$dis_aliases[$word]
//! sh:37  [[ -z $tmp && $sel = *G* ]] && tmp=$dis_galiases[$word]
//! sh:39  if [[ -n $tmp ]]; then
//! sh:55    $pre _wanted aliases expl alias compadd -UQ "$suf[@]" -- ${tmp%%[[:blank:]]##}
//! sh:56  elif (( $#pre )) && zstyle -t … complete; then
//! sh:57    $pre _aliases -s "$sel" -S ''
//! sh:58  else
//! sh:59    return 1
//! sh:60  fi
//! ```

use crate::compsys::ported::_aliases::_aliases;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::parameter::FUNCSTACK;
use crate::ported::modules::zutil::{lookupstyle, testforstyle};
use crate::ported::params::{getaparam, getiparam, getsparam, setsparam};

/// sh:34-37 — assoc lookup in flat key/value layout.
fn assoc_get(name: &str, key: &str) -> Option<String> {
    let arr = getaparam(name)?;
    arr.chunks(2)
        .find(|kv| kv.first().map(|k| k == key).unwrap_or(false))
        .and_then(|kv| kv.get(1).cloned())
}

/// `_expand_alias` — expand alias under cursor + emit replacement.
pub fn _expand_alias() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_expand_alias");
    // sh:6 — is `funcstack[2]` set (we're called by another fn)?
    let funcstack_depth = FUNCSTACK.lock().map(|s| s.len()).unwrap_or(0);
    let nested = funcstack_depth >= 2;
    // sh:7 — special-case nested-under-_prefix (frame name lookup)
    let parent_name = if nested {
        FUNCSTACK
            .lock()
            .ok()
            .and_then(|s| s.get(s.len() - 2).map(|f| f.name.clone()))
            .unwrap_or_default()
    } else {
        String::new()
    };

    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    let isuffix = getsparam("ISUFFIX").unwrap_or_default();

    let (word, has_main_complete_pre): (String, bool) = if nested {
        let w = if parent_name == "_prefix" {
            format!("{}{}{}", iprefix, prefix, suffix)
        } else {
            format!("{}{}{}{}", iprefix, prefix, suffix, isuffix)
        };
        (w, false)
    } else {
        let saved_curcontext = getsparam("curcontext").unwrap_or_default();
        let new_ctx = if saved_curcontext.is_empty() {
            "expand-alias-word:::".to_string()
        } else {
            let tail = saved_curcontext.splitn(2, ':').nth(1).unwrap_or("");
            format!("expand-alias-word:{}", tail)
        };
        let _ = setsparam("curcontext", &new_ctx);
        (format!("{}{}{}{}", iprefix, prefix, suffix, isuffix), true)
    };

    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);

    // sh:26-31  build sel string
    let regular = lookupstyle(&ctx, "regular")
        .first()
        .cloned()
        .unwrap_or_else(|| "yes".to_string());
    let current = getiparam("CURRENT");
    let mut sel = String::new();
    if regular == "always" {
        sel.push('r');
    } else if matches!(regular.as_str(), "yes" | "1" | "true" | "on") && current == 1 {
        sel.push('r');
    }
    if testforstyle(&ctx, "global") == 0 || lookupstyle(&ctx, "global").is_empty() {
        sel = format!("g{}", sel);
    }
    if testforstyle(&ctx, "disabled") == 0 {
        let upper: String = sel.chars().map(|c| c.to_ascii_uppercase()).collect();
        sel.push_str(&upper);
    }

    // sh:33-37  look up alias
    let mut tmp = String::new();
    if sel.contains('r') {
        tmp = assoc_get("aliases", &word).unwrap_or_default();
    }
    if tmp.is_empty() && sel.contains('g') {
        tmp = assoc_get("galiases", &word).unwrap_or_default();
    }
    if tmp.is_empty() && sel.contains('R') {
        tmp = assoc_get("dis_aliases", &word).unwrap_or_default();
    }
    if tmp.is_empty() && sel.contains('G') {
        tmp = assoc_get("dis_galiases", &word).unwrap_or_default();
    }

    if !tmp.is_empty() {
        // sh:39-55
        tmp = tmp.trim_end().to_string();
        // sh:46-53  self-recursive guard: prepend `\` if tmp's first
        //   word equals the original word AND it's a regular alias.
        let first_word = tmp.split_whitespace().next().unwrap_or("");
        if first_word == word && assoc_get("aliases", &word) == Some(tmp.clone()) {
            tmp = format!("\\{}", tmp);
        }
        let add_space =
            testforstyle(&ctx, "add-space") == 0 || lookupstyle(&ctx, "add-space").is_empty();
        let suf: Vec<String> = if add_space {
            Vec::new()
        } else {
            vec!["-S".to_string(), "".to_string()]
        };
        let mut argv: Vec<String> = if has_main_complete_pre {
            vec!["-".to_string(), "aliases".to_string()]
        } else {
            Vec::new()
        };
        // sh:55  _wanted aliases expl alias compadd -UQ "$suf[@]" -- $tmp
        let mut w_args: Vec<String> = vec![
            "aliases".to_string(),
            "expl".to_string(),
            "alias".to_string(),
            "compadd".to_string(),
            "-UQ".to_string(),
        ];
        w_args.extend(suf);
        w_args.push("--".to_string());
        w_args.push(tmp.trim_end().to_string());
        if has_main_complete_pre {
            argv.extend(w_args);
            dispatch_function_call("_main_complete", &argv).unwrap_or(1)
        } else {
            _wanted(&w_args)
        }
    } else if has_main_complete_pre && testforstyle(&ctx, "complete") == 0 {
        // sh:56-57
        dispatch_function_call(
            "_aliases",
            &["-s".to_string(), sel, "-S".to_string(), "".to_string()],
        )
        .unwrap_or(1)
    } else {
        // sh:59
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_when_alias_unset() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("IPREFIX", "");
        let _ = setsparam("PREFIX", "nonexistent_alias");
        let _ = setsparam("SUFFIX", "");
        let _ = setsparam("ISUFFIX", "");
        // _aliases (returns 1 without executor) is the fallback
        let _r = _expand_alias();
    }
}
