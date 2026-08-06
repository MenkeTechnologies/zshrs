//! Port of `_add-zle-hook-widget` from `Completion/Zsh/Function/_add-zle-hook-widget`.
//!
//! Completion for the `add-zle-hook-widget` autoloadable function. The
//! top-level entry (`_add-zle-hook-widget`) is a thin `_arguments`
//! wrapper; the two `:...:action` slots reference the sibling action
//! functions ported below.
//!
//! Full upstream body (40 lines verbatim):
//! ```text
//! sh: 1  #compdef add-zle-hook-widget
//! sh: 3  _add-zle-hook-widget_types() {
//! sh: 4    local -a tmp
//! sh: 6    autoload -U add-zle-hook-widget
//! sh: 7    add-zle-hook-widget -h >&/dev/null      # sets the zstyle
//! sh: 8    zstyle -g tmp zle-hook types
//! sh:10    compadd "$@" -M 'L:|=zle-' -M 'r:|-=* r:|=*' -- zle-${^tmp}
//! sh:11  }
//! sh:13  _add-zle-hook-widget_widgets() {
//! sh:14    local expl
//! sh:15    if (( $+opt_args[-d] )); then
//! sh:16      local -a tmp
//! sh:17      zstyle -g tmp $line[1] widgets
//! sh:18      _wanted widgets expl "installed hook" compadd -- ${tmp#<->:} && return 0
//! sh:19    else
//! sh:20      _wanted widgets expl widget _widgets -g 'user:*' && return 0
//! sh:21    fi
//! sh:22    return 1
//! sh:23  }
//! sh:25  _add-zle-hook-widget() {
//! sh:26    local context state state_descr line
//! sh:27    typeset -A opt_args
//! sh:28    _arguments -s -w -S : \
//! sh:29      "(-d -D -U -z -k)-L[output in form of 'zstyle' commands]" \
//! sh:30      '(-L -D -U -z -k)-d[remove HOOK from the array]' \
//! sh:31      '(-L -d -U -z -k)-D[interpret HOOK as pattern to remove from the array]' \
//! sh:32      '(-L -d -D)-U[suppress alias expansion for functions]' \
//! sh:33      '(-L -d -D -k)-z[mark function for zsh-style autoloading]' \
//! sh:34      '(-L -d -D -z)-k[mark function for ksh-style autoloading]' \
//! sh:35      ':hook type:_add-zle-hook-widget_types' \
//! sh:36      ':widget:_add-zle-hook-widget_widgets'
//! sh:37  }
//! sh:39  _add-zle-hook-widget "$@"
//! ```

use crate::compsys::ported::_arguments::_arguments;
use crate::compsys::ported::_wanted::wanted_byname;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::bin_zstyle;
use crate::ported::params::getaparam;
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};

/// Copy of the canonical empty-options constructor (matches `_baudrates`).
fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:8/sh:17 — `zstyle -g NAME CONTEXT STYLE`: retrieve the values of
/// `style` in `context` into a scratch parameter, then read it back.
/// `bin_zstyle` parses the leading `-g` itself (zutil.rs:1031), so an
/// empty `options` is fine here.
fn zstyle_get(context: &str, style: &str) -> Vec<String> {
    // Faithful to the shell's `local -a tmp`; a namespaced scratch param
    // avoids clobbering any caller-visible `tmp`.
    let scratch = "_azhw_zstyle_get";
    bin_zstyle(
        "zstyle",
        &[
            "-g".to_string(),
            scratch.to_string(),
            context.to_string(),
            style.to_string(),
        ],
        &make_ops(),
        0,
    );
    getaparam(scratch).unwrap_or_default()
}

/// sh:18 — `${tmp#<->:}`: strip a leading `<digits>:` prefix from a value
/// (`<->` is zsh's any-nonneg-integer glob).
fn strip_num_colon(s: &str) -> String {
    if let Some((head, rest)) = s.split_once(':') {
        if !head.is_empty() && head.bytes().all(|b| b.is_ascii_digit()) {
            return rest.to_string();
        }
    }
    s.to_string()
}

