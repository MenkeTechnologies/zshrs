//! Port of `_shadow` / `_unshadow` from
//! `Completion/Base/Utility/_shadow`.
//!
//! Full upstream body (97 lines verbatim, abridged):
//! ```text
//! sh: 1  #autoload
//! sh:35  _shadow() {
//! sh:38    local -A fsfx=( -s ${funcstack[2]}:${functrace[2]}:$((.shadow.depth+1)) )
//! sh:42    zparseopts -K -A fsfx -D s:
//! sh:43    for fname; do
//! sh:44      shadowname=${fname}@${fsfx[-s]}
//! sh:45      if (( ${+functions[$shadowname]} )); then continue
//! sh:46      elif (( ${+functions[$fname]} )); then functions -c -- $fname $shadowname
//! sh:50      elif (( ${+builtins[$fname]} )); then alias-shadow
//! sh:53      else alias-cmd
//! sh:54      fi
//! sh:60    builtin set -A .shadow.stack ${fsfx[-s]} $fnames -- ${.shadow.stack}
//! sh:62  }
//! sh:65  _unshadow() {
//! sh:73    while [[ ${.shadow.stack[1]?no shadows} != -- ]]; do
//! sh:74      fname=…; shadowname=…
//! sh:76      if function defined: unfunction; restore from shadowname
//! sh:88    done
//! sh:96  (( ARGC )) && _shadow -s ${funcstack[2]}:${functrace[2]}:1 "$@"
//! ```
//!
//! Real `shfunctab`-backed port. Each `_shadow fname` call:
//!   1. Reads the current `shfunctab` entry for `fname`.
//!   2. Copies it into `<fname>@<suffix>` so the caller can dispatch
//!      to the original via that name (the `_approximate` pattern
//!      reads `compadd@_approximate` to call through to the
//!      compadd-builtin shim).
//!   3. Records the prior entry into a per-call frame on
//!      `SHADOW_STACK` so `_unshadow` can restore.
//!
//! `_unshadow` pops the topmost frame and restores each saved
//! entry (or deletes the user override when no prior entry
//! existed at shadow time).

use crate::ported::hashtable::shfunctab_lock;
use crate::ported::params::{getaparam, setaparam};
use crate::ported::zsh_h::shfunc;
use std::sync::Mutex;

/// One shadow frame — captures the prior state for every name
/// shadowed in a single `_shadow` call.
#[derive(Default)]
struct ShadowFrame {
    /// Suffix tag (`fsfx[-s]` in upstream) used to derive
    /// `<fname>@<suffix>` backup names.
    suffix: String,
    /// One entry per name shadowed in this call: `(fname,
    /// prior_shfunc_or_None)`.
    saved: Vec<(String, Option<shfunc>)>,
}

/// Process-wide stack of shadow frames. Pushed on `_shadow`,
/// popped on `_unshadow`.
static SHADOW_STACK: Mutex<Vec<ShadowFrame>> = Mutex::new(Vec::new());

/// Mirrors `.shadow.depth` from upstream sh:33 — incremented each
/// `_shadow`, decremented each `_unshadow`. Used for suffix
/// generation when the caller doesn't pass `-s`.
static SHADOW_DEPTH: Mutex<i64> = Mutex::new(0);

const STACK_PARAM: &str = ".shadow.stack";
const DEPTH_PARAM: &str = ".shadow.depth";

fn next_suffix(explicit: Option<&str>) -> String {
    if let Some(s) = explicit {
        return s.to_string();
    }
    let mut d = SHADOW_DEPTH.lock().unwrap();
    *d += 1;
    format!("shadow_{}", *d)
}

fn shadow_with_suffix(args: &[String], suffix: &str) -> i32 {
    let mut frame = ShadowFrame {
        suffix: suffix.to_string(),
        saved: Vec::with_capacity(args.len()),
    };
    {
        let mut tab = match shfunctab_lock().write() {
            Ok(t) => t,
            Err(_) => return 1,
        };
        for fname in args {
            // sh:45 — if `<fname>@<suffix>` already exists, this
            //   is a re-shadow with the same suffix; skip.
            let backup_name = format!("{}@{}", fname, suffix);
            if tab.contains_key(&backup_name) {
                frame.saved.push((fname.clone(), None));
                continue;
            }
            let prior = tab.get_including_disabled(fname).cloned();
            if let Some(orig) = prior.as_ref() {
                // sh:47 — `functions -c -- $fname $shadowname`.
                //   Copy the original definition under the backup
                //   name so caller-side `<fname>@<suffix> …` works.
                let mut backup = orig.clone();
                backup.node.nam = backup_name.clone();
                tab.add(backup);
            }
            // sh:50-54 — builtin/command-name shadow paths are
            //   collapsed: we just record whether the name was
            //   present in shfunctab. If not (likely a builtin),
            //   `_unshadow` will simply delete whatever override
            //   the caller installed.
            frame.saved.push((fname.clone(), prior));
        }
    }
    SHADOW_STACK.lock().unwrap().push(frame);

    // Mirror state into shell-side params for diagnostic visibility.
    publish_shell_side_state();
    0
}

