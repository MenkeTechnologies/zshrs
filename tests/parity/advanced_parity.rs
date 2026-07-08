//! Advanced zsh feature parity — covers the deeper idioms that
//! `zinit`/`p10k`/`oh-my-zsh` rely on but earlier suites don't yet:
//!
//!  - `add-zsh-hook` + chpwd/precmd/preexec arrays
//!  - History expansion `!N`, `!!`, `^X^Y`
//!  - `[[ -o option ]]` option-test
//!  - Coproc + process substitution where parseable
//!  - Heredoc and herestring
//!  - `setopt` / `unsetopt` chains
//!  - `(`...`)` subshell semantics (env isolation)
//!  - `read -t` / `read -k`
//!  - `select` loop, `repeat N { … }`
//!  - `time CMD` (wall clock)

use std::path::PathBuf;
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("target").join("debug").join("zshrs")
}
fn zsh_path() -> &'static str {
    use std::path::Path;
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
    #[allow(dead_code)]
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
        "stdout divergence on script:\n{}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        s, z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit, "exit divergence on:\n{}", s);
}

// ───────────────────────── option tests ─────────────────────────

mod option_tests {
    use super::*;

    /// `[[ -o option ]]` true when option set.
    #[test]
    fn dash_o_set_after_setopt() {
        assert_parity(r#"setopt errexit; [[ -o errexit ]] && echo set || echo unset"#);
    }

    #[test]
    fn dash_o_unset_after_unsetopt() {
        assert_parity(
            r#"setopt errexit; unsetopt errexit; [[ -o errexit ]] && echo set || echo unset"#,
        );
    }

    #[test]
    fn dash_o_no_prefix_form() {
        assert_parity(r#"setopt no_clobber; [[ -o noclobber ]] && echo set || echo unset"#);
    }
}

// ───────────────────────── add-zsh-hook ─────────────────────────

mod hooks {
    use super::*;

    /// `add-zsh-hook -L` lists current hook arrays. With no hooks added,
    /// output should be empty (or both shells should agree).
    #[test]
    fn hook_list_empty() {
        assert_parity(
            r#"autoload -Uz add-zsh-hook 2>/dev/null
add-zsh-hook -L 2>/dev/null | head -5"#,
        );
    }

    /// chpwd_functions array exists and starts empty.
    #[test]
    fn chpwd_functions_array_default() {
        assert_parity(r#"print -- "${#chpwd_functions[@]}""#);
    }
}

// ───────────────────────── heredocs ─────────────────────────

mod heredocs {
    use super::*;

    /// Plain heredoc emits content as stdin to a command.
    #[test]
    fn heredoc_basic() {
        assert_parity(
            r#"cat <<EOF
line1
line2
EOF"#,
        );
    }

    /// `<<-` strips leading tabs.
    #[test]
    fn heredoc_dash_strips_tabs() {
        // Note: real \t needed in script
        let z = run_zsh("cat <<-EOF\n\tindented\n\tagain\n\tEOF");
        let r = run_zshrs("cat <<-EOF\n\tindented\n\tagain\n\tEOF");
        assert_eq!(z.stdout, r.stdout, "z={:?} r={:?}", z.stdout, r.stdout);
    }

    /// Heredoc with variable expansion.
    #[test]
    fn heredoc_expands_variables() {
        assert_parity(
            r#"x=hello; cat <<EOF
value: $x
EOF"#,
        );
    }

    /// Quoted heredoc delimiter disables expansion.
    #[test]
    fn heredoc_quoted_delim_no_expand() {
        assert_parity(
            r#"x=hello; cat <<'EOF'
value: $x
EOF"#,
        );
    }

    /// Herestring `<<<`.
    #[test]
    fn herestring_basic() {
        assert_parity(r#"cat <<< "hello world""#);
    }

    /// Herestring with variable.
    #[test]
    fn herestring_with_var() {
        assert_parity(r#"x=42; cat <<< "value=$x""#);
    }
}

// ───────────────────────── subshells ─────────────────────────

mod subshells {
    use super::*;

    /// `(cd /tmp; pwd)` doesn't change outer shell's cwd.
    #[test]
    fn subshell_cd_isolated() {
        assert_parity(r#"cd /; (cd /tmp; pwd); pwd"#);
    }

    /// Subshell variable changes don't propagate.
    #[test]
    fn subshell_var_isolated() {
        assert_parity(r#"x=outer; (x=inner; echo "in:$x"); echo "out:$x""#);
    }

    /// `{ … }` group runs in CURRENT shell (no isolation).
    #[test]
    fn brace_group_no_isolation() {
        assert_parity(r#"x=outer; { x=changed; }; echo $x"#);
    }
}

// ───────────────────────── process substitution ──────────────────

mod proc_subst {
    use super::*;

    /// `< <(cmd)` reads command's output. Both shells must match.
    /// Skip if the parser doesn't support the form.
    #[test]
    fn proc_subst_input() {
        assert_parity(r#"cat < <(echo hello)"#);
    }

    /// `<(cmd)` expands to a path.
    #[test]
    fn proc_subst_path_arg() {
        // We can't pin the exact path (varies per run); just verify
        // both shells emit ONE non-empty line.
        let z = run_zsh(r#"echo <(echo x) | head -1 | wc -l | tr -d ' '"#);
        let r = run_zshrs(r#"echo <(echo x) | head -1 | wc -l | tr -d ' '"#);
        assert_eq!(z.stdout, r.stdout);
    }
}

// ───────────────────────── select loop ─────────────────────────

mod select_loop {
    use super::*;

    /// `select x in a b c; do break; done < /dev/null` — read EOF
    /// immediately, exits cleanly.
    #[test]
    fn select_with_eof_input() {
        assert_parity(
            r#"select x in a b c; do break; done < /dev/null
echo done"#,
        );
    }
}

// ───────────────────────── repeat loop ─────────────────────────

mod repeat_loop {
    use super::*;

    /// `repeat 3 { echo hi }` — runs body 3 times.
    #[test]
    fn repeat_n_curly_form() {
        assert_parity(r#"repeat 3 { echo hi }"#);
    }

    /// `repeat 3 do; echo hi; done` — alternative syntax.
    #[test]
    fn repeat_n_do_done_form() {
        assert_parity(r#"repeat 3; do echo hi; done"#);
    }

    /// `repeat 0` runs nothing.
    #[test]
    fn repeat_zero_no_output() {
        assert_parity(r#"repeat 0; do echo hi; done; echo done"#);
    }
}

// ───────────────────────── for arithmetic ─────────────────────────

mod arith_for {
    use super::*;

    /// C-style `for ((i=0; i<3; i++))`.
    #[test]
    fn for_arith_basic() {
        assert_parity(r#"for ((i=0; i<3; i++)); do echo $i; done"#);
    }

    /// for arith with no init/cond/step.
    #[test]
    fn for_arith_step_only() {
        assert_parity(r#"i=0; for ((; i<3; i++)); do echo $i; done"#);
    }
}

// ───────────────────────── case statement ─────────────────────────

mod case_stmt {
    use super::*;

    /// Basic case with literal patterns.
    #[test]
    fn case_literal_match() {
        assert_parity(
            r#"x=foo
case $x in
    foo) echo matched-foo;;
    bar) echo matched-bar;;
    *) echo default;;
esac"#,
        );
    }

    /// Case with glob patterns.
    #[test]
    fn case_glob_match() {
        assert_parity(
            r#"x=hello
case $x in
    h*) echo glob;;
    *) echo other;;
esac"#,
        );
    }

    /// Case with alternation.
    #[test]
    fn case_alternation() {
        assert_parity(
            r#"x=b
case $x in
    a|b|c) echo abc;;
    *) echo other;;
esac"#,
        );
    }

    /// Case fall-through `;&`.
    #[test]
    fn case_fall_through() {
        assert_parity(
            r#"x=a
case $x in
    a) echo first ;&
    b) echo second ;;
    c) echo third ;;
esac"#,
        );
    }
}

// ───────────────────────── if/elif/else ─────────────────────────

mod if_chains {
    use super::*;

    #[test]
    fn elif_chain() {
        assert_parity(
            r#"x=2
if [[ $x -eq 1 ]]; then
    echo one
elif [[ $x -eq 2 ]]; then
    echo two
elif [[ $x -eq 3 ]]; then
    echo three
else
    echo other
fi"#,
        );
    }

    #[test]
    fn if_with_command_status() {
        assert_parity(r#"if true; then echo ok; else echo nope; fi"#);
    }

    #[test]
    fn if_negated() {
        assert_parity(r#"if ! false; then echo ok; fi"#);
    }
}

// ───────────────────────── arrays ─────────────────────────

mod arrays {
    use super::*;

    /// `arr=(a b c); echo "${arr[2]}"` — 1-based indexing in zsh.
    #[test]
    fn array_one_based_indexing() {
        assert_parity(r#"arr=(a b c); echo "${arr[2]}""#);
    }

    /// `${arr[-1]}` — negative index from end.
    #[test]
    fn array_negative_index() {
        assert_parity(r#"arr=(a b c); echo "${arr[-1]}""#);
    }

    /// `${arr[2,4]}` — range slice. zsh's nojoin gating per
    /// Src/subst.c paramsubst: in DQ the slice JOINS with first IFS
    /// char to a scalar; unquoted it stays array. Closed by
    /// adding compile-time `\u{02}` DQ sentinel that BUILTIN_ARRAY_INDEX
    /// reads to switch return shape.
    #[test]
    fn array_range_slice_dq_joins() {
        assert_parity(r#"arr=(a b c d e); print -l "${arr[2,4]}""#);
    }

    /// Unquoted range slice — multi-element array.
    #[test]
    fn array_range_slice_unquoted() {
        assert_parity(r#"arr=(a b c d e); print -l ${arr[2,4]}"#);
    }

    /// `[@]` splice with offset — explicit-array form.
    #[test]
    fn array_range_slice_at_splice() {
        assert_parity(r#"arr=(a b c d e); print -l "${arr[@]:1:3}""#);
    }

    /// Array length.
    #[test]
    fn array_length() {
        assert_parity(r#"arr=(a b c d e); echo ${#arr[@]}"#);
    }

    /// `${arr[@]:offset:length}` — element-wise slicing.
    #[test]
    fn array_slice_offset_length() {
        assert_parity(r#"arr=(a b c d e); print -l "${arr[@]:1:2}""#);
    }

    /// `${(@)arr}` — keep array (vs default join in DQ).
    #[test]
    fn array_at_flag() {
        assert_parity(r#"arr=(a b c); print -l -- "${(@)arr}""#);
    }

    /// `arr+=(d e)` append.
    #[test]
    fn array_append() {
        assert_parity(r#"arr=(a b); arr+=(c d); echo "${arr[@]}""#);
    }

    /// `arr[2]=X` element assign.
    #[test]
    fn array_element_assign() {
        assert_parity(r#"arr=(a b c); arr[2]=X; echo "${arr[@]}""#);
    }
}

// ───────────────────────── associative arrays ─────────────────

mod assocs {
    use super::*;

    /// Basic assoc set + get.
    #[test]
    fn assoc_basic() {
        assert_parity(r#"typeset -A m=(k1 v1 k2 v2); echo "${m[k1]}""#);
    }

    /// `${(k)assoc}` keys.
    #[test]
    fn assoc_keys() {
        assert_parity(r#"typeset -A m=(a 1 b 2 c 3); print -l -- "${(k)m}" | sort"#);
    }

    /// `${(v)assoc}` values.
    #[test]
    fn assoc_values() {
        assert_parity(r#"typeset -A m=(a 1 b 2 c 3); print -l -- "${(v)m}" | sort"#);
    }

    /// `${(kv)assoc}` flat key-value pairs.
    #[test]
    fn assoc_kv_flat() {
        assert_parity(r#"typeset -A m=(a 1 b 2); print -l -- "${(kv)m}" | sort"#);
    }

    /// `unset assoc[key]`.
    #[test]
    fn assoc_unset_key() {
        assert_parity(r#"typeset -A m=(a 1 b 2 c 3); unset 'm[b]'; print -l -- "${(k)m}" | sort"#);
    }

    /// `assoc[k]=val` assigns.
    #[test]
    fn assoc_element_assign() {
        assert_parity(r#"typeset -A m; m[foo]=bar; echo "${m[foo]}""#);
    }

    /// Empty assoc `(kv)`/`(k)`/`(v)` splat under `[@]` must yield ZERO
    /// words (not one empty word). zinit's `zinit()` opens with
    /// `ICE=( "${(kv)ZINIT_ICES[@]}" )` on an empty ZINIT_ICES; an odd
    /// word count triggered "bad set of key/value pairs for associative
    /// array", breaking every zinit invocation.
    #[test]
    fn assoc_kv_empty_at_splat() {
        assert_parity(r#"typeset -A m; x=( "${(kv)m[@]}" ); echo "${#x}""#);
    }

    #[test]
    fn assoc_k_empty_at_splat() {
        assert_parity(r#"typeset -A m; x=( "${(k)m[@]}" ); echo "${#x}""#);
    }

    #[test]
    fn assoc_v_empty_at_splat() {
        assert_parity(r#"typeset -A m; x=( "${(v)m[@]}" ); echo "${#x}""#);
    }

    /// `[*]` joins-via-IFS → a single (empty) word even when empty.
    #[test]
    fn assoc_kv_empty_star_splat() {
        assert_parity(r#"typeset -A m; x=( "${(kv)m[*]}" ); echo "${#x}""#);
    }

    /// An empty splat followed by a real word must keep exactly that word.
    #[test]
    fn assoc_kv_empty_at_splat_with_tail() {
        assert_parity(r#"typeset -A m; x=( "${(kv)m[@]}" tail ); echo "${#x}:${x[1]}""#);
    }

    /// The exact zinit assignment pattern.
    #[test]
    fn assoc_kv_zinit_ice_pattern() {
        assert_parity(
            r#"typeset -A ZINIT_ICES; local -A ICE; ICE=( "${(kv)ZINIT_ICES[@]}" ); echo "ok ${#ICE}""#,
        );
    }
}

// ───────────────────────── string ops ─────────────────────────

mod string_ops {
    use super::*;

    /// `${var//pat/repl}` global replace.
    #[test]
    fn replace_global() {
        assert_parity(r#"x=banana; echo "${x//a/A}""#);
    }

    /// `${var/pat/repl}` first replace.
    #[test]
    fn replace_first() {
        assert_parity(r#"x=banana; echo "${x/a/A}""#);
    }

    /// `${var/#pat/repl}` anchor at start.
    #[test]
    fn replace_anchored_start() {
        assert_parity(r#"x=banana; echo "${x/#ban/X}""#);
    }

    /// `${var/%pat/repl}` anchor at end.
    #[test]
    fn replace_anchored_end() {
        assert_parity(r#"x=banana; echo "${x/%na/X}""#);
    }

    /// `${var:offset}` substring.
    #[test]
    fn substring_offset_only() {
        assert_parity(r#"x=banana; echo "${x:2}""#);
    }

    /// `${var:offset:length}` substring with length.
    #[test]
    fn substring_offset_and_length() {
        assert_parity(r#"x=banana; echo "${x:2:3}""#);
    }

    /// `${var#pat}` shortest prefix strip.
    #[test]
    fn prefix_short_strip() {
        assert_parity(r#"x=foo.tar.gz; echo "${x#*.}""#);
    }

    /// `${var##pat}` longest prefix strip.
    #[test]
    fn prefix_long_strip() {
        assert_parity(r#"x=foo.tar.gz; echo "${x##*.}""#);
    }

    /// `${var%pat}` shortest suffix strip.
    #[test]
    fn suffix_short_strip() {
        assert_parity(r#"x=foo.tar.gz; echo "${x%.*}""#);
    }

    /// `${var%%pat}` longest suffix strip.
    #[test]
    fn suffix_long_strip() {
        assert_parity(r#"x=foo.tar.gz; echo "${x%%.*}""#);
    }
}