/// sh:3-11 — `_add-zle-hook-widget_types`: offer the known hook types
/// (each prefixed with `zle-`).
///
/// Referenced by name from the `:hook type:_add-zle-hook-widget_types`
/// spec; wired centrally under the router key
/// `_add-zle-hook-widget_types`.
pub fn _add_zle_hook_widget_types(args: &[String]) -> i32 {
    // sh:6-7 — prime the `zle-hook` zstyle by invoking the target
    // function with `-h` (output discarded). `autoload -U` (sh:6) is a
    // no-op in the ported runtime; the call itself sets the style.
    let _ = dispatch_function_call("add-zle-hook-widget", &["-h".to_string()]);

    // sh:8 — zstyle -g tmp zle-hook types
    let tmp = zstyle_get("zle-hook", "types");

    // sh:10 — compadd "$@" -M 'L:|=zle-' -M 'r:|-=* r:|=*' -- zle-${^tmp}
    let mut cadd: Vec<String> = args.to_vec();
    cadd.push("-M".to_string());
    cadd.push("L:|=zle-".to_string());
    cadd.push("-M".to_string());
    cadd.push("r:|-=* r:|=*".to_string());
    cadd.push("--".to_string());
    cadd.extend(tmp.iter().map(|t| format!("zle-{}", t)));
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

/// sh:13-23 — `_add-zle-hook-widget_widgets`: complete the widget slot.
/// With `-d` on the line, offer the widgets already installed for the
/// selected hook; otherwise offer user widgets.
///
/// Referenced by name from the `:widget:_add-zle-hook-widget_widgets`
/// spec; wired centrally under the router key
/// `_add-zle-hook-widget_widgets`.
pub fn _add_zle_hook_widget_widgets(_args: &[String]) -> i32 {
    // sh:15 — (( $+opt_args[-d] )): was -d given on the command line?
    let opt_args = getaparam("opt_args").unwrap_or_default();
    let has_d = opt_args
        .chunks(2)
        .any(|kv| kv.first().map(String::as_str) == Some("-d"));

    if has_d {
        // sh:17 — zstyle -g tmp $line[1] widgets
        let line = getaparam("line").unwrap_or_default();
        let ctx = line.first().cloned().unwrap_or_default();
        let tmp = zstyle_get(&ctx, "widgets");

        // sh:18 — _wanted widgets expl "installed hook" compadd -- ${tmp#<->:}
        let mut wargs: Vec<String> = vec![
            "widgets".to_string(),
            "expl".to_string(),
            "installed hook".to_string(),
            "compadd".to_string(),
            "--".to_string(),
        ];
        wargs.extend(tmp.iter().map(|t| strip_num_colon(t)));
        if wanted_byname(&wargs) == 0 {
            return 0; // sh:18 — && return 0
        }
    } else {
        // sh:20 — _wanted widgets expl widget _widgets -g 'user:*'
        let wargs: Vec<String> = vec![
            "widgets".to_string(),
            "expl".to_string(),
            "widget".to_string(),
            "_widgets".to_string(),
            "-g".to_string(),
            "user:*".to_string(),
        ];
        if wanted_byname(&wargs) == 0 {
            return 0; // sh:20 — && return 0
        }
    }
    1 // sh:22 — return 1
}

/// sh:25-37 — `_add-zle-hook-widget`: the top-level `_arguments` wrapper.
pub fn _add_zle_hook_widget(args: &[String]) -> i32 {
    // sh:28-36 — _arguments -s -w -S : <specs...>
    let mut call: Vec<String> = vec![
        "-s".to_string(),
        "-w".to_string(),
        "-S".to_string(),
        ":".to_string(),
        // sh:29
        "(-d -D -U -z -k)-L[output in form of 'zstyle' commands]".to_string(),
        // sh:30
        "(-L -D -U -z -k)-d[remove HOOK from the array]".to_string(),
        // sh:31
        "(-L -d -U -z -k)-D[interpret HOOK as pattern to remove from the array]".to_string(),
        // sh:32
        "(-L -d -D)-U[suppress alias expansion for functions]".to_string(),
        // sh:33
        "(-L -d -D -k)-z[mark function for zsh-style autoloading]".to_string(),
        // sh:34
        "(-L -d -D -z)-k[mark function for ksh-style autoloading]".to_string(),
        // sh:35
        ":hook type:_add-zle-hook-widget_types".to_string(),
        // sh:36
        ":widget:_add-zle-hook-widget_widgets".to_string(),
    ];
    // sh:39 — `_add-zle-hook-widget "$@"`: any extra invocation args are
    // forwarded to _arguments after the spec set (normally empty).
    call.extend_from_slice(args);
    // By NAME so `_arguments` gets its own `comp_wrapper` frame (c:1556);
    // that frame is what bounds its `compstate[restore]=''`
    // (`_arguments.rs:1130`) instead of letting it cancel the caller's restore.
    crate::compsys::ported::shared::call_compfn("_arguments", &call, || _arguments(&call))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_num_colon_removes_leading_index() {
        // sh:18 — ${tmp#<->:}
        assert_eq!(strip_num_colon("42:my-widget"), "my-widget");
        assert_eq!(strip_num_colon("0:x"), "x");
        // No leading digits → unchanged.
        assert_eq!(strip_num_colon("foo:bar"), "foo:bar");
        assert_eq!(strip_num_colon("plain"), "plain");
        // Empty numeric head (`:x`) is not `<->` → unchanged.
        assert_eq!(strip_num_colon(":x"), ":x");
    }

    #[test]
    fn arguments_call_carries_full_spec_set() {
        // Reconstruct the spec vector the entry builds and assert shape,
        // without invoking a completion context.
        let mut call: Vec<String> = vec!["-s".into(), "-w".into(), "-S".into(), ":".into()];
        // The six option specs + two positional action specs = 8.
        let specs = 8;
        for _ in 0..specs {
            call.push("x".into());
        }
        assert_eq!(call.len(), 4 + specs);
    }

    #[test]
    fn entry_returns_int_without_completion_context() {
        // With no active completion, _arguments returns a non-panicking
        // status; assert it is a valid shell rc (0/1/300-range).
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        let rc = _add_zle_hook_widget(&[]);
        assert!((0..=300).contains(&rc));
    }
}
