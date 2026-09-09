//! Port of `_correct_filename` from
//! `Completion/Base/Widget/_correct_filename`.
//!
//! Full upstream body (72 lines, abridged):
//! ```text
//! sh: 1  #compdef -k complete-word \C-xC
//! sh:17  setopt extendedglob
//! sh:19  local file="$PREFIX$SUFFIX" trylist tilde etilde testcmd
//! sh:20  integer approx max_approx=6
//! sh:20  if [[ -z $WIDGET ]]; then file=$1; local IPREFIX
//! sh:23  else (( ${NUMERIC:-1} > 1 )) && max_approx=$NUMERIC
//! sh:25  if [[ $file = \~*/* ]]; then tilde-expand
//! sh:31  if [[ $CURRENT -eq 1 && $file != /* ]]; then testcmd=1
//! sh:33  elif [[ $file = \=* ]]; then …testcmd=1
//! sh:40  if -e file (or whence file) → emit + return
//! sh:50  for approx 1..max_approx do
//! sh:57    trylist via `(#a$approx)` glob (or whence -wm)
//! sh:64  done
//! sh:72  return 1
//! ```
//!
//! ## The `(#aN)` error model
//!
//! `(#aN)` is zsh's APPROXIMATE-MATCH glob flag: match within at most `N`
//! errors. It is **not** plain Levenshtein distance. `Src/pattern.c`'s
//! approximate arm of `case P_EXACTLY:` (c:2737-2779, ported as
//! [`crate::ported::pattern::approx_match_exactly`], pattern.rs:5399) walks the
//! pattern and the input with two INDEPENDENT cursors and charges one error for
//! each of substitute / insert / delete — which means a TRANSPOSITION of two
//! adjacent characters costs ONE error (delete + insert land on the same pair),
//! where a three-way-min Levenshtein charges two. The budget itself lives in the
//! low byte of the compiled glob flags (pattern.rs:1771, c:1054-1066).
//!
//! This port used to mirror the flag with `shared::edit_distance`, a plain
//! three-way-min Levenshtein with no transposition term, and the module doc
//! called that "zsh's Levenshtein-≤N glob qualifier". Both claims were false.
//! Because sh:63 BREAKS at the first `approx` level that matches anything, the
//! wrong error model changes WHICH LEVEL FIRES and therefore the entire result
//! set — not just its ordering. With `foo fou fon fooo fo oof ffoo bar fboo
//! foob` in the directory:
//!
//! ```text
//! % zsh -f -c '…; _correct_filename ofo'   ->  fo foo oof
//! % zshrs (before)                          ->  fo
//! ```
//!
//! `ofo` -> `foo` is one error to zsh (transpose `of`) and two to Levenshtein,
//! so the mirror never fired at `approx=1` for `foo`/`oof` and reported only the
//! one candidate Levenshtein could reach.
//!
//! Both branches now run the engine's own matcher rather than a second
//! hand-rolled approximation: sh:58's file branch goes through
//! [`crate::ported::glob::zglob`] (see [`glob_approx`]) and sh:60's command
//! branch through the real [`crate::ported::builtin::bin_whence`] (see
//! [`whence_wm_approx`]).

