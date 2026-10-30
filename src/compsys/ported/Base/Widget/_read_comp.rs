//! Port of `_read_comp` from `Completion/Base/Widget/_read_comp`
//! (`#compdef -k complete-word \C-x\C-r`).
//!
//! Prompts in the status line (`zle -R "Completion: …"`), reads keys with
//! `read -k` until RET, offering rudimentary completion of `_`-function
//! names, then evals the string — `_files -/` runs as a command, anything
//! else as `compadd ARGS`. The string is kept in the global `$_read_comp`
//! and reused by the next call unless a numeric argument is given.
//!
//! Every step runs the real builtin (`read`, `zle -R`, `whence`, `print`)
//! and `eval`, so key handling, the status line and the eval'd command go
//! through exactly the substrate the shell function uses.

use crate::compsys::ported::shared::{
    capture_builtin_stdout, declare_locals, eval_comp, LocalOptions,
};
use crate::ported::builtin::{bin_read, bin_whence};
use crate::ported::params::{getsparam, setsparam};
use crate::ported::zle::zle_thingy::bin_zle_refresh;
use crate::ported::zsh_h::{isset, options, AUTOLIST, BEEP, MAX_OPS};

fn make_ops(flags: &[u8]) -> options {
    let mut ops = options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    };
    for &f in flags {
        ops.ind[f as usize] = 1;
    }
    ops
}

/// `zle -R MSG LIST…` (`-c` too when `clear`): show MSG in the status line,
/// LIST below it.
fn zle_refresh(clear: bool, msg: &str, list: &[String]) {
    let mut args = vec![msg.to_string()];
    args.extend_from_slice(list);
    let ops = make_ops(if clear { b"c" } else { b"" });
    let _ = bin_zle_refresh("zle", &args, &ops, 0);
}

/// `read -k key` — one key into `$key`; `None` when the read fails.
fn read_key() -> Option<String> {
    let ops = make_ops(b"k");
    if bin_read("read", &["key".to_string()], &ops, 0) != 0 {
        return None;
    }
    Some(getsparam("key").unwrap_or_default())
}

/// `${$(whence -m "$str*" 2>/dev/null)%: function}` — the function names
/// (and other commands) matching `$str*`, word-split.
fn whence_m(prefix: &str) -> Vec<String> {
    let out = capture_builtin_stdout(true, || {
        let _ = bin_whence("whence", &[format!("{}*", prefix)], &make_ops(b"m"), 0);
    });
    out.split_whitespace()
        .map(|w| w.strip_suffix(": function").unwrap_or(w).to_string())
        .collect()
}

/// `print -nb $ZBEEP` / `print -n "\a"` (sh:74-78, sh:97-101): "We can't
/// call zle beep, because this isn't a zle widget."
fn beep() {
    use std::io::Write;
    let zbeep = getsparam("ZBEEP").unwrap_or_default();
    let mut out = std::io::stdout();
    if !zbeep.is_empty() {
        let ops = make_ops(b"nb");
        let _ = crate::ported::builtin::bin_print("print", &[zbeep], &ops, 0);
    } else if isset(BEEP) {
        let _ = out.write_all(b"\x07");
    }
    let _ = out.flush();
}

/// `[[ $str = _[^\ ]# ]]` — a `_`-word with no space in it.
fn is_func_word(s: &str) -> bool {
    s.starts_with('_') && !s.contains(' ')
}

