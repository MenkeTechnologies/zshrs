//! **Expected to fail** against reference `zsh -fc` on substantive behavior:
//! each test compares **exit code and stdout** between `zshrs --zsh -fc` and
//! `zsh -fc`. **Stderr is ignored** for pass/fail so diagnostics that differ only
//! in shell name / path / minor wording still count as matched.
//!
//! When `zsh` is missing, tests return early.

use std::path::PathBuf;
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
    stderr: String,
    exit: i32,
}

fn run_zsh(script: &str) -> ShellResult {
    let out = Command::new(zsh_path())
        .args(["-fc", script])
        .output()
        .expect("invoke zsh");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

fn run_zshrs(script: &str) -> ShellResult {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-fc", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

/// **Exit code + stdout** only; stderr is context in the panic message.
fn assert_parity(script: &str, label: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(script);
    let r = run_zshrs(script);
    if z.stdout != r.stdout || z.exit != r.exit {
        panic!(
            "parity gap (exit + stdout): {label}\nscript:\n{script}\n\
--- zsh stdout ---\n{:?}\n--- zshrs stdout ---\n{:?}\n\
--- zsh stderr (context) ---\n{:?}\n--- zshrs stderr (context) ---\n{:?}\n\
--- exit zsh={} zshrs={}",
            z.stdout,
            r.stdout,
            z.stderr,
            r.stderr,
            z.exit,
            r.exit
        );
    }
}

/// Expands to one `#[test] fn` per `name => (label, script)` row.
macro_rules! parity_gap_tests {
    ($($name:ident => ($label:literal, $script:expr);)+) => {
        $(
            #[test]
            fn $name() {
                assert_parity($script, $label);
            }
        )+
    };
}

mod context_and_state {
    use super::*;

    parity_gap_tests! {
        zsh_eval_context_matches_reference => ("ZSH_EVAL_CONTEXT", r#"echo $ZSH_EVAL_CONTEXT"#);
        plus_special_assoc_table_flags_match => ("$+commands … $+zsh_scheduled_events", r#"print $+commands $+functions $+aliases $+history $+terminfo $+parameters $+options $+builtins $+galiases $+dis_aliases $+dis_builtins $+usergroups $+widgets $+dis_functions $+dirstack $+functrace $+module_path $+patchars $+ZPFX $+pipestatus $+zsh_scheduled_events"#);
        builtins_table_element_count => ("${#builtins}", r#"print ${#builtins}"#);
        zsh_execution_string_set_under_dash_c => ("ZSH_EXECUTION_STRING", r#"print -r "$ZSH_EXECUTION_STRING""#);
    }
}

mod special_parameters {
    use super::*;

    parity_gap_tests! {
        path_glob_flag_t_reports_tied_special => ("${(t)path}", r#"print ${(t)path}"#);
        fpath_glob_flag_t_reports_tied_special => ("${(t)fpath}", r#"print ${(t)fpath}"#);
        ifs_glob_flag_t_reports_scalar_special => ("${(t)IFS}", r#"print ${(t)IFS}"#);
        histchars_non_empty_like_zsh => ("HISTCHARS", r#"print -r "$HISTCHARS""#);
        module_path_element_count => ("${#module_path}", r#"print ${#module_path}"#);
        argv0_is_shell_binary_path => ("$0", r#"print -r "$0""#);
        errno_scalar_after_startup => ("ERRNO", r#"print -r "$ERRNO""#);
        host_parameter_type_and_plus_line => ("HOST (t)+", r#"print -r "t=${(t)HOST} plus=$+HOST""#);
        dirstacksize_parameter_type_and_plus_line => ("DIRSTACKSIZE (t)+", r#"print -r "t=${(t)DIRSTACKSIZE} plus=$+DIRSTACKSIZE""#);
        plus_usergroups => ("$+usergroups", r#"print $+usergroups"#);
        plus_mailpath => ("$+mailpath", r#"print $+mailpath"#);
        plus_watch => ("$+WATCH", r#"print $+WATCH"#);
        plus_psvar => ("$+psvar", r#"print $+psvar"#);
        plus_patchars_assoc_flag => ("$+patchars", r#"print $+patchars"#);
        historywords_parameter_metadata_line => ("historywords (t)+", r#"print -r "t=${(t)historywords} plus=$+historywords""#);
        plus_zsh_scheduled_events_flag => ("$+zsh_scheduled_events", r#"print $+zsh_scheduled_events"#);
        listmax_scalar => ("LISTMAX", r#"print $LISTMAX"#);
        prompt3_parameter_metadata_line => ("PROMPT3 (t)+", r#"print -r "t=${(t)PROMPT3} plus=$+PROMPT3""#);
        zsh_argzero_parameter_metadata_line => ("ZSH_ARGZERO (t)+", r#"print -r "t=${(t)ZSH_ARGZERO} plus=$+ZSH_ARGZERO""#);
        fignore_glob_flag_t => ("${(t)FIGNORE}", r#"print ${(t)FIGNORE}"#);
        cdpath_glob_flag_t => ("${(t)cdpath}", r#"print ${(t)cdpath}"#);
        manpath_glob_flag_t => ("${(t)manpath}", r#"print ${(t)manpath}"#);
        term_glob_flag_t => ("${(t)TERM}", r#"print ${(t)TERM}"#);
        prompt2_parameter_metadata_line => ("PROMPT2 (t)+", r#"print -r "t=${(t)PROMPT2} plus=$+PROMPT2""#);
    }
}

mod typeset_and_dump {
    use super::*;

    parity_gap_tests! {
        typeset_p_missing_precmd_functions_stderr => ("typeset -p precmd_functions", r#"typeset -p precmd_functions"#);
        typeset_p_ifs_default_quoting => ("typeset -p IFS", r#"typeset -p IFS"#);
        typeset_p_path_line => ("typeset -p path", r#"typeset -p path"#);
        typeset_p_fpath_line => ("typeset -p fpath", r#"typeset -p fpath"#);
        typeset_p1_scalar_form => ("typeset -p1 PWD", r#"typeset -p1 PWD"#);
        set_plus_o_full_dump => ("set +o", r#"set +o"#);
        export_minus_p_full_dump => ("export -p", r#"export -p"#);
    }
}

mod parse_and_options_builtin {
    use super::*;

    parity_gap_tests! {
        zparseopts_missing_default_array_stderr => ("zparseopts no default array defined", r#"zparseopts d=del -- -d foo; print -r "del=$del""#);
        unsetopt_unknown_option_stderr => ("unsetopt no such option diagnostic", r#"unsetopt badopt_name_xyz; echo after"#);
        read_qt_noninteractive_stderr => ("read -qt non-interactive", r#"read -qt 0; echo "st:$?""#);
        enable_r_unknown_builtin_stderr => ("enable -r no such hash element", r#"enable -r nonexistent_bi_zzz"#);
        setopt_numeric_token_rejected => ("setopt 999", r#"setopt 999"#);
        unsetopt_mixed_with_unknown => ("unsetopt nonomatch + unknown", r#"unsetopt nonomatch badoptx_gap_unknown; print after"#);
        zparseopts_simple_opt_arg => ("zparseopts a=aval -- -a", r#"zparseopts a=aval -- -a 2>&1; print -r "aval=$aval""#);
        zparseopts_only_double_dash => ("zparseopts --", r#"zparseopts -- 2>&1; print after"#);
    }
}

mod paths_source_dot_cd {
    use super::*;

    parity_gap_tests! {
        source_missing_file_stderr => ("source missing file", r#"source /nonexistent/path/parity_gap_file_xyz"#);
        dot_missing_file_stderr_builtin_name => (". missing file (builtin name in diagnostic)", r#". /nonexistent/dotfile_p_gap_xyz"#);
        cd_missing_dir_stderr => ("cd no such file or directory", r#"cd /nonexistent_dir_xyz_999"#);
        hash_unknown_command_stderr => ("hash no such command", r#"hash foo_nonexistent_zz"#);
        chdir_missing_dir_stderr => ("chdir missing dir", r#"chdir /nonexistent_chdir_dir_gap999"#);
        pushd_no_args_stderr => ("pushd no args", r#"pushd 2>&1; print ex:$?"#);
        popd_empty_stack_stderr => ("popd empty stack", r#"popd 2>&1; print ex:$?"#);
    }
}

mod diagnostics_and_command_wrappers {
    use super::*;

    parity_gap_tests! {
        command_not_found_stderr_prefix => ("command not found: progname prefix", r#"nonexistent_command_xyz_abc_ungrep"#);
        command_wrapper_not_found_stderr => ("command builtin not found", r#"command noexist_cmd_wrap_gap999"#);
        builtin_unknown_name_stderr => ("builtin unknown name", r#"builtin no_such_builtin_xyz_gap"#);
        colors_autoload_not_loaded_stderr => ("colors not on PATH", r#"colors"#);
    }
}

mod jobs_and_wait {
    use super::*;

    parity_gap_tests! {
        disown_invalid_job_stderr => ("disown %999", r#"disown %999 2>&1; print ex:$?"#);
        fg_no_current_job_stderr => ("fg no job", r#"fg 2>&1; print ex:$?"#);
        bg_no_current_job_stderr => ("bg no job", r#"bg 2>&1; print ex:$?"#);
        wait_n_no_children_stderr => ("wait -n", r#"wait -n 2>&1; print ex:$?"#);
    }
}

mod builtins_misc {
    use super::*;

    parity_gap_tests! {
        umask_invalid_mode_stderr => ("umask 999", r#"umask 999 2>&1; print ex:$?"#);
        autoload_capital_X_missing => ("autoload -X missing", r#"autoload -X nonexistent_autoload_fn_gap999"#);
        unfunction_missing_function => ("unfunction missing", r#"unfunction _nonexistent_fn_gap999"#);
        whence_bad_flag_Z => ("whence -Z", r#"whence -Z foo"#);
        print_bad_flag_Z => ("print -Z", r#"print -Z"#);
        getopts_empty_optstring => ("getopts empty optstring", r#"getopts "" opt -- -b 2>&1; print -r "opt=$opt""#);
        zmodload_missing_module => ("zmodload missing", r#"zmodload nosuchmodule999_gap"#);
        zformat_invalid_directive_stderr => ("zformat %s", r#"zformat -f out_gap hello %s world 2>&1; print -r "out=$out""#);
        functions_missing_name_stderr => ("functions missing name", r#"functions this_fn_does_not_exist_gap999"#);
        zstat_no_arguments_stderr => ("zstat no args", r#"zstat 2>&1; print ex:$?"#);
        readonly_reassign_fails => ("readonly reassignment", r#"readonly foo_gap_ro=1; foo_gap_ro=2 2>&1; print st:$?"#);
        typeset_r_reassign_fails => ("typeset -r reassignment", r#"typeset -r rop_gap=1; rop_gap=2 2>&1; print ex:$?"#);
        unhash_r_unknown => ("unhash -r unknown", r#"unhash -r nohash999_gap"#);
        limit_unknown_resource => ("limit unknown", r#"limit badlimit999_gap"#);
        ulimit_unknown_flag => ("ulimit -x", r#"ulimit -x"#);
        echotc_co => ("echotc co", r#"echotc co 2>&1; print ex:$?"#);
        echoti_cols => ("echoti cols", r#"echoti cols 2>&1; print ex:$?"#);
        printf_pct_n_invalid => ("printf %n", r#"printf '%n' x 2>&1; print ex:$?"#);
        funcnest_recursion_limit_scalar => ("FUNCNEST", r#"print $FUNCNEST"#);
        alias_illegal_equals_syntax => ("alias ===", r#"alias bad_alias_gap=== 2>&1"#);
        hash_m_pattern_no_matches => ("hash -m pattern", r#"hash -m nomatchpat_gap_zzz999 2>&1"#);
        compaudit_completion_audit => ("compaudit", r#"compaudit"#);
    }
}

mod expansion_eval_arithmetic {
    use super::*;

    parity_gap_tests! {
        process_substitution_word => ("<(true) word form", r#"print -r <(true)"#);
        eval_parse_error_stderr => ("eval parse error", r#"eval ')syntax_error_gap_paren' 2>&1; print ex:$?"#);
        sysparams_pid_subscript => ("sysparams[pid]", r#"print -r "pid=<${sysparams[pid]}>""#);
        arithmetic_hex_output_form => ("$(( [##16] )) output", r#"print $(( [##16] 255 ))"#);
        nomatch_when_nonomatch_unset => ("nomatch glob", r#"unsetopt nonomatch; print nonexist_glob_gap999*(.) 2>&1; print ex:$?"#);
        let_division_by_zero => ("let 1/0", r#"let x_gap=1/0 2>&1; print ex:$?"#);
        let_no_expression => ("let bare", r#"let 2>&1; print ex:$?"#);
        param_Q_flag_quoted_form => ("param (Q) quoting", r#"print -r "${(Q)HOME:-}""#);
        brace_join_flag_j_dot => ("${(j.:.) brace}", r#"print -r "${(j.:.){a,b,c}}""#);
        nested_param_subst_hash_strip => ("nested # strip", r#"print -r "${${:-foo}#f}""#);
        fc_push_pop_directory_stack => ("fc -p PWD stack", r#"fc -p $PWD; print ex:$?; fc -P"#);
    }
}

mod io_and_read {
    use super::*;

    parity_gap_tests! {
        read_k_one_byte_non_tty => ("read -k 1", r#"read -k 1 <<< x 2>&1; print ex:$?"#);
        read_k_zero_non_tty => ("read -k 0", r#"read -k 0 <<< x 2>&1; print ex:$?"#);
    }
}

mod history_and_fc {
    use super::*;

    parity_gap_tests! {
        history_zero_event => ("history 0", r#"history 0"#);
        fc_dash_e_colon_recursion_guard => ("fc -e :", r#"fc -e :"#);
    }
}

mod zle_bindkey_regex {
    use super::*;

    parity_gap_tests! {
        bindkey_list_prefixes => ("bindkey -l", r#"bindkey -l"#);
        zregexparse_no_args_stderr => ("zregexparse no args", r#"zregexparse"#);
    }
}

mod exec_path {
    use super::*;

    parity_gap_tests! {
        exec_missing_file => ("exec missing binary", r#"exec /nonexistent/exec999_gap_path"#);
    }
}

mod getopts_cli {
    use super::*;

    parity_gap_tests! {
        getopts_end_of_options_parses_dash_a => ("getopts with -- -a", r#"getopts ':a' opt -- -a 2>&1; echo "opt=$opt""#);
    }
}

mod coproc {
    use super::*;

    parity_gap_tests! {
        coproc_sets_bang_to_child_pid => ("coproc $!", r#"coproc cat; echo "coproc=$!""#);
    }
}

mod prompt_and_fc {
    use super::*;

    parity_gap_tests! {
        print_p_last_status_escape => ("print -P %?", r#"true; print -P %?"#);
        fc_recursion_error_stderr_format => ("fc recursion diagnostic", r#"fc 99999"#);
        fc_list_no_such_event_message => ("fc -l no such event", r#"fc -l"#);
    }
}

mod zle_and_modules {
    use super::*;

    parity_gap_tests! {
        vared_requires_terminal_like_zsh => ("vared -c non-tty", r#"vared -c x <<< hi 2>&1; echo "after"; echo "x=$x""#);
        zmodload_capital_f_zsh_stat_b_zstat => ("zmodload -F zsh/stat b:zstat", r#"zmodload -F zsh/stat b:zstat"#);
        zregexparse_not_enough_arguments_stderr => ("zregexparse -c too few args", r#"zregexparse -c foo bar"#);
    }
}

/// Additional scripted probes (**exit + stdout** vs reference zsh).
mod corpus_additional_probes {
    use super::*;

    parity_gap_tests! {
        plus_sprompt_assoc => ("$+SPROMPT", r#"print $+SPROMPT"#);
        plus_prompt4_assoc => ("$+PROMPT4", r#"print $+PROMPT4"#);
        plus_termcap_assoc => ("$+termcap", r#"print $+termcap"#);
        plus_zsh_eval_context_assoc => ("$+ZSH_EVAL_CONTEXT", r#"print $+ZSH_EVAL_CONTEXT"#);
        plus_funcfiletrace_assoc => ("$+funcfiletrace", r#"print $+funcfiletrace"#);
        plus_functrace_assoc => ("$+functrace", r#"print $+functrace"#);
        mailcheck_scalar => ("MAILCHECK", r#"print $MAILCHECK"#);
        watchfmt_scalar_default => ("WATCHFMT", r#"print -r "$WATCHFMT""#);
        underscore_after_simple_command => ("$_ after true", r#"true; print -r "$_""#);
        parameters_index_i_path => ("parameters[(i)PATH]", r#"print ${parameters[(i)PATH]}"#);
        emulate_sh_posixargzero_option => ("emulate sh -L posixargzero", r#"emulate sh -L; print $options[posixargzero]"#);
        builtins_keys_line_count_wc => ("${(k)builtins} | wc -c", r#"print -l ${(k)builtins} 2>&1 | wc -c"#);
        getopts_leading_plus_colon_form => ("getopts '+:a:'", r#"OPTIND=1; getopts '+:a:' o -- -a 2>&1; print -r "o=$o""#);
        typeset_plus_x_with_r => ("typeset +x -r", r#"typeset +x -r 2>&1"#);
        unsetopt_glob_pattern_nomatch => ("unsetopt '*pattern'", r#"unsetopt '*badpattern_gapxyz' 2>&1"#);
        setopt_two_unknown_names => ("setopt two unknown", r#"setopt badopt_gap_a badopt_gap_b 2>&1; print after"#);
        set_plus_o_unknown_name => ("set +o unknown", r#"set +o badopt_gap_setname 2>&1"#);
        enable_disabled_list_byte_count => ("enable -p | wc -c", r#"enable -p 2>&1 | wc -c"#);
        zmodload_capital_R_complete => ("zmodload -R", r#"zmodload -R zsh/complete 2>&1"#);
        zftp_stderr_or_exit => ("zftp", r#"zftp 2>&1"#);
        logout_builtin_stderr => ("logout", r#"logout 2>&1; print -r "ex=$?""#);
        getopts_missing_option_argument => ("getopts 'a:' without value", r#"OPTIND=1; getopts 'a:' o -- -a 2>&1; print -r "o=$o""#);
        read_t0_k1_herestring => ("read -t0 -k1", r#"read -t 0 -k 1 <<< a 2>&1; print -r "ex=$?""#);
        read_q_noninteractive_herestring => ("read -q non-interactive", r#"read -q <<< y 2>&1; print -r "ex=$?""#);
        glob_qual_stat_prefix_s0 => ("glob *(s+0)", r#"print *(s+0) 2>&1; print -r "ex=$?""#);
        glob_qual_capital_Lk0 => ("glob *(Lk+0)", r#"print *(Lk+0) 2>&1; print -r "ex=$?""#);
        comptry_builtin_stderr => ("comptry", r#"comptry 2>&1"#);
        kern_argv_at_bracket_word => ("$@[@]", r#"print $@[@]"#);
        shift_beyond_positional_count => ("shift 9 one arg", r#"set -- a; shift 9 2>&1; print -r "ex=$?""#);
        getln_console_flag => ("getln -c", r#"getln -c var_gap_ln 2>&1"#);
        param_hash_colon_grammar => ("${#:-foo}", r#"print ${#:-foo}"#);
        print_wrapped_array_word => ("print ($array)", r#"a=(x y); print ($a)"#);
        getopts_dash_only_emits_question => ("getopts bare dash", r#"OPTIND=1; getopts 'a' o -- 2>&1; print -r "o=$o arg=$OPTARG""#);
        noclobber_second_redir_stderr => ("noclobber double >", r#"setopt noclobber; rm -f /tmp/gap_clob_$$; echo x > /tmp/gap_clob_$$; echo y > /tmp/gap_clob_$$ 2>&1; print -r "ex=$?"; rm -f /tmp/gap_clob_$$"#);
        echo_arithmetic_hex_output => ("echo $(( [##16] ))", r#"echo $(([##16] 255))"#);
        time_prefix_builtin => ("time true", r#"time true 2>&1"#);
        zsocket_invocation => ("zsocket", r#"zsocket 2>&1"#);
        ztcp_invocation => ("ztcp", r#"ztcp 2>&1"#);
        unsetopt_known_plus_unknown => ("unsetopt good + bad", r#"unsetopt interactivecomments badopt_xyz_gap 2>&1; print ok"#);
        bindkey_alternate_keymap => ("bindkey -a | wc -l", r#"bindkey -a 2>&1 | wc -l"#);
        echoti_cap_co_altcase => ("echoti Co", r#"echoti Co 2>&1; print -r "ex=$?""#);
        empty_command_equals_split_expansion => ("= : split", r#"=:; print ${=:-foo bar}"#);
        dirs_file_option => ("dirs -f", r#"dirs -f 2>&1"#);
        remain_rest_zparseopts => ("zparseopts r=rest", r#"zparseopts r=rest -- -a av tail 2>&1; print -r "r=$rest""#);
        mkdir_nonexistent_path => ("mkdir deep", r#"mkdir /nonexistent/extremely/long/path/gap/mk 2>&1"#);
        shlvl_parameter_glob_t_flag => ("${(t)SHLVL}", r#"print ${(t)SHLVL}"#);
        zformat_percent_s_one_arg => ("zformat -f one %s", r#"zformat -f zff %s hi 2>&1"#);
        strftime_epoch_zero => ("strftime -s", r#"strftime -s st %Y 0; print -r "$st""#);
    }
}

/// Expansion / parameter / arithmetic quirks where **stdout or exit** differ (not stderr wording).
mod corpus_behavior_expansion {
    use super::*;

    parity_gap_tests! {
        arith_bracket_radix_16_42 => ("$(( [#16] 42 ))", r#"print $(( [#16] 42 ))"#);
        param_s_join_dot_brace => ("${(s.:.) brace}", r#"print ${(s.:.)a:b:c}"#);
        param_qqq_multiquote => ("${(qqq) } words", r#"print ${(qqq)hi there}"#);
        positional_argv_slice_subscript => ("$@[@] with set --", r#"set -- 1 2; print $@[@]"#);
        argv_zero_colon_htail => ("$0:t", r#"print $0:t"#);
        pad_left_l_colon_zeros => ("${(l:8::0:) }", r#"print ${(l:8::0:)7}"#);
        pad_right_r_colon_zeros => ("${(r:8::0:) }", r#"print ${(r:8::0:)7}"#);
        arith_ksh_nvl2 => ("NVL2 math", r#"print $(( NVL2(0,1,2) ))"#);
        param_j_join_comma_brace => ("${(j.,.) brace}", r#"print ${(j.,.){one,two,three}}"#);
        shwordsplit_ifs_colon_word => ("shwordsplit IFS :", r#"setopt shwordsplit; export IFS=:; s=a:b:c; print $s"#);
        ifs_equals_split_word_count => ("IFS equals-split argv count", r#"IFS=_; s=a_b_c; argv=( ${=s} ); print $#argv"#);
        arith_int_builtins => ("int() float", r#"print $(( int(1.9) ))"#);
    }
}

/// More parameter / arithmetic / array behavior (stdout or exit; stderr not compared).
mod corpus_behavior_expansion_b {
    use super::*;

    parity_gap_tests! {
        arith_bracket_radix_oct_64 => ("$(( [#8] 64 ))", r#"print $(( [#8] 64 ))"#);
        param_c_split_words_flag => ("${(c) } words", r#"print ${(c)hello world}"#);
        param_q_backslash_escaped_word => ("${(Q) escape}", r#"print ${(Q)one\ two}"#);
        arith_abs_builtin => ("abs()", r#"print $(( abs(-3) ))"#);
        arith_ceil_builtin => ("ceil()", r#"print $(( ceil(1.2) ))"#);
        arith_float_cast_fn => ("float()", r#"print $(( float(2) ))"#);
        arith_sign_builtin => ("sign()", r#"print $(( sign(-0.0) ))"#);
        at_nested_default_words_array => ("@ nested ${:- words}", r#"print ${(@)${:-a b c}}"#);
        at_nested_z_assign_split => ("@ z parameter assign", r#"print ${(@)${(@)z:='a b'}}"#);
        param_match_start_glob => ("(M) ## pattern", r#"x=aba; print ${(MS)x##a}"#);
        param_match_end_glob => ("(M) %% pattern", r#"x=aba; print ${(MS)x%%a}"#);
        param_mk_glob_prefix => ("(Mk) prefix", r#"print ${(Mk)a*}"#);
        seconds_float_fraction_assign => ("SECONDS=1.5", r#"SECONDS=1.5; print $SECONDS"#);
        array_caret_all_elements => ("@^ array", r#"A=(x y z); print ${(@)^A}"#);
        caret_hyphen_default_brace => ("^:- brace", r#"print ${^:-a b}"#);
        join_flag_newline_brace => ("(j.\\\\n.) brace", r#"print ${(j.\n.){x,y}}"#);
        integer_literal_with_base_hash => ("typeset -i 3#8", r#"typeset -i x=3#8; print $x"#);
        ksh_zero_subscript_first_element => ("kshzerosubscript [0]", r#"setopt kshzerosubscript; a=(q); print $a[0]"#);
        typeset_float_seconds_builtin => ("typeset -F SECONDS", r#"typeset -F SECONDS; SECONDS=1; print $SECONDS"#);
        arith_rand48 => ("rand48()", r#"print $(( rand48() ))"#);
    }
}

/// Flags, associative dumps, quoting letters, and unknown math builtins (stdout/exit only).
mod corpus_behavior_expansion_c {
    use super::*;

    parity_gap_tests! {
        typeset_r_pad_z_four => ("typeset -RZ4", r#"typeset -RZ4 n=ab; print $n"#);
        param_z_base_prefix => ("(Z) 2#…", r#"print ${(Z)2#1010}"#);
        param_oa_array_sort_brace => ("(Oa) brace list", r#"print ${(Oa){a,B,c}}"#);
        param_ok_assoc_keys_single => ("(ok) parameters[]", r#"print ${(ok)parameters[PATH]}"#);
        param_i_capital_ident_flag => ("(I) name", r#"print ${(I)ZSH_VERSION}"#);
        param_zb_base_flag => ("(Zb)", r#"print ${(Zb)foo}"#);
        options_assoc_keys_sorted => ("(k)options", r#"print ${(k)options}"#);
        param_uas_upper_segments => ("(UAs) per segment", r#"s=hi; print ${(UAs)s}"#);
        param_las_lower_segments => ("(LAs) per segment", r#"s=hi; print ${(LAs)s}"#);
        param_j_dot_join_brace => ("(j.S.) brace", r#"print ${(j.S.){a,b}}"#);
        param_q_hyphen_quoting => ("(q-)", r#"print ${(q-)hi there}"#);
        param_q_plus_quoting => ("(q+)", r#"print ${(q+)hi there}"#);
        arith_hex_fn_unknown_zsh => ("hex()", r#"print $(( hex(255) ))"#);
        arith_oct_fn_unknown_zsh => ("oct()", r#"print $(( oct(64) ))"#);
        arith_word_fn_unknown_zsh => ("word()", r#"print $(( word(3,4,5) ))"#);
        arith_sum_fn_unknown_zsh => ("sum()", r#"print $(( sum(1,2,3) ))"#);
        arith_prod_fn_unknown_zsh => ("prod()", r#"print $(( prod(2,3) ))"#);
        arith_min_fn_unknown_zsh => ("min()", r#"print $(( min(1,2) ))"#);
        arith_max_fn_unknown_zsh => ("max()", r#"print $(( max(1,2) ))"#);
        arith_radix8_print_octal_var => ("[#8] octal value", r#"o=012; print $(( [#8] o ))"#);
        param_x_trace_flag_word => ("(x) word", r#"print ${(x)gap}"#);
        arith_radix16_hash_hex_escape => ("[#16] ## \\xFF", r#"print $(( [#16] ## \xFF ))"#);
        param_y_key_index_flag => ("(y) key", r#"print ${(y)str_gap}"#);
    }
}

mod corpus_behavior_expansion_d {
    use super::*;

    parity_gap_tests! {
        param_oe_word_split_flag => ("(oe) words", r#"print ${(oe)a b}"#);
        param_in_nested_brace_union => ("(in) nested brace", r#"print ${(in){{a,B},{c,d}}}"#);
        caret_double_hyphen_default_colon => ("^^:- default", r#"print ${(@)^^:-x y}"#);
        param_z_plus_numeric => ("(Z+)", r#"print ${(Z+)2}"#);
        param_z_decimal => ("(Z) decimal", r#"print ${(Z)12}"#);
        utf_grapheme_length_hash => ("UTF $'\\\\u3042' ${#}", r#"utf=$'\u3042'; print ${#utf}"#);
        param_ww_double_word_split => ("(ww)", r#"print ${(ww)one two}"#);
        param_z_unset_parameter_name => ("(Z) unset param", r#"print ${(Z)parameter_name_gap}"#);
        equals_split_whitespace_trim => ("= split ws", r#"s='  a  b'; print ${=${s}}"#);
        z_tokenize_shellwords_count => ("(z) count", r#"v=(${(z)"a 'b' c"}); print $#v"#);
        z_tokenize_shellwords_dump => ("(z) dump", r#"print ${(z)"a 'b' c"}"#);
        param_j_tab_join_three => ("(j tab)", r#"print ${(j:\t.)a b c}"#);
        modules_assoc_kv_at => ("@kv modules", r#"print ${(@kv)modules}"#);
        dirstack_glob_t_flag => ("(t)dirstack", r#"print ${(t)dirstack}"#);
    }
}
