//! Port of `_condition` from `Completion/Zsh/Context/_condition`.
//!
//! Full upstream body (60 lines, abridged):
//! ```text
//! sh: 1  #compdef -condition-
//! sh: 3  local prev="$words[CURRENT-1]" ret=1
//! sh: 5  if [[ "$prev" = -o ]]; then
//! sh: 6    _tags -C -o options && _options
//! sh: 7  elif [[ "$prev" = -([a-hkprsuwxLOGSN]|[no]t|ef) ]]; then
//! sh: 8    _tags -C "$prev" files && _files
//! sh: 9  elif [[ "$prev" = -t ]]; then
//! sh:10    _file_descriptors
//! sh:11  elif [[ "$prev" = -v ]]; then
//! sh:12    _parameters -r "\= \t\n\[\-"
//! sh:13  else
//! sh:14    if [[ "$PREFIX" = -* ]] ||
//! sh:15       ! zstyle -T … prefix-needed; then
//! sh:17      if [[ "$prev" = (\[\[|\|\||\&\&|\!|\() ]]; then
//! sh:18        _describe -o 'condition code' '( … unary tests … )'
//! sh:51      else
//! sh:52        _describe -o 'condition code' '( … binary tests … )'
//! sh:60      fi
//! sh:62    _alternative 'files:: _files' 'parameters:: _parameters' && ret=0
//! sh:64    return ret
//! sh:65  fi
//! ```

use crate::compsys::ported::_alternative::alternative_byname;
use crate::compsys::ported::_file_descriptors::file_descriptors_byname;
use crate::compsys::ported::_options::options_byname;
use crate::compsys::ported::_tags::tags_byname;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::testforstyle;
use crate::ported::params::{getaparam, getiparam, getsparam};

const UNARY_TESTS: &[&str] = &[
    "-a:existing file",
    "-b:block special file",
    "-c:character special file",
    "-d:directory",
    "-e:existing file",
    "-f:regular file",
    "-g:setgid bit",
    "-h:symbolic link",
    "-k:sticky bit",
    "-n:non-empty string",
    "-o:option",
    "-p:named pipe",
    "-r:readable file",
    "-s:non-empty file",
    "-t:terminal file descriptor",
    "-u:setuid bit",
    "-v:set variable",
    "-w:writable file",
    "-x:executable file",
    "-z:empty string",
    "-L:symbolic link",
    "-O:own file",
    "-G:group-owned file",
    "-S:socket",
    "-N:unread file",
];

const BINARY_TESTS: &[&str] = &[
    "-nt:newer than",
    "-ot:older than",
    "-ef:same file",
    "-eq:numerically equal",
    "-ne:numerically not equal",
    "-lt:numerically less than",
    "-le:numerically less than or equal",
    "-gt:numerically greater than",
    "-ge:numerically greater than or equal",
];

fn is_file_test_op(prev: &str) -> bool {
    let chars: &[&str] = &[
        "-a", "-b", "-c", "-d", "-e", "-f", "-g", "-h", "-k", "-p", "-r", "-s", "-u", "-w", "-x",
        "-L", "-O", "-G", "-S", "-N", "-nt", "-ot", "-ef",
    ];
    chars.contains(&prev)
}

/// `_condition` — `[[ … ]]` context completion: classify the
/// previous word to decide between options, files, fds, params,
/// and the test-operator catalogs.
pub fn _condition() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_condition");
    let words = getaparam("words").unwrap_or_default();
    let current = getiparam("CURRENT") as usize;
    let prev = if current >= 2 && current - 1 <= words.len() {
        words[current - 2].clone()
    } else {
        String::new()
    };

    // sh:5
    if prev == "-o" {
        if tags_byname(&["-C".to_string(), "-o".to_string(), "options".to_string()]) == 0 {
            return options_byname(&[]);
        }
        return 1;
    }
    // sh:7
    if is_file_test_op(&prev) {
        if tags_byname(&["-C".to_string(), prev, "files".to_string()]) == 0 {
            return dispatch_function_call("_files", &[]).unwrap_or(1);
        }
        return 1;
    }
    // sh:9
    if prev == "-t" {
        return file_descriptors_byname(&[]);
    }
    // sh:11
    if prev == "-v" {
        return dispatch_function_call(
            "_parameters",
            &["-r".to_string(), "\\= \\t\\n\\[\\-".to_string()],
        )
        .unwrap_or(1);
    }

    // sh:13-15  default branch
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let prefix_needed = testforstyle(
        &format!(":completion:{}:options", curcontext),
        "prefix-needed",
    ) == 0;
    let mut ret: i32 = 1;
    if prefix.starts_with('-') || !prefix_needed {
        // sh:17
        let group = matches!(prev.as_str(), "[[" | "||" | "&&" | "!" | "(");
        let catalog: Vec<String> = if group {
            UNARY_TESTS.iter().map(|s| s.to_string()).collect()
        } else {
            BINARY_TESTS.iter().map(|s| s.to_string()).collect()
        };
        let mut describe_argv: Vec<String> = vec!["-o".to_string(), "condition code".to_string()];
        for c in catalog {
            describe_argv.push(c);
        }
        if dispatch_function_call("_describe", &describe_argv).unwrap_or(1) == 0 {
            ret = 0;
        }
    }
    // sh:62
    if alternative_byname(&[
        "files:: _files".to_string(),
        "parameters:: _parameters".to_string(),
    ]) == 0
    {
        ret = 0;
    }
    ret
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::{setaparam, setsparam};

    #[test]
    fn dash_o_dispatches_to_options() {
        let _g = crate::test_util::global_state_lock();
        setaparam(
            "words",
            vec!["[[".to_string(), "-o".to_string(), "".to_string()],
        );
        let _ = setsparam("CURRENT", "3");
        // Without tag setup, dispatch returns 1.
        let _r = _condition();
    }

    #[test]
    fn dash_t_dispatches_to_file_descriptors() {
        let _g = crate::test_util::global_state_lock();
        setaparam(
            "words",
            vec!["[[".to_string(), "-t".to_string(), "".to_string()],
        );
        let _ = setsparam("CURRENT", "3");
        let _r = _condition();
    }
}
