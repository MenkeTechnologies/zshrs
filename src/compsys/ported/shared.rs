//! Tiny helpers shared between per-fn ports. Kept here (not in
//! library.rs) so the `ported/` tree stands alone.

use std::path::Path;

// =====================================================================
// Function-local parameter declarations for the Rust ports.
// =====================================================================
//
// An upstream completion function opens with a `local`/`typeset` line
// (`Completion/Base/Core/_main_complete:11,27-54`). Every scratch name
// it touches is therefore created at `locallevel`, reads back as
// `…-local` through `${(t)name}`, and is unwound by `endparamscope`
// when the function returns.
//
// A Rust port assigns the same names with `setsparam`/`setaparam`,
// which routes through `createparam(name, PM_SCALAR)` — no PM_LOCAL,
// so the parameter is born at level 0 and both properties are lost:
// `${(t)_comp_tags}` reads `scalar` and the name survives the call.
// That is observable: the user's `_parameters`
// (`~/.zpwr/autoload/comp_utils/_parameters:34`) filters candidates
// with `${(@k)parameters[(R)${pattern[2]}~*local*]}`, i.e. it drops
// every parameter whose type string contains `local`. Leaked port
// scratch names slipped through that filter and `unset <TAB>` offered
// them alongside the user's real parameters.
//
// `declare_locals` is the one place that gap is closed: it is the
// Rust-port spelling of the upstream `local NAME …` line and mirrors
// the PM_LOCAL branch of `bin_typeset` (`src/ported/builtin.rs:5570`,
// port of `Src/builtin.c:2469-2575`).

pub use crate::ported::zsh_h::{PM_ARRAY, PM_HASHED, PM_INTEGER, PM_READONLY, PM_UNIQUE};

/// Declare `names` local to the CURRENT function scope — the Rust-port
/// equivalent of an upstream completion function's `local NAME …` line.
///
/// `kind` carries the type/attribute bits the shell source spells out
/// (`PM_ARRAY` for `local -a`, `PM_HASHED` for `local -A`, `PM_UNIQUE`
/// for `typeset -U`); pass `0` for a plain scalar `local`.
///
/// Mirrors `Src/builtin.c:2469-2575` (`typeset_single`'s PM_LOCAL arm):
/// only allocate a shadow when the visible parameter lives at a LOWER
/// scope than the current `locallevel`, then stamp `pm->level =
/// locallevel` so `endparamscope` unwinds it. At top level
/// (`locallevel == 0`) C's `pm->level < locallevel` can never hold, so
/// nothing is declared — the port then behaves exactly as it does today
/// when run outside a function (unit tests, `--doctor`).
pub fn declare_locals(names: &[&str], kind: u32) {
    use crate::ported::params::{createparam, locallevel, paramtab};
    use crate::ported::zsh_h::{PM_HIDE, PM_LOCAL, PM_SPECIAL};
    use std::sync::atomic::Ordering;

    let cur = locallevel.load(Ordering::Relaxed); // c:2469 locallevel
    if cur == 0 {
        return;
    }
    for name in names {
        // c:2469 — `(!pm || pm->level < locallevel)`.
        //
        // The same table read also settles `newspecial`. c:2083-2085:
        //   if ((pm->node.flags & PM_SPECIAL)
        //       && !(on & PM_HIDE) && !(pm->node.flags & PM_HIDE & ~off))
        //       newspecial = NS_NORMAL;
        // i.e. localizing a PM_SPECIAL parameter keeps it special unless
        // `-h` hides it, on either the special itself or this statement.
        let (needs_shadow, newspecial) = paramtab()
            .read()
            .ok()
            .and_then(|t| {
                t.get(*name).map(|pm| {
                    let special = (pm.node.flags as u32 & PM_SPECIAL) != 0
                        && (kind & PM_HIDE) == 0
                        && (pm.node.flags as u32 & PM_HIDE) == 0;
                    (pm.level < cur, special)
                })
            })
            .unwrap_or((true, false));
        if !needs_shadow {
            continue;
        }
        // c:2470 — `createparam(pname, on | PM_LOCAL)`.
        let _ = createparam(name, (kind | PM_LOCAL) as i32);
        // c:2575 — `else if (on & PM_LOCAL) pm->level = locallevel;`
        // plus the attribute stamp so `typeset -U` keeps PM_UNIQUE.
        if let Ok(mut tab) = paramtab().write() {
            if let Some(pm) = tab.get_mut(*name) {
                pm.level = cur;
                pm.node.flags |= (kind & PM_UNIQUE) as i32;
                // c:2425 — `pm->node.flags = (PM_TYPE(pm->node.flags) | on
                // | PM_SPECIAL) & ~off;`. `createparam` deliberately drops
                // the bit (`Src/params.c:1174` stores `flags & ~PM_LOCAL`
                // on the fresh struct), so the special-ness of the shadowed
                // parameter has to be re-stamped here the way the
                // `newspecial` arm of `typeset_single` does. Without it
                // `integer SECONDS=0` (_main_complete sh:162) read
                // `integer-local` where zsh reads `integer-local-special`,
                // and `_parameters` — which drops every candidate whose
                // type matches `*local*` — mis-classified the shadow.
                if newspecial {
                    pm.node.flags |= PM_SPECIAL as i32;
                }
            }
        }
    }
}