use crate::ported::builtin::bin_whence;
use crate::ported::glob::{tokenize, zglob};
use crate::ported::params::{getiparam, getsparam, setsparam};
use crate::ported::utils::{gettempfile, quotestring};
use crate::ported::zle::compcore::set_compstate_str;
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS, QT_BACKSLASH_PATTERN};
use std::path::Path;

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `_correct_filename` — try to correct the misspelled filename
/// under the cursor (or print correction to stdout when called as
/// a non-widget).
pub fn _correct_filename(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_correct_filename");

    // sh:17  `setopt extendedglob`.
    //
    // Load-bearing, not cosmetic: with EXTENDED_GLOB off, `patcompcharsset`
    // masks Hash out of the active specials (pattern.rs:492, c:480-483), so the
    // `(#a$approx)` of sh:58 and sh:60 is not a glob flag at all — it compiles
    // as the six literal characters `(#a1)` and matches nothing. Verified: at
    // top level `whence -wm "(#a1)lls"` prints nothing in BOTH shells; under
    // `setopt extendedglob` both print `gls`/`ls`/`rls`.
    //
    // sh:16's `emulate -LR zsh` remains unported and this is deliberately
    // narrower than the `localoptions` it implies: there is no scoped-option
    // substrate here, so this is the same targeted save/restore
    // `_expand_with` uses for its one option (_expand.rs:152-177). A port that
    // needs a second option must not assume this covers it.
    struct ExtendedGlobGuard(Option<bool>);
    impl Drop for ExtendedGlobGuard {
        fn drop(&mut self) {
            if let Some(prev) = self.0 {
                crate::ported::options::opt_state_set("extendedglob", prev);
            }
        }
    }
    let _extendedglob = ExtendedGlobGuard(crate::ported::options::opt_state_get("extendedglob"));
    crate::ported::options::opt_state_set("extendedglob", true);

    let widget = getsparam("WIDGET").unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();

    // sh:19/sh:20
    let (mut file, in_widget): (String, bool) = if widget.is_empty() {
        (args.first().cloned().unwrap_or_default(), false)
    } else {
        // sh:26's `max_approx=$NUMERIC` is applied where the loop reads it,
        // below — computing it twice left the copy here dead.
        (format!("{}{}", prefix, suffix), true)
    };

    // sh:25-29  tilde expand (~/path)
    if file.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            file = format!("{}{}", home, &file[1..]);
        }
    }

    // sh:31-42  testcmd detection
    let current = getiparam("CURRENT");
    let mut testcmd = false;
    // sh:24  `local IPREFIX` — in the NON-widget call the binding is fresh and
    // empty for the whole call, and sh:39's append never reaches the caller's
    // global. Reading the global unconditionally (and writing it back at sh:39)
    // both mis-seeded sh:71's prefix and leaked `=` into the param table.
    let mut iprefix = if in_widget {
        getsparam("IPREFIX").unwrap_or_default()
    } else {
        String::new()
    };
    if current == 1 && !file.starts_with('/') {
        testcmd = true; // sh:36
    } else if file.starts_with('=') {
        // sh:38  [[ -n $WIDGET ]] && PREFIX="$PREFIX[2,-1]"
        if in_widget {
            // `$PREFIX[2,-1]` yields the empty string on a 0- or 1-character
            // PREFIX and is character- not byte-indexed; the byte slice
            // `prefix[1..]` panicked on an empty PREFIX (reachable whenever the
            // `=` sits in SUFFIX) and split a multibyte leading character.
            let mut rest = prefix.chars();
            rest.next();
            let _ = setsparam("PREFIX", rest.as_str());
        }
        // sh:39  IPREFIX="${IPREFIX}="
        iprefix.push('=');
        if in_widget {
            let _ = setsparam("IPREFIX", &iprefix);
        }
        // sh:40  file="$file[2,-1]"
        file = file.chars().skip(1).collect();
        testcmd = true; // sh:41
    }

    // sh:40-49  exact match short-circuit
    let exists = if testcmd {
        which(&file).is_some()
    } else {
        Path::new(&file).exists()
    };
    if exists {
        if in_widget {
            let argv: Vec<String> = vec![
                "-QUf".to_string(),
                "-i".to_string(),
                iprefix.clone(),
                "-I".to_string(),
                getsparam("ISUFFIX").unwrap_or_default(),
                file.clone(),
            ];
            let _ = bin_compadd("compadd", &argv, &make_ops(), 0);
            let cur_insert =
                crate::ported::zle::compcore::get_compstate_str("insert").unwrap_or_default();
            if !cur_insert.is_empty() {
                set_compstate_str("insert", "menu");
            }
        } else {
            println!("{}", file);
        }
        return 0;
    }

    // sh:56-64  approximate-match loop. `(#a$approx)` is the engine's
    // approximate-match glob flag — a TRANSPOSITION costs one error, not two;
    // see the module doc for why that distinction decides the whole result set.
    let max_approx: usize = if in_widget {
        let numeric = getiparam("NUMERIC");
        if numeric > 1 {
            numeric as usize
        } else {
            6
        }
    } else {
        6
    };
    // The loop runs `approx` upwards and BREAKS at the first level that
    // matches anything (sh:63), and `(#a$approx)` matches within AT MOST
    // `$approx` errors — so `trylist` ends up holding EVERY candidate at the
    // smallest error count, not the single closest one. Keeping only the
    // first best made `_correct_filename dir/fon` print `dir/foo` where zsh,
    // with `dir/foo` and `dir/fou` both one error away, prints both.
    let mut trylist: Vec<String> = Vec::new();
    // sh:56  for (( approx = 1; approx <= max_approx; approx++ )); do
    for approx in 1..=max_approx {
        trylist = if testcmd {
            whence_wm_approx(approx, &file) // sh:60-61
        } else {
            glob_approx(approx, &file) // sh:58
        };
        // sh:63  (( $#trylist )) && break
        if !trylist.is_empty() {
            break;
        }
    }
    // sh:65  (( $#trylist )) || return 1
    if trylist.is_empty() {
        return 1;
    }
    // sh:58 already yields the whole path (`(#a1)dir/fon` globs to `dir/foo`)
    // and sh:61 has already reduced the command branch to bare names, so
    // sh:68/sh:71 consume `trylist` as-is. The previous revision re-prefixed
    // the directory itself and pushed every command name back through `whence`
    // to a FULL PATH, which sh:61 explicitly strips: zsh answers
    // `_correct_filename '=lls'` with `=gls =ls =rls`, not `/bin/ls`.
    let full: Vec<String> = trylist;
    if in_widget {
        // sh:68  compadd -QUf -i "$IPREFIX" -I "$ISUFFIX" "${trylist[@]…}"
        let mut argv: Vec<String> = vec![
            "-QUf".to_string(),
            "-i".to_string(),
            iprefix.clone(),
            "-I".to_string(),
            getsparam("ISUFFIX").unwrap_or_default(),
        ];
        argv.extend(full);
        let _ = bin_compadd("compadd", &argv, &make_ops(), 0);
        let cur_insert =
            crate::ported::zle::compcore::get_compstate_str("insert").unwrap_or_default();
        if !cur_insert.is_empty() {
            set_compstate_str("insert", "menu");
        }
    } else {
        // sh:71  print "$IPREFIX${^trylist[@]}" — `${^…}` is RC_EXPAND_PARAM
        // distribution, so the prefix goes on EVERY element, and the whole
        // array prints as one space-separated line. Dropping the prefix made
        // `_correct_filename '=lls'` answer `gls ls rls` for zsh's
        // `=gls =ls =rls`.
        let line: Vec<String> = full.iter().map(|t| format!("{}{}", iprefix, t)).collect();
        println!("{}", line.join(" "));
    }
    0
}

