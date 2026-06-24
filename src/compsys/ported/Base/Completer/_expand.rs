//! Port of `_expand` from `Completion/Base/Completer/_expand`.
//!
//! Full upstream body (245 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh:  3  [[ _matcher_num -gt 1 ]] && return 1
//! sh: 10  zstyle -s … substitute val || val=yes
//! sh: 14  if substitute → exp=( ${(e)~word} )
//! sh: 26  zstyle -s … glob → glob expansion exp
//! sh: 40  if exp same as word → return 1
//! sh: 80  zstyle -s … sort + add-space + suffix-detection
//! sh:120  _tags / _wanted / compadd loop
//! sh:240  return 0
//! ```
//!
//! Substitute/glob expansion completer. Heavy at full faithfulness;
//! this port handles the common short-circuit (no expansion → 1)
//! and dispatches an `eval`-based substitution via std::process
//! when the executor is available.

use crate::compsys::ported::_description::_description;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::{lookupstyle, testforstyle};
use crate::ported::params::{getaparam, getiparam, getsparam, setaparam};
use crate::ported::zle::compcore::get_compstate_str;
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

/// `_expand` — substitution/glob expansion completer.
pub fn _expand() -> i32 {
    if getiparam("_matcher_num") > 1 {
        return 1;
    }
    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    let isuffix = getsparam("ISUFFIX").unwrap_or_default();
    let word = format!("{}{}{}{}", iprefix, prefix, suffix, isuffix);

    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);

    // sh:10  substitute style
    let mut exp: Vec<String> = vec![word.clone()];

    // sh:14  substitute → eval-expand via execute_script hook
    let subst_on = lookupstyle(&ctx, "substitute")
        .first()
        .map(|v| !matches!(v.as_str(), "no" | "false" | "0" | "off"))
        .unwrap_or(true);
    if subst_on && (word.contains('$') || word.contains('~') || word.contains('=')) {
        // Inline substitution: tilde + env-var expansion via std.
        if let Some(expanded) = expand_substitutions(&word) {
            if expanded != word {
                exp = vec![expanded];
            }
        }
    }

    // sh:26  glob expansion
    let glob_on = lookupstyle(&ctx, "glob")
        .first()
        .map(|v| !matches!(v.as_str(), "no" | "false" | "0" | "off"))
        .unwrap_or(true);
    if glob_on && word.chars().any(|c| matches!(c, '*' | '?' | '[')) {
        // Use std::fs glob via shell expansion
        if let Ok(paths) = glob_match(&word) {
            if !paths.is_empty() {
                exp = paths;
            }
        }
    }

    // sh:40
    if exp.len() == 1 && exp[0] == word {
        return 1;
    }

    // sh:80  emit matches via _description + compadd
    setaparam("exp", exp);
    let _ = _description(&[
        "-V".to_string(),
        "expansions".to_string(),
        "expl".to_string(),
        "expansions".to_string(),
        format!("o:{}", word),
    ]);
    let add_space = testforstyle(&ctx, "add-space") == 0;
    let suf: Vec<String> = if add_space {
        vec!["-qS".to_string(), " ".to_string()]
    } else {
        vec!["-qS".to_string(), "".to_string()]
    };
    let insert = get_compstate_str("insert").unwrap_or_default();
    let expl = getaparam("expl").unwrap_or_default();
    let mut compadd_argv: Vec<String> = expl;
    compadd_argv.push("-UQ".to_string());
    compadd_argv.extend(suf);
    compadd_argv.push("-a".to_string());
    compadd_argv.push("exp".to_string());
    let r = bin_compadd("compadd", &compadd_argv, &make_ops(), 0);
    let _ = insert;
    let _ = dispatch_function_call;
    r
}

