//! Parity failures discovered during a focused `zshrs --zsh` vs real
//! `/opt/homebrew/bin/zsh` probe session.
//!
//! Each test is anchored to a concrete divergence — both the script
//! and the observed outputs are documented in the test body comments.
//! All tests are `#[ignore = "ZSHRS BUG: ..."]` per the established
//! pin-then-fix workflow: the test bodies are correct (call
//! `assert_parity`), they simply fail today because of the bug.
//! Removing the `#[ignore]` should be the proof the bug is closed.

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("target").join("debug").join("zshrs")
}
fn zsh_path() -> &'static str {
    if Path::new("/opt/homebrew/bin/zsh").exists() {
        "/opt/homebrew/bin/zsh"
    } else if Path::new("/usr/local/bin/zsh").exists() {
        "/usr/local/bin/zsh"
    } else {
        "/bin/zsh"
    }
}
fn zsh_available() -> bool {
    Command::new(zsh_path())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct ShellResult {
    stdout: String,
    stderr: String,
    exit: i32,
}

fn run_zsh(s: &str) -> ShellResult {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("zsh");
    ShellResult {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn run_zshrs(s: &str) -> ShellResult {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-c", s])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    ShellResult {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn assert_parity(s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(s);
    let r = run_zshrs(s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n{}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        s, z.stdout, r.stdout,
    );
    assert_eq!(
        z.exit, r.exit,
        "exit divergence on:\n{}\n--- zsh exit={} zshrs exit={}\n--- zsh stderr ---\n{}\n--- zshrs stderr ---\n{}",
        s, z.exit, r.exit, z.stderr, r.stderr,
    );
}

// ─── 1. Closure-of-closure stack overflow ────────────────────────

/// `[[ fofo = (fo#)# ]]` panics with stack overflow in zshrs;
/// zsh treats as no-match silently (exit 1, no output).
#[test]
#[ignore = "ZSHRS BUG: (fo#)# closure-of-closure causes recursion stack overflow panic"]
fn parity_closure_of_closure_overflow() {
    assert_parity("[[ fofo = (fo#)# ]] && echo m");
}

// ─── 2. (q-) minimal-quote with apostrophe ───────────────────────

/// `${(q-)a}` with `a="it's"` produces `it\'s` in zsh (backslash-escape
/// form), but `'it'\''s'` in zshrs (heavy single-quote-wrap form).
#[test]
#[ignore = "ZSHRS BUG: (q-) minimal-quote uses '...'\\\\''...' wrap instead of backslash-escape style"]
fn parity_q_minus_flag_apostrophe_quote_style() {
    assert_parity(r#"a="it's"; print -- ${(q-)a}"#);
}

// ─── 3. (Q) on quoted literal — bad-substitution error ──────────

/// `${(Q)"abc"}` is a syntax error in zsh ("bad substitution") since
/// (Q) requires a variable name, not a literal. zshrs silently returns
/// empty instead of raising the error.
#[test]
#[ignore = "ZSHRS BUG: ${(Q)\"literal\"} should raise 'bad substitution' error but returns empty"]
fn parity_Q_flag_on_literal_should_error() {
    assert_parity(r#"print -- ${(Q)"abc"}"#);
}

// ─── 4. (#b) capture-group nested replacement ────────────────────

/// `${foo//(#b)(*)left/${match//a/X}}` with foo=aleftkept produces
/// `aleftkept` in zsh (nested ${match//...} not processed at this
/// layer) but `kept` in zshrs (matched+stripped without nested subst).
#[test]
#[ignore = "ZSHRS BUG: ${var//(#b)(*)pat/${match//.../...}} nested-match replacement differs from zsh"]
fn parity_pound_b_nested_match_replacement_differs() {
    assert_parity(r#"foo=aleftkept; print -- ${foo//(#b)(*)left/${match//a/X}}"#);
}

// ─── 5. (x)# closure without extendedglob ────────────────────────

/// `[[ x = (x)# ]]` with extendedglob OFF: zsh does NOT match
/// (because `#` is a literal char, not a quantifier outside extglob).
/// zshrs matches anyway.
#[test]
#[ignore = "ZSHRS BUG: (pat)# closure operator active even when extendedglob is OFF"]
fn parity_closure_hash_requires_extendedglob() {
    assert_parity(r#"unsetopt extendedglob; [[ "x" = (x)# ]] && echo yes || echo no"#);
}

// ─── 6. ## one-or-more without extendedglob ──────────────────────

/// `[[ "abc123" = [a-z]##[0-9]## ]]` only matches under extendedglob.
/// Default emulate-zsh state has extendedglob OFF, so zsh outputs no
/// match. zshrs matches anyway.
#[test]
#[ignore = "ZSHRS BUG: ## one-or-more quantifier active even when extendedglob is OFF"]
fn parity_double_hash_requires_extendedglob() {
    assert_parity(r#"[[ "abc123" = [a-z]##[0-9]## ]] && echo m || echo nope"#);
}

// ─── 7. echo "\$var" — backslash-dollar literal ──────────────────

/// `echo "\$var"` produces literal `\$var` in zsh (the `\$` is the
/// dollar-escape inside double quotes, preserved verbatim by echo).
/// zshrs produces empty — treats `\$` as escape then expands the
/// empty-named variable.
#[test]
#[ignore = "ZSHRS BUG: echo \"\\$var\" produces empty instead of literal '\\$var'"]
fn parity_double_quoted_backslash_dollar_var_literal() {
    assert_parity(r#"echo "\$var""#);
}

// ─── 8. Math functions auto-loaded vs zmodload-required ──────────

/// `$((sqrt(4)))` in zsh requires `zmodload zsh/mathfunc` first;
/// without it, zsh errors "unknown function: sqrt". zshrs auto-resolves
/// the math function, returning `2.` directly. (More user-friendly,
/// but a parity failure.)
#[test]
#[ignore = "ZSHRS BUG: math functions sqrt/sin/floor auto-loaded vs zsh requires zmodload zsh/mathfunc"]
fn parity_mathfunc_requires_zmodload() {
    assert_parity("echo $((sqrt(4)))");
}

#[test]
#[ignore = "ZSHRS BUG: math functions sqrt/sin/floor auto-loaded vs zsh requires zmodload zsh/mathfunc"]
fn parity_mathfunc_sin_requires_zmodload() {
    assert_parity("echo $((sin(0)))");
}

#[test]
#[ignore = "ZSHRS BUG: math functions sqrt/sin/floor auto-loaded vs zsh requires zmodload zsh/mathfunc"]
fn parity_mathfunc_floor_requires_zmodload() {
    assert_parity("echo $((floor(3.7)))");
}

// ─── 9. $=a — word-split parameter expansion flag ────────────────

/// `for w in $=a; do print [$w]; done` in zsh splits the contents of
/// `$a` on IFS. zshrs treats `$=a` as a literal token — doesn't even
/// parse the `$=` expansion flag.
#[test]
#[ignore = "ZSHRS BUG: $=var word-split flag not parsed — left as literal token"]
fn parity_dollar_equals_word_split_flag() {
    assert_parity(r#"a="x  y  z"; for w in $=a; do print -- "[$w]"; done"#);
}

// ─── 10. $~b — glob-eval parameter expansion flag ────────────────

/// `$~var` in zsh enables glob/pattern interpretation on the result
/// of expansion. Critically, it does NOT recursively expand `$`
/// references inside the value. With `b="\$a"`, zsh outputs `\$a`
/// (the literal backslash-dollar-a). zshrs outputs `foo` — wrongly
/// treats `$~` as recursive variable expansion.
#[test]
#[ignore = "ZSHRS BUG: $~var triggers recursive var expansion instead of glob-evaluation flag"]
fn parity_dollar_tilde_glob_eval_flag() {
    assert_parity(r#"a=foo; b='\$a'; print -- $~b"#);
}

// ─── 11. read -A doesn't split into array ────────────────────────

/// `IFS=: read -A arr <<< "a:b:c"` should split into 3 elements:
/// arr=(a b c). zshrs assigns the entire string to arr[1], giving
/// ${#arr}=1 instead of 3.
#[test]
#[ignore = "ZSHRS BUG: read -A with custom IFS does not split input into array elements"]
fn parity_read_minus_A_array_split() {
    assert_parity(r#"IFS=: read -A arr <<< "a:b:c"; echo "${#arr}=${arr[2]}""#);
}

// ─── 12. Modifier on array subscript applies to whole array ──────

/// `${a[1]:t}` should apply `:t` (tail) only to the indexed element.
/// zsh on `a=(/p/a /q/b)` gives `a` (tail of `/p/a`). zshrs gives
/// `a b` — applies `:t` to every element, then joins.
#[test]
#[ignore = "ZSHRS BUG: ${arr[N]:modifier} applies modifier to whole array instead of single element"]
fn parity_modifier_on_array_subscript() {
    assert_parity(r#"a=("/p/a" "/q/b"); echo ${a[1]:t}"#);
}

// ─── 13. NOMATCH error on unmatched glob ─────────────────────────

/// In zsh (with NOMATCH set, default in emulate zsh), a glob that
/// matches no files errors with "no matches found". zshrs returns
/// the literal pattern unchanged (bash-style behavior).
#[test]
#[ignore = "ZSHRS BUG: NOMATCH option not enforced on unmatched glob — pattern left literal"]
fn parity_nomatch_error_on_unmatched_glob() {
    assert_parity("echo /not_a_real_path_zshrs_xyz_unique_abc/*");
}

// ─── 14. Process substitution path scheme ────────────────────────

/// `echo <(true)` in zsh prints `/dev/fd/N` (uses anonymous pipe via
/// procfs). zshrs uses `/tmp/zshrs_psub_PID_N` (named temp file
/// fallback). Behavior-equivalent for reads but path differs — any
/// script that inspects the path will break.
#[test]
#[ignore = "ZSHRS BUG: process substitution uses /tmp/zshrs_psub_* instead of /dev/fd/N"]
fn parity_process_subst_path_scheme() {
    assert_parity("echo <(true)");
}

// ─── 15. Brace range with zero step ──────────────────────────────

/// `echo {1..5..0}` zero-step is invalid in zsh — outputs literal
/// `1..5..0`. zshrs ignores the zero step and produces `1 2 3 4 5`.
#[test]
#[ignore = "ZSHRS BUG: brace {a..b..0} zero-step expands to range instead of leaving literal"]
fn parity_brace_zero_step_invalid_literal() {
    assert_parity("echo {1..5..0}");
}

// ─── 16. select prompt format ────────────────────────────────────

/// `select` loop in zsh uses ANSI-styled `?#` prompt, zshrs uses
/// plain `?# `. (Actually the diff shows zsh emits an ANSI escape
/// sequence `\E[1;34m-->>>> \E[0m`; this is a recent zsh feature.)
/// At minimum the trailing-prompt bytes differ.
#[test]
#[ignore = "ZSHRS BUG: select-loop prompt missing ANSI styling per zsh 5.9"]
fn parity_select_prompt_format() {
    assert_parity("select x in a b c; do break; done <<< 1");
}

// ─── 17. print -P %F wraps in \001..\002 markers ─────────────────

/// `print -P "%F{red}red%f"` in zsh emits raw SGR (`\E[31mred\E[39m`).
/// zshrs wraps each escape in `\001...\002` zero-width markers
/// (designed for prompt width-counting but they leak into print -P
/// output, which is wrong — that wrapping is only for prompts).
#[test]
#[ignore = "ZSHRS BUG: print -P emits \\001..\\002 width-marker wrap around SGR escapes"]
fn parity_print_P_color_marker_wrapping() {
    assert_parity(r#"print -P "%F{red}red%f""#);
}

#[test]
#[ignore = "ZSHRS BUG: print -P emits \\001..\\002 width-marker wrap around SGR escapes (bold case)"]
fn parity_print_P_bold_marker_wrapping() {
    assert_parity(r#"print -P "%B%bnoop""#);
}

// ─── 18. (#s) start-anchor without extendedglob ──────────────────

/// `[[ "abc" = (#s)a* ]]` requires extendedglob in zsh. With it off,
/// no match (zsh treats `(#s)` literally). zshrs matches anyway.
#[test]
#[ignore = "ZSHRS BUG: (#s) start-anchor active even when extendedglob is OFF"]
fn parity_pound_s_anchor_requires_extendedglob() {
    assert_parity(r#"[[ "abc" = (#s)a* ]] && echo m || echo no"#);
}

// ─── 19. (#e) end-anchor without extendedglob ────────────────────

/// `[[ "abc" = *c(#e) ]]` — same as (#s), requires extglob.
#[test]
#[ignore = "ZSHRS BUG: (#e) end-anchor active even when extendedglob is OFF"]
fn parity_pound_e_anchor_requires_extendedglob() {
    assert_parity(r#"[[ "abc" = *c(#e) ]] && echo m || echo no"#);
}

// ─── 20. Multibyte string slice ${var:0:1} ────────────────────────

/// `a=日本; echo "${a:0:1}"` should output "日" (one codepoint).
/// zshrs outputs "å" — extracts 1 BYTE from UTF-8 encoding, mangling.
#[test]
#[ignore = "ZSHRS BUG: ${var:N:M} slice operates on bytes, not codepoints — mangles multibyte"]
fn parity_multibyte_substring_slice() {
    assert_parity("a=日本; echo \"${a:0:1}\"");
}

// ─── 21. Multibyte array-style subscript ${var[1]} ───────────────

/// `a=日本; echo "${a[1]}"` should output "日". zshrs returns garbled
/// byte due to byte-not-codepoint subscripting.
#[test]
#[ignore = "ZSHRS BUG: ${var[N]} scalar subscript on multibyte mangles codepoints"]
fn parity_multibyte_scalar_subscript() {
    assert_parity("a=日本; echo \"${a[1]}\"");
}

// ─── 22. declare -p PATH output format ──────────────────────────

/// `declare -p PATH` differs:
///   zsh:   `export -T PATH path=( ... )`
///   zshrs: `typeset -T PATH=''`
/// Two mismatches: (a) `export -T` vs `typeset -T`, (b) zshrs doesn't
/// inherit the parent shell's PATH so value is empty.
#[test]
#[ignore = "ZSHRS BUG: declare -p PATH uses 'typeset -T' instead of 'export -T' and shows empty value"]
fn parity_declare_p_path_format_and_inheritance() {
    assert_parity("declare -p PATH");
}

// ─── 23. ${a:^b} array-zip operator with missing b ──────────────

/// `a=(1 2 3); echo ${a:^b}` — when b is unset, zsh treats `:^` as
/// a no-op and outputs `a` content. zshrs outputs empty.
#[test]
#[ignore = "ZSHRS BUG: ${a:^b} with unset b returns empty instead of a's content"]
fn parity_array_zip_with_missing_right_operand() {
    assert_parity("a=(1 2 3); echo ${a:^b}");
}

// ─── 24. ${a:^b} quoted-zip with IFS joining differs ────────────

/// `a=(1 2 3); b=(x y z); echo "${a:^b}"` — quoted zip:
///   zsh:   `1 2 3 x`  (joins differently — appears to be zsh quirk)
///   zshrs: `1 x 2 y 3 z`  (proper interleave)
/// Either interpretation is defensible but they DO differ.
#[test]
#[ignore = "ZSHRS BUG: ${a:^b} quoted-zip output joining semantics differ from zsh"]
fn parity_array_zip_quoted_joining_differs() {
    assert_parity(r#"a=(1 2 3); b=(x y z); echo "${a:^b}""#);
}

// ─── 25. [[ ... -a ... ]] in [[ ]] is a parse error in zsh ───────

/// `[[ "" -a "x" ]]` — ksh-style `-a` boolean AND inside `[[ ]]` is
/// a zsh PARSE ERROR ("condition expected"), zshrs silently accepts
/// it and returns empty (no match path triggers).
#[test]
#[ignore = "ZSHRS BUG: [[ ... -a ... ]] should raise parse error per zsh but silently fails"]
fn parity_double_bracket_minus_a_should_error() {
    assert_parity(r#"[[ "" -a "x" ]] && echo m || echo n"#);
}
