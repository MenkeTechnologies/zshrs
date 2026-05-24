//! Port of `_call_program` (zsh Completion/Base/Utility/_call_program).
//!
//! Local shell reference: `compsys/functions/Base/Utility/_call_program`.
//!
//! Shell source (41 lines):
//! ```text
//! local -xi COLUMNS=999
//! local curcontext="${curcontext}" tmp err_fd=-1 clocale='_comp_locale;'
//! local -a prefix
//!
//! if [[ "$1" = -p ]]; then
//!   shift
//!   if (( $#_comp_priv_prefix )); then
//!     curcontext="${curcontext%:*}/${${(@M)_comp_priv_prefix:#^*[^\\]=*}[1]}:"
//!     zstyle -t ":completion:${curcontext}:${1}" gain-privileges &&
//!         prefix=( $_comp_priv_prefix )
//!   fi
//! elif [[ "$1" = -l ]]; then
//!   shift
//!   clocale=''
//! fi
//!
//! if (( ${debug_fd:--1} > 2 )) || [[ ! -t 2 ]]
//! then exec {err_fd}>&2
//! else exec {err_fd}>/dev/null
//! fi
//!
//! if zstyle -s ":completion:${curcontext}:${1}" command tmp; then
//!   if [[ "$tmp" = -* ]]; then
//!     eval $clocale "$tmp[2,-1]" "$argv[2,-1]"
//!   else
//!     eval $clocale $prefix "$tmp"
//!   fi
//! else
//!   eval $clocale $prefix "$argv[2,-1]"
//! fi 2>&$err_fd
//! ```
//!
//! Three behaviors the previous stubs missed:
//!   1. `-p` flag → consult `$_comp_priv_prefix` (sudo/doas state) and,
//!      gated by the `gain-privileges` zstyle, PREFIX the command with
//!      the priv-prefix. The previous stub had no -p at all.
//!   2. `-l` flag → skip the `_comp_locale` C-locale env trampoline.
//!      The previous stub never set locale env vars regardless.
//!   3. `command` zstyle override starting with `-` → REPLACE the
//!      command wholesale with `$tmp[2,-1]` (strip the leading `-`).
//!      Otherwise the style value is treated as a complete replacement
//!      command. The previous stub treated every override as a
//!      wholesale replacement, missing the prefix-append branch.
//!
//! Also sets `COLUMNS=999` so commands that wrap output (ls, ps,
//! …) emit one-per-line when invoked from completion.

use crate::base::MainCompleteState;
use std::process::Command;

/// Outcome of [`call_program`]. The shell function dumps stdout to
/// fd 1 (read by the caller via `$(_call_program …)`); we return it
/// as a String instead.
pub struct CallProgramResult {
    pub stdout: String,
    /// True when `-p` was specified, `_comp_priv_prefix` was non-empty,
    /// and the `gain-privileges` style was set — i.e. the call WAS
    /// actually re-prefixed with the priv command. Lets callers know
    /// whether output reflects elevated context.
    pub used_priv_prefix: bool,
}

#[derive(Default)]
pub struct CallProgramOpts<'a> {
    /// `-p` flag — gate priv-prefix application on the
    /// `gain-privileges` zstyle.
    pub use_priv_prefix: bool,
    /// `-l` flag — skip the `_comp_locale` C-locale env trampoline.
    /// Default is to set `LC_ALL=C LANGUAGE=C LC_MESSAGES=C` so
    /// command output is parser-friendly.
    pub skip_locale: bool,
    /// Current `_comp_priv_prefix` value — passed in by the caller
    /// since compsys is a leaf crate and can't reach the parent
    /// zshrs global.
    pub priv_prefix: &'a [String],
}

