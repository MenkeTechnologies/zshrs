//! Port of `_absolute_command_paths` from
//! `Completion/Unix/Type/_absolute_command_paths`.
//!
//! Full upstream body (37 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  # This function completes 'ls' to '/bin/ls'
//! sh: 4  _hashed_absolute_command_paths() {
//! sh: 5    local -aU set_of_dirs_of_hashed_commands=( ${^commands%/*}/ )
//! sh: 6    local i
//! sh: 7    integer ret=1
//! sh: 8    for i in $set_of_dirs_of_hashed_commands
//! sh: 9    do
//! sh:10      local -a matches=( "${(@)commands[(R)${~i}[^/]#]}" )
//! sh:11      local -a descs=( $matches:t )
//! sh:12      compadd -M "l:|=$i" -d descs "$@" -a matches
//! sh:13      ret=0
//! sh:14    done
//! sh:15    return ret
//! sh:16  }
//! sh:19  _typed-in_absolute_command_paths() {
//! sh:21    if [[ -z $PREFIX ]]; then
//! sh:22      _path_files -/ -g '*(-*)' -P / -W /
//! sh:23    elif [[ $PREFIX[1] == / ]]; then
//! sh:24      _path_files -/ -g '*(-*)' -W /
//! sh:25    else
//! sh:26      return 1
//! sh:27    fi
//! sh:28  }
//! sh:30  _absolute_command_paths() {
//! sh:31    _alternative \
//! sh:32      'commands:hashed command by absolute path:_hashed_absolute_command_paths' \
//! sh:33      'commands:full path to an executable:_typed-in_absolute_command_paths'
//! sh:34  }
//! sh:37  _absolute_command_paths "$@"
//! ```
//!
//! Pure delegation to `_alternative` (dispatched via exec accessors).
//! The two helper inner-shell-fns are exposed as separate Rust
//! functions that the executor's `_alternative` can call back into
//! when wired up. The leaf `_absolute_command_paths()` entry point
//! is what compsys consumers invoke.

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getaparam, getsparam, setaparam};
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

/// sh:4-16 — inner helper exposed at the same name so an
/// `_alternative` callback can dispatch us. Reads `$commands` assoc
/// (the shell's hashed-command table) and emits each entry's
/// absolute path under its own basename-`descs` alias.
pub fn _hashed_absolute_command_paths(args: &[String]) -> i32 {
    // c:Src/Modules/parameter.c:213/245 — `commands` is a module SPECIAL
    // backed by getfn/scanfn, NOT paramtab-hashed storage, so
    // `getaparam("commands")` returned None and this completer emitted
    // nothing at all. Same defect as the `has_command` helpers in
    // `_users_on.rs` / `_x_color.rs` and as `_set_command.rs:99-116`.
    //
    // Rebuild the flat key/value layout the rest of this function walks, via
    // the two entry points that DO route through the special: `gethkparam`
    // for the keys (c:3138 `paramvalarr(pm->gsu.h->getfn(pm),
    // SCANPM_WANTKEYS)`) and `getpmcommand` for each value.
    crate::vm_helper::mark_module_param_used("commands");
    let mut commands: Vec<String> = Vec::new();
    for key in crate::ported::params::gethkparam("commands").unwrap_or_default() {
        let val = crate::ported::modules::parameter::getpmcommand(std::ptr::null_mut(), &key)
            .and_then(|pm| pm.u_str.clone())
            .unwrap_or_default();
        commands.push(key);
        commands.push(val);
    }
    // sh:5 — extract unique directories from each command's value (paths)
    let mut dirs: Vec<String> = Vec::new();
    for v in commands.iter().skip(1).step_by(2) {
        // value at odd indices in flat key/value layout
        if let Some(slash) = v.rfind('/') {
            let dir = format!("{}/", &v[..slash]);
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
    }

    let mut ret: i32 = 1;
    for dir in &dirs {
        // sh:10  matches = commands values starting with $dir
        let matches: Vec<String> = commands
            .chunks(2)
            .filter_map(|kv| kv.get(1).filter(|v| v.starts_with(dir)).cloned())
            .collect();
        // sh:11
        let descs: Vec<String> = matches.iter().map(|m| basename(m)).collect();
        // sh:12  compadd -M "l:|=$dir" -d descs "$@" -a matches
        setaparam("descs", descs);
        setaparam("matches", matches);
        let mut compadd_argv: Vec<String> = vec![
            "-M".to_string(),
            format!("l:|={}", dir),
            "-d".to_string(),
            "descs".to_string(),
        ];
        compadd_argv.extend(args.iter().cloned());
        compadd_argv.push("-a".to_string());
        compadd_argv.push("matches".to_string());
        let _ = bin_compadd("compadd", &compadd_argv, &make_ops(), 0);
        ret = 0;
    }
    ret
}

/// sh:19-28 — inner helper.
pub fn _typed_in_absolute_command_paths(args: &[String]) -> i32 {
    let prefix = getsparam("PREFIX").unwrap_or_default();
    if prefix.is_empty() {
        // sh:22
        let mut a: Vec<String> = vec![
            "-/".to_string(),
            "-g".to_string(),
            "*(-*)".to_string(),
            "-P".to_string(),
            "/".to_string(),
            "-W".to_string(),
            "/".to_string(),
        ];
        a.extend(args.iter().cloned());
        dispatch_function_call("_path_files", &a).unwrap_or(1)
    } else if prefix.starts_with('/') {
        // sh:24
        let mut a: Vec<String> = vec![
            "-/".to_string(),
            "-g".to_string(),
            "*(-*)".to_string(),
            "-W".to_string(),
            "/".to_string(),
        ];
        a.extend(args.iter().cloned());
        dispatch_function_call("_path_files", &a).unwrap_or(1)
    } else {
        // sh:26
        1
    }
}

/// `_absolute_command_paths` — entry point. Delegates to
/// `_alternative` with the two helper specs.
pub fn _absolute_command_paths() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_absolute_command_paths");
    dispatch_function_call(
        "_alternative",
        &[
            "commands:hashed command by absolute path:_hashed_absolute_command_paths".to_string(),
            "commands:full path to an executable:_typed-in_absolute_command_paths".to_string(),
        ],
    )
    .unwrap_or(1)
}

fn basename(s: &str) -> String {
    s.rsplit('/').next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_absolute_command_paths(), 1);
    }

    #[test]
    fn typed_in_returns_one_for_relative_prefix() {
        // sh:26 — non-empty, non-/ PREFIX → 1.
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PREFIX", "ls");
        assert_eq!(_typed_in_absolute_command_paths(&[]), 1);
    }
}
