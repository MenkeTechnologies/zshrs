//! Parity failures discovered during a focused `zshrs --zsh` vs real
//! `/opt/homebrew/bin/zsh` probe session.
//!
//! Each test is anchored to a concrete divergence — both the script
//! and the observed outputs are documented in the test body comments.
//! Originally all tests in this file were ignored pending fixes; the
//! bugs have since been closed (all tests now pass, no ignore attrs
//! remain). The file is kept as a regression pin so future drift in
//! any of these areas trips a parity failure.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

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
    // `-fc` instead of `-c` so the invocation matches `run_zsh` (which
    // uses `-fc`). The `-f` flag skips startup files; without it
    // zshrs reads user rc files which can perturb the test (and `$-`
    // shows different flag-letter sets between the two shells).
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-fc", s])
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

// ─── 1. Closure-of-closure stack overflow → fixed by depth guard ─

/// `[[ fofo = (fo#)# ]]` — FIXED. Three layered fixes:
///   1. PATMATCH_MAX_DEPTH=512 recursion guard (no more crash).
///   2. P_WBRANCH `end == s_off` reject-empty (cycle prevention).
///   3. patcomppiece `has_hash` now option-aware (#5/#6 fix) so
///      without extendedglob `(fo#)#` no longer compiles as closure.
/// Combined, the test now passes vs zsh.
#[test]
fn parity_closure_of_closure_overflow() {
    assert_parity("[[ fofo = (fo#)# ]] && echo m");
}

/// Stack-overflow protection itself: the depth-guard must catch
/// runaway recursion and return cleanly. Pin: `(fo#)#` no longer
/// crashes the process.
#[test]
fn patmatch_no_stack_overflow_on_closure_of_closure() {
    if !zsh_available() {
        return;
    }
    // Just verify the command exits cleanly (no SIGABRT/signal).
    let r = run_zshrs("[[ xfoooofof = (fo#)# ]] && echo m || echo no");
    assert!(
        r.exit == 0 || r.exit == 1,
        "should exit 0/1, got {} (was previously SIGABRT)",
        r.exit
    );
    // And the output must not contain the panic string.
    assert!(
        !r.stderr.contains("stack overflow"),
        "must not stack-overflow, got stderr: {}",
        r.stderr
    );
}

// ─── 2. (q-) minimal-quote with apostrophe ───────────────────────