/// `tag` — the compsys tag this call belongs to (used for the
/// `:completion:…:TAG command` zstyle lookup).
///
/// `argv` — the command to run. `argv[0]` is the binary, the rest are
/// arguments. The shell version takes a `tag` as `$1` and the command
/// as `$2..$#`; we hoist tag to its own parameter for clarity.
pub fn _call_program(
    state: &MainCompleteState,
    tag: &str,
    argv: &[String],
    opts: &CallProgramOpts<'_>,
) -> Option<CallProgramResult> {
    if argv.is_empty() {
        return None;
    }

    // Build curcontext. The shell mutates it when -p + priv_prefix
    // are active: `${curcontext%:*}/${first non-=-bearing priv-prefix
    // arg}:` — we mirror.
    let mut curcontext = state.ctx.context.clone();
    let mut used_priv_prefix = false;
    let mut prefix: Vec<String> = Vec::new();

    if opts.use_priv_prefix && !opts.priv_prefix.is_empty() {
        // Find the first priv-prefix entry that doesn't look like
        // an env-var assignment (`name=value`, not `\=value`).
        let first_non_env = opts
            .priv_prefix
            .iter()
            .find(|p| {
                let pat = p.as_str();
                // Skip entries that are `[^\\]=` (an env-style assign
                // unlike escaped `\=`).
                !pat.contains('=') || pat.contains("\\=")
            })
            .cloned()
            .unwrap_or_else(|| opts.priv_prefix[0].clone());

        // Trim trailing `:` if present, append `/firstarg:`.
        let base = curcontext.trim_end_matches(':');
        let stripped = match base.rfind(':') {
            Some(i) => &base[..i],
            None => base,
        };
        curcontext = format!("{}/{}:", stripped, first_non_env);

        // Check `:completion:${curcontext}:${tag} gain-privileges`.
        let style_ctx = format!(":completion:{}:{}", curcontext, tag);
        let gain = state
            .styles
            .lookup_bool(&style_ctx, "gain-privileges")
            .unwrap_or(false);
        if gain {
            prefix = opts.priv_prefix.to_vec();
            used_priv_prefix = true;
        }
    }

    // Build the command to actually exec. Three cases (shell:27-34):
    //
    //   case A: `command` style is set AND value starts with `-`
    //           → use `$tmp[2,-1] $argv[2,-1]` (strip the `-` from
    //             $tmp, then append every arg AFTER argv[1])
    //   case B: `command` style is set AND value doesn't start `-`
    //           → use `$prefix $tmp` (just the style value, prefixed
    //             by priv-prefix when -p)
    //   case C: no style set
    //           → use `$prefix $argv[2,-1]` (every arg after argv[1])
    //
    // NOTE: shell `$argv[2,-1]` is "all args from index 2" — i.e.
    // dropping `$1` which was the TAG. Our `argv` already has the
    // tag hoisted out, so `argv[2,-1]` maps to `argv[1..]` for cases
    // A and C. For case B the style value REPLACES the command, no
    // appending of our argv.
    let style_ctx = format!(":completion:{}:{}", curcontext, tag);
    let tmp = state
        .styles
        .lookup_str(&style_ctx, "command")
        .map(String::from);

    let (final_cmd, final_args): (String, Vec<String>) = if let Some(t) = tmp {
        if let Some(rest) = t.strip_prefix('-') {
            // Case A: shell-eval the stripped tmp, append our args
            // beyond [0]. We don't have a shell evaluator at this
            // layer — split on whitespace which covers the common
            // override forms (`-pacman -Qq` etc.) without quoting.
            let mut parts: Vec<String> = rest.split_whitespace().map(String::from).collect();
            if argv.len() > 1 {
                parts.extend(argv[1..].iter().cloned());
            }
            if parts.is_empty() {
                return None;
            }
            (parts.remove(0), parts)
        } else {
            // Case B: priv-prefix + tmp, replacing argv wholesale.
            let mut parts: Vec<String> =
                prefix.iter().cloned().collect::<Vec<_>>();
            parts.extend(t.split_whitespace().map(String::from));
            if parts.is_empty() {
                return None;
            }
            (parts.remove(0), parts)
        }
    } else {
        // Case C: priv-prefix + original argv.
        let mut parts: Vec<String> = prefix.iter().cloned().collect();
        parts.extend(argv.iter().cloned());
        (parts.remove(0), parts)
    };

    // Run.
    let mut cmd = Command::new(&final_cmd);
    cmd.args(&final_args);
    cmd.env("COLUMNS", "999");
    if !opts.skip_locale {
        // _comp_locale: pin C locale so command output isn't
        // translated and is parser-friendly.
        cmd.env("LC_ALL", "C");
        cmd.env("LANG", "C");
        cmd.env("LC_MESSAGES", "C");
    }

    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(CallProgramResult {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        used_priv_prefix,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_state() -> MainCompleteState {
        let mut s = MainCompleteState::new("", 0);
        s.ctx.context = ":complete::git:".into();
        s
    }

    #[test]
    fn case_a_dash_prefix_strips_and_appends_remaining_args() {
        let mut state = mk_state();
        // `:completion::complete::git::branches command -echo hello`
        // means: run `echo hello`, then append every arg from argv[1..].
        state.styles.set(
            ":completion::complete::git::branches",
            "command",
            vec!["-echo HELLO".into()],
            false,
        );
        let r = _call_program(
            &state,
            "branches",
            &["git".into(), "branch".into(), "--list".into()],
            &CallProgramOpts {
                skip_locale: true,
                ..Default::default()
            },
        );
        let out = r.expect("ran").stdout;
        // Echo emits "HELLO branch --list\n".
        assert!(out.contains("HELLO"), "got {out:?}");
        assert!(out.contains("branch"), "got {out:?}");
        assert!(out.contains("--list"), "got {out:?}");
    }

    #[test]
    fn case_b_no_dash_replaces_argv_wholesale() {
        let mut state = mk_state();
        state.styles.set(
            ":completion::complete::git::branches",
            "command",
            vec!["echo override-only".into()],
            false,
        );
        let r = _call_program(
            &state,
            "branches",
            &["git".into(), "branch".into(), "--list".into()],
            &CallProgramOpts {
                skip_locale: true,
                ..Default::default()
            },
        );
        let out = r.expect("ran").stdout;
        // No `branch --list` — the style replaces.
        assert!(out.contains("override-only"), "got {out:?}");
        assert!(!out.contains("branch"), "argv leaked into wholesale-replace path: {out:?}");
    }

    #[test]
    fn case_c_no_style_runs_argv_directly() {
        let state = mk_state();
        let r = _call_program(
            &state,
            "branches",
            &["echo".into(), "raw-args".into()],
            &CallProgramOpts {
                skip_locale: true,
                ..Default::default()
            },
        );
        let out = r.expect("ran").stdout;
        assert!(out.contains("raw-args"), "got {out:?}");
    }

    #[test]
    fn used_priv_prefix_flag_set_when_gain_privileges_active() {
        let mut state = mk_state();
        // Starting curcontext is `:complete::git:`. With `-p` and a
        // non-empty priv_prefix, my port mutates curcontext to
        // `:complete:/<first non-env priv arg>:` (shell:11). With
        // priv_prefix=["echo"], that's `:complete:/echo:`. The
        // gain-privileges style is then looked up at
        // `:completion::complete:/echo::branches`.
        state.styles.set(
            ":completion::complete:/echo::branches",
            "gain-privileges",
            vec!["true".into()],
            false,
        );
        let r = _call_program(
            &state,
            "branches",
            &["echo".into(), "hi".into()],
            &CallProgramOpts {
                use_priv_prefix: true,
                priv_prefix: &["echo".into()], // priv-prefix; must be a runnable cmd so the
                                               // call still succeeds and we see the flag.
                skip_locale: true,
            },
        );
        let r = r.expect("ran");
        assert!(r.used_priv_prefix);
    }

    #[test]
    fn no_priv_prefix_when_gain_privileges_unset() {
        let mut state = mk_state();
        // No gain-privileges style => priv prefix is NOT applied.
        let r = _call_program(
            &state,
            "branches",
            &["echo".into(), "hi".into()],
            &CallProgramOpts {
                use_priv_prefix: true,
                priv_prefix: &["sudo".into()],
                skip_locale: true,
            },
        );
        let r = r.expect("ran");
        assert!(!r.used_priv_prefix, "should not elevate without gain-privileges style");
    }

    #[test]
    fn empty_argv_returns_none() {
        let state = mk_state();
        let r = _call_program(&state, "x", &[], &CallProgramOpts::default());
        assert!(r.is_none());
    }
}