/// `_shadow [-s suffix] fname1 fname2 ...` — snapshot one or more
/// shfunctab entries onto the shadow stack so `_unshadow` can
/// restore them later.
pub fn _shadow(args: &[String]) -> i32 {
    if args.is_empty() {
        return 0;
    }
    // sh:42 — `zparseopts -K -A fsfx -D s:`. Parse `-s SUFFIX`
    //   (value-taking) flag.
    let (explicit_suffix, fnames): (Option<String>, Vec<String>) =
        if args.len() >= 2 && args[0] == "-s" {
            (Some(args[1].clone()), args[2..].to_vec())
        } else if let Some(rest) = args[0].strip_prefix("-s") {
            if !rest.is_empty() {
                (Some(rest.to_string()), args[1..].to_vec())
            } else if args.len() >= 2 {
                (Some(args[1].clone()), args[2..].to_vec())
            } else {
                (None, args[1..].to_vec())
            }
        } else {
            (None, args.to_vec())
        };
    let suffix = next_suffix(explicit_suffix.as_deref());
    shadow_with_suffix(&fnames, &suffix)
}

/// `_unshadow` — pop the topmost shadow frame, restoring each
/// saved entry. Returns 0 on success, 1 if the stack was empty.
pub fn _unshadow() -> i32 {
    let frame = match SHADOW_STACK.lock().unwrap().pop() {
        Some(f) => f,
        None => return 1,
    };
    {
        let mut tab = match shfunctab_lock().write() {
            Ok(t) => t,
            Err(_) => return 1,
        };
        for (fname, prior) in frame.saved.into_iter() {
            // sh:76-85 — restore the saved entry. If `prior` is
            //   None, the fn didn't exist at shadow time → remove
            //   the user's override now too.
            match prior {
                Some(orig) => {
                    tab.add(orig);
                }
                None => {
                    tab.remove(&fname);
                }
            }
            // sh:84 — drop the `<fname>@<suffix>` backup
            let backup_name = format!("{}@{}", fname, frame.suffix);
            tab.remove(&backup_name);
        }
    }
    {
        let mut d = SHADOW_DEPTH.lock().unwrap();
        if *d > 0 {
            *d -= 1;
        }
    }
    publish_shell_side_state();
    0
}

/// Mirror shadow-stack state into shell-side params so diagnostic
/// code that inspects `.shadow.stack` / `.shadow.depth` sees a
/// snapshot. Each frame is rendered as
/// `<suffix> {f@,n@}<name>* --` (matching the upstream
/// `set -A .shadow.stack ${fsfx[-s]} $fnames -- ${.shadow.stack}`
/// layout per sh:60).
fn publish_shell_side_state() {
    let stack = SHADOW_STACK.lock().unwrap();
    let depth = SHADOW_DEPTH.lock().unwrap();
    let mut flat: Vec<String> = Vec::new();
    for f in stack.iter() {
        flat.push(f.suffix.clone());
        for (name, saved) in &f.saved {
            let marker = if saved.is_some() { "f@" } else { "n@" };
            flat.push(format!("{}{}", marker, name));
        }
        flat.push("--".to_string());
    }
    setaparam(STACK_PARAM, flat);
    setaparam(DEPTH_PARAM, vec![depth.to_string()]);
}

/// Test-only helper: reset all shadow state.
#[cfg(test)]
pub fn reset_shadow_state() {
    SHADOW_STACK.lock().unwrap().clear();
    *SHADOW_DEPTH.lock().unwrap() = 0;
    setaparam(STACK_PARAM, Vec::new());
    setaparam(DEPTH_PARAM, Vec::new());
}