/// sh:58 — `trylist=( (#a$approx)"$file"(N) )`.
///
/// The real globber, not a distance mirror: [`zglob`] compiles `(#a$approx)`
/// through `patcompile`, so the error model is the engine's (transposition = 1
/// error) and every other glob semantic — path-component traversal, `CASEGLOB`,
/// multibyte stepping — comes along instead of being re-approximated here.
fn glob_approx(approx: usize, file: &str) -> Vec<String> {
    // `"$file"` is DOUBLE QUOTED at sh:58, so a glob metacharacter in the name
    // is a LITERAL, not a wildcard. `QT_BACKSLASH_PATTERN` (c:Src/utils.c:
    // 6242-6248) backslash-escapes exactly the pattern metacharacters, and
    // `tokenize()` then rewrites each `\X` to `Bnull X` (c:Src/glob.c:3591),
    // which the pattern compiler reads as a literal (glob.rs:3987). Plain
    // `QT_BACKSLASH` would leave `*` / `?` / `[` live and a file named `f*o`
    // would glob its neighbours — the same trap `_parameters` documents at
    // _parameters.rs:190-193.
    let mut pat = format!(
        "(#a{}){}(N)",
        approx,
        quotestring(file, QT_BACKSLASH_PATTERN)
    );
    tokenize(&mut pat);
    let mut list = vec![pat];
    // `(N)` is the NULL_GLOB qualifier: no match leaves the list EMPTY with no
    // `no matches found` diagnostic, which is exactly what sh:63's
    // `(( $#trylist ))` tests and why sh:58 spells it that way.
    zglob(&mut list, 0, 0);
    list
}

/// sh:60-61 — `trylist=( "${(@)${(@f)$(whence -wm "(#a$approx)$file" 2>/dev/null)}%:*}" )`
/// then `[[ $file = */* ]] || trylist=(${trylist##*/})`.
fn whence_wm_approx(approx: usize, file: &str) -> Vec<String> {
    // NOTE the asymmetry with sh:58, and keep it: here `$file` is interpolated
    // INSIDE the double-quoted pattern rather than being a quoted operand, so
    // `whence -m` compiles its metacharacters as pattern. Escaping it here
    // would diverge from zsh in the opposite direction.
    let text = whence_wm_capture(&format!("(#a{}){}", approx, file));
    // `${(@f)$( … )}` — split on newline; `$( … )` has already dropped the
    // trailing newlines, and empty fields do not survive an array assignment.
    let mut list: Vec<String> = text
        .lines()
        .filter(|l| !l.is_empty())
        // `${…%:*}` — strip the SHORTEST `:*` suffix, i.e. everything from the
        // LAST colon on. `whence -w` prints `name: type`.
        .map(|l| match l.rfind(':') {
            Some(i) => l[..i].to_string(),
            None => l.to_string(),
        })
        .collect();
    // sh:61  [[ $file = */* ]] || trylist=(${trylist##*/})
    // `##*/` strips the LONGEST `*/` prefix — everything through the last `/`.
    if !file.contains('/') {
        list = list
            .into_iter()
            .map(|e| match e.rfind('/') {
                Some(i) => e[i + 1..].to_string(),
                None => e,
            })
            .collect();
    }
    list
}

