//! Port of `_shadow` / `_unshadow` from
//! `Completion/Base/Utility/_shadow` (upstream 97 lines, verbatim
//! spec below).
//!
//! ```text
//! sh: 1  #autoload
//! sh:31  zmodload zsh/parameter
//! sh:36  builtin typeset -gHi .shadow.depth=0
//! sh:37  builtin typeset -gHa .shadow.stack
//! sh:40  _shadow() {
//! sh:41    emulate -L zsh
//! sh:42    local -A fsfx=( -s ${funcstack[2]}:${functrace[2]}:$((.shadow.depth+1)) )
//! sh:43    local fname shadowname
//! sh:44    local -a fnames
//! sh:45    zparseopts -K -A fsfx -D s:
//! sh:46    for fname; do
//! sh:47      shadowname=${fname}@${fsfx[-s]}
//! sh:48      if (( ${+functions[$shadowname]} ))
//! sh:49      then
//! sh:50        # Called again with the same -s, just ignore it
//! sh:51        continue
//! sh:52      elif (( ${+functions[$fname]} ))
//! sh:53      then
//! sh:54        builtin functions -c -- $fname $shadowname
//! sh:55        fnames+=(f@$fname)
//! sh:56      elif (( ${+builtins[$fname]} ))
//! sh:57      then
//! sh:58        eval "function -- ${(q-)shadowname} { builtin ${(q-)fname} \"\$@\" }"
//! sh:59        fnames+=(b@$fname)
//! sh:60      else
//! sh:61        eval "function -- ${(q-)shadowname} { command ${(q-)fname} \"\$@\" }"
//! sh:62        fnames+=(c@$fname)
//! sh:63      fi
//! sh:64    done
//! sh:65    [[ -z $REPLY ]] && REPLY=${fsfx[-s]}
//! sh:66    builtin set -A .shadow.stack ${fsfx[-s]} $fnames -- ${.shadow.stack}
//! sh:67    ((.shadow.depth++))
//! sh:68  }
//! sh:71  _unshadow() {
//! sh:72    emulate -L zsh
//! sh:73    local fname shadowname fsfx=${.shadow.stack[1]}
//! sh:74    local -a fnames
//! sh:75    [[ -n $fsfx ]] || return 1
//! sh:76    shift .shadow.stack
//! sh:77    while [[ ${.shadow.stack[1]?no shadows} != -- ]]; do
//! sh:78      fname=${.shadow.stack[1]#?@}
//! sh:79      shadowname=${fname}@${fsfx}
//! sh:80      if (( ${+functions[$fname]} )); then
//! sh:81        builtin unfunction -- $fname
//! sh:82      fi
//! sh:83      case ${.shadow.stack[1]} in
//! sh:84        (f@*) builtin functions -c -- $shadowname $fname ;&
//! sh:85        ([bc]@*) builtin unfunction -- $shadowname ;;
//! sh:86      esac
//! sh:87      shift .shadow.stack
//! sh:88    done
//! sh:89    [[ -z $REPLY ]] && REPLY=$fsfx
//! sh:90    shift .shadow.stack
//! sh:91    ((.shadow.depth--))
//! sh:92  }
//! sh:97  (( ARGC )) && _shadow -s ${funcstack[2]}:${functrace[2]}:1 "$@"
//! ```
//!
//! `_shadow` makes a copy of each named function/builtin/command
//! under the backup name `<fname>@<suffix>` so a caller may
//! redefine `fname` and still dispatch to the original through the
//! backup. `_approximate` (sh:56 in that file) does exactly this:
//! `_shadow -s _approximate compadd`, redefines `compadd`, then its
//! override calls `compadd@_approximate "$@"` to reach the real
//! `compadd`. `_unshadow` pops the topmost stack frame and restores
//! every name in it.
//!
//! State is kept in the SAME shell parameters the upstream uses, so
//! behaviour and inspection match:
//!   * `.shadow.depth` (`typeset -gHi`, sh:36) — per-call counter.
//!   * `.shadow.stack` (`typeset -gHa`, sh:37) — flat frame stack,
//!     each frame laid out as `suffix <marker>@name... --` where
//!     marker is `f` (was a function), `b` (was a builtin), or `c`
//!     (was a command / undefined) — exactly the upstream markers.
//!   * `REPLY` (sh:65 / sh:89).
//!
//! Approximations (documented, not silent):
//!   * `functions -c` (sh:54, sh:84) is a plain shfunc clone here;
//!     the upstream BUGS note says `functions -c` "acts like
//!     `autoload +X`" — we do not force-load an autoload stub before
//!     cloning, we copy whatever is currently in `shfunctab`.
//!   * The default `${funcstack[2]}:${functrace[2]}:<depth+1>`
//!     suffix (sh:42) is built from the live `funcstack`/`functrace`
//!     special params; callers that pass `-s` (e.g. `_approximate`)
//!     never exercise this path.
//!   * `${(q-)…}` quoting of the wrapper name/body (sh:58, sh:61) is
//!     omitted: shadowed names here are shell identifiers
//!     (`compadd`, `compcall`, …) that need no quoting.
//!   * The top-level autoload trailer (sh:97) is subsumed: invoking
//!     `_shadow(args)` corresponds to calling the `_shadow()` shell
//!     function directly, which is what the router does.

