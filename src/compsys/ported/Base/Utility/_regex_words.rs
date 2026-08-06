//! Port of `_regex_words` from
//! `Completion/Base/Utility/_regex_words`.
//!
//! Full upstream body (52 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 4  local term=$'\0'
//! sh: 6  while getopts "t:" opt; do …  -t TERM
//! sh:17  shift $(( OPTIND - 1 ))
//! sh:18  local tag=$1 desc=$2; shift 2
//! sh:20  if (( $# )); then reply=( "(" ) else reply=( ); return; fi
//! sh:34  if [[ $term = $'\0' ]]; then
//! sh:28    matches=":${tag}:${desc}:(( "; end="))"
//! sh:29  else matches=":${tag}:${desc}:_values -s ${term} ${desc}"
//! sh:32  for word_i in argv ...
//! sh:41    reply += /pattern/
//! sh:42    matches += word(:desc?)
//! sh:48  reply += /[]/ matches end )
//! ```
//!
//! Generates `$reply` array entries for `_regex_arguments` callers.
//! Translates the human-friendly "word:description:action" spec
//! into a regex/match pair.

use crate::ported::params::{getsparam, setaparam};

/// `_regex_words` — emit `(  /word$term/ … /[]/ matches )` into
/// `$reply` for `_regex_arguments` consumption.
pub fn _regex_words(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_regex_words");
    let mut term = "\0".to_string();
    let mut idx = 0usize;

    // sh:6-15  -t<term>
    while idx + 1 < args.len() {
        if args[idx] == "-t" {
            term = args[idx + 1].clone();
            idx += 2;
        } else {
            break;
        }
    }

    // sh:18
    if args.len() < idx + 2 {
        setaparam("reply", Vec::new());
        return 0;
    }
    let tag = args[idx].clone();
    let desc = args[idx + 1].clone();
    let words_start = idx + 2;
    let word_args: &[String] = &args[words_start..];

    // sh:20
    if word_args.is_empty() {
        setaparam("reply", Vec::new());
        return 0;
    }

    let mut reply: Vec<String> = vec!["(".to_string()];
    // sh:34
    let (mut matches, end): (String, String) = if term == "\0" {
        (format!(":{}:{}:(( ", tag, desc), "))".to_string())
    } else {
        let q = shell_quote(&term);
        let qd = shell_quote(&desc);
        (
            format!(":{}:{}:_values -s {} {}", tag, desc, q, qd),
            String::new(),
        )
    };

    // sh:41-47
    for arg in word_args {
        // wds = split-on-':'
        let wds: Vec<&str> = arg.splitn(3, ':').collect();
        let w0 = wds.first().copied().unwrap_or("");
        let w1 = wds.get(1).copied().unwrap_or("");
        let w2 = wds.get(2).copied().unwrap_or("");

        // sh:43 regex form: /word${term}/ with `*` → `[^term]#`
        let class = if term == "\0" {
            "[^\0]#".to_string()
        } else {
            format!("[^{}]#", term)
        };
        let regex = format!("/{}{}/", w0.replace('*', &class), term);
        reply.push(regex);

        // sh:44 — accumulate matches text
        if term == "\0" {
            let mut m = w0.replace('*', "");
            if !w1.is_empty() {
                m.push_str(&format!("\\:{}", w1));
            }
            m.push(' ');
            matches.push_str(&m);
        } else {
            matches.push_str(&format!(
                " {}\\[{}\\]",
                shell_quote(&w0.replace('*', "")),
                shell_quote(w1),
            ));
        }
        // sh:47  eval "reply+=($wds[3])" — append action expression
        //   (already a parenthesized list); we push verbatim
        reply.push(w2.to_string());
        // sh:48
        reply.push("|".to_string());
    }
    // sh:52
    reply.push("/[]/".to_string());
    reply.push(format!("{}{}", matches, end));
    reply.push(")".to_string());

    setaparam("reply", reply);
    0
}

/// Approximate zsh `${(q)v}` quoting — wrap with single quotes,
/// escape any embedded single quotes.
fn shell_quote(s: &str) -> String {
    let esc = s.replace('\'', "'\\''");
    format!("'{}'", esc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_args_clears_reply() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_regex_words(&[]), 0);
        let reply = crate::ported::params::getaparam("reply").unwrap_or_default();
        assert!(reply.is_empty());
    }

    #[test]
    fn emits_reply_for_simple_spec() {
        let _g = crate::test_util::global_state_lock();
        let _ = _regex_words(&[
            "options".to_string(),
            "option name".to_string(),
            "foo:my foo:".to_string(),
            "bar:my bar:".to_string(),
        ]);
        let reply = crate::ported::params::getaparam("reply").unwrap_or_default();
        assert!(reply.first().map(|s| s == "(").unwrap_or(false));
        assert!(reply.last().map(|s| s == ")").unwrap_or(false));
        let _ = getsparam("PREFIX");
    }
}
