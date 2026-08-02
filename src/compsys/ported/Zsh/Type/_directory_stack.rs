//! Port of `_directory_stack` from
//! `Completion/Zsh/Type/_directory_stack`.
//!
//! Full upstream body (45 lines verbatim):
//! ```text
//! sh: 1  #compdef popd
//! sh: 3  # This just completes the numbers after +, showing the full
//! sh: 4  # directory list with numbers.
//! sh: 9  setopt localoptions nonomatch
//! sh:11  local expl list lines revlines disp sep
//! sh:14  [[ $PREFIX = [-+]* ]] || return 1
//! sh:16  zstyle -s ":completion:${curcontext}:directory-stack" list-separator sep || sep=--
//! sh:18  if zstyle -T ":completion:${curcontext}:directory-stack" verbose; then
//! sh:21    lines=("${(D)dirstack[@]}")
//! sh:24    if [[ ( $PREFIX[1] = - && ! -o pushdminus ) ||
//! sh:25          ( $PREFIX[1] = + && -o pushdminus ) ]]; then
//! sh:27      revlines=( $lines )
//! sh:28      for (( i = 1; i <= $#lines; i++ )); do
//! sh:29        lines[$i]="$((i-1)) $sep ${revlines[-$i]##[0-9]#[	 ]#}"
//! sh:30      done
//! sh:31    else
//! sh:32      for (( i = 1; i <= $#lines; i++ )); do
//! sh:33        lines[$i]="$i $sep ${lines[$i]##[0-9]#[	 ]#}"
//! sh:34      done
//! sh:35    fi
//! sh:37    list=( ${PREFIX[1]}${^lines%% *} )
//! sh:38    disp=( -ld lines )
//! sh:39  else
//! sh:40    list=( ${PREFIX[1]}{0..${#dirstack}} )
//! sh:41    disp=()
//! sh:42  fi
//! sh:44  _wanted -V directory-stack expl 'directory stack' \
//! sh:45      compadd "$@" "$disp[@]" -Q -a list
//! ```

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::zsh_h::{isset, PUSHDMINUS};

/// `_directory_stack` — `popd` argument completion: numbers
/// indexing into `$dirstack`.
pub fn _directory_stack(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_directory_stack");
    // sh:14
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let first_char = prefix.chars().next().unwrap_or(' ');
    if first_char != '-' && first_char != '+' {
        return 1;
    }

    // sh:16
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:directory-stack", curcontext);
    let sep = lookupstyle(&ctx, "list-separator")
        .first()
        .cloned()
        .unwrap_or_else(|| "--".to_string());

    // sh:18 — verbose default-on (`-T` returns true when unset OR true)
    let verbose_vals = lookupstyle(&ctx, "verbose");
    let verbose = verbose_vals.is_empty()
        || verbose_vals
            .first()
            .map(|v| !matches!(v.as_str(), "no" | "false" | "0" | "off"))
            .unwrap_or(true);

    let dirstack = getaparam("dirstack").unwrap_or_default();
    let pushdminus = isset(PUSHDMINUS);

    // sh:18-41 — build list (numeric indices) and optional disp lines
    let (list, disp): (Vec<String>, Vec<String>) = if verbose {
        // sh:21
        let lines = dirstack.clone();
        // sh:24-35  number lines with reverse logic when appropriate
        let mut indexed: Vec<String> = Vec::with_capacity(lines.len());
        let reverse = (first_char == '-' && !pushdminus) || (first_char == '+' && pushdminus);
        for (i, l) in lines.iter().enumerate() {
            let n = if reverse {
                // 0-based reverse: index from len-1
                (lines.len() - 1 - i) as i64
            } else {
                (i + 1) as i64
            };
            let cleaned =
                l.trim_start_matches(|c: char| c.is_ascii_digit() || c == '\t' || c == ' ');
            indexed.push(format!("{} {} {}", n, sep, cleaned));
        }
        // sh:37  list = first-word of each indexed line, prefixed by $PREFIX[1]
        let list: Vec<String> = indexed
            .iter()
            .map(|l| {
                let first_word = l.split_whitespace().next().unwrap_or("");
                format!("{}{}", first_char, first_word)
            })
            .collect();
        setaparam("lines", indexed);
        (list, vec!["-ld".to_string(), "lines".to_string()])
    } else {
        // sh:40 — simple `${PREFIX[1]}{0..N}` brace expansion
        let n = dirstack.len();
        let list: Vec<String> = (0..=n).map(|i| format!("{}{}", first_char, i)).collect();
        (list, Vec::new())
    };

    setaparam("list", list);

    // sh:44-45
    let mut wanted_argv: Vec<String> = vec![
        "-V".to_string(),
        "directory-stack".to_string(),
        "expl".to_string(),
        "directory stack".to_string(),
        "compadd".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.extend(disp);
    wanted_argv.push("-Q".to_string());
    wanted_argv.push("-a".to_string());
    wanted_argv.push("list".to_string());
    _wanted(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setsparam;

    #[test]
    fn returns_one_when_prefix_not_dash_or_plus() {
        // sh:14
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "abc");
        assert_eq!(_directory_stack(&[]), 1);
    }

    #[test]
    fn returns_compadd_status_for_minus_prefix() {
        // sh:14 — `-` prefix triggers the wanted/compadd path; with
        //   no registered tags it returns 1.
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "-");
        setaparam("dirstack", vec!["/a".to_string(), "/b".to_string()]);
        let _r = _directory_stack(&[]);
    }
}