use crate::ported::hashtable::{removeshfuncnode, shfunctab_lock};
use crate::ported::modules::parameter::setfunction;
use crate::ported::params::{getaparam, getiparam, getsparam, setaparam, setiparam, setsparam};
use crate::ported::utils::getshfunc;

const STACK_PARAM: &str = ".shadow.stack";
const DEPTH_PARAM: &str = ".shadow.depth";
const SEP: &str = "--";

/// `${+functions[$name]}` (sh:48, sh:52, sh:80) — a shell function
/// with this name is currently defined.
fn function_defined(name: &str) -> bool {
    getshfunc(name).is_some()
}

/// `${+builtins[$name]}` (sh:56) — an enabled builtin with this name
/// exists.
fn builtin_defined(name: &str) -> bool {
    crate::ported::builtin::createbuiltintable().contains_key(name)
}

/// `builtin functions -c -- $src $dst` (sh:54, sh:84) — clone the
/// `shfunc` currently registered under `src` into `dst`. Returns
/// `true` when `src` existed and was copied. See module-doc
/// approximation note re: `autoload +X`.
fn functions_copy(src: &str, dst: &str) -> bool {
    let mut tab = match shfunctab_lock().write() {
        Ok(t) => t,
        Err(_) => return false,
    };
    match tab.get_including_disabled(src).cloned() {
        Some(mut copy) => {
            copy.node.nam = dst.to_string();
            tab.add(copy);
            true
        }
        None => false,
    }
}

/// `builtin unfunction -- $name` (sh:81, sh:85).
fn unfunction(name: &str) {
    let _ = removeshfuncnode(name);
}

/// sh:42 default suffix: `${funcstack[2]}:${functrace[2]}:<depth+1>`.
/// funcstack/functrace are 1-based; element `[2]` is index 1 here.
fn default_suffix(depth: i64) -> String {
    let fs = crate::ported::modules::parameter::funcstackgetfn(std::ptr::null_mut());
    let ft = crate::ported::modules::parameter::functracegetfn(std::ptr::null_mut());
    let f2 = fs.get(1).cloned().unwrap_or_default();
    let t2 = ft.get(1).cloned().unwrap_or_default();
    format!("{}:{}:{}", f2, t2, depth + 1)
}

/// `zparseopts -K -A fsfx -D s:` (sh:45). Extract an optional
/// leading `-s SUFFIX` (or glued `-sSUFFIX`); `-K` keeps the
/// pre-seeded default when absent, `-D` removes the flag from the
/// positionals. zparseopts stops at the first non-option word, so
/// for `_shadow`'s inputs (`-s _approximate compadd`, or bare
/// `compadd`) a leading-only scan is exact.
fn parse_opts(args: &[String], default: String) -> (String, Vec<String>) {
    if args.is_empty() {
        return (default, Vec::new());
    }
    if args[0] == "-s" {
        // `-s SUFFIX rest...`
        if args.len() >= 2 {
            return (args[1].clone(), args[2..].to_vec());
        }
        return (default, args[1..].to_vec());
    }
    if let Some(glued) = args[0].strip_prefix("-s") {
        if !glued.is_empty() {
            // `-sSUFFIX rest...`
            return (glued.to_string(), args[1..].to_vec());
        }
    }
    (default, args.to_vec())
}