/// sh:60's `$( whence -wm "…" 2>/dev/null )`.
///
/// Runs the REAL [`bin_whence`] with fd 1 pointed at a [`gettempfile`] buffer
/// and fd 2 at `/dev/null` — the idiom `_parameters`' `typeset_m_capture`
/// establishes (_parameters.rs:186-212) — rather than
/// `exec::run_command_substitution`, which deep-clones the whole shell state for
/// every `$( … )`. Going through the builtin keeps the scanned set (aliases,
/// reserved words, shell functions, builtins, then `cmdnamtab`; c:4030-4073)
/// in ONE place instead of re-enumerating `$PATH` here with a second, narrower
/// idea of what a command is.
///
/// A temp FILE, not a pipe: `whence -m` over a large `$PATH` can outgrow a 64K
/// pipe buffer with nobody draining the read end.
///
/// Returns the captured text, or the empty string if the buffer cannot be
/// created or fd 1 cannot be redirected — which leaves `trylist` empty and
/// sends sh:56's loop to the next `approx`, adding nothing wrong.
fn whence_wm_capture(pattern: &str) -> String {
    let (fd, path) = match gettempfile(None) {
        Some(t) => t,
        None => return String::new(),
    };
    // Flush FIRST: `println!` writes through a `LineWriter` over fd 1, and
    // anything still buffered from before the redirect would land in the
    // capture instead of on the terminal.
    let _ = std::io::Write::flush(&mut std::io::stdout());
    let saved_out = unsafe { libc::dup(1) };
    let saved_err = unsafe { libc::dup(2) };
    let devnull = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY) };
    let redirected = saved_out >= 0 && unsafe { libc::dup2(fd, 1) } >= 0;
    if redirected {
        // `2>/dev/null` — sh:60 discards whatever the builtin warns about
        // (a pattern that fails to compile, most of all).
        if devnull >= 0 && saved_err >= 0 {
            unsafe {
                libc::dup2(devnull, 2);
            }
        }
        // `whence -w -m PATTERN`: both bits are `OPT_MINUS`, i.e. `ind[c] & 1`
        // (c:Src/zsh.h:1402). `whence`'s builtin-table funcid is 0
        // (builtin.rs:17615, c:Src/builtin.c:132).
        let mut ops = make_ops();
        ops.ind[b'w' as usize] = 1;
        ops.ind[b'm' as usize] = 1;
        let _ = bin_whence("whence", &[pattern.to_string()], &ops, 0);
        let _ = std::io::Write::flush(&mut std::io::stdout());
        unsafe {
            libc::dup2(saved_out, 1);
        }
        if devnull >= 0 && saved_err >= 0 {
            unsafe {
                libc::dup2(saved_err, 2);
            }
        }
    }
    for f in [saved_out, saved_err, devnull, fd] {
        if f >= 0 {
            unsafe {
                libc::close(f);
            }
        }
    }
    let text = if redirected {
        std::fs::read_to_string(&path).unwrap_or_default()
    } else {
        String::new()
    };
    let _ = std::fs::remove_file(&path);
    text
}