/// Test-only helper: peek at the current backup name for the
/// topmost frame's shadow of `fname`.
#[cfg(test)]
pub fn current_backup_name(fname: &str) -> Option<String> {
    let stack = SHADOW_STACK.lock().unwrap();
    let frame = stack.last()?;
    if frame.saved.iter().any(|(n, _)| n == fname) {
        Some(format!("{}@{}", fname, frame.suffix))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zsh_h::hashnode;

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

    #[test]
    fn shadow_existing_function_creates_backup() {
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.remove("compadd");
            tab.add(make_shfunc("compadd", "echo original"));
        }
        assert_eq!(_shadow(&["compadd".to_string()]), 0);
        let backup = current_backup_name("compadd").unwrap();
        {
            let tab = shfunctab_lock().read().unwrap();
            let b = tab.get_including_disabled(&backup).unwrap();
            assert_eq!(b.body.as_deref(), Some("echo original"));
        }
        reset_shadow_state();
        let mut tab = shfunctab_lock().write().unwrap();
        tab.remove("compadd");
        tab.remove(&backup);
    }

    #[test]
    fn unshadow_restores_original_definition() {
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
        {
            let tab = shfunctab_lock().read().unwrap();
            assert_eq!(
                tab.get_including_disabled("myfn").unwrap().body.as_deref(),
                Some("echo override-body")
            );
        }
        assert_eq!(_unshadow(), 0);
        {
            let tab = shfunctab_lock().read().unwrap();
            assert_eq!(
                tab.get_including_disabled("myfn").unwrap().body.as_deref(),
                Some("echo original-body")
            );
        }
        reset_shadow_state();
        let mut tab = shfunctab_lock().write().unwrap();
        tab.remove("myfn");
    }

    #[test]
    fn unshadow_removes_when_no_prior_definition() {
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.remove("myfn_nonexistent");
        }
        let _ = _shadow(&["myfn_nonexistent".to_string()]);
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.add(make_shfunc("myfn_nonexistent", "echo created"));
        }
        assert_eq!(_unshadow(), 0);
        let tab = shfunctab_lock().read().unwrap();
        assert!(tab.get_including_disabled("myfn_nonexistent").is_none());
    }

    #[test]
    fn unshadow_on_empty_stack_returns_one() {
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        assert_eq!(_unshadow(), 1);
    }

    #[test]
    fn shadow_and_unshadow_balances_depth() {
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.add(make_shfunc("a", ""));
            tab.add(make_shfunc("b", ""));
        }
        let _ = _shadow(&["a".to_string(), "b".to_string()]);
        assert_eq!(*SHADOW_DEPTH.lock().unwrap(), 1);
        let _ = _unshadow();
        assert_eq!(*SHADOW_DEPTH.lock().unwrap(), 0);
        let mut tab = shfunctab_lock().write().unwrap();
        tab.remove("a");
        tab.remove("b");
    }

    #[test]
    fn explicit_suffix_via_dash_s() {
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.add(make_shfunc("targetfn", ""));
        }
        let _ = _shadow(&["-s".to_string(), "my-suffix".to_string(), "targetfn".to_string()]);
        let backup = current_backup_name("targetfn").unwrap();
        assert_eq!(backup, "targetfn@my-suffix");
        let _ = _unshadow();
        let mut tab = shfunctab_lock().write().unwrap();
        tab.remove("targetfn");
        tab.remove(&backup);
    }

    #[test]
    fn multiple_frames_stack_lifo() {
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.add(make_shfunc("x", "v1"));
        }
        let _ = _shadow(&["x".to_string()]);
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.add(make_shfunc("x", "v2"));
        }
        let _ = _shadow(&["x".to_string()]);
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.add(make_shfunc("x", "v3"));
        }
        assert_eq!(_unshadow(), 0);
        {
            let tab = shfunctab_lock().read().unwrap();
            assert_eq!(
                tab.get_including_disabled("x").unwrap().body.as_deref(),
                Some("v2")
            );
        }
        assert_eq!(_unshadow(), 0);
        {
            let tab = shfunctab_lock().read().unwrap();
            assert_eq!(
                tab.get_including_disabled("x").unwrap().body.as_deref(),
                Some("v1")
            );
        }
        reset_shadow_state();
        let mut tab = shfunctab_lock().write().unwrap();
        tab.remove("x");
    }

    #[test]
    fn published_state_records_frame_layout() {
        // sh:60 — verify the shell-side `.shadow.stack` mirror has
        //   the upstream `suffix fnames... --` layout.
        let _g = crate::test_util::global_state_lock();
        reset_shadow_state();
        {
            let mut tab = shfunctab_lock().write().unwrap();
            tab.add(make_shfunc("fn1", ""));
        }
        let _ = _shadow(&[
            "-s".to_string(),
            "frame1".to_string(),
            "fn1".to_string(),
        ]);
        let stack = getaparam(STACK_PARAM).unwrap_or_default();
        assert_eq!(stack, vec!["frame1", "f@fn1", "--"]);
        let _ = _unshadow();
        let mut tab = shfunctab_lock().write().unwrap();
        tab.remove("fn1");
    }
}