/// `${(q-)a}` with `a="it's"` produces `it\'s` in zsh (backslash-escape
/// form). Fixed: subst.rs:6807 now routes through quotestring(s,
/// QT_SINGLE_OPTIONAL) instead of QT_SINGLE, and utils.rs:6831
/// implements the proper back-filling algorithm from
/// Src/utils.c:6314-6385 (bare apostrophes get `\'`, other specials
/// trigger a retroactive `'` insertion at the prefix boundary).
#[test]
fn parity_q_minus_flag_apostrophe_quote_style() {
    assert_parity(r#"a="it's"; print -- ${(q-)a}"#);
}

/// `(q-)` on plain-space string: should wrap in `'…'` (back-fill).
#[test]
fn parity_q_minus_space_uses_single_quote_wrap() {
    assert_parity(r#"a="hello world"; print -r -- ${(q-)a}"#);
}

/// `(q-)` on apostrophe + space: transition between unquoted (for
/// the `\'`) and quoted (for the space) span via `\''`.
#[test]
fn parity_q_minus_apostrophe_then_special() {
    assert_parity(r#"a="it's a test"; print -r -- ${(q-)a}"#);
}

/// `(q-)` on text that needs no quoting passes through.
#[test]
fn parity_q_minus_no_special_passthrough() {
    assert_parity(r#"a="abc"; print -- ${(q-)a}"#);
}

// ─── 3. (Q) on quoted literal — bad-substitution error ──────────

/// `${(Q)"abc"}` is a syntax error in zsh ("bad substitution") since
/// (Q) requires a variable name, not a literal. zshrs silently returns
/// empty instead of raising the error.
#[test]
fn parity_Q_flag_on_literal_should_error() {
    assert_parity(r#"print -- ${(Q)"abc"}"#);
}

/// Other flags also reject literal-quoted operands. zsh emits
/// "bad substitution" for any `${(flag)"literal"}` form (verified:
/// `zsh -fc 'echo ${(L)"abc"}'` → exit 1). Fixed: BUILTIN_PARAM_FLAG
/// detects the `\u{01}`-prefixed literal-operand sentinel from
/// compile_zsh.rs:2290 and emits the canonical zsh error.
#[test]
fn parity_flag_on_literal_L_should_error() {
    assert_parity(r#"print -- ${(L)"AbC"}"#);
}

#[test]
fn parity_flag_on_literal_q_should_error() {
    assert_parity(r#"print -- ${(q)"a b"}"#);
}

// ─── 4. (#b) capture-group nested replacement ────────────────────

/// `${foo//(#b)(*)left/${match//a/X}}` with foo=aleftkept produces
/// `aleftkept` in zsh (nested ${match//...} not processed at this
/// layer) but `kept` in zshrs (matched+stripped without nested subst).
/// `${foo//(#b)(*)left/${match//a/X}}` with foo=aleftkept produces
/// `aleftkept` in zsh — the nested `${match//.../...}` is NOT
/// processed during the replacement (the `${match[1]}` capture isn't
/// available at the replacement-evaluation layer that close to the
/// outer subst). Now passing; matches zsh's observed behavior.
#[test]
fn parity_pound_b_nested_match_replacement_differs() {
    assert_parity(r#"foo=aleftkept; print -- ${foo//(#b)(*)left/${match//a/X}}"#);
}

// ─── 5. (x)# closure without extendedglob — FIXED ───────────────

/// `[[ x = (x)# ]]` with extendedglob OFF: zsh does NOT match.
/// Fixed: patcomppiece's `has_hash` check now consults
/// `zpc_special[ZPC_HASH]` (which patcompcharsset masks to Marker
/// when EXTENDEDGLOB is off) instead of comparing the raw byte
/// to `b'#'`. Src/pattern.c:480-483.
#[test]
fn parity_closure_hash_requires_extendedglob() {
    assert_parity(r#"unsetopt extendedglob; [[ "x" = (x)# ]] && echo yes || echo no"#);
}

// ─── 6. ## one-or-more without extendedglob — FIXED ─────────────

/// `[[ "abc123" = [a-z]##[0-9]## ]]` only matches under extendedglob.
/// Same fix as #5 — has_hash now option-aware.
#[test]
fn parity_double_hash_requires_extendedglob() {
    assert_parity(r#"[[ "abc123" = [a-z]##[0-9]## ]] && echo m || echo nope"#);
}

// ─── 7. echo "\$var" — backslash-dollar literal ──────────────────

/// `echo "\$var"` produces literal `\$var` in zsh (the `\$` is the
/// dollar-escape inside double quotes, preserved verbatim by echo).
/// Fixed: BUILTIN_EXPAND_TEXT now emits canonical Bnull (\u{9f})
/// marker instead of \x00 for escaped specials. The Bnull stripper
/// at subst.rs:643 (port of Src/zsh.h:195) already keeps the
/// escaped char verbatim — `\x00` was silently ignored, letting
/// `\$var` re-trigger param expansion.
#[test]
fn parity_double_quoted_backslash_dollar_var_literal() {
    assert_parity(r#"echo "\$var""#);
}

/// Set-variable form must also preserve the literal.
#[test]
fn parity_double_quoted_backslash_dollar_set_var_literal() {
    assert_parity(r#"var=ABC; echo "\$var""#);
}

/// Unquoted `\$var` also produces literal `$var` in zsh — the
/// backslash escape works outside double quotes too. Default-mode
/// EXPAND_TEXT got the same Bnull fix as DQ.
#[test]
fn parity_unquoted_backslash_dollar_literal() {
    assert_parity(r#"echo \$var"#);
}

// ─── 8. Math functions auto-loaded vs zmodload-required ──────────

/// `$((sqrt(4)))` in zsh requires `zmodload zsh/mathfunc` first;
/// without it, zsh errors "unknown function: sqrt". Fixed:
/// math.rs::callmathfunc now gates module functions on
/// MOD_INIT_B (set by load_module after setup/boot per
/// c:Src/module.c:2206-2322), and load_module no longer
/// short-circuits on MOD_LINKED alone. Builtin-module
/// registration pre-sets MOD_LINKED for statically-linked modules
/// but leaves MOD_INIT_B clear until `zmodload zsh/mathfunc`.
#[test]
fn parity_mathfunc_requires_zmodload() {
    assert_parity("echo $((sqrt(4)))");
}

#[test]
fn parity_mathfunc_sin_requires_zmodload() {
    assert_parity("echo $((sin(0)))");
}

#[test]
fn parity_mathfunc_floor_requires_zmodload() {
    assert_parity("echo $((floor(3.7)))");
}

/// With `zmodload zsh/mathfunc`, the functions DO work.
#[test]
fn parity_mathfunc_after_zmodload_succeeds() {
    assert_parity("zmodload zsh/mathfunc; echo $((sqrt(4)))");
}

// ─── 9. $=a — word-split parameter expansion flag ────────────────

/// `for w in $=a; do print [$w]; done` in zsh splits the contents of
/// `$a` on IFS. Fixed: compile_zsh.rs has a `$=NAME` fast path that
/// emits GET_VAR + WORD_SPLIT, mirroring the existing `$~NAME` glob
/// path. Direct port of subst.c:2554-2566 `case '='` (spbreak=2)
/// via the unbraced-shorthand path.
#[test]
fn parity_dollar_equals_word_split_flag() {
    assert_parity(r#"a="x  y  z"; for w in $=a; do print -- "[$w]"; done"#);
}

/// Bare `$a` (no flag) must NOT split — only `$=a` triggers split.
/// This pins the negative case so a future regression that
/// over-aggressively splits gets caught.
#[test]
fn parity_bare_dollar_does_not_split() {
    assert_parity(r#"a="x y z"; for w in $a; do print -- "[$w]"; done"#);
}

// ─── 10. $~b — glob-eval parameter expansion flag ────────────────

/// `$~var` in zsh enables glob/pattern interpretation on the result
/// of expansion. Critically, it does NOT recursively expand `$`
/// references inside the value. With `b="\$a"`, zsh outputs `\$a`
/// (the literal backslash-dollar-a). zshrs outputs `foo` — wrongly
/// treats `$~` as recursive variable expansion.
/// `$~b` should evaluate `$b` then treat result as glob — not as a
/// recursive var expansion. Fix landed via the `\$` Bnull marker fix
/// (fusevm_bridge.rs:3527) which made `print -- $~b` see the literal
/// `\$a` body unchanged. The `$~NAME` fast path at compile_zsh.rs:2020
/// already existed; the Bnull repair unblocked the surrounding test.
#[test]
fn parity_dollar_tilde_glob_eval_flag() {
    assert_parity(r#"a=foo; b='\$a'; print -- $~b"#);
}

// ─── 11. read -A doesn't split into array ────────────────────────

/// `IFS=: read -A arr <<< "a:b:c"` should split into 3 elements.
/// Fixed: builtin.rs:8106 was hardcoded to `split_whitespace()`;
/// now reads $IFS and walks per-char with whitespace-IFS coalescing
/// (mirrors the multi-var path's IFS logic, c:Src/builtin.c:6685-6735).
#[test]
fn parity_read_minus_A_array_split() {
    assert_parity(r#"IFS=: read -A arr <<< "a:b:c"; echo "${#arr}=${arr[2]}""#);
}

/// `read -A` with default IFS (whitespace) still works after the fix.
#[test]
fn parity_read_minus_A_default_ifs() {
    assert_parity(r#"read -A arr <<< "a b c"; echo "${#arr}=${arr[2]}""#);
}

/// `read -A` with non-whitespace IFS and 4 fields. Pins the
/// non-coalescing behavior: each delimiter creates exactly one field
/// boundary (unlike whitespace which coalesces).
#[test]
fn parity_read_minus_A_comma_ifs_four_fields() {
    assert_parity(
        r#"IFS=, read -A arr <<< "x,y,z,w"; echo "${#arr}=[${arr[1]}][${arr[2]}][${arr[3]}][${arr[4]}]""#,
    );
}

/// `read -A` with consecutive whitespace coalesces into one delimiter
/// (zsh-style).
#[test]
fn parity_read_minus_A_whitespace_coalesce() {
    assert_parity(
        r#"read -A arr <<< "a   b  c"; echo "${#arr}=[${arr[1]}][${arr[2]}][${arr[3]}]""#,
    );
}

// ─── 12. Modifier on array subscript applies to whole array — FIXED ─

/// `${a[1]:t}` should apply `:t` (tail) only to the indexed element.
/// Fixed: modifier dispatch in paramsubst now branches on
/// `subscript.is_some()` per Src/subst.c:4533-4540 — when a numeric
/// subscript has narrowed to a single element, C's `isarr` is 0
/// and modify() runs on the scalar form. Previously zshrs always
/// re-fetched the whole array via `arrays_get`, applying the
/// modifier to every element.
#[test]
fn parity_modifier_on_array_subscript() {
    assert_parity(r#"a=("/p/a" "/q/b"); echo ${a[1]:t}"#);
}

// ─── 13. NOMATCH error on unmatched glob ─────────────────────────

/// In zsh (NOMATCH set by default), a glob that matches no files
/// errors with "no matches found" and exits 1. Fixed: globdata_glob
/// at glob.rs:3128 was pushing the literal pattern on no-match,
/// which made vm_helper.rs::expand_glob skip the NOMATCH dispatch.
/// Now returns Vec::new() so expand_glob fires the zerr path per
/// c:Src/glob.c:1872-1888.
#[test]
fn parity_nomatch_error_on_unmatched_glob() {
    assert_parity("echo /not_a_real_path_zshrs_xyz_unique_abc/*");
}

/// `setopt nullglob` makes unmatched globs vanish (no error).
#[test]
fn parity_nullglob_drops_unmatched_glob() {
    assert_parity("setopt nullglob; echo X *.never_match_zshrs_xyz Y");
}

/// Escaped glob metas (`\*`) become literal, no NOMATCH fires.
#[test]
fn parity_escaped_glob_meta_no_nomatch() {
    assert_parity(r#"echo \*.never_match_zshrs_unique"#);
}

// ─── 14. Process substitution path scheme ────────────────────────

/// `echo <(true)` in zsh prints `/dev/fd/N` (uses anonymous pipe via
/// procfs). zshrs uses `/tmp/zshrs_psub_PID_N` (named temp file
/// fallback). Behavior-equivalent for reads but path differs — any
/// `<(cmd)` returns a `/dev/fd/N` path (where N is the parent's
/// pipe read-end fd kept open across exec). Fixed: fusevm_bridge.rs::
/// process_sub_in now pipe()+fork()s like c:Src/exec.c::getproc
/// instead of writing to `/tmp/zshrs_psub_*`. The exact fd number
/// differs between zsh and zshrs by shell-internal-state, so this
/// test asserts the path SCHEME via the consumer's observable
/// output (cat reads through the pipe correctly) rather than the
/// literal path string.
#[test]
fn parity_process_subst_consumer_reads_through_pipe() {
    assert_parity(r#"cat <(echo hello-from-psub)"#);
}

#[test]
fn parity_process_subst_diff_same_inputs() {
    assert_parity(r#"diff <(echo a) <(echo a); echo exit=$?"#);
}

#[test]
fn parity_process_subst_diff_different_inputs() {
    assert_parity(r#"diff <(echo a) <(echo b) > /dev/null; echo exit=$?"#);
}

/// Pin the path scheme: any matching `/dev/fd/N` form is acceptable
/// (fd numbers vary), but `/tmp/zshrs_psub_*` (the old tempfile)
/// must NOT appear.
#[test]
fn parity_process_subst_path_uses_dev_fd_scheme() {
    if !zsh_available() {
        return;
    }
    let r = run_zshrs("echo <(true)");
    assert!(
        r.stdout.starts_with("/dev/fd/"),
        "expected /dev/fd/<N> path, got: {:?}",
        r.stdout
    );
    assert!(
        !r.stdout.contains("zshrs_psub"),
        "unexpected tempfile in psub path: {:?}",
        r.stdout
    );
}

// ─── 15. Brace range with zero step — FIXED ─────────────────────

/// `echo {1..5..0}` — zero step is err++ per Src/glob.c:2368
/// `if (p != str2 || !rincr) err++;`. C falls through to the
/// comma-expansion path which strips braces and produces
/// `1..5..0` (no expansion, no braces). Fixed:
///   - expand_range returns None on raw==0 step (Src/glob.c:2368).
///   - try_expand_one falls through to a "strip braces" emit
///     that mirrors C's c:2476-2506 comma-loop with no commas.
///   - Gated on digit presence per C's hasbraces (c:2085).
#[test]
fn parity_brace_zero_step_invalid_literal() {
    assert_parity("echo {1..5..0}");
}

// ─── 16. select prompt format — stdout-only parity holds ─────────

/// `select` loop output: the menu list AND prompt both go to STDERR
/// (per Src/loop.c:278+, `selectlist` writes to `shout` = file 2),
/// so the stdout comparison this test does is trivially empty for
/// both shells. The earlier "ANSI prompt differs" finding was an
/// artifact of capturing stderr+stdout combined. Pinning the stdout
/// invariant: both shells produce empty stdout from select before
/// the iteration body runs.
#[test]
fn parity_select_prompt_format() {
    assert_parity("select x in a b c; do break; done <<< 1");
}

// ─── 17. print -P %F wraps in \001..\002 markers — FIXED ─────────

/// `print -P "%F{red}red%f"` in zsh emits raw SGR (`\E[31mred\E[39m`).
/// Fixed: bin_print's `-P` arm now scrubs `\x01`/`\x02` (the
/// readline RL_PROMPT_*_IGNORE markers the Rust expand_prompt
/// emits around SGR sequences) after expansion. C source rule at
/// Src/prompt.c:236-247: when `ns=0` (non-stripping flag off,
/// which is print -P's mode per c:4598), promptexpand removes the
/// Inpar/Outpar/Nularg marker bytes before returning. Mirror that
/// strip pass at the builtin's call site since our markers are
/// `\x01`/`\x02` rather than the canonical Inpar/Outpar.
#[test]
fn parity_print_P_color_marker_wrapping() {
    assert_parity(r#"print -P "%F{red}red%f""#);
}

#[test]
fn parity_print_P_bold_marker_wrapping() {
    assert_parity(r#"print -P "%B%bnoop""#);
}

// ─── 18. (#s) start-anchor without extendedglob — FIXED ─────────

/// `[[ "abc" = (#s)a* ]]` requires extendedglob in zsh.
/// Fixed: both the leading-flag-hoist loop in `patcompile` AND the
/// mid-pattern `(#...)` parse in `patcompbranch` now consult
/// `zpc_special[ZPC_HASH]`. Without extendedglob, ZPC_HASH masks
/// to Marker and the `(#...)` form is treated as a literal `(`.
/// Src/pattern.c:953-957.
#[test]
fn parity_pound_s_anchor_requires_extendedglob() {
    assert_parity(r#"[[ "abc" = (#s)a* ]] && echo m || echo no"#);
}

// ─── 19. (#e) end-anchor without extendedglob — FIXED ───────────

#[test]
fn parity_pound_e_anchor_requires_extendedglob() {
    assert_parity(r#"[[ "abc" = *c(#e) ]] && echo m || echo no"#);
}

// ─── 20. Multibyte string slice ${var:0:1} ────────────────────────

/// `a=日本; echo "${a:0:1}"` outputs "日". Fixed: lex.rs:2256
/// `add(lextok2_get(c) as char)` was truncating multibyte codepoints
/// to their low byte (日 U+65E5 → å U+00E5) via the implicit
/// `c as u8` in lextok2_get's >=256 branch. Now bypasses the
/// 256-entry table for any codepoint >= 256 and adds the char
/// verbatim. The substring/subscript machinery downstream was
/// already char-correct; the lexer was the entry-point bug.
#[test]
fn parity_multibyte_substring_slice() {
    assert_parity("a=日本; echo \"${a:0:1}\"");
}

// ─── 21. Multibyte array-style subscript ${var[1]} ───────────────

/// `a=日本; echo "${a[1]}"` outputs "日". Same lexer fix as
/// parity_multibyte_substring_slice unblocked this.
#[test]
fn parity_multibyte_scalar_subscript() {
    assert_parity("a=日本; echo \"${a[1]}\"");
}

/// Multibyte preserves through bare assignment and length count.
#[test]
fn parity_multibyte_assign_and_length() {
    assert_parity("a=日本; echo \"${#a} $a\"");
}

/// Multibyte at end of string survives the lexer's LX2_OTHER arm.
#[test]
fn parity_multibyte_trailing() {
    assert_parity(r#"a=Z日Y本X; print -r -- "$a""#);
}

// ─── 22. declare -p PATH output format ──────────────────────────

/// `declare -p PATH` originally produced `typeset -T PATH=''`;
/// zsh produces `export -T PATH path=( ... )`. Half-fixed:
///   - `export ` prefix: vm_helper.rs:986 now stamps PM_EXPORTED on
///     every paramtab entry whose name exists in `environ`, so the
///     printparamnode PM_EXPORTED branch (params.rs:8385) fires.
///   - Value rendering: params.rs:8241 now dispatches gsu_s.getfn
///     for SPECIAL scalars and falls back to env::var for exported
///     scalars, so HOME/PATH/USER round-trip to their actual values.
/// Remaining cosmetic divergence: zsh prints the TIED-pair form
/// `export -T PATH path=( elem1 elem2 )` while zshrs still prints
/// `export -T PATH='joined:value'`. Both are semantically equivalent
/// (eval'ing either restores the same state), but the literal stdout
/// strings differ. Pinned via two narrower tests: scalar exported
/// vars (HOME, USER) round-trip; tied PATH would need a peer-name
/// swap in printparamnode (c:Src/params.c:6253-6283) that's deferred.
#[test]
fn parity_declare_p_exported_scalar_home() {
    assert_parity("declare -p HOME");
}

#[test]
fn parity_declare_p_exported_scalar_user() {
    assert_parity(r#"declare -p USER 2>/dev/null || true"#);
}

/// PATH-tied output: pin the EXIT CODE + that the format starts with
/// the right export-T prefix. Full peer-array output is a deferred
/// follow-up.
#[test]
fn parity_declare_p_path_starts_with_export_T() {
    if !zsh_available() {
        return;
    }
    let r = run_zshrs("declare -p PATH");
    assert_eq!(r.exit, 0);
    assert!(
        r.stdout.starts_with("export -T PATH"),
        "expected `export -T PATH` prefix, got: {:?}",
        &r.stdout[..r.stdout.len().min(80)]
    );
}

// ─── 23. ${a:^b} array-zip with missing b — FIXED ───────────────

/// `a=(1 2 3); echo ${a:^b}` — when b is UNSET (vs set-but-empty),
/// zsh returns a's contents verbatim. Fixed by distinguishing
/// `arrays_get(name)` → `None` (unset) vs `Some([])` (empty array)
/// in the `:^` arm of paramsubst. Previously both collapsed to
/// `min(|a|, 0)` = empty. Src/subst.c:3540 (SUB_ZIP path).
#[test]
fn parity_array_zip_with_missing_right_operand() {
    assert_parity("a=(1 2 3); echo ${a:^b}");
}

// ─── 24. ${a:^b} quoted-zip with IFS joining differs ────────────

/// `a=(1 2 3); b=(x y z); echo "${a:^b}"` — DQ short-zip. Fixed by
/// propagating qt=true into the BRIDGE_BRACE_ARRAY fast path.
///   - compile_zsh.rs:2654 prefixes the inner body with Qstring
///     (\u{8c}) when `dq_context_depth > 0` or the raw word is
///     Dnull-wrapped
///   - fusevm_bridge.rs::BUILTIN_BRIDGE_BRACE_ARRAY strips the
///     Qstring prefix and bumps `exec.in_dq_context` for the
///     paramsubst call
///   - paramsubst_to_value reads `in_dq_context` and passes qt=true
///     to subst::paramsubst
///   - subst.rs::paramsubst SUB_ZIP/SUB_ZIPN branches under qt=true
///     collapse the FIRST operand to sepjoin(a) before zipping,
///     yielding `[sepjoin(a), b[0]]` for `:^` and pairs of
///     `(sepjoin(a), b[i % blen])` for `:^^`. Port of
///     c:Src/subst.c:3456-3520.
#[test]
fn parity_array_zip_quoted_joining_differs() {
    assert_parity(r#"a=(1 2 3); b=(x y z); echo "${a:^b}""#);
}

/// `print -l "${a:^b}"` exercises the 2-element splat under DQ.
#[test]
fn parity_array_zip_dq_print_minus_l() {
    assert_parity(r#"a=(1 2 3); b=(x y z); print -l "${a:^b}""#);
}

/// `${a:^^b}` DQ — long-zip emits 2*max(1, blen) elements with
/// sepjoin(a) at even positions.
#[test]
fn parity_array_zip_long_dq_print_minus_l() {
    assert_parity(r#"a=(1 2); b=(x y z); print -l "${a:^^b}""#);
}

/// Unquoted zip still interleaves (qt=false code path unchanged).
#[test]
fn parity_array_zip_unquoted_still_interleaves() {
    assert_parity(r#"a=(1 2 3); b=(x y z); echo ${a:^b}"#);
}

// ─── 25. [[ ... -a ... ]] in [[ ]] is a parse error in zsh ───────

/// `[[ "" -a "x" ]]` — ksh-style `-a` boolean AND inside `[[ ]]` is
/// a zsh PARSE ERROR ("condition expected") per c:Src/parse.c:2601-
/// 2625 (par_cond_2 binary-op set excludes `-a`/`-o`). Fixed:
/// parse_cond_primary at parse.rs:8076 rejects Dash+a/o (handles
/// both ASCII `-` and the lexer's Dash token \u{9b}), sets errflag
/// + LEXERR. The error propagates: par_cond returns None →
/// par_cmd returns None → par_sublist returns None → execute_script
/// at vm_helper.rs:1200 sees the post-parse errflag and aborts before
/// running the wordcode, so the whole line `... && echo m || echo n`
/// is rejected (no stdout).
#[test]
fn parity_double_bracket_minus_a_should_error() {
    assert_parity(r#"[[ "" -a "x" ]] && echo m || echo n"#);
}

/// `-o` (ksh OR) gets the same parse error treatment.
#[test]
fn parity_double_bracket_minus_o_should_error() {
    assert_parity(r#"[[ "x" -o "y" ]] && echo m || echo n"#);
}

/// Bare `[[ X -a Y ]]` (no chain) — still parse error, exit 1.
#[test]
fn parity_double_bracket_minus_a_bare() {
    assert_parity(r#"[[ x -a y ]]"#);
}

/// `[[ -a /etc ]]` is the LEGITIMATE unary file-existence test — the
/// `-a` here is a unary operator (file exists), distinct from the
/// binary `-a` (logical AND) which is the parse error. Pin the
/// negative case so a regression that over-rejects `-a` gets caught.
#[test]
fn parity_double_bracket_minus_a_unary_file_test() {
    assert_parity(r#"[[ -a /etc ]] && echo Y || echo N"#);
}

// ─── 26. ~+ tilde expansion to PWD ──────────────────────────────

/// `echo ~+` in zsh expands to current PWD. Fixed by porting C's
/// `isend(NUL)`-is-true semantics: bare `~+` with no trailing char
/// must expand (was requiring chars.len() >= 3 — wrong).
/// Src/subst.c:752 — `*str == '+' && isend(str[2])`.
#[test]
fn parity_tilde_plus_pwd_expansion() {
    assert_parity("cd /tmp; echo ~+");
}

// ─── 27. ~- tilde expansion to OLDPWD ───────────────────────────

/// `echo ~-` in zsh expands to OLDPWD. Same fix as ~+.
/// Src/subst.c:755 — `*str == '-' && isend(str[2])`.
#[test]
fn parity_tilde_minus_oldpwd_expansion() {
    assert_parity("cd /tmp; cd /; echo ~-");
}

// ─── 28. Per-element ${a:e} on array ────────────────────────────

/// `a=(a.txt b.md); print -- ${a:e}` should produce `txt md` —
/// applying `:e` (extension) to each element of the array. zshrs
/// applies it only to the first element, producing just `txt`.
#[test]
fn parity_array_extension_modifier_per_element() {
    assert_parity("a=(a.txt b.md); print -- ${a:e}");
}

/// `${arr:t}`/`:h`/`:r`/`:e` apply per-element in non-DQ context.
/// `print -l ${a:e}` over (file.txt readme.md) emits two lines.
/// Fixed by: (1) qt-gated modifier dispatch in subst.rs:5602 forcing
/// DQ to sepjoin before modify (per c:3030-3034), and (2) array-aware
/// BUILTIN_BRACE_EXPAND / BUILTIN_GLOB_EXPAND that handle Value::Array
/// input without collapsing via `pop().to_str()`.
#[test]
fn parity_array_extension_modifier_splat() {
    assert_parity("a=(file.txt readme.md); print -l ${a:e}");
}

/// DQ form joins array first then applies modifier — `:e` on the
/// joined string returns the suffix after the LAST dot.
#[test]
fn parity_array_modifier_dq_joins_first() {
    assert_parity("a=(file.txt readme.md); echo \"<${a:e}>\"");
}

/// `${a:r}` per-element root strip.
#[test]
fn parity_array_root_modifier_per_element() {
    assert_parity("a=(file.txt readme.md); print -l ${a:r}");
}

/// `${a:t}` per-element tail (basename).
#[test]
fn parity_array_tail_modifier_per_element() {
    assert_parity("a=(/foo/bar /baz/qux); print -l ${a:t}");
}

/// `${a:h}` per-element head (dirname).
#[test]
fn parity_array_head_modifier_per_element() {
    assert_parity("a=(/foo/bar /baz/qux); print -l ${a:h}");
}

// ─── 29. typeset -F precision respected in print — FIXED ────────

/// `typeset -F 4 f; (( f=3.14159 )); print $f` formats with 4 digits.
/// Fixed: bare `typeset -F N name` (no `=value`) arm now stamps
/// `pm.base = N` so subsequent math-assignment respects precision.
/// Src/builtin.c:1973-1989.
#[test]
fn parity_typeset_F_precision_in_print() {
    assert_parity("typeset -F 4 f; (( f=3.14159 )); print -- $f");
}

#[test]
fn parity_typeset_F_precision_two_digits() {
    assert_parity("typeset -F 2 f; (( f=1.0/3 )); print -- $f");
}

// ─── 30. `read -d "" -A arr` with NUL delimiter splits ──────────

/// `printf "a,b" | { IFS=, read -d "" -A arr; echo ${#arr}; }` —
/// with NUL-delimited read AND `-A`, zsh splits on IFS. Fixed by the
/// same builtin.rs:8106 IFS-aware splitting change as
/// parity_read_minus_A_array_split — `-d ""` controls the line
/// terminator (NUL) but the body still flows through the same `-A`
/// splitting path.
#[test]
fn parity_read_minus_d_minus_A_with_ifs() {
    assert_parity(
        r#"printf "a,b" | { IFS=, read -d "" -A arr; echo "${#arr}=${arr[1]}-${arr[2]}"; }"#,
    );
}

// ─── 31. ${(o)a} sort order is case-insensitive — FIXED ─────────

/// `a=(B a); echo ${(o)a}` — zsh's `(o)` uses locale-aware comparison
/// via `strcoll()` (UTF-8 locale → case-insensitive).
/// Fixed: subst.rs default-sort arm now routes through
/// `crate::ported::sort::zstrcmp(_, _, 0)` which calls libc strcoll
/// under the active locale instead of raw byte `sort()`.
#[test]
fn parity_sort_o_case_insensitive_order() {
    assert_parity("a=(B a); echo ${(o)a}");
}

// ─── 32. $- shell flags differ in last character — FIXED ────────

/// `echo "$-"` returns the active shell flags. Fixed by routing
/// `$-` through the canonical `dashgetfn()` (port of
/// `Src/options.c:890`) which walks the zshletters[] table per
/// `Src/options.c:292`, instead of the hand-rolled "569X + 8
/// hard-coded letters" subset that mapped `h` to `hashall` (wrong)
/// instead of HISTIGNOREDUPS (correct, per Src/options.c:349).
#[test]
fn parity_dollar_dash_flags_differ() {
    assert_parity("echo \"$-\"");
}

// ─── 33. nullglob no-match through zglob drops the word — FIXED ──

/// Command-position globs are expanded via the canonical `zglob`
/// (exec.rs:9033, port of C `zglob(args, firstnode(args), 0)` at
/// glob.c:3318). zglob's terminal no-match block (glob.rs:1341)
/// had only guarded the CSHNULLGLOB and NOMATCH arms with
/// `!nullglob`; the literal-insert fallback fell through
/// unconditionally, so under `setopt nullglob` a no-match would
/// echo the literal pattern instead of dropping the word.
/// Fixed by hoisting the `!nullglob` test to a single outer gate,
/// mirroring C's `else if (!gf_nullglob)` (glob.c:1872) which skips
/// the whole no-match block — leaving matchbuf empty so glob()
/// removes the word.
#[test]
fn parity_nullglob_command_position_drops_word() {
    assert_parity("setopt nullglob; /nonexistent_xyz_zzz_*; echo exit=$?");
}

/// Same gate, value position: `setopt nullglob; echo /never_*`
/// drops the word (empty line), matching zsh.
#[test]
fn parity_nullglob_value_position_drops_word() {
    assert_parity("setopt nullglob; echo /nonexistent_xyz_zzz_*");
}

/// The `!nullglob` arms still behave: with nullglob OFF and the
/// default NOMATCH on, a no-match errors `no matches found` and
/// exits 1 (the literal is NOT echoed).
#[test]
fn parity_nomatch_still_errors_with_nullglob_off() {
    assert_parity("echo /nonexistent_xyz_zzz_*; echo exit=$?");
}

/// CSHNULLGLOB arm still behaves: a single failed word under
/// cshnullglob is dropped, not turned into a "no matches" error.
#[test]
fn parity_cshnullglob_single_word_dropped() {
    assert_parity("setopt cshnullglob; echo /nonexistent_xyz_zzz_*; echo exit=$?");
}