/// A parameter scope for a Rust port that is invoked as a DIRECT Rust
/// call rather than through `dispatch_function_call`.
///
/// `declare_locals` only stamps `pm->level = locallevel`; the unwind is
/// `endparamscope`'s job, and that runs from `doshfunc`. A port reached
/// by a plain Rust call (`_alternative` -> `_tags(&…)` /
/// `_next_label(&…)` -> `_description(&…)`) therefore never gets one, so
/// every name in its `declare_locals` list stayed shadowed for the rest
/// of the CALLER's body.
///
/// Concretely: `_tags` declares `tmp` and `_description` declares
/// `opts`, and both names are `_files`' own locals holding the results
/// of its `zparseopts -a opts '/=tmp' 'g+:-=tmp' … W: …` line. After
/// `_alternative` ran either port, `_files` saw `opts=()` / `tmp=()`, so
/// `-W /dev` and `-g '*(-%b,-/)'` were both dropped — `mount /dev/<TAB>`
/// listed every file in `$PWD` and `PATH=…:<TAB>` listed files instead
/// of directories.
///
/// Holding one of these for the port's body reproduces the visible half
/// of what a real shell function gets from `endparamscope`
/// (`Src/params.c:5867-5933`, the `pm->level > locallevel` arm): each
/// declared name is put back exactly as the caller left it.
///
/// It restores by NAME rather than by bumping `locallevel` and calling
/// `endparamscope`, because a port's body also writes caller-visible
/// state (`_comp_tags`, `curtag`, the `expl` array named by
/// `_description`'s `$2`). A whole-scope unwind takes those with it —
/// `_tags` then reported "comptags: no tags registered" for every
/// context.
pub struct LocalScope {
    saved: Vec<(String, Option<Box<crate::ported::zsh_h::param>>)>,
}

impl LocalScope {
    /// Declare `names` local (see [`declare_locals`]) and remember what
    /// each one looked like beforehand.
    pub fn declare(names: &[&str], kind: u32) -> Self {
        let mut scope = LocalScope { saved: Vec::new() };
        scope.also(names, kind);
        scope
    }

    /// Add more names to an existing scope — the port equivalent of a
    /// second `local -a …` line.
    pub fn also(&mut self, names: &[&str], kind: u32) {
        if let Ok(tab) = crate::ported::params::paramtab().read() {
            for name in names {
                self.saved
                    .push(((*name).to_string(), tab.get(*name).cloned()));
            }
        }
        declare_locals(names, kind);
    }

    /// `local NAME="$NAME"` — see [`declare_locals_keeping_value`].
    pub fn also_keeping_value(&mut self, names: &[&str]) {
        if let Ok(tab) = crate::ported::params::paramtab().read() {
            for name in names {
                self.saved
                    .push(((*name).to_string(), tab.get(*name).cloned()));
            }
        }
        declare_locals_keeping_value(names);
    }
}

impl Drop for LocalScope {
    fn drop(&mut self) {
        if let Ok(mut tab) = crate::ported::params::paramtab().write() {
            for (name, prev) in self.saved.iter().rev() {
                match prev {
                    Some(pm) => {
                        tab.insert(name.clone(), pm.clone());
                    }
                    None => {
                        tab.remove(name);
                    }
                }
            }
        }
    }
}