/// `_shadow [-s suffix] fname...` — sh:40-68.
pub fn _shadow(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_shadow");
    // sh:42 — seed fsfx[-s] with the default before zparseopts may
    //   override it.
    let depth = getiparam(DEPTH_PARAM);
    let (suffix, fnames_in) = parse_opts(args, default_suffix(depth)); // sh:45

    // sh:44 — local -a fnames (the marker list appended below).
    let mut fnames: Vec<String> = Vec::with_capacity(fnames_in.len());

    for fname in &fnames_in {
        // sh:47
        let shadowname = format!("{}@{}", fname, suffix);
        if function_defined(&shadowname) {
            // sh:48-51 — same -s used again; ignore.
            continue;
        } else if function_defined(fname) {
            // sh:52-55 — copy the function under the backup name.
            functions_copy(fname, &shadowname);
            fnames.push(format!("f@{}", fname));
        } else if builtin_defined(fname) {
            // sh:56-59 — wrapper dispatching to the real builtin.
            setfunction(&shadowname, format!("builtin {} \"$@\"", fname), 0);
            fnames.push(format!("b@{}", fname));
        } else {
            // sh:60-63 — wrapper dispatching to an external command.
            setfunction(&shadowname, format!("command {} \"$@\"", fname), 0);
            fnames.push(format!("c@{}", fname));
        }
    }

    // sh:65 — [[ -z $REPLY ]] && REPLY=${fsfx[-s]}
    if getsparam("REPLY").unwrap_or_default().is_empty() {
        let _ = setsparam("REPLY", &suffix);
    }

    // sh:66 — set -A .shadow.stack ${fsfx[-s]} $fnames -- ${.shadow.stack}
    let old = getaparam(STACK_PARAM).unwrap_or_default();
    let mut new_stack: Vec<String> = Vec::with_capacity(2 + fnames.len() + old.len());
    new_stack.push(suffix);
    new_stack.extend(fnames);
    new_stack.push(SEP.to_string());
    new_stack.extend(old);
    setaparam(STACK_PARAM, new_stack);

    // sh:67 — ((.shadow.depth++))
    setiparam(DEPTH_PARAM, depth + 1);
    0
}

/// `_unshadow` — sh:71-92. Pop the topmost frame, restoring every
/// name in it. Returns 1 when the stack is empty (sh:75).
pub fn _unshadow() -> i32 {
    let mut stack = getaparam(STACK_PARAM).unwrap_or_default();

    // sh:73 — fsfx=${.shadow.stack[1]}
    let fsfx = stack.first().cloned().unwrap_or_default();
    // sh:75 — [[ -n $fsfx ]] || return 1
    if fsfx.is_empty() {
        return 1;
    }
    // sh:76 — shift .shadow.stack (drop the suffix element)
    stack.remove(0);

    // sh:77 — while the leading marker entry isn't the `--` separator.
    while stack.first().map(|s| s.as_str()) != Some(SEP) {
        // sh:77 — ${.shadow.stack[1]?no shadows}: a malformed stack
        //   with no separator would be a programming error; bail
        //   rather than loop off the end.
        let entry = match stack.first().cloned() {
            Some(e) => e,
            None => break,
        };
        // sh:78 — fname=${.shadow.stack[1]#?@}
        let fname = strip_marker(&entry);
        // sh:79 — shadowname=${fname}@${fsfx}
        let shadowname = format!("{}@{}", fname, fsfx);

        // sh:80-82 — drop the caller's redefinition of fname.
        if function_defined(&fname) {
            unfunction(&fname);
        }

        // sh:83-86 — restore from the backup, then remove the backup.
        if entry.starts_with("f@") {
            // sh:84 — functions -c -- $shadowname $fname ;& (falls
            //   through to unfunction $shadowname below).
            functions_copy(&shadowname, &fname);
            unfunction(&shadowname);
        } else if entry.starts_with("b@") || entry.starts_with("c@") {
            // sh:85 — unfunction the wrapper.
            unfunction(&shadowname);
        }

        // sh:87 — shift .shadow.stack
        stack.remove(0);
    }

    // sh:89 — [[ -z $REPLY ]] && REPLY=$fsfx
    if getsparam("REPLY").unwrap_or_default().is_empty() {
        let _ = setsparam("REPLY", &fsfx);
    }

    // sh:90 — shift .shadow.stack (drop the `--` separator)
    if stack.first().map(|s| s.as_str()) == Some(SEP) {
        stack.remove(0);
    }
    setaparam(STACK_PARAM, stack);

    // sh:91 — ((.shadow.depth--))
    setiparam(DEPTH_PARAM, getiparam(DEPTH_PARAM) - 1);
    0
}

/// sh:78 — `${entry#?@}`: strip a single leading char plus `@`
/// (the `f@`/`b@`/`c@` marker). If the entry has no such prefix it
/// is returned unchanged (matching `#?@` no-op on a non-match).
fn strip_marker(entry: &str) -> String {
    let mut chars = entry.chars();
    match (chars.next(), chars.next()) {
        (Some(_), Some('@')) => chars.collect(),
        _ => entry.to_string(),
    }
}

/// Test-only helper: reset the shadow state params.
#[cfg(test)]
pub fn reset_shadow_state() {
    setaparam(STACK_PARAM, Vec::new());
    setiparam(DEPTH_PARAM, 0);
    let _ = setsparam("REPLY", "");
}

