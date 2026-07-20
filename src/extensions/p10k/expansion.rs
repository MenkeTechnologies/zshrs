//! p10k user-override EXPANSION machinery — port of the
//! CONTENT_EXPANSION / VISUAL_IDENTIFIER_EXPANSION template handling
//! in `_p9k_left_prompt_segment` (p10k.zsh:720-757, mirrored for the
//! right side at p10k.zsh:951-988) and the SHOW_ON_UPGLOB up-tree glob
//! search (`_p9k_fetch_cwd` p10k.zsh:202-234, `_p9k_glob`
//! p10k.zsh:236-256, `_p9k_upglob` p10k.zsh:258-295, applied at
//! p10k.zsh:833-840).
//!
//! In the zsh theme these params hold an EXPANSION TEMPLATE — a string
//! that is spliced into the deferred prompt inside double quotes
//! (p10k:722 `icon_exp_=${_p9k__ret:+\"$_p9k__ret\"}`, p10k:726 same
//! for content) and evaluated by the prompt expander with
//! `P9K_CONTENT` / `P9K_VISUAL_IDENTIFIER` set (p10k:730/846). This
//! port renders eagerly, so the template is evaluated here, once, via
//! the canonical `${(e)...}`-eval pair the rest of zshrs uses:
//! `subst_parse_str` (src/ported/subst.rs:2722, Src/subst.c:1460) →
//! `singsub` (src/ported/subst.rs:1460, Src/subst.c:514) →
//! `untokenize` (src/ported/lex.rs:4812) — the same chain as the
//! `(e)` flag arm at src/ported/subst.rs:14524 and
//! src/extensions/ext_builtins.rs:9323.
//!
//! Cost model, in order:
//!   * default template (`${P9K_CONTENT}` / `${P9K_VISUAL_IDENTIFIER}`)
//!     → returned without ANY shell machinery (the no-override common
//!     case is zero-cost); p10k itself special-cases exactly these
//!     strings (p10k:728-729/738).
//!   * empty template → hide (p10k:722/726 `${_p9k__ret:+...}` — an
//!     empty expansion contributes nothing; p10k:745 `has_icon=0`).
//!   * `$`-free and backquote-free template → literal replacement, no
//!     evaluation (p10k:735 `$icon_exp_ == *'$'*` gate; e.g. the
//!     `'❎'` override at p10k:8902).
//!   * anything else → shell evaluation, memoized in a small LRU keyed
//!     on (template, input value). Templates containing an unescaped
//!     `$(` are non-hermetic (p10k:721/725) and are re-evaluated every
//!     time, never cached — matching `_p9k__non_hermetic_expansion`.
//!
//! Failure is never fatal to the prompt: any parse/eval error logs at
//! debug level and returns the input unchanged.

use crate::extensions::p10k::config::p9k_param;
use crate::ported::lex::untokenize;
use crate::ported::params::{getsparam, setsparam};
use crate::ported::subst::{singsub, subst_parse_str};
use crate::ported::utils::errflag;
use crate::vm_helper::glob_match_static;
use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------
// Template evaluation
// ---------------------------------------------------------------------

/// p10k:724 — `_p9k_param $1 CONTENT_EXPANSION '${P9K_CONTENT}'`, then
/// splice + eval with `P9K_CONTENT` set (p10k:846
/// `${P9K_CONTENT::=...}` runs right before the cached template).
/// The `\r` strip that follows (p10k:749 `${_p9k__c//$'\r'}`) stays at
/// the render.rs call site — it applies to the EXPANDED content there
/// exactly as in the theme.
pub fn apply_content_expansion(segment: &str, state: Option<&str>, content: &str) -> String {
    apply_expansion(
        segment,
        state,
        "CONTENT_EXPANSION",
        "${P9K_CONTENT}",
        "P9K_CONTENT",
        content,
    )
}

/// p10k:720 — `_p9k_param $1 VISUAL_IDENTIFIER_EXPANSION
/// '${P9K_VISUAL_IDENTIFIER}'`; evaluated with `P9K_VISUAL_IDENTIFIER`
/// set to the resolved icon (p10k:730
/// `${P9K_VISUAL_IDENTIFIER::=$icon_}`).
pub fn apply_visual_identifier_expansion(segment: &str, state: Option<&str>, icon: &str) -> String {
    apply_expansion(
        segment,
        state,
        "VISUAL_IDENTIFIER_EXPANSION",
        "${P9K_VISUAL_IDENTIFIER}",
        "P9K_VISUAL_IDENTIFIER",
        icon,
    )
}