/// `typeset -r NAME` applied AFTER the value is in place — the second
/// half of an upstream `local -ar NAME=(…)` / `local -r NAME=…` line.
///
/// [`declare_locals`] cannot carry `PM_READONLY` itself: `createparam`
/// stamps the bit immediately, and the port assigns the value on the
/// NEXT statement, so the assignment would be rejected as a write to a
/// read-only parameter. Upstream has no such split — `local -ar x=(…)`
/// is one operation whose value lands before the bit does — so the port
/// declares, assigns, then calls this.
///
/// Mirrors the `PM_READONLY` arm of `typeset_single`
/// (`Src/builtin.c:2469-2575`): the bit is OR'd onto the existing
/// `pm->node.flags`, and because the param already lives at
/// `locallevel`, `endparamscope` unwinds it with the rest of the scope.
///
/// Skipped at `locallevel == 0` for the same reason [`declare_locals`]
/// returns early there: with no function scope there is no shadow to
/// stamp and no `endparamscope` to unstamp it, so the bit would pin the
/// caller's GLOBAL parameter read-only forever — the next completion's
/// own assignment would then fail with "read-only variable". Upstream
/// cannot reach that state at all: `local -ar` is a syntax error outside
/// a function.
pub fn mark_readonly(names: &[&str]) {
    use crate::ported::params::{locallevel, paramtab};
    use crate::ported::zsh_h::PM_READONLY;
    use std::sync::atomic::Ordering;
    if locallevel.load(Ordering::Relaxed) == 0 {
        return;
    }
    if let Ok(mut tab) = paramtab().write() {
        for name in names {
            if let Some(pm) = tab.get_mut(*name) {
                pm.node.flags |= PM_READONLY as i32;
            }
        }
    }
}

/// `local NAME="$NAME"` — declare `names` local while carrying the
/// enclosing scope's scalar value into the shadow.
///
/// Upstream spells this out where the completer chain must keep
/// reading an inherited value it is also allowed to overwrite:
/// `_main_complete:31` (`curcontext="$curcontext"`), `_tags:19`,
/// `_dispatch:4`. A bare [`declare_locals`] would hand the port an
/// empty parameter instead.
pub fn declare_locals_keeping_value(names: &[&str]) {
    for name in names {
        let inherited = crate::ported::params::getsparam(name);
        declare_locals(&[name], 0);
        if let Some(v) = inherited {
            let _ = crate::ported::params::setsparam(name, &v);
        }
    }
}
/// The directory list `compinit` must scan: `$fpath` as it stands at
/// call time (`Completion/compinit:523` `for _i_dir in $fpath`, and
/// `compaudit` at sh:455), falling back to `env_fpath` when the array is
/// unset or empty.
///
/// `ShellExecutor::fpath` (vm_helper.rs:532) is seeded once at startup
/// from `$FPATH` (vm_helper.rs:1174/1287) and never resynced, so the
/// `fpath=( … )` line that precedes `compinit` in every .zshrc was
/// invisible to the scan. With `$FPATH` exported the two agreed by
/// accident; without it — `zsh -f`, a login shell that builds `fpath` in
/// .zshrc, the parity harness's child env — the scan got ZERO
/// directories, and the worker's empty result was then written over the
/// completion cache, leaving `$_comps` empty and every command falling
/// through to `-default-`.
pub fn compinit_scan_dirs(env_fpath: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    match crate::ported::params::getaparam("fpath") {
        Some(live) if !live.is_empty() => live.iter().map(std::path::PathBuf::from).collect(),
        _ => env_fpath.to_vec(),
    }
}

/// `is_executable` — see implementation.
pub fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = path.metadata() {
            let mode = meta.permissions().mode();
            return mode & 0o111 != 0;
        }
    }
    #[cfg(not(unix))]
    {
        if let Some(ext) = path.extension() {
            let ext = ext.to_string_lossy().to_lowercase();
            return matches!(ext.as_str(), "exe" | "bat" | "cmd" | "com");
        }
    }
    false
}

/// Shell-glob matcher — supports `*`, `?`, and `(a|b|c)`
/// alternation (zsh extended-glob's `(…|…)` form). Sufficient for
/// the patterns end-user completion files use (e.g.
/// `*.(md|rs|toml)` from `_suffix_alias_files`).
pub fn glob_matches(pattern: &str, text: &str) -> bool {
    // Handle leading `(alt1|alt2|…)` at the top level — split at the
    // matching close paren, try each alternative concatenated with
    // the remainder.
    if let Some(rest) = pattern.strip_prefix('(') {
        if let Some(close) = find_top_close_paren(rest) {
            let group = &rest[..close];
            let after = &rest[close + 1..];
            return group.split('|').any(|alt| {
                let combined = format!("{}{}", alt, after);
                glob_matches(&combined, text)
            });
        }
    }
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_helper(&pat, &txt)
}