/// Test-only helper: the backup name for `fname` in the topmost
/// frame, or `None` if `fname` isn't shadowed there. Reads the same
/// `.shadow.stack` layout `_unshadow` walks.
#[cfg(test)]
pub fn current_backup_name(fname: &str) -> Option<String> {
    let stack = getaparam(STACK_PARAM).unwrap_or_default();
    let suffix = stack.first()?.clone();
    for entry in stack.iter().skip(1) {
        if entry == SEP {
            break;
        }
        if strip_marker(entry) == fname {
            return Some(format!("{}@{}", fname, suffix));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zsh_h::{hashnode, shfunc};

    fn make_shfunc(name: &str, body: &str) -> shfunc {
        shfunc {
            node: hashnode {
                nam: name.to_string(),
                next: None,
                flags: 0,
            },
            filename: None,
            lineno: 0,
            funcdef: None,
            redir: None,
            sticky: None,
            body: Some(body.to_string()),
        }
    }

    fn body_of(name: &str) -> Option<String> {
        let tab = shfunctab_lock().read().unwrap();
        tab.get_including_disabled(name)
            .and_then(|f| f.body.clone())
    }

    #[test]
    fn shadow_existing_function_creates_f_marker_backup() {
        // sh:52-55 — existing function is cloned under the backup name.
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.remove("compadd");
            tab.add(make_shfunc("compadd", "echo original"));
        }
        assert_eq!(_shadow(&["compadd".to_string()]), 0);
        let backup = current_backup_name("compadd").unwrap();
        assert_eq!(body_of(&backup).as_deref(), Some("echo original"));
        // Marker must be `f@`, not the fabricated `n@`.
        let stack = getaparam(STACK_PARAM).unwrap();
        assert!(stack.iter().any(|e| e == "f@compadd"));
        reset_shadow_state();
        let mut tab = shfunctab_lock().write().unwrap();
        tab.remove("compadd");
        tab.remove(&backup);
    }

    #[test]
    fn shadow_builtin_installs_wrapper_with_b_marker() {
        // sh:56-59 — a builtin (compadd) with no shell-fn override
        //   gets a `{ builtin compadd "$@" }` wrapper so the caller's
        //   redefinition has a backup to dispatch to.
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.remove("compadd");
        }
        assert!(builtin_defined("compadd"), "compadd must be a builtin");
        let _ = _shadow(&[
            "-s".to_string(),
            "_approximate".to_string(),
            "compadd".to_string(),
        ]);
        let backup = current_backup_name("compadd").unwrap();
        assert_eq!(backup, "compadd@_approximate");
        assert_eq!(body_of(&backup).as_deref(), Some("builtin compadd \"$@\""));
        let stack = getaparam(STACK_PARAM).unwrap();
        assert!(stack.iter().any(|e| e == "b@compadd"));
        let _ = _unshadow();
        // Wrapper removed on unshadow.
        assert!(body_of(&backup).is_none());
    }

    #[test]
    fn shadow_command_installs_wrapper_with_c_marker() {
        // sh:60-63 — neither function nor builtin → command wrapper.
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        let name = "zz_not_a_builtin_or_fn";
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.remove(name);
        }
        assert!(!function_defined(name));
        assert!(!builtin_defined(name));
        let _ = _shadow(&["-s".to_string(), "sfx".to_string(), name.to_string()]);
        let backup = current_backup_name(name).unwrap();
        assert_eq!(
            body_of(&backup).as_deref(),
            Some(format!("command {} \"$@\"", name).as_str())
        );
        let stack = getaparam(STACK_PARAM).unwrap();
        assert!(stack.iter().any(|e| e == &format!("c@{}", name)));
        let _ = _unshadow();
        assert!(body_of(&backup).is_none());
    }

    #[test]
    fn reshadow_same_suffix_is_ignored() {
        // sh:48-51 — second call with the same -s must not re-clone.
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.remove("f");
            tab.add(make_shfunc("f", "v1"));
        }
        let _ = _shadow(&["-s".to_string(), "sfx".to_string(), "f".to_string()]);
        // Redefine and re-shadow with same suffix.
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.add(make_shfunc("f", "v2"));
        }
        let _ = _shadow(&["-s".to_string(), "sfx".to_string(), "f".to_string()]);
        // Backup must still hold the ORIGINAL v1, not v2.
        assert_eq!(body_of("f@sfx").as_deref(), Some("v1"));
        reset_shadow_state();
        let mut tab = shfunctab_lock().write().unwrap();
        tab.remove("f");
        tab.remove("f@sfx");
    }

    #[test]
    fn unshadow_restores_original_definition() {
        // sh:80-86 (f@ path) — caller redefinition dropped, original
        //   copied back from the backup.
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.remove("myfn");
            tab.add(make_shfunc("myfn", "echo original-body"));
        }
        let _ = _shadow(&["myfn".to_string()]);
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.add(make_shfunc("myfn", "echo override-body"));
        }
        assert_eq!(body_of("myfn").as_deref(), Some("echo override-body"));
        assert_eq!(_unshadow(), 0);
        assert_eq!(body_of("myfn").as_deref(), Some("echo original-body"));
        reset_shadow_state();
        let mut tab = shfunctab_lock().write().unwrap();
        tab.remove("myfn");
    }

    #[test]
    fn unshadow_removes_wrapper_when_no_prior_function() {
        // sh:60-63 then sh:85 — a name that was neither fn nor
        //   builtin gets a c@ wrapper; unshadow removes both the
        //   caller redefinition and the wrapper.
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        let name = "zz_ghost_cmd";
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.remove(name);
        }
        let _ = _shadow(&[name.to_string()]);
        let backup = current_backup_name(name).unwrap();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.add(make_shfunc(name, "echo created"));
        }
        assert_eq!(_unshadow(), 0);
        assert!(!function_defined(name));
        assert!(body_of(&backup).is_none());
    }

    #[test]
    fn unshadow_on_empty_stack_returns_one() {
        // sh:75
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        assert_eq!(_unshadow(), 1);
    }

    #[test]
    fn shadow_and_unshadow_balances_depth() {
        // sh:67 / sh:91
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.add(make_shfunc("a", ""));
            tab.add(make_shfunc("b", ""));
        }
        let _ = _shadow(&["a".to_string(), "b".to_string()]);
        assert_eq!(getiparam(DEPTH_PARAM), 1);
        let _ = _unshadow();
        assert_eq!(getiparam(DEPTH_PARAM), 0);
        let mut tab = shfunctab_lock().write().unwrap();
        tab.remove("a");
        tab.remove("b");
    }

    #[test]
    fn explicit_suffix_via_dash_s() {
        // sh:45 — -s overrides the default suffix.
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.add(make_shfunc("targetfn", ""));
        }
        let _ = _shadow(&[
            "-s".to_string(),
            "my-suffix".to_string(),
            "targetfn".to_string(),
        ]);
        assert_eq!(
            current_backup_name("targetfn").unwrap(),
            "targetfn@my-suffix"
        );
        let _ = _unshadow();
        let mut tab = shfunctab_lock().write().unwrap();
        tab.remove("targetfn");
        tab.remove("targetfn@my-suffix");
    }

    #[test]
    fn multiple_frames_stack_lifo() {
        // sh:66 prepend / sh:76-90 pop — nested shadow/unshadow.
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.remove("x");
            tab.add(make_shfunc("x", "v1"));
        }
        let _ = _shadow(&["-s".to_string(), "s1".to_string(), "x".to_string()]);
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.add(make_shfunc("x", "v2"));
        }
        let _ = _shadow(&["-s".to_string(), "s2".to_string(), "x".to_string()]);
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.add(make_shfunc("x", "v3"));
        }
        assert_eq!(_unshadow(), 0);
        assert_eq!(body_of("x").as_deref(), Some("v2"));
        assert_eq!(_unshadow(), 0);
        assert_eq!(body_of("x").as_deref(), Some("v1"));
        reset_shadow_state();
        let mut tab = shfunctab_lock().write().unwrap();
        tab.remove("x");
    }

    #[test]
    fn stack_layout_uses_real_markers_and_separator() {
        // sh:66 — a frame is `suffix <marker>@name... --`.
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.remove("fn1");
            tab.add(make_shfunc("fn1", ""));
        }
        let _ = _shadow(&["-s".to_string(), "frame1".to_string(), "fn1".to_string()]);
        let stack = getaparam(STACK_PARAM).unwrap_or_default();
        assert_eq!(stack, vec!["frame1", "f@fn1", "--"]);
        let _ = _unshadow();
        let mut tab = shfunctab_lock().write().unwrap();
        tab.remove("fn1");
    }

    #[test]
    fn strip_marker_matches_hash_question_at() {
        assert_eq!(strip_marker("f@compadd"), "compadd");
        assert_eq!(strip_marker("b@zstyle"), "zstyle");
        assert_eq!(strip_marker("c@ls"), "ls");
        // No `?@` prefix → unchanged.
        assert_eq!(strip_marker("plain"), "plain");
    }
}