/// Shared body of the two expansion entry points. `var_name` is the
/// `P9K_*` parameter the template reads; `value` is what the segment
/// computed (content or icon).
fn apply_expansion(
    segment: &str,
    state: Option<&str>,
    param: &str,
    default_template: &str,
    var_name: &str,
    value: &str,
) -> String {
    let template = p9k_param(segment, state, param, default_template);

    // p10k:728/738 — the exact default template passes the value
    // through untouched; no shell evaluation (common case).
    if template == default_template {
        return value.to_string();
    }
    // p10k:722/726 — `${_p9k__ret:+\"$_p9k__ret\"}`: empty expansion
    // contributes nothing (hides the icon / content).
    if template.is_empty() {
        return String::new();
    }
    // p10k:735 — `$icon_exp_ == *'$'*` gate: a template with no `$`
    // (and no backquote cmdsub) is spliced literally, never evaluated.
    if !template.contains('$') && !template.contains('`') {
        return template;
    }

    // p10k:721/725 — `[[ $_p9k__ret == (|*[^\\])'$('* ]]`: an
    // unescaped `$(` makes the template non-hermetic; p10k re-expands
    // those on every prompt (`_p9k__non_hermetic_expansion`,
    // p10k:842), so they must never be served from the cache.
    let hermetic = !is_non_hermetic(&template);
    if hermetic {
        if let Some(hit) = cache_get(&template, value) {
            return hit;
        }
    }

    // p10k:846 — `${P9K_CONTENT::=$_p9k__ret}` runs before the cached
    // template; the Rust analogue is a paramtab write the evaluated
    // template reads back.
    setsparam(var_name, value);

    match eval_template(&template) {
        Some(expanded) => {
            if hermetic {
                cache_put(&template, value, &expanded);
            }
            expanded
        }
        None => {
            // Guard: template evaluation must never error the prompt.
            tracing::debug!(target: "p10k", segment, param, template = %template,
                "expansion template failed to evaluate; using raw value");
            value.to_string()
        }
    }
}

/// Evaluate one expansion template through the canonical `(e)`-eval
/// chain: `subst_parse_str` re-lexes the raw text into token form
/// (subst.rs:2722; `single=true` because p10k templates are evaluated
/// inside double quotes — p10k:722/726 `\"$_p9k__ret\"`, and
/// subst.rs:2703 documents `single` as exactly that context), then
/// `singsub` (subst.rs:1460) performs parameter / arithmetic / command
/// substitution, then `untokenize` (lex.rs:4812) produces the plain
/// string.
///
/// errflag is cleared before and restored after — the C idiom of
/// `substevalchar` (Src/subst.c:1490: `saved_errflag = errflag;
/// errflag = 0; ... errflag = saved_errflag`) — so a broken template
/// neither aborts the prompt nor leaks error state into the session.
/// `None` = parse or eval failure.
fn eval_template(template: &str) -> Option<String> {
    let saved = errflag.load(Ordering::Relaxed);
    errflag.store(0, Ordering::Relaxed);
    let result = subst_parse_str(template, true, false).map(|parsed| singsub(&parsed));
    let failed = errflag.load(Ordering::Relaxed) != 0;
    errflag.store(saved, Ordering::Relaxed);
    match result {
        Some(expanded) if !failed => Some(untokenize(&expanded)),
        _ => None,
    }
}