fn find_top_close_paren(s: &str) -> Option<usize> {
    let mut depth: i32 = 1;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn glob_helper(pat: &[char], txt: &[char]) -> bool {
    if pat.is_empty() {
        return txt.is_empty();
    }
    // Inline alternation at any position: when we encounter `(...)`,
    // re-route through `glob_matches` on the remainder.
    if pat[0] == '(' {
        let rest: String = pat[1..].iter().collect();
        let txt_str: String = txt.iter().collect();
        if let Some(close) = find_top_close_paren(&rest) {
            let group = &rest[..close];
            let after = &rest[close + 1..];
            return group.split('|').any(|alt| {
                let combined = format!("{}{}", alt, after);
                glob_matches(&combined, &txt_str)
            });
        }
    }
    match pat[0] {
        '*' => {
            for i in 0..=txt.len() {
                if glob_helper(&pat[1..], &txt[i..]) {
                    return true;
                }
            }
            false
        }
        '?' => !txt.is_empty() && glob_helper(&pat[1..], &txt[1..]),
        c => !txt.is_empty() && txt[0] == c && glob_helper(&pat[1..], &txt[1..]),
    }
}

/// Shell-glob matcher mirror of the helper that used to live in
/// `compsys/functions.rs` — kept as a separate symbol because callers
/// were spelled `functions::glob_match(...)`, distinct from
/// `glob_matches` above (which the `library.rs`/`ported/_path_files`
/// code used). Both share semantics; the duplicate is intentional for
/// API-shape compat with both call-site ()/* styles */.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    glob_matches(pattern, text)
}

/// Levenshtein edit distance, used by `_approximate`, `_correct`,
/// `_correct_filename`, and `_correct_word`. Moved out of
/// `compsys/functions.rs` so it can be shared across the per-fn ports
/// without introducing a circular dependency between them.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();

    let mut dp = vec![vec![0; n + 1]; m + 1];

    // Levenshtein DP base row/col init — needless_range_loop trips here
    // but the index IS the value being written, not a positional access.
    #[allow(clippy::needless_range_loop)]
    for i in 0..=m {
        dp[i][0] = i;
    }
    #[allow(clippy::needless_range_loop)]
    for j in 0..=n {
        dp[0][j] = j;
    }

    for i in 1..=m {
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }

    dp[m][n]
}

/// Check if a string matches any ignored pattern. Extracted from
/// `compsys/base.rs::is_ignored`. Uses the same `glob_match` helper
/// as the rest of the per-fn ports.
pub fn is_ignored(s: &str, patterns: &[String]) -> bool {
    for pattern in patterns {
        if glob_match(pattern, s) {
            return true;
        }
    }
    false
}