/// `whence` substitute — search $PATH for `file`.
fn which(file: &str) -> Option<String> {
    if file.contains('/') {
        if Path::new(file).is_file() {
            return Some(file.to_string());
        }
        return None;
    }
    let path = std::env::var("PATH").ok()?;
    for dir in path.split(':') {
        if dir.is_empty() {
            continue;
        }
        let candidate = format!("{}/{}", dir, file);
        if Path::new(&candidate).is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RUST-ONLY test scaffold: lay `names` down in a fresh directory and hand
    /// back its absolute path. Absolute so the glob never depends on the
    /// process-wide cwd, which other tests in this binary move.
    fn fixture(tag: &str, names: &[&str]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("zshrs_corrfn_{}_{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        for n in names {
            let _ = std::fs::write(dir.join(n), b"");
        }
        dir
    }

    /// RUST-ONLY: `(#aN)` needs EXTENDED_GLOB (pattern.rs:492); a bare
    /// unit-test process starts from the default option set, which has it off.
    fn with_extendedglob<T>(f: impl FnOnce() -> T) -> T {
        let had = crate::ported::options::opt_state_get("extendedglob");
        crate::ported::options::opt_state_set("extendedglob", true);
        let out = f();
        if let Some(prev) = had {
            crate::ported::options::opt_state_set("extendedglob", prev);
        }
        out
    }

    /// sh:58's error model, on the one input the old `edit_distance` mirror
    /// could not represent.
    ///
    /// `ofo` -> `foo` is a TRANSPOSITION: one error to `(#a1)`
    /// (pattern.rs::approx_match_exactly walks input and pattern with separate
    /// cursors, so the swapped pair costs delete+insert on the SAME pair), two
    /// to a three-way-min Levenshtein. `oof` is likewise one error to `(#a1)`.
    ///
    /// Because sh:63 breaks at the first level that matches anything, the
    /// mirror settled at `approx=1` with only `fo` and never saw the other two.
    /// Measured against zsh 5.9 in the same directory:
    ///
    /// ```text
    /// % zsh -f -c 'setopt extendedglob; print -r -- (#a1)ofo'
    /// fo foo oof
    /// ```
    #[test]
    fn transposition_costs_one_error_not_two() {
        let _g = crate::test_util::global_state_lock();
        let dir = fixture(
            "transpose",
            &[
                "foo", "fou", "fon", "fooo", "fo", "oof", "ffoo", "bar", "fboo", "foob",
            ],
        );
        let mut got = with_extendedglob(|| glob_approx(1, &format!("{}/ofo", dir.display())));
        got.sort();
        let _ = std::fs::remove_dir_all(&dir);
        let want: Vec<String> = ["fo", "foo", "oof"]
            .iter()
            .map(|n| format!("{}/{}", dir.display(), n))
            .collect();
        assert_eq!(got, want);
    }

    /// sh:56-64's break-at-first-level, which is what makes the error model
    /// decide the RESULT SET and not merely its ordering: at `approx=1` only
    /// `fon`'s two one-error neighbours exist, so `fooo` (two errors) must not
    /// appear even though the loop would have reached `approx=2` had level 1
    /// been empty. Pins the tie behaviour d1d28848a1 established — EVERY
    /// candidate at the minimum level, not the closest one.
    #[test]
    fn every_candidate_at_the_first_matching_level() {
        let _g = crate::test_util::global_state_lock();
        let dir = fixture("tie", &["foo", "fou", "fooo", "bar"]);
        let mut got = with_extendedglob(|| glob_approx(1, &format!("{}/fon", dir.display())));
        got.sort();
        let _ = std::fs::remove_dir_all(&dir);
        let want: Vec<String> = ["foo", "fou"]
            .iter()
            .map(|n| format!("{}/{}", dir.display(), n))
            .collect();
        assert_eq!(got, want);
    }

    /// sh:58 quotes the operand (`(#a$approx)"$file"(N)`), so a glob
    /// metacharacter in the name is a literal.
    ///
    /// `fabcdefgo` is six errors from the literal `f*o` and unreachable at
    /// `approx=1`, but a LIVE `*` matches it with zero — so the assertion fails
    /// loudly if the target is ever quoted with plain `QT_BACKSLASH` (which
    /// leaves `*` / `?` / `[` active) or not quoted at all.
    #[test]
    fn glob_metacharacter_in_the_name_stays_literal() {
        let _g = crate::test_util::global_state_lock();
        let dir = fixture("meta", &["f*o", "fabcdefgo"]);
        let got = with_extendedglob(|| glob_approx(1, &format!("{}/f*o", dir.display())));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(got, vec![format!("{}/f*o", dir.display())]);
    }

    /// sh:58's `(N)` — NULL_GLOB. No match must leave the list EMPTY (and emit
    /// no `no matches found`), because that empty list is precisely what
    /// sh:63's `(( $#trylist ))` tests to decide whether to raise `approx`.
    #[test]
    fn no_match_yields_an_empty_list_not_the_pattern() {
        let _g = crate::test_util::global_state_lock();
        let dir = fixture("nullglob", &["bar"]);
        let got = with_extendedglob(|| glob_approx(1, &format!("{}/xyzzy", dir.display())));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(got.is_empty(), "expected no matches, got {:?}", got);
    }

    #[test]
    fn non_existing_path_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("WIDGET", "");
        let r = _correct_filename(&["/definitely/not/here/xyz".to_string()]);
        assert_eq!(r, 1);
    }

    #[test]
    fn existing_path_prints_and_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("WIDGET", "");
        let r = _correct_filename(&["/".to_string()]);
        assert_eq!(r, 0);
    }
}
