//! **Expected to fail** against reference `zsh -fc` on substantive behavior:
//! each test compares **exit code and stdout** between `zshrs --zsh -fc` and
//! `zsh -fc`. **Stderr is ignored** for pass/fail so diagnostics that differ only
//! in shell name / path / minor wording still count as matched.
//!
//! When `zsh` is missing, tests return early.
//!
//! **Coverage**: language surface under `zsh -fc` / `zshrs --zsh -fc` (state, options,
//! parameters, builtins, expansion, redirections, history, ZLE-adjacent, jobs, plus
//! larger scripted corpora, `corpus_dash_fc_surface_extra`, `corpus_dash_fc_compounds_misc`,
//! `corpus_dash_fc_control_flow`, `corpus_dash_fc_params_redir`, `corpus_dash_fc_bulk_a`). Pass/fail is **stdout + exit** only (see `assert_parity`).

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

/// Expands to one `#[test] fn` per `name => (label, script)` row (label + script: raw strings).
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
        zsh_eval_context_matches_reference => (r#"ZSH_EVAL_CONTEXT"#, r#"echo $ZSH_EVAL_CONTEXT"#);
        plus_special_assoc_table_flags_match => (r#"$+commands … $+zsh_scheduled_events"#, r#"print $+commands $+functions $+aliases $+history $+terminfo $+parameters $+options $+builtins $+galiases $+dis_aliases $+dis_builtins $+usergroups $+widgets $+dis_functions $+dirstack $+functrace $+module_path $+patchars $+ZPFX $+pipestatus $+zsh_scheduled_events"#);
        builtins_table_element_count => (r#"${#builtins}"#, r#"print ${#builtins}"#);
        zsh_execution_string_set_under_dash_c => (r#"ZSH_EXECUTION_STRING"#, r#"print -r "$ZSH_EXECUTION_STRING""#);
    }
}

mod special_parameters {
    use super::*;

    parity_gap_tests! {
        path_glob_flag_t_reports_tied_special => (r#"${(t)path}"#, r#"print ${(t)path}"#);
        fpath_glob_flag_t_reports_tied_special => (r#"${(t)fpath}"#, r#"print ${(t)fpath}"#);
        ifs_glob_flag_t_reports_scalar_special => (r#"${(t)IFS}"#, r#"print ${(t)IFS}"#);
        histchars_non_empty_like_zsh => (r#"HISTCHARS"#, r#"print -r "$HISTCHARS""#);
        module_path_element_count => (r#"${#module_path}"#, r#"print ${#module_path}"#);
        argv0_is_shell_binary_path => (r#"$0"#, r#"print -r "$0""#);
        errno_scalar_after_startup => (r#"ERRNO"#, r#"print -r "$ERRNO""#);
        host_parameter_type_and_plus_line => (r#"HOST (t)+"#, r#"print -r "t=${(t)HOST} plus=$+HOST""#);
        dirstacksize_parameter_type_and_plus_line => (r#"DIRSTACKSIZE (t)+"#, r#"print -r "t=${(t)DIRSTACKSIZE} plus=$+DIRSTACKSIZE""#);
        plus_usergroups => (r#"$+usergroups"#, r#"print $+usergroups"#);
        plus_mailpath => (r#"$+mailpath"#, r#"print $+mailpath"#);
        plus_watch => (r#"$+WATCH"#, r#"print $+WATCH"#);
        plus_psvar => (r#"$+psvar"#, r#"print $+psvar"#);
        plus_patchars_assoc_flag => (r#"$+patchars"#, r#"print $+patchars"#);
        historywords_parameter_metadata_line => (r#"historywords (t)+"#, r#"print -r "t=${(t)historywords} plus=$+historywords""#);
        plus_zsh_scheduled_events_flag => (r#"$+zsh_scheduled_events"#, r#"print $+zsh_scheduled_events"#);
        listmax_scalar => (r#"LISTMAX"#, r#"print $LISTMAX"#);
        prompt3_parameter_metadata_line => (r#"PROMPT3 (t)+"#, r#"print -r "t=${(t)PROMPT3} plus=$+PROMPT3""#);
        zsh_argzero_parameter_metadata_line => (r#"ZSH_ARGZERO (t)+"#, r#"print -r "t=${(t)ZSH_ARGZERO} plus=$+ZSH_ARGZERO""#);
        fignore_glob_flag_t => (r#"${(t)FIGNORE}"#, r#"print ${(t)FIGNORE}"#);
        cdpath_glob_flag_t => (r#"${(t)cdpath}"#, r#"print ${(t)cdpath}"#);
        manpath_glob_flag_t => (r#"${(t)manpath}"#, r#"print ${(t)manpath}"#);
        term_glob_flag_t => (r#"${(t)TERM}"#, r#"print ${(t)TERM}"#);
        prompt2_parameter_metadata_line => (r#"PROMPT2 (t)+"#, r#"print -r "t=${(t)PROMPT2} plus=$+PROMPT2""#);
    }
}

mod typeset_and_dump {
    use super::*;

    parity_gap_tests! {
        typeset_p_missing_precmd_functions_stderr => (r#"typeset -p precmd_functions"#, r#"typeset -p precmd_functions"#);
        typeset_p_ifs_default_quoting => (r#"typeset -p IFS"#, r#"typeset -p IFS"#);
        typeset_p_path_line => (r#"typeset -p path"#, r#"typeset -p path"#);
        typeset_p_fpath_line => (r#"typeset -p fpath"#, r#"typeset -p fpath"#);
        typeset_p1_scalar_form => (r#"typeset -p1 PWD"#, r#"typeset -p1 PWD"#);
        set_plus_o_full_dump => (r#"set +o"#, r#"set +o"#);
        export_minus_p_full_dump => (r#"export -p"#, r#"export -p"#);
    }
}

mod parse_and_options_builtin {
    use super::*;

    parity_gap_tests! {
        zparseopts_missing_default_array_stderr => (r#"zparseopts no default array defined"#, r#"zparseopts d=del -- -d foo; print -r "del=$del""#);
        unsetopt_unknown_option_stderr => (r#"unsetopt no such option diagnostic"#, r#"unsetopt badopt_name_xyz; echo after"#);
        read_qt_noninteractive_stderr => (r#"read -qt non-interactive"#, r#"read -qt 0; echo "st:$?""#);
        enable_r_unknown_builtin_stderr => (r#"enable -r no such hash element"#, r#"enable -r nonexistent_bi_zzz"#);
        setopt_numeric_token_rejected => (r#"setopt 999"#, r#"setopt 999"#);
        unsetopt_mixed_with_unknown => (r#"unsetopt nonomatch + unknown"#, r#"unsetopt nonomatch badoptx_gap_unknown; print after"#);
        zparseopts_simple_opt_arg => (r#"zparseopts a=aval -- -a"#, r#"zparseopts a=aval -- -a 2>&1; print -r "aval=$aval""#);
        zparseopts_only_double_dash => (r#"zparseopts --"#, r#"zparseopts -- 2>&1; print after"#);
    }
}

mod paths_source_dot_cd {
    use super::*;

    parity_gap_tests! {
        source_missing_file_stderr => (r#"source missing file"#, r#"source /nonexistent/path/parity_gap_file_xyz"#);
        dot_missing_file_stderr_builtin_name => (r#". missing file (builtin name in diagnostic)"#, r#". /nonexistent/dotfile_p_gap_xyz"#);
        cd_missing_dir_stderr => (r#"cd no such file or directory"#, r#"cd /nonexistent_dir_xyz_999"#);
        hash_unknown_command_stderr => (r#"hash no such command"#, r#"hash foo_nonexistent_zz"#);
        chdir_missing_dir_stderr => (r#"chdir missing dir"#, r#"chdir /nonexistent_chdir_dir_gap999"#);
        pushd_no_args_stderr => (r#"pushd no args"#, r#"pushd 2>&1; print ex:$?"#);
        popd_empty_stack_stderr => (r#"popd empty stack"#, r#"popd 2>&1; print ex:$?"#);
    }
}

mod diagnostics_and_command_wrappers {
    use super::*;

    parity_gap_tests! {
        command_not_found_stderr_prefix => (r#"command not found: progname prefix"#, r#"nonexistent_command_xyz_abc_ungrep"#);
        command_wrapper_not_found_stderr => (r#"command builtin not found"#, r#"command noexist_cmd_wrap_gap999"#);
        builtin_unknown_name_stderr => (r#"builtin unknown name"#, r#"builtin no_such_builtin_xyz_gap"#);
        colors_autoload_not_loaded_stderr => (r#"colors not on PATH"#, r#"colors"#);
    }
}

mod jobs_and_wait {
    use super::*;

    parity_gap_tests! {
        disown_invalid_job_stderr => (r#"disown %999"#, r#"disown %999 2>&1; print ex:$?"#);
        fg_no_current_job_stderr => (r#"fg no job"#, r#"fg 2>&1; print ex:$?"#);
        bg_no_current_job_stderr => (r#"bg no job"#, r#"bg 2>&1; print ex:$?"#);
        wait_n_no_children_stderr => (r#"wait -n"#, r#"wait -n 2>&1; print ex:$?"#);
    }
}

mod builtins_misc {
    use super::*;

    parity_gap_tests! {
        umask_invalid_mode_stderr => (r#"umask 999"#, r#"umask 999 2>&1; print ex:$?"#);
        autoload_capital_X_missing => (r#"autoload -X missing"#, r#"autoload -X nonexistent_autoload_fn_gap999"#);
        unfunction_missing_function => (r#"unfunction missing"#, r#"unfunction _nonexistent_fn_gap999"#);
        whence_bad_flag_Z => (r#"whence -Z"#, r#"whence -Z foo"#);
        print_bad_flag_Z => (r#"print -Z"#, r#"print -Z"#);
        getopts_empty_optstring => (r#"getopts empty optstring"#, r#"getopts "" opt -- -b 2>&1; print -r "opt=$opt""#);
        zmodload_missing_module => (r#"zmodload missing"#, r#"zmodload nosuchmodule999_gap"#);
        zformat_invalid_directive_stderr => (r#"zformat %s"#, r#"zformat -f out_gap hello %s world 2>&1; print -r "out=$out""#);
        functions_missing_name_stderr => (r#"functions missing name"#, r#"functions this_fn_does_not_exist_gap999"#);
        zstat_no_arguments_stderr => (r#"zstat no args"#, r#"zstat 2>&1; print ex:$?"#);
        readonly_reassign_fails => (r#"readonly reassignment"#, r#"readonly foo_gap_ro=1; foo_gap_ro=2 2>&1; print st:$?"#);
        typeset_r_reassign_fails => (r#"typeset -r reassignment"#, r#"typeset -r rop_gap=1; rop_gap=2 2>&1; print ex:$?"#);
        unhash_r_unknown => (r#"unhash -r unknown"#, r#"unhash -r nohash999_gap"#);
        limit_unknown_resource => (r#"limit unknown"#, r#"limit badlimit999_gap"#);
        ulimit_unknown_flag => (r#"ulimit -x"#, r#"ulimit -x"#);
        echotc_co => (r#"echotc co"#, r#"echotc co 2>&1; print ex:$?"#);
        echoti_cols => (r#"echoti cols"#, r#"echoti cols 2>&1; print ex:$?"#);
        printf_pct_n_invalid => (r#"printf %n"#, r#"printf '%n' x 2>&1; print ex:$?"#);
        funcnest_recursion_limit_scalar => (r#"FUNCNEST"#, r#"print $FUNCNEST"#);
        alias_illegal_equals_syntax => (r#"alias ==="#, r#"alias bad_alias_gap=== 2>&1"#);
        hash_m_pattern_no_matches => (r#"hash -m pattern"#, r#"hash -m nomatchpat_gap_zzz999 2>&1"#);
        compaudit_completion_audit => (r#"compaudit"#, r#"compaudit"#);
    }
}

mod expansion_eval_arithmetic {
    use super::*;

    parity_gap_tests! {
        process_substitution_word => (r#"<(true) word form"#, r#"print -r <(true)"#);
        eval_parse_error_stderr => (r#"eval parse error"#, r#"eval ')syntax_error_gap_paren' 2>&1; print ex:$?"#);
        sysparams_pid_subscript => (r#"sysparams[pid]"#, r#"print -r "pid=<${sysparams[pid]}>""#);
        arithmetic_hex_output_form => (r#"$(( [##16] )) output"#, r#"print $(( [##16] 255 ))"#);
        nomatch_when_nonomatch_unset => (r#"nomatch glob"#, r#"unsetopt nonomatch; print nonexist_glob_gap999*(.) 2>&1; print ex:$?"#);
        let_division_by_zero => (r#"let 1/0"#, r#"let x_gap=1/0 2>&1; print ex:$?"#);
        let_no_expression => (r#"let bare"#, r#"let 2>&1; print ex:$?"#);
        param_Q_flag_quoted_form => (r#"param (Q) quoting"#, r#"print -r "${(Q)HOME:-}""#);
        brace_join_flag_j_dot => (r#"${(j.:.) brace}"#, r#"print -r "${(j.:.){a,b,c}}""#);
        nested_param_subst_hash_strip => (r#"nested # strip"#, r#"print -r "${${:-foo}#f}""#);
        fc_push_pop_directory_stack => (r#"fc -p PWD stack"#, r#"fc -p $PWD; print ex:$?; fc -P"#);
    }
}

mod io_and_read {
    use super::*;

    parity_gap_tests! {
        read_k_one_byte_non_tty => (r#"read -k 1"#, r#"read -k 1 <<< x 2>&1; print ex:$?"#);
        read_k_zero_non_tty => (r#"read -k 0"#, r#"read -k 0 <<< x 2>&1; print ex:$?"#);
    }
}

mod history_and_fc {
    use super::*;

    parity_gap_tests! {
        history_zero_event => (r#"history 0"#, r#"history 0"#);
        fc_dash_e_colon_recursion_guard => (r#"fc -e :"#, r#"fc -e :"#);
    }
}

mod zle_bindkey_regex {
    use super::*;

    parity_gap_tests! {
        bindkey_list_prefixes => (r#"bindkey -l"#, r#"bindkey -l"#);
        zregexparse_no_args_stderr => (r#"zregexparse no args"#, r#"zregexparse"#);
    }
}

mod exec_path {
    use super::*;

    parity_gap_tests! {
        exec_missing_file => (r#"exec missing binary"#, r#"exec /nonexistent/exec999_gap_path"#);
    }
}

mod getopts_cli {
    use super::*;

    parity_gap_tests! {
        getopts_end_of_options_parses_dash_a => (r#"getopts with -- -a"#, r#"getopts ':a' opt -- -a 2>&1; echo "opt=$opt""#);
    }
}

mod coproc {
    use super::*;

    parity_gap_tests! {
        coproc_sets_bang_to_child_pid => (r#"coproc $!"#, r#"coproc cat; echo "coproc=$!""#);
    }
}

mod prompt_and_fc {
    use super::*;

    parity_gap_tests! {
        print_p_last_status_escape => (r#"print -P %?"#, r#"true; print -P %?"#);
        fc_recursion_error_stderr_format => (r#"fc recursion diagnostic"#, r#"fc 99999"#);
        fc_list_no_such_event_message => (r#"fc -l no such event"#, r#"fc -l"#);
    }
}

mod zle_and_modules {
    use super::*;

    parity_gap_tests! {
        vared_requires_terminal_like_zsh => (r#"vared -c non-tty"#, r#"vared -c x <<< hi 2>&1; echo "after"; echo "x=$x""#);
        zmodload_capital_f_zsh_stat_b_zstat => (r#"zmodload -F zsh/stat b:zstat"#, r#"zmodload -F zsh/stat b:zstat"#);
        zregexparse_not_enough_arguments_stderr => (r#"zregexparse -c too few args"#, r#"zregexparse -c foo bar"#);
    }
}

/// Additional scripted probes (**exit + stdout** vs reference zsh).
mod corpus_additional_probes {
    use super::*;

    parity_gap_tests! {
        plus_sprompt_assoc => (r#"$+SPROMPT"#, r#"print $+SPROMPT"#);
        plus_prompt4_assoc => (r#"$+PROMPT4"#, r#"print $+PROMPT4"#);
        plus_termcap_assoc => (r#"$+termcap"#, r#"print $+termcap"#);
        plus_zsh_eval_context_assoc => (r#"$+ZSH_EVAL_CONTEXT"#, r#"print $+ZSH_EVAL_CONTEXT"#);
        plus_funcfiletrace_assoc => (r#"$+funcfiletrace"#, r#"print $+funcfiletrace"#);
        plus_functrace_assoc => (r#"$+functrace"#, r#"print $+functrace"#);
        mailcheck_scalar => (r#"MAILCHECK"#, r#"print $MAILCHECK"#);
        watchfmt_scalar_default => (r#"WATCHFMT"#, r#"print -r "$WATCHFMT""#);
        underscore_after_simple_command => (r#"$_ after true"#, r#"true; print -r "$_""#);
        parameters_index_i_path => (r#"parameters[(i)PATH]"#, r#"print ${parameters[(i)PATH]}"#);
        emulate_sh_posixargzero_option => (r#"emulate sh -L posixargzero"#, r#"emulate sh -L; print $options[posixargzero]"#);
        builtins_keys_line_count_wc => (r#"${(k)builtins} | wc -c"#, r#"print -l ${(k)builtins} 2>&1 | wc -c"#);
        getopts_leading_plus_colon_form => (r#"getopts '+:a:'"#, r#"OPTIND=1; getopts '+:a:' o -- -a 2>&1; print -r "o=$o""#);
        typeset_plus_x_with_r => (r#"typeset +x -r"#, r#"typeset +x -r 2>&1"#);
        unsetopt_glob_pattern_nomatch => (r#"unsetopt '*pattern'"#, r#"unsetopt '*badpattern_gapxyz' 2>&1"#);
        setopt_two_unknown_names => (r#"setopt two unknown"#, r#"setopt badopt_gap_a badopt_gap_b 2>&1; print after"#);
        set_plus_o_unknown_name => (r#"set +o unknown"#, r#"set +o badopt_gap_setname 2>&1"#);
        enable_disabled_list_byte_count => (r#"enable -p | wc -c"#, r#"enable -p 2>&1 | wc -c"#);
        zmodload_capital_R_complete => (r#"zmodload -R"#, r#"zmodload -R zsh/complete 2>&1"#);
        zftp_stderr_or_exit => (r#"zftp"#, r#"zftp 2>&1"#);
        logout_builtin_stderr => (r#"logout"#, r#"logout 2>&1; print -r "ex=$?""#);
        getopts_missing_option_argument => (r#"getopts 'a:' without value"#, r#"OPTIND=1; getopts 'a:' o -- -a 2>&1; print -r "o=$o""#);
        read_t0_k1_herestring => (r#"read -t0 -k1"#, r#"read -t 0 -k 1 <<< a 2>&1; print -r "ex=$?""#);
        read_q_noninteractive_herestring => (r#"read -q non-interactive"#, r#"read -q <<< y 2>&1; print -r "ex=$?""#);
        glob_qual_stat_prefix_s0 => (r#"glob *(s+0)"#, r#"print *(s+0) 2>&1; print -r "ex=$?""#);
        glob_qual_capital_Lk0 => (r#"glob *(Lk+0)"#, r#"print *(Lk+0) 2>&1; print -r "ex=$?""#);
        comptry_builtin_stderr => (r#"comptry"#, r#"comptry 2>&1"#);
        kern_argv_at_bracket_word => (r#"$@[@]"#, r#"print $@[@]"#);
        shift_beyond_positional_count => (r#"shift 9 one arg"#, r#"set -- a; shift 9 2>&1; print -r "ex=$?""#);
        getln_console_flag => (r#"getln -c"#, r#"getln -c var_gap_ln 2>&1"#);
        param_hash_colon_grammar => (r#"${#:-foo}"#, r#"print ${#:-foo}"#);
        print_wrapped_array_word => (r#"print ($array)"#, r#"a=(x y); print ($a)"#);
        getopts_dash_only_emits_question => (r#"getopts bare dash"#, r#"OPTIND=1; getopts 'a' o -- 2>&1; print -r "o=$o arg=$OPTARG""#);
        noclobber_second_redir_stderr => (r#"noclobber double >"#, r#"setopt noclobber; rm -f /tmp/gap_clob_$$; echo x > /tmp/gap_clob_$$; echo y > /tmp/gap_clob_$$ 2>&1; print -r "ex=$?"; rm -f /tmp/gap_clob_$$"#);
        echo_arithmetic_hex_output => (r#"echo $(( [##16] ))"#, r#"echo $(([##16] 255))"#);
        time_prefix_builtin => (r#"time true"#, r#"time true 2>&1"#);
        zsocket_invocation => (r#"zsocket"#, r#"zsocket 2>&1"#);
        ztcp_invocation => (r#"ztcp"#, r#"ztcp 2>&1"#);
        unsetopt_known_plus_unknown => (r#"unsetopt good + bad"#, r#"unsetopt interactivecomments badopt_xyz_gap 2>&1; print ok"#);
        bindkey_alternate_keymap => (r#"bindkey -a | wc -l"#, r#"bindkey -a 2>&1 | wc -l"#);
        echoti_cap_co_altcase => (r#"echoti Co"#, r#"echoti Co 2>&1; print -r "ex=$?""#);
        empty_command_equals_split_expansion => (r#"= : split"#, r#"=:; print ${=:-foo bar}"#);
        dirs_file_option => (r#"dirs -f"#, r#"dirs -f 2>&1"#);
        remain_rest_zparseopts => (r#"zparseopts r=rest"#, r#"zparseopts r=rest -- -a av tail 2>&1; print -r "r=$rest""#);
        mkdir_nonexistent_path => (r#"mkdir deep"#, r#"mkdir /nonexistent/extremely/long/path/gap/mk 2>&1"#);
        shlvl_parameter_glob_t_flag => (r#"${(t)SHLVL}"#, r#"print ${(t)SHLVL}"#);
        zformat_percent_s_one_arg => (r#"zformat -f one %s"#, r#"zformat -f zff %s hi 2>&1"#);
        strftime_epoch_zero => (r#"strftime -s"#, r#"strftime -s st %Y 0; print -r "$st""#);
    }
}

/// Expansion / parameter / arithmetic quirks where **stdout or exit** differ (not stderr wording).
mod corpus_behavior_expansion {
    use super::*;

    parity_gap_tests! {
        arith_bracket_radix_16_42 => (r#"$(( [#16] 42 ))"#, r#"print $(( [#16] 42 ))"#);
        param_s_join_dot_brace => (r#"${(s.:.) brace}"#, r#"print ${(s.:.)a:b:c}"#);
        param_qqq_multiquote => (r#"${(qqq) } words"#, r#"print ${(qqq)hi there}"#);
        positional_argv_slice_subscript => (r#"$@[@] with set --"#, r#"set -- 1 2; print $@[@]"#);
        argv_zero_colon_htail => (r#"$0:t"#, r#"print $0:t"#);
        pad_left_l_colon_zeros => (r#"${(l:8::0:) }"#, r#"print ${(l:8::0:)7}"#);
        pad_right_r_colon_zeros => (r#"${(r:8::0:) }"#, r#"print ${(r:8::0:)7}"#);
        arith_ksh_nvl2 => (r#"NVL2 math"#, r#"print $(( NVL2(0,1,2) ))"#);
        param_j_join_comma_brace => (r#"${(j.,.) brace}"#, r#"print ${(j.,.){one,two,three}}"#);
        shwordsplit_ifs_colon_word => (r#"shwordsplit IFS :"#, r#"setopt shwordsplit; export IFS=:; s=a:b:c; print $s"#);
        ifs_equals_split_word_count => (r#"IFS equals-split argv count"#, r#"IFS=_; s=a_b_c; argv=( ${=s} ); print $#argv"#);
        arith_int_builtins => (r#"int() float"#, r#"print $(( int(1.9) ))"#);
    }
}

/// More parameter / arithmetic / array behavior (stdout or exit; stderr not compared).
mod corpus_behavior_expansion_b {
    use super::*;

    parity_gap_tests! {
        arith_bracket_radix_oct_64 => (r#"$(( [#8] 64 ))"#, r#"print $(( [#8] 64 ))"#);
        param_c_split_words_flag => (r#"${(c) } words"#, r#"print ${(c)hello world}"#);
        param_q_backslash_escaped_word => (r#"${(Q) escape}"#, r#"print ${(Q)one\ two}"#);
        arith_abs_builtin => (r#"abs()"#, r#"print $(( abs(-3) ))"#);
        arith_ceil_builtin => (r#"ceil()"#, r#"print $(( ceil(1.2) ))"#);
        arith_float_cast_fn => (r#"float()"#, r#"print $(( float(2) ))"#);
        arith_sign_builtin => (r#"sign()"#, r#"print $(( sign(-0.0) ))"#);
        at_nested_default_words_array => (r#"@ nested ${:- words}"#, r#"print ${(@)${:-a b c}}"#);
        at_nested_z_assign_split => (r#"@ z parameter assign"#, r#"print ${(@)${(@)z:='a b'}}"#);
        param_match_start_glob => (r#"(M) ## pattern"#, r#"x=aba; print ${(MS)x##a}"#);
        param_match_end_glob => (r#"(M) %% pattern"#, r#"x=aba; print ${(MS)x%%a}"#);
        param_mk_glob_prefix => (r#"(Mk) prefix"#, r#"print ${(Mk)a*}"#);
        seconds_float_fraction_assign => (r#"SECONDS=1.5"#, r#"SECONDS=1.5; print $SECONDS"#);
        array_caret_all_elements => (r#"@^ array"#, r#"A=(x y z); print ${(@)^A}"#);
        caret_hyphen_default_brace => (r#"^:- brace"#, r#"print ${^:-a b}"#);
        join_flag_newline_brace => (r#"(j.\n.) brace"#, r#"print ${(j.\n.){x,y}}"#);
        integer_literal_with_base_hash => (r#"typeset -i 3#8"#, r#"typeset -i x=3#8; print $x"#);
        ksh_zero_subscript_first_element => (r#"kshzerosubscript [0]"#, r#"setopt kshzerosubscript; a=(q); print $a[0]"#);
        typeset_float_seconds_builtin => (r#"typeset -F SECONDS"#, r#"typeset -F SECONDS; SECONDS=1; print $SECONDS"#);
        arith_rand48 => (r#"rand48()"#, r#"print $(( rand48() ))"#);
    }
}

/// Flags, associative dumps, quoting letters, and unknown math builtins (stdout/exit only).
mod corpus_behavior_expansion_c {
    use super::*;

    parity_gap_tests! {
        typeset_r_pad_z_four => (r#"typeset -RZ4"#, r#"typeset -RZ4 n=ab; print $n"#);
        param_z_base_prefix => (r#"(Z) 2#…"#, r#"print ${(Z)2#1010}"#);
        param_oa_array_sort_brace => (r#"(Oa) brace list"#, r#"print ${(Oa){a,B,c}}"#);
        param_ok_assoc_keys_single => (r#"(ok) parameters[]"#, r#"print ${(ok)parameters[PATH]}"#);
        param_i_capital_ident_flag => (r#"(I) name"#, r#"print ${(I)ZSH_VERSION}"#);
        param_zb_base_flag => (r#"(Zb)"#, r#"print ${(Zb)foo}"#);
        options_assoc_keys_sorted => (r#"(k)options"#, r#"print ${(k)options}"#);
        param_uas_upper_segments => (r#"(UAs) per segment"#, r#"s=hi; print ${(UAs)s}"#);
        param_las_lower_segments => (r#"(LAs) per segment"#, r#"s=hi; print ${(LAs)s}"#);
        param_j_dot_join_brace => (r#"(j.S.) brace"#, r#"print ${(j.S.){a,b}}"#);
        param_q_hyphen_quoting => (r#"(q-)"#, r#"print ${(q-)hi there}"#);
        param_q_plus_quoting => (r#"(q+)"#, r#"print ${(q+)hi there}"#);
        arith_hex_fn_unknown_zsh => (r#"hex()"#, r#"print $(( hex(255) ))"#);
        arith_oct_fn_unknown_zsh => (r#"oct()"#, r#"print $(( oct(64) ))"#);
        arith_word_fn_unknown_zsh => (r#"word()"#, r#"print $(( word(3,4,5) ))"#);
        arith_sum_fn_unknown_zsh => (r#"sum()"#, r#"print $(( sum(1,2,3) ))"#);
        arith_prod_fn_unknown_zsh => (r#"prod()"#, r#"print $(( prod(2,3) ))"#);
        arith_min_fn_unknown_zsh => (r#"min()"#, r#"print $(( min(1,2) ))"#);
        arith_max_fn_unknown_zsh => (r#"max()"#, r#"print $(( max(1,2) ))"#);
        arith_radix8_print_octal_var => (r#"[#8] octal value"#, r#"o=012; print $(( [#8] o ))"#);
        param_x_trace_flag_word => (r#"(x) word"#, r#"print ${(x)gap}"#);
        arith_radix16_hash_hex_escape => (r#"[#16] ## \xFF"#, r#"print $(( [#16] ## \xFF ))"#);
        param_y_key_index_flag => (r#"(y) key"#, r#"print ${(y)str_gap}"#);
    }
}

mod corpus_behavior_expansion_d {
    use super::*;

    parity_gap_tests! {
        param_oe_word_split_flag => (r#"(oe) words"#, r#"print ${(oe)a b}"#);
        param_in_nested_brace_union => (r#"(in) nested brace"#, r#"print ${(in){{a,B},{c,d}}}"#);
        caret_double_hyphen_default_colon => (r#"^^:- default"#, r#"print ${(@)^^:-x y}"#);
        param_z_plus_numeric => (r#"(Z+)"#, r#"print ${(Z+)2}"#);
        param_z_decimal => (r#"(Z) decimal"#, r#"print ${(Z)12}"#);
        utf_grapheme_length_hash => (r#"UTF $'\u3042' ${#}"#, r#"utf=$'\u3042'; print ${#utf}"#);
        param_ww_double_word_split => (r#"(ww)"#, r#"print ${(ww)one two}"#);
        param_z_unset_parameter_name => (r#"(Z) unset param"#, r#"print ${(Z)parameter_name_gap}"#);
        equals_split_whitespace_trim => (r#"= split ws"#, r#"s='  a  b'; print ${=${s}}"#);
        z_tokenize_shellwords_count => (r#"(z) count"#, r#"v=(${(z)"a 'b' c"}); print $#v"#);
        z_tokenize_shellwords_dump => (r#"(z) dump"#, r#"print ${(z)"a 'b' c"}"#);
        param_j_tab_join_three => (r#"(j tab)"#, r#"print ${(j:\t.)a b c}"#);
        modules_assoc_kv_at => (r#"@kv modules"#, r#"print ${(@kv)modules}"#);
        dirstack_glob_t_flag => (r#"(t)dirstack"#, r#"print ${(t)dirstack}"#);
    }
}

/// More **`zsh -fc` language surface**: strict options, `typeset` listing, `$TRY_BLOCK_ERROR`,
/// `local`, `=`-word splitting, parameter metadata tables, and fd quirks.
mod corpus_dash_fc_language_surface {
    use super::*;

    parity_gap_tests! {
        errreturn_aborts_after_false => (r#"ERR_RETURN"#, r#"setopt errreturn; false; print after_er"#);
        try_block_error_scalar => (r#"TRY_BLOCK_ERROR"#, r#"print $TRY_BLOCK_ERROR"#);
        param_flag_dot_caret_parse => (r#"${(.)^RANDOM}"#, r#"print ${(.)^RANDOM}"#);
        close_fd_stdout_print => (r#"print >&-"#, r#"print >&-"#);
        print_to_explicit_fd_one => (r#"print -u1"#, r#"print -u1 direct"#);
        typeset_plus_m_name_list => (r#"typeset +m"#, r#"typeset +m; print after"#);
        typeset_plus_list_all => (r#"typeset +"#, r#"typeset +; print after_typeset_plus"#);
        local_decl_top_level_visibility => (r#"local at top-level -fc"#, r#"local x=1 2>/dev/null; print defined:$+x"#);
        dot_slash_argv_zero_tail => (r#"./$0:t"#, r#"print ./$0:t"#);
        equals_form_splits_parameter_value => (r#"$=PWD"#, r#"print $=PWD"#);
        array_subscript_capital_i_on_empty => (r#"empty $a[(I)2]"#, r#"a=(); : ${a[(I)2]}; print tail"#);
        reswords_hash_for => (r#"$reswords[for]"#, r#"print $reswords[for]"#);
        parameters_hash_path => (r#"$parameters[PATH]"#, r#"print $parameters[PATH]"#);
        cd_root_print_pwdtail => (r#"cd / $PWD:t"#, r#"cd /; print $PWD:t"#);
        histchars_percent_prompt_expand => (r#"(%) HISTCHARS"#, r#"print ${(%)histchars}"#);
        ksharrays_zero_based_index => (r#"ksharrays $a[0]"#, r#"setopt ksharrays; a=(z); print $a[0]"#);
    }
}

/// Pipelines, **`$options`**, `typeset` shapes, deprecated `**$[ ]**` math, and top-level `break`/`return`.
mod corpus_dash_fc_surface_extra {
    use super::*;

    parity_gap_tests! {
        pipestatus_after_false_true_pipeline => (r#"pipestatus false|true"#, r#"false | true; print -r "$pipestatus""#);
        exit_after_pipefail_mixed_pipeline => (r#"pipefail exit false|true"#, r#"setopt pipefail; false | true; print $?"#);
        deprecated_bracket_arith_scalar => (r#"$[x+y]"#, r#"x=1; y=2; print $[x+y]"#);
        arithmetic_char_code_hash_scalar => (r#"$(( #a )) char"#, r#"a=A; print $(( #a ))"#);
        array_subscript_negative_one => (r#"$a[-1]"#, r#"a=(p q r); print $a[-1]"#);
        argv_slice_range_after_set => (r#"$@[2,-1]"#, r#"set -- a b c; print $@[2,-1]"#);
        argv_last_index_bracket => (r#"$@[-1]"#, r#"set -- u v w; print $@[-1]"#);
        option_nomatch_print => (r#"options[nomatch]"#, r#"print $options[nomatch]"#);
        option_ksharrays_print => (r#"options[ksharrays]"#, r#"print $options[ksharrays]"#);
        option_promptsubst_print => (r#"options[promptsubst]"#, r#"print $options[promptsubst]"#);
        option_transientrprompt_print => (r#"options[transientrprompt]"#, r#"print $options[transientrprompt]"#);
        typeset_pad_left_l5 => (r#"typeset -L5"#, r#"typeset -L5 x=abcdef; print $x"#);
        typeset_pad_right_r5 => (r#"typeset -R5"#, r#"typeset -R5 x=ab; print "'$x'""#);
        typeset_assoc_curly_key_access => (r#"typeset -A $…[]"#, r#"typeset -A h=(k1 v1); print $h[k1]"#);
        unique_array_flag_u_modifier => (r#"${(u)a}"#, r#"a=(z y z x); print ${(u)a}"#);
        param_w_word_count_scalar => (r#"${(w)a}"#, r#"a='one two three'; print ${(w)a}"#);
        colon_noop_then_print => (r#": ; print"#, r#":; print after_colon"#);
        printf_one_line => (r#"printf line"#, r#"printf '%s\n' gap_pf"#);
        logcheck_scalar => (r#"LOGCHECK"#, r#"print $LOGCHECK"#);
        break_outside_loop => (r#"break top-level"#, r#"break 2>&1; print -r "ex=$?""#);
        continue_outside_loop => (r#"continue top-level"#, r#"continue 2>&1; print -r "ex=$?""#);
        return_outside_function => (r#"return top-level"#, r#"return 1 2>&1; print -r "ex=$?""#);
        zparseopts_capital_d_split => (r#"zparseopts D=del"#, r#"zparseopts D=del -- -d one two 2>&1; print -r "del=$del""#);
        print_minus_l_multiline => (r#"print -l 3 words"#, r#"print -l one two three"#);
        limit_stack_query => (r#"limit stack"#, r#"limit stack 2>&1; print -r "ex=$?""#);
        sched_list_builtin => (r#"sched"#, r#"sched 2>&1; print -r "ex=$?""#);
        ttyctl_builtin => (r#"ttyctl"#, r#"ttyctl 2>&1; print -r "ex=$?""#);
        logger_builtin_one_arg => (r#"logger"#, r#"logger gap_parity_logger_msg 2>&1; print -r "ex=$?""#);
    }
}

/// Compound commands (`case`, `[[ ]]`, loops), parameter surgery, **`hash` / `cd`**, `emulate`, anon `() { }`.
mod corpus_dash_fc_compounds_misc {
    use super::*;

    parity_gap_tests! {
        case_keyword_matching_branch => (r#"case … esac"#, r#"case z_gap in z_gap) print case_ok;; *) print case_bad;; esac"#);
        cond_double_bracket_string_eq => (r#"[[ str = str ]]"#, r#"[[ gap_x = gap_x ]]; print $?"#);
        cond_double_bracket_glob_match => (r#"[[ = glob ]]"#, r#"[[ gap_name.txt == *.txt ]]; print $?"#);
        for_loop_arithmetic_three => (r#"for (( )) 1..3"#, r#"for (( j=1; j<=3; j++ )); print $j"#);
        repeat_two_body_print => (r#"repeat 2"#, r#"repeat 2 print rep_gap"#);
        until_loop_true_first => (r#"until true"#, r#"until true; do print never_gap; done; print after_until"#);
        brace_expand_sequence_one_three => (r#"{1..3}"#, r#"print {1..3}"#);
        brace_expand_zero_padded => (r#"{01..03}"#, r#"print {01..03}"#);
        param_substitute_slash_once => (r#"${s/a/b} once"#, r#"s=foo/bar/baz; print ${s/foo/qua}"#);
        param_substitute_slash_all => (r#"${s//x/y} all"#, r#"s=x:x:y; print ${s//:/-}"#);
        cd_dash_oldpwd_tail => (r#"cd - OLDPWD :t"#, r#"cd /; cd /tmp 2>/dev/null; cd - >/dev/null 2>&1; print -r "$OLDPWD:t""#);
        hash_r_reset_table => (r#"hash -r"#, r#"hash -r; print after_hash_r"#);
        command_capital_v_builtin_word => (r#"command -V whence"#, r#"command -V whence 2>&1; print -r "ex=$?""#);
        timefmt_default_scalar => (r#"TIMEFMT"#, r#"print -r "$TIMEFMT""#);
        typeset_float_capital_F_two_places => (r#"typeset -F 2 pi"#, r#"typeset -F 2 pi_gap=3.14159; print $pi_gap"#);
        typeset_zero_fill_Z3 => (r#"typeset -Z 3"#, r#"typeset -Z 3 n_gap=7; print $n_gap"#);
        emulate_capital_R_reset => (r#"emulate -R zsh"#, r#"emulate -R zsh; print emulate_R_ok"#);
        option_extendedglob_enabled_print => (r#"options[extendedglob]"#, r#"print $options[extendedglob]"#);
        option_multios_print => (r#"options[multios]"#, r#"print $options[multios]"#);
        option_flowcontrol_print => (r#"options[flowcontrol]"#, r#"print $options[flowcontrol]"#);
        print_capital_P_cond_yes_after_true => (r#"print -P %(.y.n) true"#, r#"true; print -P "%(?.yes.no)""#);
        print_capital_P_cond_no_after_false => (r#"print -P %(.y.n) false"#, r#"false; print -P "%(?.yes.no)""#);
        zmodload_exists_module_complete => (r#"zmodload -e zsh/complete"#, r#"zmodload -e zsh/complete 2>&1; print -r "ex=$?""#);
        dirstack_count_initial => (r#"$#dirstack"#, r#"print $#dirstack"#);
        test_builtin_int_eq => (r#"test -eq"#, r#"test 2 -eq 2; print $?"#);
        let_multiple_assign_print_sum => (r#"let a=1 b=2"#, r#"let 'la_gap=1' 'lb_gap=2'; print $(( la_gap + lb_gap ))"#);
        anonymous_function_runs_body => (r#"() { } anon"#, r#"() { print anon_gap; }"#);
        anonymous_function_local_scalar => (r#"local inside ()"#, r#"() { local z_gap_ln=1; print $z_gap_ln; }"#);
        array_append_plus_equals => (r#"a+=( )"#, r#"a_gap=(first); a_gap+=second; print ${a_gap[2]}"#);
        noglob_then_literal_globword => (r#"noglob *.z"#, r#"noglob print *.zsh_no_expand_gap_xyz 2>&1; print -r "ex=$?""#);
        brace_concat_two_segments => (r#"x{y,z} concat"#, r#"print pre_{u,v}_suf"#);
    }
}

/// `if` / `elif`, `while` / `for`, `[[ -n/-z ]]`, **`always`**, nested functions, groups, subshell scope, `(( ))`.
mod corpus_dash_fc_control_flow {
    use super::*;

    parity_gap_tests! {
        if_elif_else_chain => (r#"if / elif / else"#, r#"if false; then print gap_a; elif false; then print gap_b; else print gap_c; fi"#);
        while_loop_two_iters => (r#"while (( ))"#, r#"idx_gap=0; while (( idx_gap < 2 )); do print "wloop$idx_gap"; (( idx_gap++ )); done"#);
        for_loop_word_list => (r#"for w in …"#, r#"for w_gap in aa bb; do print $w_gap; done"#);
        cond_double_bracket_n_empty => (r#"[[ -n '' ]]"#, r#"[[ -n '' ]]; print $?"#);
        cond_double_bracket_z_empty => (r#"[[ -z '' ]]"#, r#"[[ -z '' ]]; print $?"#);
        cond_double_bracket_file_exists_root => (r#"[[ -e / ]]"#, r#"[[ -e / ]]; print $?"#);
        short_circuit_and_skips_second => (r#"false && …"#, r#"false && print gap_and_skip; print after_and"#);
        short_circuit_or_skips_second => (r#"true || …"#, r#"true || print gap_or_skip; print after_or"#);
        grouped_list_braces_two_prints => (r#"{ …; } group"#, r#"{ print gap_g1; print gap_g2; }"#);
        subshell_assignment_not_outer => (r#"(x=) subshell"#, r#"( inner_assign_gap=9 ); print $+inner_assign_gap"#);
        arith_double_paren_assign => (r#"((var = …))"#, r#"(( sum_gap = 4 + 9 )); print $sum_gap"#);
        arith_double_paren_condition_true => (r#"((1<2)); $?"#, r#"(( 1 < 2 )); print $?"#);
        always_block_after_brace => (r#"{ } always { }"#, r#"true; { print gap_try_enter; } always { print gap_always_run; }; print gap_after_try"#);
        nested_named_functions => (r#"outer inner ()"#, r#"outer_gap() { inner_gap() { print gap_nest; }; inner_gap; }; outer_gap"#);
        zmodload_short_list_loaded => (r#"zmodload -s"#, r#"zmodload -s 2>&1; print -r "ex=$?""#);
        export_minus_p_one_name => (r#"export -p PATH"#, r#"export -p PATH 2>&1; print -r "ex=$?""#);
        unset_removes_parameter_flag => (r#"unset + print \$+"#, r#"unset unset_gap_x; print $+unset_gap_x"#);
        named_fn_local_scalar => (r#"fn () { local }"#, r#"fn_gap_loc() { local lgv=1; print $lgv; }; fn_gap_loc"#);
        option_shglob_print => (r#"options[shglob]"#, r#"print $options[shglob]"#);
        option_globassign_print => (r#"options[globassign]"#, r#"print $options[globassign]"#);
        option_hist_subst_print => (r#"options[histsubstpattern]"#, r#"print $options[histsubstpattern]"#);
        option_chaselinks_print => (r#"options[chaselinks]"#, r#"print $options[chaselinks]"#);
        tty_device_param_or_empty => (r#"$TTY"#, r#"print -r "${TTY:-empty_tty}""#);
        times_builtin_summary => (r#"times"#, r#"times 2>&1; print -r "ex=$?""#);
        setopt_no_err_exit => (r#"set +e"#, r#"set +e; print after_set_plus_e"#);
        precmd_functions_array_count => (r#"$#precmd_functions"#, r#"print $#precmd_functions"#);
        chpwd_functions_array_count => (r#"$#chpwd_functions"#, r#"print $#chpwd_functions"#);
    }
}

/// Parameter expansion (indirect, `:-` / `:+`, `(C)`), **`typeset -l`/`-u`**, **`$status`**, **`multios`**, **`read -A`**, **`zstyle` / `zle`** listings.
mod corpus_dash_fc_params_redir {
    use super::*;

    parity_gap_tests! {
        param_indirect_P_name => (r#"${(P)name}"#, r#"n_gap_ind=VARPX; VARPX=indirect_val; print ${(P)n_gap_ind}"#);
        param_default_colon_minus => (r#"${unset:-…}"#, r#"unset gap_undef_d; print ${gap_undef_d:-fallback_d}"#);
        param_alternate_colon_plus_set => (r#"${set:+…}"#, r#"gap_set_p=1; print ${gap_set_p:+present_alt}"#);
        param_alternate_colon_plus_unset => (r#"${unset:+…}"#, r#"unset gap_unset_ap; print "x${gap_unset_ap:+no}x""#);
        param_capitalize_C_flag => (r#"${(C) …}"#, r#"gap_cap=hello; print ${(C)gap_cap}"#);
        typeset_capital_l_lower_case_attr => (r#"typeset -l"#, r#"typeset -l gap_lo=AbCdE; print $gap_lo"#);
        typeset_capital_u_upper_case_attr => (r#"typeset -u"#, r#"typeset -u gap_up=xyZ; print $gap_up"#);
        status_after_false_command => (r#"\$status after false"#, r#"false; print $status"#);
        extendedglob_null_qual_nomatch => (r#"*(#qN) nomatch"#, r#"setopt extendedglob; print *.gap_qn_nomatch_xyz(#qN); print -r "ex=$?""#);
        hash_f_refresh => (r#"hash -f"#, r#"hash -f 2>&1; print after_hash_f"#);
        rehash_builtin_command_table => (r#"rehash"#, r#"rehash 2>&1; print after_rehash"#);
        command_p_path_true => (r#"command -p true"#, r#"command -p true; print after_cmd_p"#);
        builtin_print_word => (r#"builtin print"#, r#"builtin print gap_builtin_print"#);
        command_subst_inner_print => (r#"$("… ")"#, r#"print $(print gap_cmdsubst_inner)"#);
        setopt_multios_two_redirs => (r#"multios > >"#, r#"setopt multios; ga=/tmp/gap_mo_a_$$; gb=/tmp/gap_mo_b_$$; command rm -f $ga $gb; print gap_multiline > $ga > $gb; command cat $ga; command cat $gb; command rm -f $ga $gb"#);
        read_capital_a_array_herestring_ifs => (r#"read -rA IFS"#, r#"IFS=_; line_s=a_b_c; read -rA arr_gap <<< $line_s; print $#arr_gap $arr_gap[2]"#);
        array_append_plus_paren_elems => (r#"a+=( … )"#, r#"ary_gap=(one); ary_gap+=(two three); print ${#ary_gap} $ary_gap[3]"#);
        float_type_scalar => (r#"float"#, r#"float fz_gap=2.25; print $fz_gap"#);
        integer_hex_assignment => (r#"integer 0x"#, r#"integer iz_gap=0x1f; print $iz_gap"#);
        param_strip_shortest_suffix_percent => (r#"${s%pat}"#, r#"sf=name.ext; print ${sf%.*}"#);
        param_strip_shortest_prefix_hash => (r#"${s#pat}"#, r#"pf=pre_suf; print ${pf#pre_}"#);
        zstyle_list_patterns => (r#"zstyle -L"#, r#"zstyle -L 2>&1; print -r "ex=$?""#);
        zle_list_widgets => (r#"zle -l"#, r#"zle -l 2>&1; print -r "ex=$?""#);
        option_promptbang_print => (r#"options[promptbang]"#, r#"print $options[promptbang]"#);
        option_warn_create_global_print => (r#"options[warncreateglobal]"#, r#"print $options[warncreateglobal]"#);
    }
}

/// Large batch: more parameter flags, **`[[ ]]` / `=~`**, **`typeset -T`**, arrays **`:|`** / **`:*`**, many **`$options`**, **`getopts`**, **`zmodload zsh/mathfunc`**, **`dirs`**, **`trap`**, **`signals`**.
mod corpus_dash_fc_bulk_a {
    use super::*;

    parity_gap_tests! {
        bulk_param_lower_L_flag => (r#"${(L)}"#, r#"lk_gap=HeLLo; print ${(L)lk_gap}"#);
        bulk_param_upper_U_flag => (r#"${(U)}"#, r#"uk_gap=hello; print ${(U)uk_gap}"#);
        bulk_param_split_s_space_words => (r#"${(s: :) }"#, r#"sk_gap='p q r'; wk_gap=(${(s: :)sk_gap}); print $#wk_gap $wk_gap[2]"#);
        bulk_param_split_f_lines_printf => (r#"${(f)}"#, r#"fk_gap=$(printf "a\nb"); lines_gap=(${(f)fk_gap}); print $#lines_gap"#);
        bulk_param_visual_V_escapes => (r#"${(V)}"#, r#"vk_gap=$'x\tz'; print ${(V)vk_gap}"#);
        bulk_assoc_sorted_keys_ok_br => (r#"${(ok)}"#, r#"typeset -A az_ok=(k2 v2 k1 v1); print ${(ok)az_ok}"#);
        bulk_typeset_tied_T_scalar_array => (r#"typeset -T"#, r#"typeset -T TDX arx=(one two); print $TDX $arx"#);
        bulk_array_subscript_range_inclusive => (r#"$a[1,2]"#, r#"rg_gap=(10 20 30); print ${rg_gap[1,2]}"#);
        bulk_array_reverse_sort_Oa => (r#"${(Oa)}"#, r#"og_gap=(3 1 2); print ${(Oa)og_gap}"#);
        bulk_array_colon_bar_exclude => (r#"${a:|b}"#, r#"ag_gap=(a b c); bg_gap=(b); print ${ag_gap:|bg_gap}"#);
        bulk_array_colon_star_intersect => (r#"${a:*b}"#, r#"xg_gap=(1 2 3); yg_gap=(2 9); print ${xg_gap:*yg_gap}"#);
        bulk_cond_glob_rhs_double_bracket => (r#"[[ = *.txt ]]"#, r#"[[ gap_nm2.txt == *.txt ]]; print $?"#);
        bulk_cond_numeric_name_eq => (r#"[[ name -eq ]]"#, r#"ival_gap=42; [[ ival_gap -eq 42 ]]; print $?"#);
        bulk_cond_regex_match_operator => (r#"[[ =~ ]]"#, r#"gapreg=gapfoo; [[ gapreg =~ ^gap ]]; print $?"#);
        bulk_opt_braceccl => (r#"options[braceccl]"#, r#"print $options[braceccl]"#);
        bulk_opt_pathdirs => (r#"options[pathdirs]"#, r#"print $options[pathdirs]"#);
        bulk_opt_autopushd => (r#"options[autopushd]"#, r#"print $options[autopushd]"#);
        bulk_opt_magic_equal_subst => (r#"options[magic_equal_subst]"#, r#"print $options[magic_equal_subst]"#);
        bulk_opt_equals_separate => (r#"options[equals]"#, r#"print $options[equals]"#);
        bulk_opt_bslashquote => (r#"options[bslashquote]"#, r#"print $options[bslashquote]"#);
        bulk_opt_appendhistory => (r#"options[appendhistory]"#, r#"print $options[appendhistory]"#);
        bulk_opt_nullglob => (r#"options[nullglob]"#, r#"print $options[nullglob]"#);
        bulk_opt_globdots => (r#"options[globdots]"#, r#"print $options[globdots]"#);
        bulk_opt_caseglob => (r#"options[caseglob]"#, r#"print $options[caseglob]"#);
        bulk_opt_shortloops => (r#"options[shortloops]"#, r#"print $options[shortloops]"#);
        bulk_opt_typesetsilent => (r#"options[typesetsilent]"#, r#"print $options[typesetsilent]"#);
        bulk_opt_nounset => (r#"options[nounset]"#, r#"print $options[nounset]"#);
        bulk_opt_cshjunkiequotes => (r#"options[cshjunkiequotes]"#, r#"print $options[cshjunkiequotes]"#);
        bulk_opt_rcquotes => (r#"options[rcquotes]"#, r#"print $options[rcquotes]"#);
        bulk_opt_interactivecomments => (r#"options[interactivecomments]"#, r#"print $options[interactivecomments]"#);
        bulk_opt_function_argzero => (r#"options[functionargzero]"#, r#"print $options[functionargzero]"#);
        bulk_opt_bsd_echo => (r#"options[bsd_echo]"#, r#"print $options[bsd_echo]"#);
        bulk_opt_errreturn_flag => (r#"options[errreturn]"#, r#"print $options[errreturn]"#);
        bulk_opt_combiningchars => (r#"options[combiningchars]"#, r#"print $options[combiningchars]"#);
        bulk_opt_verbose => (r#"options[verbose]"#, r#"print $options[verbose]"#);
        bulk_opt_xtrace => (r#"options[xtrace]"#, r#"print $options[xtrace]"#);
        bulk_opt_octalzeroes => (r#"options[octalzeroes]"#, r#"print $options[octalzeroes]"#);
        bulk_opt_cbases => (r#"options[cbases]"#, r#"print $options[cbases]"#);
        bulk_background_pid_wait => (r#"$! wait"#, r#"true & print bang_$!; wait; print waited_gap"#);
        bulk_read_herestring_scalar => (r#"read <<<"#, r#"read rv_gap <<< rd_here_val; print $rv_gap"#);
        bulk_emulate_sh_dash_c_inline => (r#"emulate sh -c"#, r#"emulate sh -c 'print emulate_flag_$0'"#);
        bulk_getopts_f_takes_arg => (r#"getopts f:"#, r#"OPTIND=1; getopts "f:" og_go -f gv; print -r "og=${og_go} arg=${OPTARG}""#);
        bulk_arith_logical_and_or => (r#"$(( && || ))"#, r#"print $(( 1 && 0 )) $(( 0 || 1 ))"#);
        bulk_arith_power_int => (r#"** 10"#, r#"print $(( 2 ** 10 ))"#);
        bulk_zmodload_mathfunc_sqrt => (r#"zmodload mathfunc sqrt"#, r#"zmodload zsh/mathfunc 2>&1; print -r "s9=$(( sqrt(9) ))""#);
        bulk_cond_readable_root => (r#"[[ -r / ]]"#, r#"[[ -r / ]]; print $?"#);
        bulk_cond_executable_sh_or_bash => (r#"[[ -x /bin/sh ]]"#, r#"[[ -x /bin/sh ]] || [[ -x /bin/bash ]]; print $?"#);
        bulk_keytimeout_param => (r#"KEYTIMEOUT"#, r#"print -r "kt=${KEYTIMEOUT:-nil}""#);
        bulk_zle_space_sep_words_param => (r#"ZLE_SPACE_SEP_WORDS"#, r#"print -r "zsw=${ZLE_SPACE_SEP_WORDS:-nil}""#);
        bulk_locale_builtin_exit => (r#"locale"#, r#"locale 2>&1; print -r "ex=$?""#);
        bulk_prompt_percent_event_hash => (r#"${(%)#}"#, r#"print ${(%)#}"#);
        bulk_nested_default_substitution => (r#"${:- ${:-}}"#, r#"unset nestp_gap; print ${nestp_gap:-${:-nest_inner}}"#);
        bulk_wordchars_param => (r#"WORDCHARS"#, r#"print -r "wc=${WORDCHARS:-nil}""#);
        bulk_keyboard_hack_plus => (r#"$+KEYBOARD_HACK"#, r#"print $+KEYBOARD_HACK"#);
        bulk_typeset_Z_pad_int_six => (r#"typeset -Z 6 -i"#, r#"typeset -Z 6 -i zip6=42; print $zip6"#);
        bulk_argv_slice_tail_range => (r#"$@[2,-1]"#, r#"set -- a b c d; print -r "slice=$@[2,-1]""#);
        bulk_assoc_values_singleton => (r#"${(v) A}"#, r#"typeset -A solo=(onlykid onlyval); print ${(v)solo}"#);
        bulk_histchars_string_length => (r#"${#HISTCHARS}"#, r#"print ${#HISTCHARS}"#);
        bulk_module_path_first_subscript => (r#"$module_path[1]"#, r#"print ${module_path[1]:-missing_mp}"#);
        bulk_dirs_push_pop_dirs_p => (r#"dirs -p stack"#, r#"builtin cd /tmp; pushd -q / >/dev/null; dirs -p; popd >/dev/null; print dirs_done"#);
        bulk_zsh_subshell_counter => (r#"$ZSH_SUBSHELL"#, r#"print out=$ZSH_SUBSHELL; ( print in=$ZSH_SUBSHELL )"#);
        bulk_zsh_name_string => (r#"$ZSH_NAME"#, r#"print $ZSH_NAME"#);
        bulk_jobs_builtin_list => (r#"jobs"#, r#"jobs 2>&1; print -r "ex=$?""#);
        bulk_whence_true_word => (r#"whence true"#, r#"whence true 2>&1; print -r "ex=$?""#);
        bulk_type_builtin_true => (r#"type true"#, r#"type true 2>&1; print -r "ex=$?""#);
        bulk_trap_list_handlers => (r#"trap"#, r#"trap 2>&1; print -r "ex=$?""#);
        bulk_signals_assoc_element_count => (r#"${#signals}"#, r#"print ${#signals}"#);
        bulk_param_minus_fallback_not_colon => (r#"${unset-word}"#, r#"unset md_sub_gap; print ${md_sub_gap-mddef_word}"#);
        bulk_print_rn_then_newline => (r#"print -rn"#, r#"print -rn zz_no_nl; print zz_with_ln"#);
        bulk_whence_dash_p_system_sh => (r#"whence -p sh"#, r#"whence -p sh 2>&1; print -r "ex=$?""#);
    }
}