/// `get_ignored_patterns(context)` — collect `ignored-patterns`
/// zstyle values for `context` via the real `lookupstyle` in
/// `src/ported/modules/zutil.rs`.
pub fn get_ignored_patterns(context: &str) -> Vec<String> {
    crate::ported::modules::zutil::lookupstyle(context, "ignored-patterns")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// compinit sh:523 — the scan reads `$fpath`, not the `$FPATH` the
    /// process happened to inherit.
    ///
    /// Regression: `builtin_compinit` scanned `ShellExecutor::fpath`,
    /// which is env-seeded at startup and never resynced, so
    /// `fpath=( … ); compinit` scanned the STARTUP list. With no `FPATH`
    /// exported that list is empty, the worker returned zero completers,
    /// and the empty result was written over the completion cache —
    /// `$_comps` empty, every command resolved to `-default-`.
    #[test]
    fn compinit_scans_the_live_fpath_array_not_the_startup_env() {
        use std::path::PathBuf;
        let _g = crate::test_util::global_state_lock();
        let env_seeded = vec![PathBuf::from("/from/FPATH/env")];

        crate::ported::params::setaparam(
            "fpath",
            vec!["/live/one".to_string(), "/live/two".to_string()],
        );
        assert_eq!(
            compinit_scan_dirs(&env_seeded),
            vec![PathBuf::from("/live/one"), PathBuf::from("/live/two")],
            "sh:523 scans $fpath"
        );

        // Unset / empty array — keep the env-derived list rather than
        // scanning nothing.
        crate::ported::params::setaparam("fpath", Vec::new());
        assert_eq!(compinit_scan_dirs(&env_seeded), env_seeded);
        crate::ported::params::unsetparam("fpath");
        assert_eq!(compinit_scan_dirs(&env_seeded), env_seeded);
    }

    /// `mark_readonly` is the `-r` of `local -ar` (sh:52) and must not
    /// escape the function scope: stamped at `locallevel == 0` the bit
    /// would pin the caller's global read-only forever, and the next
    /// completion's own assignment would fail with "read-only variable".
    #[test]
    fn mark_readonly_is_scoped_to_a_function() {
        use crate::ported::modules::parameter::paramtypestr;
        let _g = crate::test_util::global_state_lock();
        let type_of = |n: &str| {
            crate::ported::params::paramtab()
                .read()
                .ok()
                .and_then(|t| t.get(n).map(|pm| paramtypestr(pm)))
                .unwrap_or_default()
        };

        crate::ported::params::setaparam("_ro_probe", vec!["a".to_string()]);
        mark_readonly(&["_ro_probe"]);
        assert_eq!(type_of("_ro_probe"), "array", "no scope — no readonly bit");

        crate::ported::utils::inc_locallevel();
        declare_locals(&["_ro_probe"], PM_ARRAY);
        crate::ported::params::setaparam("_ro_probe", vec!["b".to_string()]);
        mark_readonly(&["_ro_probe"]);
        assert_eq!(type_of("_ro_probe"), "array-local-readonly");
        crate::ported::params::endparamscope();
        assert_eq!(
            type_of("_ro_probe"),
            "array",
            "endparamscope must unwind the readonly shadow"
        );
        crate::ported::params::unsetparam("_ro_probe");
    }

    // glob_match coverage migrated from `compsys/base.rs` when the
    // local glob_match helper there was removed in favor of this
    // single shared implementation.

    #[test]
    fn test_glob_match_simple() {
        assert!(glob_match("*.txt", "file.txt"));
        assert!(glob_match("*.txt", ".txt"));
        assert!(!glob_match("*.txt", "file.rs"));
    }

    #[test]
    fn test_glob_match_question() {
        assert!(glob_match("file?.txt", "file1.txt"));
        assert!(glob_match("file?.txt", "fileX.txt"));
        assert!(!glob_match("file?.txt", "file.txt"));
        assert!(!glob_match("file?.txt", "file12.txt"));
    }

    #[test]
    fn test_glob_match_star_middle() {
        assert!(glob_match("foo*bar", "foobar"));
        assert!(glob_match("foo*bar", "foo123bar"));
        assert!(glob_match("foo*bar", "fooXYZbar"));
        assert!(!glob_match("foo*bar", "foobaz"));
    }

    #[test]
    fn test_glob_match_multiple_stars() {
        assert!(glob_match("*foo*", "foo"));
        assert!(glob_match("*foo*", "afoo"));
        assert!(glob_match("*foo*", "foob"));
        assert!(glob_match("*foo*", "afoob"));
        assert!(!glob_match("*foo*", "bar"));
    }

    #[test]
    fn test_glob_match_exact() {
        assert!(glob_match("exact", "exact"));
        assert!(!glob_match("exact", "exacty"));
        assert!(!glob_match("exact", "xact"));
    }
}