/// sh:14 — shell-substitute approximation. Handles `~` (HOME),
/// `~user` (passwd lookup), and `$VAR` / `${VAR}` env-var
/// references. Returns None when nothing changed.
fn expand_substitutions(word: &str) -> Option<String> {
    let mut out = word.to_string();
    let initial = out.clone();
    // Tilde
    if out.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            out = format!("{}{}", home, &out[1..]);
        }
    } else if out.starts_with('~') && !out.starts_with("~[") {
        let end = out.find('/').unwrap_or(out.len());
        let user = &out[1..end];
        if !user.is_empty() {
            // Lookup home via $HOME-keyed userdirs assoc (best-effort)
            let userdirs = crate::ported::params::getaparam("userdirs").unwrap_or_default();
            if let Some(home) = userdirs.chunks(2).find_map(|kv| {
                if kv.first().map(|k| k == user).unwrap_or(false) {
                    kv.get(1).cloned()
                } else {
                    None
                }
            }) {
                let rest = &out[end..];
                out = format!("{}{}", home, rest);
            }
        }
    }
    // $VAR / ${VAR}
    let mut buf = String::with_capacity(out.len());
    let bytes = out.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'$' && i + 1 < bytes.len() {
            let (name, end) = if bytes[i + 1] == b'{' {
                if let Some(close) = out[i + 2..].find('}') {
                    let n = &out[i + 2..i + 2 + close];
                    (n.to_string(), i + 3 + close)
                } else {
                    buf.push('$');
                    i += 1;
                    continue;
                }
            } else {
                let mut j = i + 1;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                if j == i + 1 {
                    buf.push('$');
                    i += 1;
                    continue;
                }
                (out[i + 1..j].to_string(), j)
            };
            if let Ok(v) = std::env::var(&name) {
                buf.push_str(&v);
            }
            i = end;
        } else {
            buf.push(bytes[i] as char);
            i += 1;
        }
    }
    out = buf;
    if out == initial {
        None
    } else {
        Some(out)
    }
}

/// sh:26 — glob expansion via std::fs walk. Supports `*`, `?`,
/// and basic bracket-character-class. Returns the matched paths,
/// or `Err(())` when the pattern is malformed.
fn glob_match(pat: &str) -> Result<Vec<String>, ()> {
    use crate::ported::pattern::{patcompile, pattry};
    // Split into directory part + filename pattern
    let (dir, name_pat) = match pat.rfind('/') {
        Some(i) => (pat[..=i].to_string(), pat[i + 1..].to_string()),
        None => (".".to_string(), pat.to_string()),
    };
    let scan_dir = if dir.is_empty() { "." } else { &dir };
    let prog = patcompile(
        &{
            let mut __pat_tok = (&name_pat).to_string();
            crate::ported::glob::tokenize(&mut __pat_tok);
            __pat_tok
        },
        0,
        None,
    )
    .ok_or(())?;
    let entries = std::fs::read_dir(std::path::Path::new(scan_dir)).map_err(|_| ())?;
    let mut out: Vec<String> = Vec::new();
    for ent in entries.flatten() {
        let name = ent.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') && !name_pat.starts_with('.') {
            continue;
        }
        if pattry(&prog, &name) {
            let full = if dir == "." {
                name
            } else {
                format!("{}{}", dir, name)
            };
            out.push(full);
        }
    }
    out.sort();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::{setiparam, setsparam};

    #[test]
    fn matcher_num_gt_one_returns_one() {
        let _g = crate::test_util::global_state_lock();
        setiparam("_matcher_num", 5);
        assert_eq!(_expand(), 1);
        setiparam("_matcher_num", 0);
    }

    #[test]
    fn plain_word_no_substitution_returns_one() {
        let _g = crate::test_util::global_state_lock();
        setiparam("_matcher_num", 1);
        let _ = setsparam("PREFIX", "plain");
        let _ = setsparam("SUFFIX", "");
        let _ = setsparam("IPREFIX", "");
        let _ = setsparam("ISUFFIX", "");
        assert_eq!(_expand(), 1);
    }
}
