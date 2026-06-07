//! `print -P` prompt-escape parity tests.
//!
//! Covers the same expansion engine PS4 uses for xtrace prefixes
//! (Src/prompt.c putpromptchar) — any divergence here surfaces as a
//! visible xtrace mismatch with C zsh. The `ps4_audit` module pins
//! every documented `%`-escape byte-for-byte against
//! /opt/homebrew/bin/zsh.

#![allow(non_snake_case)]

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
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
struct R {
    stdout: String,
    exit: i32,
}
fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn run_zshrs(s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
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
        "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit);
}

mod literals {
    use super::*;

    #[test]
    fn plain_text_unchanged() {
        assert_parity(r#"print -P 'hello world'"#);
    }

    #[test]
    fn double_percent_yields_single() {
        assert_parity(r#"print -P '%%'"#);
    }

    #[test]
    fn empty_string() {
        assert_parity(r#"print -P ''"#);
    }

    #[test]
    fn percent_in_middle() {
        assert_parity(r#"print -P 'a%%b'"#);
    }
}

mod ansi_attrs {
    use super::*;

    #[test]
    fn B_bold_emits_sgr() {
        assert_parity(r#"print -P '%B' | cat -v"#);
    }

    #[test]
    fn b_reset_emits_sgr() {
        assert_parity(r#"print -P '%b' | cat -v"#);
    }

    #[test]
    fn U_underline_emits_sgr() {
        assert_parity(r#"print -P '%U' | cat -v"#);
    }

    #[test]
    fn u_underline_off_emits_sgr() {
        assert_parity(r#"print -P '%u' | cat -v"#);
    }

    #[test]
    fn S_standout_emits_sgr() {
        assert_parity(r#"print -P '%S' | cat -v"#);
    }

    #[test]
    fn s_standout_off_emits_sgr() {
        assert_parity(r#"print -P '%s' | cat -v"#);
    }
}

mod colors {
    use super::*;

    #[test]
    fn F_red_emits_sgr_31() {
        assert_parity(r#"print -P '%F{red}' | cat -v"#);
    }

    #[test]
    fn F_blue_emits_sgr_34() {
        assert_parity(r#"print -P '%F{blue}' | cat -v"#);
    }

    #[test]
    fn f_reset_fg() {
        assert_parity(r#"print -P '%f' | cat -v"#);
    }

    #[test]
    fn K_red_bg_emits_sgr_41() {
        assert_parity(r#"print -P '%K{red}' | cat -v"#);
    }

    #[test]
    fn k_reset_bg() {
        assert_parity(r#"print -P '%k' | cat -v"#);
    }

    #[test]
    fn F_numeric_index_1_is_red() {
        assert_parity(r#"print -P '%F{1}' | cat -v"#);
    }
}

mod literal_opaque {
    use super::*;

    #[test]
    fn literal_braces_content_only() {
        assert_parity(r#"print -P '%{ABCD%}' | cat"#);
    }

    #[test]
    fn literal_braces_with_text_around() {
        assert_parity(r#"print -P 'before%{ABC%}after' | cat"#);
    }
}

mod conditional {
    use super::*;

    /// `%(?.true_branch.false_branch)` — conditional on last exit code.
    #[test]
    fn conditional_dollar_q_true_branch() {
        assert_parity(r#"true; print -P '%(?.OK.FAIL)'"#);
    }

    #[test]
    fn conditional_dollar_q_false_branch() {
        assert_parity(r#"false; print -P '%(?.OK.FAIL)'"#);
    }
}

mod hash_marker {
    use super::*;

    #[test]
    fn hash_marker_non_root_is_percent() {
        assert_parity(r#"print -P '%#'"#);
    }
}

mod text_around_escape {
    use super::*;

    #[test]
    fn text_before_after_bold() {
        assert_parity(r#"print -P 'pre%Bmid%bafter' | cat -v"#);
    }

    #[test]
    fn nested_color_then_attribute() {
        assert_parity(r#"print -P '%F{red}%BHIGHLIGHT%b%f' | cat -v"#);
    }
}

mod unknown_escape {
    use super::*;

    /// Unknown escape doesn't crash either shell.
    #[test]
    fn unknown_letter_handled_safely() {
        // Pin exit code (= 0); output may vary on how unknown is handled.
        assert_parity(r#"print -P '%Q' >/dev/null; echo $?"#);
    }
}

mod no_dash_P_no_processing {
    use super::*;

    /// Without `-P`, `%` chars are literal.
    #[test]
    fn no_P_percent_is_literal() {
        assert_parity(r#"print '%B'"#);
    }

    #[test]
    fn no_P_percent_braces_literal() {
        assert_parity(r#"print '%{abc%}'"#);
    }
}

// ──────────────────────────────────────────────────────────────────
// PS4 byte-for-byte audit
//
// Every documented `%`-escape from `Src/prompt.c` putpromptchar's
// switch. Each test runs `zsh -fc 'print -P <esc>'` and
// `zshrs -fc 'print -P <esc>'` and asserts byte-equal stdout. Since
// PS4 routes through the same expansion path, parity here =>
// PS4 parity in xtrace output.
//
// Time-sensitive escapes (%T/%t/%@/%*/%w/%W/%D) are excluded from
// strict byte-equality (the two subprocesses can straddle a second
// boundary); a separate `time_escapes_format_shape` test pins their
// FORMAT instead (length + delimiter positions).
//
// Authoritative list source: prompt.c case statements at lines 396,
// 540-694, 829-1093. CMDNAMES table for %_ / %^ at prompt.c:62-71.
// ──────────────────────────────────────────────────────────────────
mod ps4_audit {
    use super::*;

    /// Helper: run `print -P` on both shells; assert byte equality.
    fn pin(escape: &str) {
        let cmd = format!("print -rn -- {}", shell_word(&format!("[{}]", escape)));
        // Wrap in print -P so the escape gets expanded.
        let cmd_p = format!("print -Prn -- {}", shell_word(&format!("[{}]", escape)));
        let _ = cmd; // keep for debugging if needed
        assert_parity(&cmd_p);
    }

    /// Quote a shell argument for single-quoted embedding.
    fn shell_word(s: &str) -> String {
        format!("'{}'", s.replace('\'', "'\\''"))
    }

    // ─── identity / user / host ───
    #[test]
    fn pct_pct_literal_percent() {
        pin("%%");
    }
    #[test]
    fn pct_n_username() {
        pin("%n");
    }
    #[test]
    fn pct_m_hostname_short() {
        pin("%m");
    }
    #[test]
    fn pct_M_hostname_full() {
        pin("%M");
    }
    #[test]
    fn pct_y_tty_short() {
        pin("%y");
    }
    #[test]
    fn pct_l_tty_short_alt() {
        pin("%l");
    }
    #[test]
    fn pct_N_scriptname() {
        pin("%N");
    }
    #[test]
    fn pct_x_script_file() {
        pin("%x");
    }

    // ─── pwd / path ───
    #[test]
    fn pct_tilde_pwd_with_home_sub() {
        pin("%~");
    }
    #[test]
    fn pct_slash_pwd_full() {
        pin("%/");
    }
    #[test]
    fn pct_d_pwd_full_alt() {
        pin("%d");
    }
    #[test]
    fn pct_c_pwd_last_component() {
        pin("%c");
    }
    #[test]
    fn pct_C_pwd_last_component_no_tilde() {
        pin("%C");
    }
    #[test]
    fn pct_dot_pwd_last_component() {
        pin("%.");
    }

    // ─── shell / state ───
    #[test]
    fn pct_h_history_event() {
        pin("%h");
    }
    #[test]
    fn pct_bang_history_event() {
        pin("%!");
    }
    #[test]
    fn pct_i_lineno() {
        pin("%i");
    }
    #[test]
    fn pct_I_abs_lineno() {
        pin("%I");
    }
    #[test]
    fn pct_j_num_jobs() {
        pin("%j");
    }
    #[test]
    fn pct_L_shlvl() {
        pin("%L");
    }
    #[test]
    fn pct_question_lastval() {
        pin("%?");
    }
    #[test]
    fn pct_hash_root_marker() {
        pin("%#");
    }
    #[test]
    fn pct_e_func_depth() {
        pin("%e");
    }
    #[test]
    fn pct_v_psvar_empty() {
        pin("%v");
    }
    #[test]
    fn pct_V_psvar_nonempty_check() {
        pin("%V");
    }

    // ─── cmdstack / parser context (PS4 xtrace's bread-and-butter) ───
    #[test]
    fn pct_underscore_cmdstack_bottom_up() {
        pin("%_");
    }
    #[test]
    fn pct_caret_cmdstack_top_down() {
        pin("%^");
    }
    #[test]
    fn pct_dash_1_underscore_first_one() {
        pin("%-1_");
    }
    #[test]
    fn pct_2_underscore_last_two() {
        pin("%2_");
    }

    // ─── attributes ───
    #[test]
    fn pct_B_bold_on() {
        pin("%B");
    }
    #[test]
    fn pct_b_bold_off() {
        pin("%b");
    }
    #[test]
    fn pct_U_underline_on() {
        pin("%U");
    }
    #[test]
    fn pct_u_underline_off() {
        pin("%u");
    }
    #[test]
    fn pct_S_standout_on() {
        // Pinned to SGR 7 — see prompt.rs applytextattributes.
        pin("%S");
    }
    #[test]
    fn pct_s_standout_off() {
        // Pinned to SGR 27.
        pin("%s");
    }

    // ─── colors ───
    #[test]
    fn pct_F_bare_black_fg() {
        // Bare %F defaults to color 0 (SGR 30). This regression
        // was the headline finding in the PS4 audit; previously
        // zshrs emitted SGR 39 (reset fg).
        pin("%F");
    }
    #[test]
    fn pct_K_bare_black_bg() {
        // Bare %K defaults to color 0 (SGR 40).
        pin("%K");
    }
    #[test]
    fn pct_f_reset_fg() {
        pin("%f");
    }
    #[test]
    fn pct_k_reset_bg() {
        pin("%k");
    }
    #[test]
    fn pct_F_red_braced() {
        pin("%F{red}");
    }
    #[test]
    fn pct_F_2_palette_green() {
        pin("%2F");
    }
    #[test]
    fn pct_K_blue_braced() {
        pin("%K{blue}");
    }
    #[test]
    fn pct_K_5_palette_magenta() {
        pin("%5K");
    }

    // ─── misc / formatting ───
    #[test]
    fn pct_E_clear_eol() {
        pin("%E");
    }
    #[test]
    fn pct_G_zero_width() {
        pin("%G");
    }
    #[test]
    fn pct_H_highlight_no_arg() {
        pin("%H");
    }
    #[test]
    fn pct_r_right_string_empty() {
        pin("%r");
    }
    #[test]
    fn pct_R_right_string_alt_empty() {
        pin("%R");
    }

    // ─── ternaries ───
    #[test]
    fn ternary_dollar_q_true() {
        assert_parity(r#"true; print -P '%(?.YES.NO)'"#);
    }
    #[test]
    fn ternary_dollar_q_false() {
        assert_parity(r#"false; print -P '%(?.YES.NO)'"#);
    }
    #[test]
    fn ternary_j_no_jobs() {
        assert_parity(r#"print -P '%(j.HAVE.NONE)'"#);
    }
    #[test]
    fn ternary_L_shlvl_check() {
        assert_parity(r#"print -P '%(2L.DEEP.SHALLOW)'"#);
    }
    #[test]
    fn ternary_c_pwd_depth() {
        assert_parity(r#"print -P '%(c.HASPWD.NOPWD)'"#);
    }
    #[test]
    fn ternary_underscore_no_cmdstack() {
        // At top level cmdsp == 0; default arg=0 means cmdsp >= 0 → true.
        assert_parity(r#"print -P '%(_.STACK.NOSTACK)'"#);
    }
    #[test]
    fn ternary_underscore_with_arg_2_unlikely() {
        // Top level cmdsp == 0 < 2 → false branch.
        assert_parity(r#"print -P '%(2_.HAS2.NO2)'"#);
    }

    // ─── time-format shape (not value — wall-clock varies) ───
    /// `%T` is `HH:MM`. Both shells must emit exactly 5 chars matching
    /// `\d\d:\d\d`. Strict byte-equality would race the clock.
    #[test]
    fn time_format_shape_T_hhmm() {
        let z = run_zsh(r#"print -Prn -- '%T'"#);
        let r = run_zshrs(r#"print -Prn -- '%T'"#);
        let re = regex_like(&z.stdout, |c| c.is_ascii_digit(), ':');
        let re_r = regex_like(&r.stdout, |c| c.is_ascii_digit(), ':');
        assert_eq!(re, re_r, "%T shape divergence");
        assert!(re, "%T must match HH:MM shape");
    }
    /// `%*` is `HH:MM:SS`.
    #[test]
    fn time_format_shape_star_hhmmss() {
        let z = run_zsh(r#"print -Prn -- '%*'"#);
        let r = run_zshrs(r#"print -Prn -- '%*'"#);
        assert_eq!(z.stdout.len(), r.stdout.len(), "%* length mismatch");
        assert_eq!(z.stdout.len(), 8);
        for (zb, rb) in z.stdout.bytes().zip(r.stdout.bytes()) {
            assert_eq!(
                zb.is_ascii_digit(),
                rb.is_ascii_digit(),
                "%* digit-vs-delim shape mismatch"
            );
        }
    }
    /// `%D` is `YY-MM-DD`. Wall-clock parity within a day is fine — the
    /// two subprocesses fire within milliseconds.
    #[test]
    fn date_D_yyyymmdd() {
        assert_parity(r#"print -Prn -- '%D'"#);
    }
    /// `%W` is `MM/DD/YY`.
    #[test]
    fn date_W_mmddyy() {
        assert_parity(r#"print -Prn -- '%W'"#);
    }

    fn regex_like(s: &str, is_digit: impl Fn(char) -> bool, sep: char) -> bool {
        let chars: Vec<char> = s.chars().collect();
        chars.len() == 5
            && is_digit(chars[0])
            && is_digit(chars[1])
            && chars[2] == sep
            && is_digit(chars[3])
            && is_digit(chars[4])
    }

    // ─── env inheritance: PS4 + PROMPT4 alias ───────────────────
    //
    // C zsh's PS4 and PROMPT4 are aliases for the same `prompt4`
    // C global (Src/params.c:381 IPDEF7R, c:421 IPDEF7), so
    // exporting PROMPT4 in the parent env sets PS4 in the child.
    // zshrs's paramtab keeps them as separate entries; the
    // ShellExecutor::new env probe walks both. Without that walk,
    // `zshrs -x` reverted to the default `+%N:%i> ` prefix
    // whenever a user's shell exported only PROMPT4 (the form
    // prompt themes / p10k use) and not PS4. Pin all four
    // combinations so regression is caught at the parity layer.

    /// `PS4=X zshrs -fxc 'echo h'` → trace uses `X`.
    #[test]
    fn xtrace_inherits_ps4_from_env() {
        if !zsh_available() {
            return;
        }
        let z = Command::new(zsh_path())
            .args(["-fxc", "echo hi"])
            .env("PS4", "DIRECT-PS4> ")
            .env_remove("PROMPT4")
            .output()
            .expect("zsh");
        let r = Command::new(zshrs_bin())
            .args(["--zsh", "-fxc", "echo hi"])
            .env("PS4", "DIRECT-PS4> ")
            .env_remove("PROMPT4")
            .env_remove("ZSHRS_CACHE")
            .output()
            .expect("zshrs");
        assert_eq!(
            String::from_utf8_lossy(&z.stderr),
            String::from_utf8_lossy(&r.stderr),
            "PS4-from-env xtrace divergence"
        );
    }

    /// `PROMPT4=X zshrs -fxc 'echo h'` → trace uses `X` (alias).
    /// This is the user-reported case: their interactive shell
    /// exports PROMPT4 (not PS4) and bare `zshrs -x` was emitting
    /// the default prefix instead of the user's customised PS4.
    #[test]
    fn xtrace_inherits_prompt4_alias_from_env() {
        if !zsh_available() {
            return;
        }
        let z = Command::new(zsh_path())
            .args(["-fxc", "echo hi"])
            .env_remove("PS4")
            .env("PROMPT4", "ALIAS-PROMPT4> ")
            .output()
            .expect("zsh");
        let r = Command::new(zshrs_bin())
            .args(["--zsh", "-fxc", "echo hi"])
            .env_remove("PS4")
            .env("PROMPT4", "ALIAS-PROMPT4> ")
            .env_remove("ZSHRS_CACHE")
            .output()
            .expect("zshrs");
        assert_eq!(
            String::from_utf8_lossy(&z.stderr),
            String::from_utf8_lossy(&r.stderr),
            "PROMPT4-alias-from-env xtrace divergence (PS4=PROMPT4 alias broken)"
        );
    }

    /// Both PS4 and PROMPT4 set: PS4 wins (lookup order).
    #[test]
    fn xtrace_ps4_wins_over_prompt4_alias() {
        if !zsh_available() {
            return;
        }
        let z = Command::new(zsh_path())
            .args(["-fxc", "echo hi"])
            .env("PS4", "PRIMARY-PS4> ")
            .env("PROMPT4", "SECONDARY-PROMPT4> ")
            .output()
            .expect("zsh");
        let r = Command::new(zshrs_bin())
            .args(["--zsh", "-fxc", "echo hi"])
            .env("PS4", "PRIMARY-PS4> ")
            .env("PROMPT4", "SECONDARY-PROMPT4> ")
            .env_remove("ZSHRS_CACHE")
            .output()
            .expect("zshrs");
        assert_eq!(
            String::from_utf8_lossy(&z.stderr),
            String::from_utf8_lossy(&r.stderr),
            "PS4-priority-over-PROMPT4 divergence"
        );
    }

    /// Neither PS4 nor PROMPT4 in env: both shells fall back to the
    /// documented default `+%N:%i> `.
    #[test]
    fn xtrace_default_prefix_when_env_empty() {
        if !zsh_available() {
            return;
        }
        let z = Command::new(zsh_path())
            .args(["-fxc", "echo hi"])
            .env_remove("PS4")
            .env_remove("PROMPT4")
            .output()
            .expect("zsh");
        let r = Command::new(zshrs_bin())
            .args(["--zsh", "-fxc", "echo hi"])
            .env_remove("PS4")
            .env_remove("PROMPT4")
            .env_remove("ZSHRS_CACHE")
            .output()
            .expect("zshrs");
        assert_eq!(
            String::from_utf8_lossy(&z.stderr),
            String::from_utf8_lossy(&r.stderr),
            "default-PS4 divergence"
        );
    }
}