/// Call another compsys completer BY NAME, so `$fpath` arbitration still runs.
///
/// zshrs-original — C has no port tree to arbitrate against. Every upstream
/// completer reaches its helpers as a bare command word (`_files "$@"`), which
/// goes through the normal function lookup, so a user's own `_files` earlier in
/// `$fpath` wins. A Rust port that calls its sibling port as a plain Rust fn
/// skips that lookup entirely: `crate::ported::exec::dispatch_function_call` is
/// the only path that consults `compsys::router::try_rust_dispatch` and its
/// `has_fpath_override` gate, so the user's file is silently dead.
///
/// This is not hypothetical. `_command_names` had the same defect (fixed in
/// b8e714f7be) and `_parameters` had it in the `-brace-parameter-` /
/// `-subscript-` contexts, which is why `echo ${<TAB>` offered zshrs's own
/// parameter list instead of the user's. On this host `_files` is overridden at
/// `~/.zpwr/autoload/comp_utils/_files` (fpath position 18, ahead of the stock
/// tree at 24) and ten ports call it directly.
///
/// `fallback` runs only when no shell function and no registered port claims
/// the name — i.e. in unit tests with no executor installed. It is a DEGRADED
/// stand-in for the `doshfunc` frame the dispatch path opens: no `FUNCSTACK`
/// entry, no param scope, no `locallevel` bump. A caller whose sh semantics
/// depend on the callee's scope depth — anything driving `comptags`, which is
/// indexed by `locallevel` (`Src/Zle/computil.c:3782` "Array of tag-set
/// infos. Index is the locallevel", `:3873` `level = locallevel -
/// (args[0][2] ? 1 : 0)`) — must supply the missing piece inside its own
/// `fallback` closure rather than assume this helper does it.
///
/// # Naming convention for ports
///
/// A port with a dispatching entry point splits in two, and the names are
/// chosen so that the OBVIOUS call is the CORRECT one:
///
/// * `_NAME` — the dispatching wrapper, one line: `call_compfn("_NAME",
///   args, || _NAME_impl(args))`. This is what every sibling port calls, and
///   it matches the zsh function name character for character.
/// * `_NAME_impl` — the raw body. Two callers, both of which must not
///   re-enter dispatch: the wrapper's own `fallback` above, and the
///   `compsys::router` arm for `"_NAME"`. **The router arm MUST name
///   `_NAME_impl`.** Pointing it at `_NAME` makes dispatch call the wrapper,
///   which calls dispatch, forever.
///
/// Anything else that names `_NAME_impl` is asserting it genuinely needs no
/// `doshfunc` frame — sh `continue` expressed as recursion
/// (`_next_label.rs`), or a callee whose `comptags` level the caller manages
/// by hand (`_message.rs`, `_wanted.rs`). Those sites carry a comment saying
/// why.
pub fn call_compfn(name: &str, args: &[String], fallback: impl FnOnce() -> i32) -> i32 {
    crate::ported::exec::dispatch_function_call(name, args).unwrap_or_else(fallback)
}

// =====================================================================
// `scriptname` for the duration of a port call.
// =====================================================================
//
// `doshfunc` sets `scriptname` to the function's own name on entry
// (`Src/exec.c:5963` — `scriptname = dupstring(name);`) and restores the
// caller's on exit (`Src/exec.c:6124` — `scriptname = funcsave->scriptname;`).
// That is what every diagnostic reads: `zwarning` prints it ahead of the
// builtin name (`Src/utils.c:147-155`), so an error raised by a builtin
// inside `_tags` reads `_tags:comptags:36: ...`.
//
// The Rust ports reach that same builtin without a `doshfunc` frame. Ports
// call each other as plain Rust calls — `_describe` invokes `_tags` directly
// (`Base/Utility/_describe.rs`) — and only
// `crate::ported::exec::dispatch_function_call` goes through `doshfunc`. So
// `scriptname` kept whatever shell function was last entered and every
// diagnostic named the wrong function:
//
//     zsh    _tags:comptags:36: can only be called from completion function
//     zshrs  _describe:comptags: can only be called from completion function
//
// `FnScope::enter` is the `scriptname` half of that prologue/epilogue, applied
// at the entry of each port so a port called either way reports identically.
//
// `FnScope` also carries the `lineno` half. In C the second field of the
// diagnostic prefix is the GLOBAL `lineno`, printed by `zerrmsg`
// (`Src/utils.c:301-305` — `if ((unset(SHINSTDIN) || locallevel) && lineno)
// fprintf(file, "%lld: ", lineno);`), and it is maintained by the wordcode
// line markers as each statement of the function body executes
// (`Src/exec.c:1356` — `lineno = code - 1;`, `Src/exec.c:2057` —
// `lineno = WC_PIPE_LINENO(pcode) - 1;`). A Rust port has no wordcode, so
// nothing advances that counter and the field came out empty:
//
//     zsh    _describe:compdescribe:129: no parsed state
//     zshrs  _describe:compdescribe: no parsed state
//
// `execlist` saves `lineno` on entry to a body and restores it on exit
// (`Src/exec.c:1429` — `oldlineno = lineno;`, `Src/exec.c:1696` —
// `lineno = oldlineno;`), which is what makes a nested call leave the
// caller's line intact. `FnScope` reproduces that save/restore, and
// [`set_sh_lineno`] is what a port calls to stand in for the line marker.
//
// Entry deliberately publishes 0 ("unknown"), not the caller's line: 0 is the
// value `zerrmsg` treats as "no line to print", so a statement that has not
// been annotated yet keeps today's behaviour (field absent) instead of
// inheriting a number belonging to a different file. A wrong line number is
// worse than a missing one — it points the reader at the wrong function.