pub fn _read_comp() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_read_comp");
    // sh:26 — `setopt localoptions extendedglob nobadpattern unset`; the
    // eval'd command (sh:32/34/149/151) runs under these too.
    let _local_options = LocalOptions::enter();
    for (opt, on) in [("extendedglob", true), ("badpattern", false), ("unset", true)] {
        crate::ported::options::opt_state_set(opt, on);
    }

    // sh:29 — `typeset -g _read_comp`
    let cached = getsparam("_read_comp").unwrap_or_default();
    if getsparam("_read_comp").is_none() {
        let _ = setsparam("_read_comp", "");
    }
    // sh:30-36 — reuse the remembered string unless a numeric arg was given.
    if getsparam("NUMERIC").is_none() && !cached.is_empty() {
        return if cached.starts_with('_') {
            eval_comp(&cached, 32)
        } else {
            eval_comp(&format!("compadd {}", cached), 34)
        };
    }

    // sh:39
    let _ = setsparam("_read_comp", "");

    // sh:41-42 — `local key …` (key is what `read -k` fills).
    declare_locals(&["key"], 0);
    let msg = "Completion: ";
    let mut s = String::new();
    let mut exact = "";
    let mut list: Vec<String> = Vec::new();

    // sh:46
    zle_refresh(false, msg, &[]);

    // sh:48-51
    let Some(mut key) = read_key() else {
        zle_refresh(true, "", &[]);
        return 1;
    };

    // sh:53 — `'#key'` is the code of the key's first character.
    let code = |k: &str| k.chars().next().map(|c| c as u32).unwrap_or(0);
    while code(&key) != 10 && code(&key) != 13 {
        let c = code(&key);
        // sh:54 — `'#key' -eq 0 && '#key' -eq 3 || '#key' -eq 7`
        if (c == 0 && c == 3) || c == 7 {
            zle_refresh(true, "", &[]);
            return 1;
        }
        if (c == 8 || c == 127) && !s.is_empty() {
            // sh:58-62 — delete character
            s.pop();
            exact = "";
            list.clear();
        } else if c == 21 {
            // sh:63-67 — ^U: delete line
            s.clear();
            exact = "";
            list.clear();
        } else if c == 4 && is_func_word(&s) {
            // sh:68-70 — ^D: list completions
            list = whence_m(&s);
        } else if (!exact.is_empty() && key != " ") || (c & 127) < 32 {
            // sh:71-79 — after an exact function only a space is allowed;
            // non-printing characters are never inserted.
            beep();
            list.clear();
        } else {
            // sh:81
            s.push_str(&key);
            if is_func_word(&s) {
                // sh:82-90 — rudimentary completion of function names.
                let funcs = whence_m(&s);
                list = if isset(AUTOLIST) && s.chars().count() > 1 {
                    funcs.clone()
                } else {
                    Vec::new()
                };
                if funcs.len() == 1 {
                    // sh:91-94 — exact match; prompt for a newline to confirm.
                    s = funcs[0].clone();
                    exact = " (Confirm)";
                } else if funcs.is_empty() {
                    // sh:95-103
                    beep();
                    s.pop();
                    list.clear();
                } else {
                    // sh:104-118 — extend to the longest unambiguous prefix:
                    // `funcs2=(${funcs##$str2*})` keeps the names that do
                    // NOT start with `$str2`.
                    let first: Vec<char> = funcs[0].chars().collect();
                    let mut str2 = s.clone();
                    let mut pos = str2.chars().count();
                    loop {
                        pos += 1;
                        let Some(&newch) = first.get(pos - 1) else {
                            break;
                        };
                        str2.push(newch);
                        if funcs.iter().any(|f| !f.starts_with(str2.as_str())) {
                            break;
                        }
                        s = str2.clone();
                    }
                }
            } else {
                exact = "";
            }
        }
        // sh:124-128
        let line = format!("{}{}{}", msg, s, exact);
        if !list.is_empty() {
            zle_refresh(false, &line, &list);
        } else {
            zle_refresh(true, &line, &[]);
        }
        // sh:129-132
        match read_key() {
            Some(k) => key = k,
            None => {
                zle_refresh(true, "", &[]);
                return 1;
            }
        }
    }

    // sh:135-140 — the string must be non-empty, and a function must be
    // known to the shell (`whence ${str%% *} >& /dev/null`).
    if s.is_empty() {
        return 1;
    }
    if s.starts_with('_') {
        let cmd = s.split(' ').next().unwrap_or("").to_string();
        let mut status = 1;
        let _ = capture_builtin_stdout(true, || {
            status = bin_whence("whence", &[cmd], &make_ops(b""), 0);
        });
        if status != 0 {
            return 1;
        }
    }
    // sh:143 — remember the string for re-use.
    let _ = setsparam("_read_comp", &s);

    // sh:146
    zle_refresh(true, "", &[]);

    // sh:148-152
    if s.starts_with('_') {
        eval_comp(&s, 149)
    } else {
        eval_comp(&format!("compadd {}", s), 151)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn func_word_is_underscore_word_without_space() {
        assert!(is_func_word("_files"));
        assert!(!is_func_word("_files -/"));
        assert!(!is_func_word("files"));
    }
}