/// p10k:721/725 — `[[ $_p9k__ret == (|*[^\\])'$('* ]]`: true when the
/// template contains `$(` at the start or preceded by a non-backslash
/// character (i.e. a live command substitution).
fn is_non_hermetic(template: &str) -> bool {
    let b = template.as_bytes();
    for i in 0..b.len().saturating_sub(1) {
        if b[i] == b'$' && b[i + 1] == b'(' && (i == 0 || b[i - 1] != b'\\') {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------
// Expansion memo — small move-to-front LRU
// ---------------------------------------------------------------------

/// Rust-side analogue of the `_p9k_cache` template memo (p10k:615/834):
/// most templates are pure functions of the input value, so
/// (template, value) → result is cached. Move-to-front Vec keeps it
/// tiny and readable; a prompt has at most a few dozen live
/// (template, value) pairs.
static EXP_CACHE: OnceLock<Mutex<Vec<((String, String), String)>>> = OnceLock::new();
const EXP_CACHE_CAP: usize = 32;

fn cache_get(template: &str, value: &str) -> Option<String> {
    let m = EXP_CACHE.get_or_init(Default::default);
    let mut guard = m.lock().ok()?;
    let pos = guard
        .iter()
        .position(|((t, v), _)| t == template && v == value)?;
    let entry = guard.remove(pos);
    let result = entry.1.clone();
    guard.insert(0, entry);
    Some(result)
}

fn cache_put(template: &str, value: &str, result: &str) {
    let m = EXP_CACHE.get_or_init(Default::default);
    if let Ok(mut guard) = m.lock() {
        guard.retain(|((t, v), _)| !(t == template && v == value));
        guard.insert(
            0,
            (
                (template.to_string(), value.to_string()),
                result.to_string(),
            ),
        );
        guard.truncate(EXP_CACHE_CAP);
    }
}

// ---------------------------------------------------------------------
// SHOW_ON_UPGLOB — port of _p9k_upglob (p10k:258-295)
// ---------------------------------------------------------------------

/// p10k:833-840 —
/// ```zsh
///   _p9k_param $1 SHOW_ON_UPGLOB ''
///   _p9k_cache_set "$p" $non_hermetic $_p9k__ret
///   ...
///   if [[ -n $_p9k__cache_val[3] ]]; then
///     _p9k__has_upglob=1
///     _p9k_upglob $_p9k__cache_val[3] && return   # status 0 = NO match → hide
///   fi
/// ```
/// No pattern configured → the segment always shows. With a pattern,
/// the segment shows only when some parent dir (cwd up to `~` or up to
/// but excluding `/`) contains a matching entry — `_p9k_upglob`
/// returns the 1-based parent index on a match (`&&` fails → segment
/// renders) and 0 on no match (`&& return` → hidden).
pub fn show_on_upglob(segment: &str) -> bool {
    let pattern = p9k_param(segment, None, "SHOW_ON_UPGLOB", "");
    if pattern.is_empty() {
        return true;
    }
    upglob_from(&cwd(), &home_dir(), &pattern)
}

/// Logical cwd — p10k's `_p9k__cwd` is `$PWD` as the shell tracks it
/// (p10k:202-207; same idiom as segments_core.rs:283).
fn cwd() -> String {
    if let Some(p) = getsparam("PWD") {
        if !p.is_empty() {
            return p;
        }
    }
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn home_dir() -> String {
    match getsparam("HOME") {
        Some(h) if !h.is_empty() => h,
        _ => std::env::var("HOME").unwrap_or_default(),
    }
}

/// The dir-walk half of `_p9k_upglob`, parameterized on cwd/home for
/// testability. True when any parent dir contains an entry matching
/// `pattern`.
fn upglob_from(cwd: &str, home: &str, pattern: &str) -> bool {
    parent_dirs(cwd, home)
        .iter()
        .any(|dir| dir_has_match(dir, pattern))
}

/// p10k:210-228 (`_p9k_fetch_cwd`) — the list of dirs `_p9k_upglob`
/// searches, deepest first:
///   * cwd `/` (or non-absolute) → nothing (p10k:211-216);
///   * cwd at/under `~` → cwd up to and INCLUDING `~`, never `~/..`
///     or above (p10k:218-220 — `parent=${:-~/..}`, parts are the
///     components below it, so the shallowest generated dir is `~`
///     itself);
///   * otherwise → cwd up to but EXCLUDING `/` (p10k:222-224 —
///     `parent=/`).
fn parent_dirs(cwd: &str, home: &str) -> Vec<String> {
    if !cwd.starts_with('/') || cwd == "/" {
        return Vec::new();
    }
    let under_home =
        !home.is_empty() && home != "/" && (cwd == home || cwd.starts_with(&format!("{home}/")));
    let mut dirs = Vec::new();
    let mut d = cwd.trim_end_matches('/').to_string();
    loop {
        if d.is_empty() || d == "/" {
            break; // p10k:222-224 — `/` itself is never searched
        }
        dirs.push(d.clone());
        if under_home && d == home {
            break; // p10k:218-220 — stop at `~`, never `~/..`
        }
        d = match d.rfind('/') {
            Some(0) => "/".to_string(),
            Some(i) => d[..i].to_string(),
            None => break,
        };
    }
    dirs
}

/// Per-(dir, pattern) match cache keyed on the dir's mtime — port of
/// `_p9k__glob_cache[$dir/$2]="$stat[1]:$#files"` (p10k:247-254).
/// A changed mtime invalidates the entry; a failed stat caches as -1
/// exactly like `zstat ... || stat=(-1)` (p10k:252). The second-level
/// `_p9k__upsearch_cache` (p10k:266-287) — a memo of the whole
/// upward search validated against the mtime LIST — is folded into
/// this per-dir layer: same freshness guarantees, one map.
static GLOB_CACHE: OnceLock<Mutex<HashMap<(String, String), (i64, bool)>>> = OnceLock::new();

/// p10k:245-256 (`_p9k_glob`) — does `dir` contain an entry matching
/// `pattern`? (The zsh original returns the match COUNT via
/// `$dir/$~2(N:t)`; every caller only tests zero/non-zero.)
fn dir_has_match(dir: &str, pattern: &str) -> bool {
    // p10k:252 — `zstat -A stat +mtime -- $dir 2>/dev/null || stat=(-1)`.
    let mtime = std::fs::metadata(dir).map(|m| m.mtime()).unwrap_or(-1);
    let m = GLOB_CACHE.get_or_init(Default::default);
    let key = (dir.to_string(), pattern.to_string());
    if let Ok(guard) = m.lock() {
        // p10k:248 — `[[ $cached == $_p9k__parent_mtimes[$1]:* ]]`.
        if let Some(&(cached_mtime, matched)) = guard.get(&key) {
            if cached_mtime == mtime {
                return matched;
            }
        }
    }
    // p10k:253 — `local files=($dir/$~2(N:t))`: glob the pattern
    // against the dir's entry NAMES (`:t`), nullglob. `|` at the top
    // level of the pattern is alternation (p10k:2651
    // 'project.json|global.json|...'); the leading-dot rule of
    // filename generation applies (GLOB_DOTS is never set by p10k), so
    // a dotfile only matches an alternative that spells the dot.
    let alts = split_alternatives(pattern);
    let mut matched = false;
    if let Ok(rd) = std::fs::read_dir(dir) {
        'entries: for entry in rd.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            for alt in &alts {
                if name.starts_with('.') && !alt.starts_with('.') {
                    continue;
                }
                // Canonical matcher: vm_helper::glob_match_static
                // (src/vm_helper.rs:4489 — tokenize + patcompile +
                // pattry over Src/pattern.c ports).
                if glob_match_static(&name, alt) {
                    matched = true;
                    break 'entries;
                }
            }
        }
    }
    if let Ok(mut guard) = m.lock() {
        guard.insert(key, (mtime, matched)); // p10k:254
    }
    matched
}

/// Split a `_p9k_upglob` pattern on top-level `|` (alternation), depth
/// counted over `(`/`)` so grouped alternations like
/// `*.(java|class|jar)` (p10k:4562) stay intact.
fn split_alternatives(pattern: &str) -> Vec<String> {
    let mut alts = Vec::new();
    let mut depth = 0usize;
    let mut cur = String::new();
    for c in pattern.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                cur.push(c);
            }
            '|' if depth == 0 => alts.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    alts.push(cur);
    alts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::options::{opt_state_get, opt_state_set};
    use crate::ported::params::{setsparam, unsetparam};

    /// Same harness as config.rs tests: serialize against other
    /// paramtab-touching tests, enable EXECOPT so setsparam works.
    fn with_exec<F: FnOnce()>(body: F) {
        let _g = crate::test_util::global_state_lock();
        let saved = opt_state_get("exec").unwrap_or(false);
        opt_state_set("exec", true);
        body();
        opt_state_set("exec", saved);
    }

    /// p10k:728/738 — the default template short-circuits: value comes
    /// back unchanged with no shell machinery involved.
    #[test]
    fn default_template_short_circuits() {
        with_exec(|| {
            unsetparam("POWERLEVEL9K_DIR_CONTENT_EXPANSION");
            unsetparam("POWERLEVEL9K_CONTENT_EXPANSION");
            assert_eq!(apply_content_expansion("dir", None, "~/src"), "~/src");
            unsetparam("POWERLEVEL9K_DIR_VISUAL_IDENTIFIER_EXPANSION");
            unsetparam("POWERLEVEL9K_VISUAL_IDENTIFIER_EXPANSION");
            assert_eq!(
                apply_visual_identifier_expansion("dir", None, "\u{e0b0}"),
                "\u{e0b0}"
            );
        });
    }

    /// p10k:722/726 — empty override hides; p10k:735-738 — a `$`-free
    /// override is a literal replacement (e.g. p10k:8902 '❎'), no eval.
    #[test]
    fn empty_and_literal_overrides() {
        with_exec(|| {
            setsparam("POWERLEVEL9K_DIR_CONTENT_EXPANSION", "");
            assert_eq!(apply_content_expansion("dir", None, "~/src"), "");
            setsparam("POWERLEVEL9K_DIR_CONTENT_EXPANSION", "fixed");
            assert_eq!(apply_content_expansion("dir", None, "~/src"), "fixed");
            unsetparam("POWERLEVEL9K_DIR_CONTENT_EXPANSION");
        });
    }

    /// A real template evaluates with P9K_CONTENT set — `${P9K_CONTENT:u}`
    /// upcases the segment content through the canonical subst chain.
    #[test]
    fn template_evaluates_with_p9k_content() {
        with_exec(|| {
            setsparam("POWERLEVEL9K_DIR_CONTENT_EXPANSION", "${P9K_CONTENT:u}");
            assert_eq!(apply_content_expansion("dir", None, "abc"), "ABC");
            // second call is the cache path — same result
            assert_eq!(apply_content_expansion("dir", None, "abc"), "ABC");
            unsetparam("POWERLEVEL9K_DIR_CONTENT_EXPANSION");
        });
    }

    /// p10k:721/725 — the non-hermetic `$(` detector: unescaped `$(`
    /// anywhere (start or after a non-backslash) is non-hermetic.
    #[test]
    fn non_hermetic_detection() {
        assert!(is_non_hermetic("$(date)"));
        assert!(is_non_hermetic("x$(date)"));
        assert!(!is_non_hermetic("\\$(date)"));
        assert!(!is_non_hermetic("${P9K_CONTENT}"));
        assert!(!is_non_hermetic(""));
    }

    /// Depth-0 alternation split keeps grouped alternations intact
    /// (p10k:4562 'pom.xml|...|*.(java|class|jar|gradle|clj|cljc)').
    #[test]
    fn alternation_split() {
        assert_eq!(split_alternatives("a"), vec!["a"]);
        assert_eq!(
            split_alternatives("project.json|*.csproj|*.(java|class)"),
            vec!["project.json", "*.csproj", "*.(java|class)"]
        );
    }

    /// p10k:210-228 — walk boundaries: `/` never searched; under-home
    /// walks stop AT home; outside-home walks stop before `/`.
    #[test]
    fn parent_dirs_boundaries() {
        assert!(parent_dirs("/", "/home/u").is_empty());
        assert!(parent_dirs("relative", "/home/u").is_empty());
        assert_eq!(
            parent_dirs("/opt/foo/bar", "/home/u"),
            vec!["/opt/foo/bar", "/opt/foo", "/opt"]
        );
        assert_eq!(
            parent_dirs("/home/u/a/b", "/home/u"),
            vec!["/home/u/a/b", "/home/u/a", "/home/u"]
        );
        assert_eq!(parent_dirs("/home/u", "/home/u"), vec!["/home/u"]);
    }

    /// _p9k_upglob semantics over a real dir tree: a marker anywhere up
    /// the walk shows the segment; nothing matching hides it; dotfiles
    /// match only dot-spelled patterns (no GLOB_DOTS).
    #[test]
    fn upglob_dir_walk() {
        let td = tempfile::tempdir().expect("tempdir");
        let root = td.path();
        let deep = root.join("a/b/c");
        std::fs::create_dir_all(&deep).expect("mkdirs");
        std::fs::write(root.join("a/Cargo.toml"), "").expect("marker");
        std::fs::write(root.join("a/b/.ruby-version"), "").expect("dotmarker");

        let cwd = deep.to_string_lossy().into_owned();
        let home = root.to_string_lossy().into_owned();

        // marker two levels up (within the home-bounded walk)
        assert!(upglob_from(&cwd, &home, "Cargo.toml"));
        // glob form
        assert!(upglob_from(&cwd, &home, "*.toml"));
        // alternation with a miss + a hit
        assert!(upglob_from(&cwd, &home, "no-such|Cargo.toml"));
        // no match anywhere
        assert!(!upglob_from(&cwd, &home, "no-such-file.xyz"));
        // dotfile: explicit dot pattern matches ...
        assert!(upglob_from(&cwd, &home, ".ruby-version"));
        // ... but `*` never matches a leading dot (GLOB_DOTS unset)
        assert!(!upglob_from(&cwd, &home, "*ruby-version"));
    }
}