/// RAII guard publishing `scriptname` and `lineno` for the body of a Rust
/// compsys port, mirroring `doshfunc`'s `scriptname` save/set/restore and
/// `execlist`'s `lineno` save/restore.
pub struct FnScope {
    saved: Option<String>,
    saved_lineno: u64,
}

impl FnScope {
    /// `scriptname = dupstring(name)` (`Src/exec.c:5963`) plus
    /// `oldlineno = lineno` (`Src/exec.c:1429`), remembering the caller's
    /// values for [`Drop`].
    pub fn enter(name: &str) -> Self {
        let saved = crate::ported::utils::scriptname_get();
        crate::ported::utils::set_scriptname(Some(name.to_string()));
        let saved_lineno = crate::ported::lex::lineno();
        // No wordcode line marker has run for this body yet, and the caller's
        // line belongs to a different file — publish "unknown" so `zerrmsg`
        // omits the field (`Src/utils.c:301` — `&& lineno`).
        crate::ported::lex::set_lineno(0);
        FnScope {
            saved,
            saved_lineno,
        }
    }
}

impl Drop for FnScope {
    /// `scriptname = funcsave->scriptname` (`Src/exec.c:6124`) and
    /// `lineno = oldlineno` (`Src/exec.c:1696`).
    fn drop(&mut self) {
        crate::ported::utils::set_scriptname(self.saved.take());
        crate::ported::lex::set_lineno(self.saved_lineno);
    }
}

/// Publish the upstream shell-source line of the statement a port is about to
/// run, standing in for the wordcode line marker C executes ahead of every
/// statement (`Src/exec.c:2057` — `lineno = WC_PIPE_LINENO(pcode) - 1;`).
///
/// Diagnostics only originate at builtin call sites, so a port only needs this
/// immediately before invoking a builtin that can call `zwarnnam`; the value is
/// then read by `zwarning`/`zerrmsg` (`src/ported/utils.rs:191`).
/// [`FnScope`] restores the caller's line when the port returns.
///
/// `line` MUST be read off the upstream `Completion/**` file the port was
/// translated from. Never estimate it — the `// sh:NN` comments in the ports
/// predate later upstream edits and have drifted (`_describe`'s
/// `compdescribe -I` was annotated `sh:118-121` but lives at line 122 of both
/// zsh 5.9.2 and master).
///
/// `scripts/check_sh_lineno.py` diffs every `sh:NN` annotation against the
/// upstream file and reports the ones whose cited line does not carry the
/// quoted code; run it before trusting an annotation as a `line` argument.
/// An annotation it reports as `unverified`, `suspect` or `out-of-range` has
/// NOT been proven and must not be passed here.
pub fn set_sh_lineno(line: u64) {
    crate::ported::lex::set_lineno(line);
}

/// `eval "$comp"` — the way every compsys dispatcher invokes the completer
/// named by `$_comps` / `$_patcomps` (`_dispatch` sh:31/63/76/87,
/// `_normal` sh:32).
///
/// Upstream never CALLS the completer by name; it `eval`s the registered
/// value as shell text. Two things follow, and a port needs both:
///
///   * the value can carry arguments (`compdef '_files -/' mycmd` stores
///     `_files -/`), which a by-name dispatch cannot express; and
///   * `eval` pushes an `FS_EVAL` funcstack frame named `(eval)`
///     (`Src/builtin.c:6164-6199`), so every completer invoked this way runs
///     one frame deeper than its caller.
///
/// The frame is not cosmetic. Completion code reads `$#funcstack` to decide
/// nesting depth — `_all_labels`/`_alternative` compare it against
/// `_tags_level` — so a missing frame silently changes completion behaviour.
/// A port calling `dispatch_function_call(&comp, &[])` pushes only the
/// completer's own `FS_FUNC` frame; `$funcstack` then reads
/// `_mytest _dispatch _normal …` where zsh reports
/// `_mytest (eval) _dispatch _normal …`.
///
/// `line` is the upstream line the `eval` sits on; publishing it via
/// [`set_sh_lineno`] is what makes `$functrace` read `_dispatch:63` instead
/// of `_dispatch:0` (the caller's line is recorded at push time by `doshfunc`
/// c:6013 / `EvalFuncstackFrame::push` c:6169).
///
/// The body mirrors `static int eval(char **argv)` (`Src/builtin.c:6151`)
/// with `argv == { comp, NULL }`; the funcstack half is the shared canonical
/// port [`crate::ported::exec::EvalFuncstackFrame`] (c:6164-6199), the same
/// one the live `eval` builtin uses, so both entry points build an identical
/// frame.
pub fn eval_comp(comp: &str, line: u64) -> i32 {
    set_sh_lineno(line);
    let oscriptname = crate::ported::utils::scriptname_get(); // c:6154
    let fstack = crate::ported::exec::EvalFuncstackFrame::push(); // c:6164-6199
    if fstack.pushed() {
        // c:6165 — `scriptname = "(eval)";` (inside the `!ineval` arm).
        crate::ported::utils::set_scriptname(Some("(eval)".to_string()));
    }
    // c:6209 — `execode(prog, 1, 0, "eval");` APPENDS its context argument to
    // `zsh_eval_context` for the duration of the body (Src/exec.c:1245-1266).
    //
    // That push is DELIBERATELY NOT made here. `docs/COMPLETION_DISPATCH.md`
    // "Divergence C" records the decision that compsys Rust ports do not
    // synthesize `$zsh_eval_context` frames, and
    // tests/zsh_eval_context_frames.rs::compsys_ports_synthesize_no_eval_context_frames
    // pins it by scanning this tree for that constructor call. (The scan is a
    // plain substring match, so naming the call verbatim here — even in prose —
    // trips it; hence the circumlocution.)
    //
    // A push was added here in 9e55378587 and broke that test. It is left out
    // rather than re-added, and the test is left alone, because the decision is
    // documented and the test is its enforcement — not because the case is
    // clear-cut. It is not: Divergence C reasons that the Rust chain "never
    // evals", whereas this function genuinely does parse and execute a string
    // below, so a frame here would arguably be truthful rather than fabricated.
    // Resolving that tension is a design call for the maintainer; silently
    // overriding a pinned decision from inside a bug fix is not.
    //
    // The funcstack half above is separate and IS pushed: it is a real frame
    // for a call that really happens, and no invariant forbids it.
    //
    // c:6203-6216 — `prog = parse_string(...); … execode(prog, …)`; a NULL
    // prog (parse failure) is `lastval = 1` at c:6215.
    let lastval = crate::ported::exec::execute_script(comp).unwrap_or(1);
    drop(fstack); // c:6218-6219 `if (fpushed) funcstack = funcstack->prev;`
    crate::ported::utils::set_scriptname(oscriptname); // c:6222
    lastval // c:6225
}

#[cfg(test)]
mod lineno_scope_tests {
    use super::*;
    use crate::ported::lex::{lineno, set_lineno};

    /// `zerrmsg` prints the line only when it is non-zero
    /// (`Src/utils.c:301` — `&& lineno`), so a port body must start at 0:
    /// an un-annotated statement has to report NO line rather than inherit
    /// the caller's, which belongs to a different file.
    #[test]
    fn fn_scope_zeroes_lineno_for_the_port_body() {
        let _g = crate::test_util::global_state_lock();
        set_lineno(218); // caller mid-body, e.g. _main_complete sh:218
        {
            let _s = FnScope::enter("_describe");
            assert_eq!(lineno(), 0, "port body must start with no known line");
        }
    }

    /// `execlist` restores the caller's line when a body finishes
    /// (`Src/exec.c:1429` / `Src/exec.c:1696`), which is what keeps a
    /// nested call from renumbering its caller's diagnostics.
    #[test]
    fn fn_scope_restores_the_callers_lineno_on_exit() {
        let _g = crate::test_util::global_state_lock();
        set_lineno(218);
        {
            let _s = FnScope::enter("_describe");
            set_sh_lineno(129);
            assert_eq!(lineno(), 129);
        }
        assert_eq!(lineno(), 218, "caller's line must survive the port call");
    }

    /// Nested ports each restore their own caller, so `_describe` calling
    /// `_tags` leaves `_describe`'s line intact for the statement after it.
    #[test]
    fn nested_fn_scopes_unwind_to_the_right_line() {
        let _g = crate::test_util::global_state_lock();
        set_lineno(0);
        let outer = FnScope::enter("_describe");
        set_sh_lineno(122);
        {
            let _inner = FnScope::enter("_tags");
            assert_eq!(lineno(), 0);
            set_sh_lineno(36); // _tags sh:36 — comptags "-i$prev" …
            assert_eq!(lineno(), 36);
        }
        assert_eq!(lineno(), 122, "_describe's line must survive _tags");
        drop(outer);
        assert_eq!(lineno(), 0);
    }
}
