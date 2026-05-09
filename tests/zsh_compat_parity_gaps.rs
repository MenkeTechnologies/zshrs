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
//! `corpus_dash_fc_control_flow`, `corpus_dash_fc_params_redir`, `corpus_dash_fc_bulk_a`,
//! `corpus_dash_fc_bulk_b`, `corpus_dash_fc_bulk_c`, `corpus_dash_fc_bulk_d`,
//! `corpus_dash_fc_bulk_e`, `corpus_dash_fc_bulk_f`, `corpus_dash_fc_bulk_g`,
//! `corpus_dash_fc_bulk_h`, `corpus_dash_fc_bulk_i`, `corpus_dash_fc_bulk_j`, `corpus_dash_fc_bulk_k`). Pass/fail is **stdout + exit** only (see `assert_parity`).

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

/// Second large batch: brace tricks, **`${PWD:h}`**, **`=`** splitting, **`read -d`**, **`zcompile`**, **`zsh/stat`**, options, arithmetic, **`bindkey`**, **`widgets`**.
mod corpus_dash_fc_bulk_b {
    use super::*;

    parity_gap_tests! {
        bulk_b_brace_triple_comma_repeat => (r#"x{,,}y"#, r#"print x{,,}y"#);
        bulk_b_braceccl_letter_range => (r#"braceccl {m-o}"#, r#"setopt braceccl; print {m-o}"#);
        bulk_b_equals_split_words_argv => (r##"set -- ${=sv}"##, r#"sv_beq='r s'; set -- ${=sv_beq}; print $#'#);
        bulk_b_pwdtail_h_modifier => ("PWD :h tail", r#"print ${PWD:h}"#);
        bulk_b_assoc_append_plus_paren => (r#"A+=([k]=v)"#, r#"typeset -A ax_b=(); ax_b+=([kx_b]=vy_b); print $ax_b[kx_b]"#);
        bulk_b_euid_and_username => (r#"EUID USERNAME"#, r#"print $EUID $USERNAME"#);
        bulk_b_lang_scalar => (r#"LANG"#, r#"print ${LANG:-nil_lang}"#);
        bulk_b_lc_all_scalar => (r#"LC_ALL"#, r#"print ${LC_ALL:-nil_lcall}"#);
        bulk_b_zsh_patchlevel_string => (r#"ZSH_PATCHLEVEL"#, r##"print -r "$ZSH_PATCHLEVEL""##);
        bulk_b_terminfo_colors_bracket => (r#"terminfo[colors]"#, r#"print ${terminfo[colors]:-terminfo_no_colors}"#);
        bulk_b_prompt_expand_ps1_pct_hash => (r#"PS1 % + (%)PS1"#, r#"PS1_bb="%#"; print ${(%)PS1_bb}"#);
        bulk_b_cond_char_dev_null => (r##"[[ -e /dev/null ]]"##, r#"[[ -e /dev/null ]]; print $?"#);
        bulk_b_cond_o_login => (r##"[[ -o login ]]"##, r#"[[ -o login ]]; print $?"#);
        bulk_b_cond_o_interactive => (r##"[[ -o interactive ]]"##, r#"[[ -o interactive ]]; print $?"#);
        bulk_b_option_functrace => (r#"options[functrace]"#, r##"print -r "<$options[functrace]>""##);
        bulk_b_option_globsubst_value => (r#"options[globsubst]"#, r#"print $options[globsubst]"#);
        bulk_b_option_bareglobqual => (r#"options[bareglobqual]"#, r#"print $options[bareglobqual]"#);
        bulk_b_option_extendedhistory => (r#"options[extendedhistory]"#, r#"print $options[extendedhistory]"#);
        bulk_b_read_ifs_delim_colon => (r#"read -d :"#, r##"printf 'a:b' | IFS= read -d : -r rd_b; print -r "$rd_b""##);
        bulk_b_builtin_false_exit => (r#"builtin false"#, r##"builtin false; print -r "st=$?""##);
        bulk_b_path_array_literal_first => (r#"path=(...)"#, r#"path_b=(/tmp); print $path_b[1]"#);
        bulk_b_zmodload_calendar_stderr => (r#"zmodload zsh/calendar"#, r##"zmodload zsh/calendar 2>&1; print -r "ex=$?""##);
        bulk_b_zmodload_net_tcp_ok => (r#"zmodload zsh/net/tcp"#, r##"zmodload zsh/net/tcp 2>&1; print -r "ex=$?""##);
        bulk_b_zcompile_tmpfile => (r#"zcompile"#, r##"print zcb >/tmp/gap_zc_b_$$; zcompile /tmp/gap_zc_b_$$ 2>&1; print -r "zc=$?"; command rm -f /tmp/gap_zc_b_$$ /tmp/gap_zc_b_$$.zwc"##);
        bulk_b_allexport_in_subshell => (r#"allexport subshell"#, r##"( setopt allexport; y_alx=9; print -r "in=$y_alx" ); print -r "out_plus=$+y_alx""##);
        bulk_b_nested_for_pair => (r#"nested for"#, r##"for a_b in 1 2; do for b_b in x; do print "${a_b}${b_b}"; done; done"##);
        bulk_b_arith_compound_add => (r#"(("+="))"#, r#"ac_b=1; (( ac_b += 3 )); print $ac_b"#);
        bulk_b_zstat_size_etc_hosts => (r#"stat +size /etc/hosts"#, r##"zmodload zsh/stat 2>&1; stat +size /etc/hosts 2>&1; print -r "ex=$?""##);
        bulk_b_columns_default => (r#"COLUMNS"#, r#"print ${COLUMNS:-col0}"#);
        bulk_b_read_dev_null_reply_len => (r#"read /dev/null"#, r##"read -r < /dev/null; print -r "replen=${#REPLY}""##);
        bulk_b_histcmd_scalar => (r#"HISTCMD"#, r##"print -r "histcmd=$HISTCMD""##);
        bulk_b_plus_histfile => (r#"$+HISTFILE"#, r#"print $+HISTFILE"#);
        bulk_b_fc_list_missing_event => (r#"fc -l bad event"#, r##"fc -l 1 2>&1; print -r "ex=$?""##);
        bulk_b_unalias_all_subshell => (r#"unalias -a ( )"#, r#"( unalias -a 2>&1; print ua_cleared_inner )"#);
        bulk_b_shift_zero_noop => (r#"shift 0"#, r##"set -- pivot_a pivot_b; shift 0; print -r "$1 $2""##);
        bulk_b_arith_float_gt => (r#"(( 3.1 > 3 ))"#, r#"(( 3.1 > 3 )); print $?"#);
        bulk_b_arith_int_divide => (r#"7/2"#, r#"print $(( 7 / 2 ))"#);
        bulk_b_arith_modulo => (r#"7%3"#, r#"print $(( 7 % 3 ))"#);
        bulk_b_arith_bitand => (r#"5&3"#, r#"print $(( 5 & 3 ))"#);
        bulk_b_arith_shl_bits => (r#"1<<4"#, r#"print $(( 1 << 4 ))"#);
        bulk_b_bindkey_caret_A => (r#"bindkey ^A"#, r##"bindkey "^A" 2>&1; print -r "ex=$?""##);
        bulk_b_keymap_name_param => (r#"KEYMAP"#, r##"print -r "${KEYMAP:-no_keymap}""##);
        bulk_b_plus_widget_history_isearch => (r#"$+widgets hist isearch"#, r#"print $+widgets[history-incremental-search-backward]"#);
        bulk_b_option_autocd => (r#"options[autocd]"#, r#"print $options[autocd]"#);
        bulk_b_option_autoparamslash => (r#"options[autoparamslash]"#, r#"print $options[autoparamslash]"#);
        bulk_b_option_autoremoveslash => (r#"options[autoremoveslash]"#, r#"print $options[autoremoveslash]"#);
        bulk_b_option_badpattern => (r#"options[badpattern]"#, r#"print $options[badpattern]"#);
        bulk_b_option_beep => (r#"options[beep]"#, r#"print $options[beep]"#);
        bulk_b_option_bindkeys => (r#"options[bindkeys]"#, r#"print $options[bindkeys]"#);
        bulk_b_option_checkjobs => (r#"options[checkjobs]"#, r#"print $options[checkjobs]"#);
        bulk_b_option_clobber => (r#"options[clobber]"#, r#"print $options[clobber]"#);
        bulk_b_option_completealiases => (r#"options[completealiases]"#, r#"print $options[completealiases]"#);
        bulk_b_option_correct => (r#"options[correct]"#, r#"print $options[correct]"#);
        bulk_b_option_dvorak => (r#"options[dvorak]"#, r#"print $options[dvorak]"#);
        bulk_b_option_braceexpand_val => (r#"options[braceexpand]"#, r#"print $options[braceexpand]"#);
        bulk_b_option_alwayslastprompt => (r#"options[alwayslastprompt]"#, r#"print $options[alwayslastprompt]"#);
        bulk_b_option_hashlistall => (r#"options[hashlistall]"#, r#"print $options[hashlistall]"#);
        bulk_b_option_histverify => (r#"options[histverify]"#, r#"print $options[histverify]"#);
        bulk_b_option_histsavebycopy => (r#"options[histsavebycopy]"#, r#"print $options[histsavebycopy]"#);
        bulk_b_option_ignoreeof => (r#"options[ignoreeof]"#, r#"print $options[ignoreeof]"#);
        bulk_b_option_mailwarning => (r#"options[mailwarning]"#, r#"print $options[mailwarning]"#);
        bulk_b_option_monitor_jobs => (r#"options[monitor]"#, r#"print $options[monitor]"#);
        bulk_b_option_pushdignoredups => (r#"options[pushdignoredups]"#, r#"print $options[pushdignoredups]"#);
        bulk_b_option_cdablevars => (r#"options[cdablevars]"#, r#"print $options[cdablevars]"#);
        bulk_b_zpfx_param_default => (r#"ZPFX"#, r#"print ${ZPFX:-empty_zpfx}"#);
        bulk_b_fpath_first_elt => (r#"fpath[1]"#, r#"print ${fpath[1]:-no_fpath}"#);
        bulk_b_module_path_join_colon => (r##"${(j.:.)module_path}"##, r#"print ${(j.:.)module_path}"#);
        bulk_b_print_octdumps_one_octet => (r#"print -o one byte"#, r##"print -o B5 2>&1; print -r "ex=$?""##);
    }
}

/// Third large batch: **`:^^`**, `:**:` / `##` / `%%`, **`((2#…))`**, **`typeset -i2`**, **`set -A`**, **`coproc`**, **`zparseopts`**, **`zmodload`**, **`printf %q`**, more **`$options`**.
mod corpus_dash_fc_bulk_c {
    use super::*;

    parity_gap_tests! {
        bulk_c_colon_caret_caret_zip => (r#"${a:^^b}"#, r#"xc=(1 2); yc=(a b); print ${xc:^^yc}"#);
        bulk_c_scalar_colon_t_colon_h => (r#"${p:t} ${p:h}"#, r#"pc=/x/y/z.name; print ${pc:t} ${pc:h}"#);
        bulk_c_hashhash_percentpercent_strip => (r#"## */ %% /*"#, r#"lsc=foo/bar/baz; print ${lsc##*/} ${lsc%%/*}"#);
        bulk_c_array_slice_offset_length => (r#"$a[1:2]"#, r#"aryc=(a b c d); print ${aryc:1:2}"#);
        bulk_c_subshell_builtin_cd => (r#"subshell cd"#, r#"( builtin cd /tmp; print -r subtmp_ok ); print -r after_subcd"#);
        bulk_c_arith_hex_ff => (r#"16#FF"#, r#"print $(( 16#FF ))"#);
        bulk_c_arith_binary_sharp_form => (r#"2#1010"#, r#"print $(( 2#1010 ))"#);
        bulk_c_bang_false_exit => (r#"! false"#, r#"! false; print $?"#);
        bulk_c_posix_bracket_empty => (r#"POSIX [ -z ]"#, r##"[ -z "" ]; print bracketc:$?"##);
        bulk_c_cond_string_lex_less => (r#"[[ a < b ]]"#, r##"[[ a < b ]]; print lex:$?"##);
        bulk_c_cond_file_hosts => (r#"[[ -f /etc/hosts ]]"#, r##"[[ -f /etc/hosts ]] || [[ -f /etc/hostname ]]; print hostsish:$?"##);
        bulk_c_cond_dir_tmp => (r#"[[ -d /tmp ]]"#, r##"[[ -d /tmp ]]; print dtmp:$?"##);
        bulk_c_integer_postincrement => (r#"(( i++ ))"#, r#"integer icc_c=0; (( icc_c++ )); print $icc_c"#);
        bulk_c_float_scientific_E => (r#"typeset -E 1e2"#, r#"typeset -E fsci_c=1e2; print $fsci_c"#);
        bulk_c_sparse_array_assign => (r#"a[5] sparse"#, r#"typeset -a spc=(); spc[5]=hi_gap; print $spc[5]"#);
        bulk_c_unset_two_names => (r#"unset a b"#, r#"unset uc_aa_c uc_bb_c; print unset_pair_done"#);
        bulk_c_command_echo_word => (r#"command echo"#, r#"command echo ce_gap_c"#);
        bulk_c_which_echo => (r#"which echo"#, r##"which echo 2>&1; print -r "ex=$?""##);
        bulk_c_umask_capture => (r#"umask"#, r##"uoutc=$(umask); print -r "um=$uoutc""##);
        bulk_c_tmpdir_default => (r#"TMPDIR"#, r#"print ${TMPDIR:-nil_tmpdir}"#);
        bulk_c_lineno_start => (r#"LINENO"#, r##"print -r "Lstart=$LINENO""##);
        bulk_c_print_dash_dash_literal => (r#"print -- -n"#, r#"print -r -- '-n_literal'"#);
        bulk_c_setopt_localoptions_nominal => (r#"setopt localoptions"#, r##"setopt localoptions 2>&1; print -r "lox=$?""##);
        bulk_c_function_localtraps_option => (r#"localtraps fn"#, r#"fn_lt() { setopt localtraps; print fn_lt_inner; }; fn_lt"#);
        bulk_c_autoload_zle_hook_helper => (r#"autoload add-zle-hook"#, r##"autoload -U add-zle-hook-widget 2>&1; print -r "alz=$?""##);
        bulk_c_zparseopts_array_accumulate => (r#"zparseopts -a"#, r##"typeset -a zpo_c=(); zparseopts -a zpo_c -- 2>&1; print -r "n=$#zpo_c ex=$?""##);
        bulk_c_zmodload_parameter_module => (r#"zmodload zsh/parameter"#, r##"zmodload zsh/parameter 2>&1; print -r "ex=$?""##);
        bulk_c_printf_q_escaped => (r#"printf %q"#, r##"printf '%q\n' 'two words c'"##);
        bulk_c_ifs_read_two_parts => (r#"IFS read :"#, r#"IFS=: read -r rc1_c rc2_c <<< 'u:v'; print $rc1_c $rc2_c"#);
        bulk_c_coproc_cat_bang => (r#"coproc cat"#, r##"coproc cat; print -r "cop=$!""##);
        bulk_c_dollar_under_after_command => (r#"$_ after cmd"#, r#"true gap_under_c; print $_"#);
        bulk_c_argv_one_after_set => (r#"$argv set --"#, r#"set -- xc1_c; print $argv[1]"#);
        bulk_c_set_capital_A_array => (r#"set -A"#, r#"set -A arrset_c a b c; print $arrset_c[2]"#);
        bulk_c_param_q_ansi_c_quote => (r#"${(q) }"#, r#"wqc='a b c'; print ${(q)wqc}"#);
        bulk_c_false_then_true_chain => (r#"false; true"#, r##"false; true; print -r "chain=$?""##);
        bulk_c_double_bracket_or_pattern => (r#"[[ ]] || [[ ]]"#, r##"[[ gap_a = gap_b ]] || [[ gap_c = gap_c ]]; print orc=$?"##);
        bulk_c_typeset_base_two_output => (r#"typeset -i2"#, r#"typeset -i2 ib2_c=5; print $ib2_c"#);
        bulk_c_subshell_exit_propagates_status => (r#"( exit 2 )"#, r##"( exit 2 ); print -r "after_sub=$?""##);
        bulk_c_command_capital_V_colon => (r#"command -V :"#, r##"command -V : 2>&1; print -r "ex=$?""##);
        bulk_c_zmodload_langinfo_module => (r#"zmodload zsh/langinfo"#, r##"zmodload zsh/langinfo 2>&1; print -r "ex=$?""##);
        bulk_c_readonly_scalar => (r#"readonly"#, r#"readonly rov_c=gapval_c; print $rov_c"#);
        bulk_c_jobs_minus_l => (r#"jobs -l"#, r##"jobs -l 2>&1; print -r "ex=$?""##);
        bulk_c_arith_zero_x_prefix => (r#"0x10"#, r#"print $(( 0x10 ))"#);
        bulk_c_opt_allexport => (r#"options[allexport]"#, r#"print $options[allexport]"#);
        bulk_c_opt_aliasfuncdef => (r#"options[aliasfuncdef]"#, r#"print $options[aliasfuncdef]"#);
        bulk_c_opt_appendcreate => (r#"options[appendcreate]"#, r#"print $options[appendcreate]"#);
        bulk_c_opt_nobadpattern => (r#"options[nobadpattern]"#, r#"print $options[nobadpattern]"#);
        bulk_c_opt_rcexpandparam => (r#"options[rcexpandparam]"#, r#"print $options[rcexpandparam]"#);
        bulk_c_opt_rematchpcre => (r#"options[rematchpcre]"#, r#"print $options[rematchpcre]"#);
        bulk_c_opt_posixidentifiers => (r#"options[posixidentifiers]"#, r#"print $options[posixidentifiers]"#);
        bulk_c_opt_histfcntllock => (r#"options[histfcntllock]"#, r#"print $options[histfcntllock]"#);
        bulk_c_opt_histnostore => (r#"options[histnostore]"#, r#"print $options[histnostore]"#);
        bulk_c_opt_sharehistory => (r#"options[sharehistory]"#, r#"print $options[sharehistory]"#);
        bulk_c_opt_incappendhistory => (r#"options[incappendhistory]"#, r#"print $options[incappendhistory]"#);
        bulk_c_opt_nobeep => (r#"options[nobeep]"#, r#"print $options[nobeep]"#);
        bulk_c_opt_noaliases => (r#"options[noaliases]"#, r#"print $options[noaliases]"#);
        bulk_c_opt_norcs => (r#"options[norcs]"#, r#"print $options[norcs]"#);
        bulk_c_opt_shinstdin => (r#"options[shinstdin]"#, r#"print $options[shinstdin]"#);
        bulk_c_opt_singlecommand => (r#"options[singlecommand]"#, r#"print $options[singlecommand]"#);
        bulk_c_opt_sourcetrace => (r#"options[sourcetrace]"#, r#"print $options[sourcetrace]"#);
        bulk_c_opt_sunkeyboardhack => (r#"options[sunkeyboardhack]"#, r#"print $options[sunkeyboardhack]"#);
        bulk_c_opt_noflowcontrol => (r#"options[noflowcontrol]"#, r#"print $options[noflowcontrol]"#);
    }
}

/// Fourth batch: **param flags `(L)`/`(U)`**, **`${param#…}` / `${param/x/y}`**, **`pipefail`**, **`getopts`**, `whence`, **associative `typeset -A`**, **`mktemp` I/O**, **`emulate`**, **globbing / `noglob`**, **`SHLVL`**, **`SECONDS`**, more **`$options`**.
mod corpus_dash_fc_bulk_d {
    use super::*;

    parity_gap_tests! {
        bulk_d_param_flag_lower_upper => (r#"${(L)(U)}"#, r#"print ${(L)HiDu} ${(U)loDu}"#);
        bulk_d_param_hash_prefix_strip => (r#"${v#pfx}"#, r#"vpdu=pfxrestDu; print ${vpdu#pfx}"#);
        bulk_d_param_percent_suffix_strip => (r#"${v%sfx}"#, r#"vsdu=restsfx; print ${vsdu%sfx}"#);
        bulk_d_param_slash_replace_one => (r#"${a/x/y}"#, r#"asdu=xxXxx; print ${asdu/x/y}"#);
        bulk_d_param_slash_replace_all => (r#"${a//x/y}"#, r#"aasdu=xaxb; print ${aasdu//x/y}"#);
        bulk_d_arith_modulo => (r#"7 % 3"#, r#"print $(( 7 % 3 ))"#);
        bulk_d_arith_integer_div => (r#"8 / 3"#, r#"print $(( 8 / 3 ))"#);
        bulk_d_logical_and_short_circuit => (r#"true && print"#, r#"true && print andDu"#);
        bulk_d_logical_or_short_circuit => (r#"false || print"#, r#"false || print orDu"#);
        bulk_d_case_simple_literal => (r#"case literal"#, r#"case caseDu in caseDu) print hit_case;; esac"#);
        bulk_d_for_list_three => (r#"for 3 words"#, r#"for wd in a b c; do print $wd; done"#);
        bulk_d_while_counter_two => (r#"while twice"#, r#"idu=0; while (( idu < 2 )); do print $idu; idu=$(( idu + 1 )); done"#);
        bulk_d_repeat_two_print => (r#"repeat 2"#, r#"repeat 2; do print rDu; done"#);
        bulk_d_if_elif_branch => (r#"if elif"#, r#"if false; then print aDu; elif true; then print bDu; fi"#);
        bulk_d_function_return_status => (r#"fn return 4"#, r#"fnretDu() { return 4; }; fnretDu; print $?"#);
        bulk_d_nested_cmd_subst => (r#"$(inner)"#, r#"print $(print innerDu)"#);
        bulk_d_shlvl_scalar => (r#"SHLVL"#, r#"print $SHLVL"#);
        bulk_d_seconds_scalar => (r#"SECONDS"#, r#"print $SECONDS"#);
        bulk_d_pipestatus_pair => (r#"pipestatus"#, r#"true; false; true; print ${pipestatus[1]} ${pipestatus[2]} ${pipestatus[3]}"#);
        bulk_d_whence_hyphen_p_cat => (r#"whence -p cat"#, r#"whence -p cat"#);
        bulk_d_whence_hyphen_v_true => (r#"whence -v true"#, r#"whence -v true"#);
        bulk_d_typeset_capital_a_assoc => (r#"typeset -A"#, r#"typeset -A mapDu; mapDu[kDu]=vDu; print $mapDu[kDu]"#);
        bulk_d_array_plus_eq_append => (r#"ary+= b"#, r#"adup=(x); adup+=y; print $adup"#);
        bulk_d_brace_brace_comma_two => (r#"a{b,c}d"#, r#"print a{bDu,cDu}d"#);
        bulk_d_brace_range_one_to_three => (r#"{1..3}"#, r#"print {1..3}"#);
        bulk_d_shift_two_argv => (r#"shift 2"#, r#"set -- s0 s1 s2 s3; shift 2; print $1"#);
        bulk_d_mktemp_write_read_rm => (r#"mktemp roundtrip"#, r##"tdf=$(mktemp); print hiDu > $tdf; cat $tdf; ec=$?; command rm -f $tdf; exit $ec"##);
        bulk_d_emulate_minus_L_sh => (r#"emulate -L sh"#, r#"emulate -L sh 2>&1; print emulateDu=$?"#);
        bulk_d_setopt_pipefail_status => (r#"setopt pipefail"#, r#"setopt pipefail; false | true; print $?"#);
        bulk_d_getopts_minus_a_flag => (r#"getopts a:"#, r##"set -- -avDu; OPTIND=1; getopts 'a:' optDu; print -r "$optDu-$OPTARG""##);
        bulk_d_word_split_flag_w => (r#"${(w)}"#, r#"wsDu='p q'; print ${(w)wsDu}"#);
        bulk_d_join_space_array => (r#"${(j. .)a}"#, r#"jadup=(a b c); print ${(j. .)jadup}"#);
        bulk_d_sort_flags_o_array => (r#"${(o)a}"#, r#"sadup=(b a); print ${(o)sadup}"#);
        bulk_d_reverse_sort_O_flag => (r#"${(Oa)}"#, r#"roadup=(a b); print ${(Oa)roadup}"#);
        bulk_d_unique_flag_u_array => (r#"${(u)a}"#, r#"uadup=(x x y); print ${(u)uadup}"#);
        bulk_d_length_hash_scalar => (r#"${#s}"#, r#"lensDu=abcde; print ${#lensDu}"#);
        bulk_d_length_hash_array => (r#"${#a}"#, r#"lenadup=(q r s t); print ${#lenadup}"#);
        bulk_d_colon_minus_default => (r#"${:-d}"#, r#"unset missDu; print ${missDu:-defDu}"#);
        bulk_d_colon_plus_nonempty => (r#"${:+set}"#, r#"setDu=1; print ${setDu:+yesDu}"#);
        bulk_d_colon_plus_empty => (r#"${:+empty}"#, r#"unset emptyDu; print ${emptyDu:+yesDu}"#);
        bulk_d_dollar_under_after_colon => (r#": ; $_"#, r#": postDu; print $_"#);
        bulk_d_typeset_i_from_float => (r#"typeset -i float assign"#, r#"typeset -i iduf=9.7; print $iduf"#);
        bulk_d_printf_lower_hex => (r#"printf %x"#, r#"printf '%x\n' 255"#);
        bulk_d_arith_plus_assign => (r#"arith +="#, r#"integer padu=2; (( padu += 3 )); print $padu"#);
        bulk_d_arith_preincrement => (r#"++i"#, r#"integer preDu=1; (( ++preDu )); print $preDu"#);
        bulk_d_float_lt_compare => (r#"(( 1.5 < 2 ))"#, r#"(( 1.5 < 2.0 )); print fcDu=$?"#);
        bulk_d_anonymous_function_call => (r#"() { }"#, r#"() { print anonDu; }"#);
        bulk_d_subshell_assign_hide => (r#"( x=1 ); $x"#, r#"( xDu=1 ); print ${xDu:-absentDu}"#);
        bulk_d_typeset_i16_display => (r#"typeset -i 16"#, r#"typeset -i 16 hxDu=255; print $hxDu"#);
        bulk_d_setopt_extended_glob_nominal => (r#"setopt extendedglob"#, r#"setopt extendedglob 2>&1; print egDu=$?"#);
        bulk_d_setopt_noglob_then_star => (r#"noglob star"#, r#"setopt noglob; print *"#);
        bulk_d_setopt_null_glob_nominal => (r#"setopt nullglob"#, r#"setopt nullglob 2>&1; print ngDu=$?"#);
        bulk_d_hash_builtin_ls => (r#"hash ls"#, r##"hash ls 2>&1; print -r "ex=$?""##);
        bulk_d_enable_builtin_print => (r#"enable print"#, r##"enable print 2>&1; print -r "ex=$?""##);
        bulk_d_command_v_ls => (r#"command -v ls"#, r#"command -v ls"#);
        bulk_d_builtins_print_e_exists => (r#"$builtins[echo]"#, r#"print ${builtins[echo]:-no_echo_builtins}"#);
        bulk_d_functions_call_self_ref => (r#"$+functions"#, r#"fnrfDu() { print inrfDu; }; print $+functions[fnrfDu]"#);
        bulk_d_aliases_table_echo => (r#"aliases[echo]"#, r#"print ${aliases[echo]:-no_echo_alias}"#);
        bulk_d_zmodload_zsh_zftp => (r#"zmodload zsh/zftp"#, r##"zmodload zsh/zftp 2>&1; print -r "ex=$?""##);
        bulk_d_zmodload_zsh_mapfile => (r#"zmodload zsh/mapfile"#, r##"zmodload zsh/mapfile 2>&1; print -r "ex=$?""##);
        bulk_d_zmodload_zsh_datetime => (r#"zmodload zsh/datetime"#, r##"zmodload zsh/datetime 2>&1; print -r "ex=$?""##);
        bulk_d_scheduled_events_plus => (r#"$+zsh_scheduled_events"#, r#"print $+zsh_scheduled_events"#);
        bulk_d_opt_bsdecho => (r#"options[bsdecho]"#, r#"print $options[bsdecho]"#);
        bulk_d_opt_noshwordsplit => (r#"options[noshwordsplit]"#, r#"print $options[noshwordsplit]"#);
        bulk_d_opt_cshjunkiequotes => (r#"options[cshjunkiequotes]"#, r#"print $options[cshjunkiequotes]"#);
        bulk_d_opt_cshjunkieloops => (r#"options[cshjunkieloops]"#, r#"print $options[cshjunkieloops]"#);
        bulk_d_opt_kshzerosubscript => (r#"options[kshzerosubscript]"#, r#"print $options[kshzerosubscript]"#);
        bulk_d_opt_octalzeroes => (r#"options[octalzeroes]"#, r#"print $options[octalzeroes]"#);
        bulk_d_opt_warncreateglobal => (r#"options[warncreateglobal]"#, r#"print $options[warncreateglobal]"#);
        bulk_d_opt_autocd => (r#"options[autocd]"#, r#"print $options[autocd]"#);
        bulk_d_opt_errreturn => (r#"options[errreturn]"#, r#"print $options[errreturn]"#);
        bulk_d_opt_continueonerror => (r#"options[continueonerror]"#, r#"print $options[continueonerror]"#);
        bulk_d_opt_printexitvalue => (r#"options[printexitvalue]"#, r#"print $options[printexitvalue]"#);
        bulk_d_opt_xtrace => (r#"options[xtrace]"#, r#"print $options[xtrace]"#);
        bulk_d_opt_verbose => (r#"options[verbose]"#, r#"print $options[verbose]"#);
        bulk_d_opt_kshoptionprint => (r#"options[kshoptionprint]"#, r#"print $options[kshoptionprint]"#);
        bulk_d_opt_errexit => (r#"options[errexit]"#, r#"print $options[errexit]"#);
        bulk_d_opt_unset => (r#"options[unset]"#, r#"print $options[unset]"#);
        bulk_d_opt_promptbang => (r#"options[promptbang]"#, r#"print $options[promptbang]"#);
        bulk_d_opt_promptpercent => (r#"options[promptpercent]"#, r#"print $options[promptpercent]"#);
        bulk_d_opt_promptsubst => (r#"options[promptsubst]"#, r#"print $options[promptsubst]"#);
        bulk_d_opt_transientrprompt => (r#"options[transientrprompt]"#, r#"print $options[transientrprompt]"#);
        bulk_d_opt_chaselinks => (r#"options[chaselinks]"#, r#"print $options[chaselinks]"#);
        bulk_d_opt_chasedots => (r#"options[chasedots]"#, r#"print $options[chasedots]"#);
        bulk_d_opt_multios => (r#"options[multios]"#, r#"print $options[multios]"#);
        bulk_d_opt_numericglobsort => (r#"options[numericglobsort]"#, r#"print $options[numericglobsort]"#);
        bulk_d_opt_markdirs => (r#"options[markdirs]"#, r#"print $options[markdirs]"#);
        bulk_d_opt_globassign => (r#"options[globassign]"#, r#"print $options[globassign]"#);
        bulk_d_opt_globdots => (r#"options[globdots]"#, r#"print $options[globdots]"#);
        bulk_d_opt_globsubst => (r#"options[globsubst]"#, r#"print $options[globsubst]"#);
        bulk_d_opt_dotglob => (r#"options[dotglob]"#, r#"print $options[dotglob]"#);
        bulk_d_opt_caseglob => (r#"options[caseglob]"#, r#"print $options[caseglob]"#);
        bulk_d_opt_casesensitive => (r#"options[casesensitive]"#, r#"print $options[casesensitive]"#);
        bulk_d_opt_casepaths => (r#"options[casepaths]"#, r#"print $options[casepaths]"#);
        bulk_d_opt_extendedhistory => (r#"options[extendedhistory]"#, r#"print $options[extendedhistory]"#);
    }
}

/// Fifth batch: **heredoc / here-string**, `eval`, **brace groups**, **`until` / `for (( ))`**, **`pushd`/`popd`**, **`trap`**, **`[[ =~ ]]`**, **`zformat`**, **`zmodload`**, **`$funcstack`**, **`$-`**, **`$MATCH`**, many **less-used `$options`**.
mod corpus_dash_fc_bulk_e {
    use super::*;

    parity_gap_tests! {
        bulk_e_here_string_read => (r#"read <<<"#, r##"read hs_e <<< 'hsline_e'; print -r "$hs_e""##);
        bulk_e_here_doc_read_line => (r#"here doc 1 line"#, r##"read hde <<'HEe'
lineone_e
HEe
print -r "$hde""##);
        bulk_e_eval_builtin_string => (r#"eval print"#, r#"eval 'print evline_e'"#);
        bulk_e_brace_group_semicolon => (r#"brace ;"#, r#"{ print bg1_e; print bg2_e; }"#);
        bulk_e_subshell_simple_print => (r#"subshell print"#, r#"( print sube_e )"#);
        bulk_e_list_and_chain => (r#"p && p"#, r#"print and1_e && print and2_e"#);
        bulk_e_list_or_chain => (r#"false || print"#, r#"false || print ore_e"#);
        bulk_e_pipe_to_cat => (r#"print | cat"#, r#"print pipee_e | cat"#);
        bulk_e_until_loop_twice => (r#"until twice"#, r#"ue=0; until (( ue > 1 )); do print $ue; ue=$(( ue + 1 )); done"#);
        bulk_e_arithmetic_for_loop => (r#"for (( ))"#, r#"for (( ie=0; ie < 3; ie++ )); do print $ie; done"#);
        bulk_e_scalar_plus_append => (r#"scalar +="#, r##"sae=foo; sae+=bar; print -r "$sae""##);
        bulk_e_array_negative_index => (r#"ary[-1]"#, r#"ane=(p q r); print $ane[-1]"#);
        bulk_e_array_range_slice => (r#"ary[2,4]"#, r#"arsl=(a b c d e); print $arsl[2,4]"#);
        bulk_e_print_hyphen_n => (r#"print -n"#, r#"print -n noNLE_ ; print trailer_e"#);
        bulk_e_printf_zero_padding => (r#"printf %04d"#, r#"printf '%04d\n' 7"#);
        bulk_e_printf_float_two => (r#"printf %.2f"#, r#"printf '%.2f\n' 3.1"#);
        bulk_e_pushd_popd_exit_codes => (r#"pushd popd $?"#, r##"pushd /tmp >/dev/null 2>&1; pe1=$?; popd >/dev/null 2>&1; pe2=$?; print -r "$pe1 $pe2""##);
        bulk_e_dirstack_count => (r#"$#dirstack"#, r#"print -r "ds0=$#dirstack"; pushd /tmp >/dev/null 2>&1; print -r "ds1=$#dirstack"; popd >/dev/null 2>&1"#);
        bulk_e_funcstack_depth_fn => (r#"funcstack in fn"#, r##"fse() { print -r "fsz=$(( 1 + $#funcstack ))"; }; fse"##);
        bulk_e_trap_exit_runs_last => (r#"trap EXIT"#, r##"trap 'print TRAPE' EXIT; print MAINE"##);
        bulk_e_alias_def_and_use => (r#"alias"#, r#"alias axe_e=print; axe_e alval_e; unalias axe_e; print postalias_e"#);
        bulk_e_cond_int_gt => (r#"[[ -gt ]]"#, r#"[[ 3 -gt 1 ]]; print gt_e=$?"#);
        bulk_e_cond_string_eq => (r#"[[ str eq ]]"#, r#"[[ xx_e = xx_e ]]; print eq_e=$?"#);
        bulk_e_cond_root_exists => (r#"[[ -e / ]]"#, r#"[[ -e / ]]; print ele_e=$?"#);
        bulk_e_cond_regex_match => (r#"[[ =~ ]]"#, r##"[[ abc_e =~ b ]]; print -r "ME=$MATCH""##);
        bulk_e_assoc_keys_k_flag => (r#"${(k)A}"#, r#"typeset -A ake=(ke ve); print ${(k)ake}"#);
        bulk_e_typeset_float_F_precision => (r#"typeset -F 3"#, r#"typeset -F 3 fe3=12.34567; print $fe3"#);
        bulk_e_typeset_integer_oct_output => (r#"typeset -i 8"#, r#"typeset -i 8 io8=10; print $io8"#);
        bulk_e_zmodload_zsh_system => (r#"zmodload zsh/system"#, r##"zmodload zsh/system 2>&1; print -r "ex=$?""##);
        bulk_e_autoload_zformat_run => (r#"autoload zformat"#, r##"autoload -Uz zformat 2>&1; zformat -f zfu_e '%s-%d' piece 3; print -r "$zfu_e""##);
        bulk_e_fn_return_zero => (r#"fn return 0"#, r#"fz_e() { return 0; }; fz_e; print r0e=$?"#);
        bulk_e_compound_and_after_brace => (r#"{ true } &&"#, r#"{ true; } && print braceand_e"#);
        bulk_e_set_hyphen_count_argv => (r#"set count"#, r##"set -- a_e 'b c'; argc_e=$#; print -r "$argc_e""##);
        bulk_e_read_ifs_colon_two_fields => (r#"IFS : read"#, r##"print -r 'u_e:v_e' | IFS=: read -r r1e r2e; print -r "$r1e $r2e""##);
        bulk_e_getopts_two_flags => (r#"getopts ab"#, r##"set -- -ab; OPTIND=1; getopts ab o1_e; g1=$o1_e; getopts ab o2_e; print -r "$g1 $o2_e""##);
        bulk_e_param_colon_offset_length => (r#"${s:2:2}"#, r#"soe=abcde; print ${soe:2:2}"#);
        bulk_e_param_colon_offset_rest => (r#"${s:3}"#, r#"sre=abcdef; print ${sre:3}"#);
        bulk_e_join_newline_j_flag => (r#"${(j.$'\n'.)a}"#, r#"jne=(l1 l2); print ${(j.$'\n'.)jne}"#);
        bulk_e_arith_mul_assign => (r#"arith *="#, r#"integer mae=3; (( mae *= 2 )); print $mae"#);
        bulk_e_arith_div_assign => (r#"arith /="#, r#"integer dae=8; (( dae /= 2 )); print $dae"#);
        bulk_e_arith_predecrement => (r#"--i"#, r#"integer pee=2; (( --pee )); print $pee"#);
        bulk_e_hyphen_parameter_flags => (r#"$-"#, r##"print -r "$-""##);
        bulk_e_cond_minus_o_noclobber => (r#"[[ -o noclobber ]]"#, r#"[[ -o noclobber ]]; print nce=$?"#);
        bulk_e_hist_literal_size => (r#"HISTSIZE"#, r#"print $HISTSIZE"#);
        bulk_e_savehist_literal => (r#"SAVEHIST"#, r#"print $SAVEHIST"#);
        bulk_e_plus_histfile => (r#"$+HISTFILE"#, r#"print $+HISTFILE"#);
        bulk_e_plus_wordchars => (r#"$+WORDCHARS"#, r#"print $+WORDCHARS"#);
        bulk_e_plus_sched => (r#"$+sched"#, r#"print $+sched"#);
        bulk_e_zle_list_ex => (r#"zle -l"#, r##"zle -l 2>&1; print -r "ex=$?""##);
        bulk_e_bindkey_list_keymaps => (r#"bindkey -l"#, r##"bindkey -l 2>&1; print -r "ex=$?""##);
        bulk_e_zmodload_zsh_complist => (r#"zmodload zsh/complist"#, r##"zmodload zsh/complist 2>&1; print -r "ex=$?""##);
        bulk_e_zmodload_zsh_zselect => (r#"zmodload zsh/zselect"#, r##"zmodload zsh/zselect 2>&1; print -r "ex=$?""##);
        bulk_e_zmodload_zsh_curses => (r#"zmodload zsh/curses"#, r##"zmodload zsh/curses 2>&1; print -r "ex=$?""##);
        bulk_e_print_zsh_name_version => (r#"ZSH_NAME"#, r##"print -r "$ZSH_NAME $ZSH_VERSION""##);
        bulk_e_argv0_default => (r#"ARGV0"#, r#"print ${ARGV0:-nil_argv0}"#);
        bulk_e_word_begin_end_match_arrays => (r#"mbegin mend"#, r##"[[ 123 =~ ([0-9]+) ]]; print -r "$#mbegin $#mend""##);
        bulk_e_zparseopts_capital_D => (r#"zparseopts -D"#, r##"typeset -a zpd_e=(); zparseopts -D -a zpd_e -- 2>&1; print -r "n=$#zpd_e ex=$?""##);
        bulk_e_opt_correctall => (r#"options[correctall]"#, r#"print $options[correctall]"#);
        bulk_e_opt_histallowclobber => (r#"options[histallowclobber]"#, r#"print $options[histallowclobber]"#);
        bulk_e_opt_magicequalsubst => (r#"options[magicequalsubst]"#, r#"print $options[magicequalsubst]"#);
        bulk_e_opt_recexact => (r#"options[recexact]"#, r#"print $options[recexact]"#);
        bulk_e_opt_warnnestedvar => (r#"options[warnnestedvar]"#, r#"print $options[warnnestedvar]"#);
        bulk_e_opt_trapsasync => (r#"options[trapsasync]"#, r#"print $options[trapsasync]"#);
        bulk_e_opt_pathscript => (r#"options[pathscript]"#, r#"print $options[pathscript]"#);
        bulk_e_opt_overstrike => (r#"options[overstrike]"#, r#"print $options[overstrike]"#);
        bulk_e_opt_notify => (r#"options[notify]"#, r#"print $options[notify]"#);
        bulk_e_opt_localpatterns => (r#"options[localpatterns]"#, r#"print $options[localpatterns]"#);
        bulk_e_opt_menucomplete => (r#"options[menucomplete]"#, r#"print $options[menucomplete]"#);
        bulk_e_opt_bgnice => (r#"options[bgnice]"#, r#"print $options[bgnice]"#);
        bulk_e_opt_checkrunningjobs => (r#"options[checkrunningjobs]"#, r#"print $options[checkrunningjobs]"#);
        bulk_e_opt_hashcmds => (r#"options[hashcmds]"#, r#"print $options[hashcmds]"#);
        bulk_e_opt_nomerge => (r#"options[nomerge]"#, r#"print $options[nomerge]"#);
        bulk_e_opt_completeinword => (r#"options[completeinword]"#, r#"print $options[completeinword]"#);
        bulk_e_opt_cshnullglob => (r#"options[cshnullglob]"#, r#"print $options[cshnullglob]"#);
        bulk_e_opt_interactive => (r#"options[interactive]"#, r#"print $options[interactive]"#);
        bulk_e_opt_zle => (r#"options[zle]"#, r#"print $options[zle]"#);
        bulk_e_opt_hashdirs => (r#"options[hashdirs]"#, r#"print $options[hashdirs]"#);
        bulk_e_opt_histbeep => (r#"options[histbeep]"#, r#"print $options[histbeep]"#);
        bulk_e_opt_histexpiredupsfirst => (r#"options[histexpiredupsfirst]"#, r#"print $options[histexpiredupsfirst]"#);
        bulk_e_opt_histfindnodups => (r#"options[histfindnodups]"#, r#"print $options[histfindnodups]"#);
        bulk_e_opt_histignoredups => (r#"options[histignoredups]"#, r#"print $options[histignoredups]"#);
        bulk_e_opt_histignorespace => (r#"options[histignorespace]"#, r#"print $options[histignorespace]"#);
        bulk_e_opt_histreduceblanks => (r#"options[histreduceblanks]"#, r#"print $options[histreduceblanks]"#);
        bulk_e_opt_histsavenodups => (r#"options[histsavenodups]"#, r#"print $options[histsavenodups]"#);
        bulk_e_opt_histsubstpattern => (r#"options[histsubstpattern]"#, r#"print $options[histsubstpattern]"#);
        bulk_e_opt_hup => (r#"options[hup]"#, r#"print $options[hup]"#);
        bulk_e_opt_ignorebraces => (r#"options[ignorebraces]"#, r#"print $options[ignorebraces]"#);
        bulk_e_opt_ignoreclosebraces => (r#"options[ignoreclosebraces]"#, r#"print $options[ignoreclosebraces]"#);
        bulk_e_opt_kshglob => (r#"options[kshglob]"#, r#"print $options[kshglob]"#);
        bulk_e_opt_longlistjobs => (r#"options[longlistjobs]"#, r#"print $options[longlistjobs]"#);
        bulk_e_opt_multibyte => (r#"options[multibyte]"#, r#"print $options[multibyte]"#);
        bulk_e_opt_promptcr => (r#"options[promptcr]"#, r#"print $options[promptcr]"#);
        bulk_e_opt_promptsp => (r#"options[promptsp]"#, r#"print $options[promptsp]"#);
        bulk_e_opt_pushdminus => (r#"options[pushdminus]"#, r#"print $options[pushdminus]"#);
        bulk_e_opt_pushdsilent => (r#"options[pushdsilent]"#, r#"print $options[pushdsilent]"#);
        bulk_e_opt_pushdtohome => (r#"options[pushdtohome]"#, r#"print $options[pushdtohome]"#);
        bulk_e_opt_rmstarsilent => (r#"options[rmstarsilent]"#, r#"print $options[rmstarsilent]"#);
        bulk_e_opt_rmstarwait => (r#"options[rmstarwait]"#, r#"print $options[rmstarwait]"#);
        bulk_e_opt_shnullcmd => (r#"options[shnullcmd]"#, r#"print $options[shnullcmd]"#);
        bulk_e_opt_shoptionletters => (r#"options[shoptionletters]"#, r#"print $options[shoptionletters]"#);
        bulk_e_opt_typesettounset => (r#"options[typesettounset]"#, r#"print $options[typesettounset]"#);
        bulk_e_opt_exec => (r#"options[exec]"#, r#"print $options[exec]"#);
        bulk_e_opt_evalunsafe => (r#"options[evalunsafe]"#, r#"print $options[evalunsafe]"#);
        bulk_e_opt_print8bit => (r#"options[print8bit]"#, r#"print $options[print8bit]"#);
        bulk_e_opt_cprecedences => (r#"options[cprecedences]"#, r#"print $options[cprecedences]"#);
    }
}

/// Sixth batch: **sorted `glob` in temp dir**, `printf %b`, **`print -C`**, **subshell `PWD`**, **`extendedglob` + `(#b)`**, **`ERR` trap**, **`typeset -Z`/`-L`**, **`zmodload zsh/{clone,attr,rlimits,regex}`**, **`autoload zmv`**, **more `$options`** (`posix*`, `privileged`, `emacs`/`vi`, etc.).
mod corpus_dash_fc_bulk_f {
    use super::*;

    parity_gap_tests! {
        bulk_f_sorted_glob_in_tmpdir => (r#"sorted glob tmpdir"#, r##"tdf=$(mktemp -d); ( builtin cd $tdf && touch ggc_ccf ggc_bbf ggc_aaf && print ${(on)*(N)} ); ec=$?; command rm -rf $tdf; exit $ec"##);
        bulk_f_printf_percent_b_escape => (r#"printf %b"#, r#"printf '%b\n' 'a\tb'"#);
        bulk_f_print_capital_C_columns => (r#"print -C"#, r#"print -C 2 a b c d e"#);
        bulk_f_cond_tty_fd_zero => (r#"[[ -t 0 ]]"#, r#"[[ -t 0 ]]; print tty0f=$?"#);
        bulk_f_tty_param_or_nil => (r#"TTY"#, r#"print ${TTY:-nil_tty}"#);
        bulk_f_umask_symbolic_capital_S => (r#"umask -S"#, r#"umask -S"#);
        bulk_f_disown_missing_job_ex => (r#"disown miss"#, r##"disown %nonexistent_job_f 2>/dev/null; print dexf=$?"##);
        bulk_f_hash_r_then_status => (r#"hash -r"#, r#"hash -r; print hrf=$?"#);
        bulk_f_builtin_print_word => (r#"builtin print"#, r#"builtin print bprint_f"#);
        bulk_f_zmodload_zsh_clone => (r#"zmodload zsh/clone"#, r##"zmodload zsh/clone 2>&1; print -r "ex=$?""##);
        bulk_f_zmodload_zsh_attr => (r#"zmodload zsh/attr"#, r##"zmodload zsh/attr 2>&1; print -r "ex=$?""##);
        bulk_f_zmodload_zsh_rlimits => (r#"zmodload zsh/rlimits"#, r##"zmodload zsh/rlimits 2>&1; print -r "ex=$?""##);
        bulk_f_zmodload_zsh_regex => (r#"zmodload zsh/regex"#, r##"zmodload zsh/regex 2>&1; print -r "ex=$?""##);
        bulk_f_local_scalar_top_level => (r#"local at top"#, r#"local ltxf=9; print $ltxf"#);
        bulk_f_subshell_builtin_cd_pwd => (r#"subshell PWD"#, r#"( builtin cd /tmp; print -r "subpwd=$PWD" )"#);
        bulk_f_tokenize_z_flag_count => (r#"${(z) }"#, r#"lzf='aa  bb'; print ${#${(z)lzf}}"#);
        bulk_f_autoload_zmv_function => (r#"autoload zmv"#, r##"autoload -U zmv 2>&1; print -r "zf=$+functions[zmv]""##);
        bulk_f_extglob_hash_b_capture => (r#"extendedglob #b"#, r##"setopt extendedglob; [[ xxf =~ (#b)(x*) ]]; print -r "mm=$match""##);
        bulk_f_array_numeric_sort_n => (r#"array (n) sort"#, r#"anf=(10 2 1); print ${(n)anf}"#);
        bulk_f_typeset_zero_pad_left => (r#"typeset -Z -L"#, r#"typeset -Z 4 -L lf4=3; print $lf4"#);
        bulk_f_case_pattern_prefix => (r#"case s*"#, r#"case strf in x*) print nope;; s*) print yesc;; esac"#);
        bulk_f_arith_logical_or_double => (r#"arith ||"#, r#"(( 0 || 1 )); print oor=$?"#);
        bulk_f_float_compare_equality => (r#"float 2.0 =="#, r#"(( 2.0 == 2 )); print feq=$?"#);
        bulk_f_join_empty_array => (r#"join empty"#, r#"ea=(); print ${(j.,.)ea}"#);
        bulk_f_whence_hyphen_w_builtin => (r#"whence -w true"#, r#"whence -w true"#);
        bulk_f_array_at_spread_argv => (r#"${(@)@}"#, r##"set -- axf byf; print -r "${(@)@}""##);
        bulk_f_unsetopt_localoptions_ex => (r#"unsetopt localoptions"#, r##"unsetopt localoptions 2>&1; print -r "uex=$?""##);
        bulk_f_limit_builtin_nominal => (r#"limit stacksize"#, r##"limit stacksize 2>&1; print -r "lex=$?""##);
        bulk_f_plus_ZLE => (r#"$+ZLE"#, r#"print $+ZLE"#);
        bulk_f_trap_ERR_after_false => (r#"trap ERR"#, r##"trap 'print TR_ERRF' ERR; false; print AFTER_ERRF"##);
        bulk_f_repeat_zero_times => (r#"repeat 0"#, r#"repeat 0; do print no_rep_f; done; print done_rep_f"#);
        bulk_f_arith_for_empty_then_break => (r#"for (( ;; )) break"#, r#"for ((;;)); do print oncef; break; done"#);
        bulk_f_param_M_hash_pattern => (r#"${(M)#}"#, r#"spf=testf; [[ -n ${(M)spf:#t*f} ]]; print mpf=$?"#);
        bulk_f_named_regex_match_groups => (r#"MATCH match[1]"#, r##"[[ abc =~ (b) ]]; print -r "mf=$MATCH rf=$match[1]""##);
        bulk_f_printf_width_string => (r#"printf %6s"#, r#"printf '%6s\n' x"#);
        bulk_f_print_one_two_three_loop => (r#"for 1..3 print"#, r#"for i in 1 2 3; do print $i; done"#);
        bulk_f_string_index_first_r => (r#"[(r) ]"#, r#"ixy=(a x c); print ${ixy[(r)x]}"#);
        bulk_f_typeset_hide_capital_H => (r#"typeset -H"#, r##"typeset -H hidf=secretf; print ${+hidf}"##);
        bulk_f_zmodload_list_loaded_e => (r#"zmodload -e"#, r##"zmodload -e zsh/zle 2>&1; print -r "zee=$?""##);
        bulk_f_bindkey_minus_l_ex => (r#"bindkey -L"#, r##"bindkey -L 2>&1; print -r "bkl=$?""##);
        bulk_f_zstyle_list_styles_ex => (r#"zstyle -L"#, r##"zstyle -L 2>/dev/null; print -r "zsx=$?""##);
        bulk_f_colon_minus_assign_in_word => (r#": ${x:=d}"#, r#"unset cwv_f; : ${cwv_f:=assignedf}; print $cwv_f"#);
        bulk_f_nested_arith_parens => (r#"(( (1+2)*3 ))"#, r#"print $(( (1 + 2) * 3 ))"#);
        bulk_f_boolean_true_false_params => (r#"$true $false"#, r##"print -r "$true $false""##);
        bulk_f_commands_assoc_echo => (r#"$commands[echo]"#, r#"print ${commands[echo]:-no_path_echo}"#);
        bulk_f_options_associative_count => (r#"${#options}"#, r#"print ${#options}"#);
        bulk_f_parameters_plus => (r#"$+parameters"#, r#"print $+parameters"#);
        bulk_f_galiases_table_plus => (r#"$+galiases"#, r#"print $+galiases"#);
        bulk_f_modules_array_first => (r#"modules[1]"#, r#"print ${modules[1]:-no_mod1}"#);
        bulk_f_opt_aliases => (r#"options[aliases]"#, r#"print $options[aliases]"#);
        bulk_f_opt_bashrematch => (r#"options[bashrematch]"#, r#"print $options[bashrematch]"#);
        bulk_f_opt_banghist => (r#"options[banghist]"#, r#"print $options[banghist]"#);
        bulk_f_opt_emacs => (r#"options[emacs]"#, r#"print $options[emacs]"#);
        bulk_f_opt_vi => (r#"options[vi]"#, r#"print $options[vi]"#);
        bulk_f_opt_privileged => (r#"options[privileged]"#, r#"print $options[privileged]"#);
        bulk_f_opt_restricted => (r#"options[restricted]"#, r#"print $options[restricted]"#);
        bulk_f_opt_posixbuiltins => (r#"options[posixbuiltins]"#, r#"print $options[posixbuiltins]"#);
        bulk_f_opt_posixcd => (r#"options[posixcd]"#, r#"print $options[posixcd]"#);
        bulk_f_opt_posixstrings => (r#"options[posixstrings]"#, r#"print $options[posixstrings]"#);
        bulk_f_opt_posixtraps => (r#"options[posixtraps]"#, r#"print $options[posixtraps]"#);
        bulk_f_opt_posixaliases => (r#"options[posixaliases]"#, r#"print $options[posixaliases]"#);
        bulk_f_opt_posixargzero => (r#"options[posixargzero]"#, r#"print $options[posixargzero]"#);
        bulk_f_opt_listrowsfirst => (r#"options[listrowsfirst]"#, r#"print $options[listrowsfirst]"#);
        bulk_f_opt_globcomplete => (r#"options[globcomplete]"#, r#"print $options[globcomplete]"#);
        bulk_f_opt_login => (r#"options[login]"#, r#"print $options[login]"#);
        bulk_f_opt_localtraps => (r#"options[localtraps]"#, r#"print $options[localtraps]"#);
        bulk_f_opt_histstripdots => (r#"options[histstripdots]"#, r#"print $options[histstripdots]"#);
        bulk_f_opt_histnoflocks => (r#"options[histnoflocks]"#, r#"print $options[histnoflocks]"#);
        bulk_f_opt_histnoflush => (r#"options[histnoflush]"#, r#"print $options[histnoflush]"#);
        bulk_f_opt_autocontinue => (r#"options[autocontinue]"#, r#"print $options[autocontinue]"#);
        bulk_f_opt_cdsilent => (r#"options[cdsilent]"#, r#"print $options[cdsilent]"#);
        bulk_f_opt_chasesymlinks => (r#"options[chasesymlinks]"#, r#"print $options[chasesymlinks]"#);
        bulk_f_opt_clobberempty => (r#"options[clobberempty]"#, r#"print $options[clobberempty]"#);
        bulk_f_opt_errbeforecmd => (r#"options[errbeforecmd]"#, r#"print $options[errbeforecmd]"#);
        bulk_f_opt_debugbeforecmd => (r#"options[debugbeforecmd]"#, r#"print $options[debugbeforecmd]"#);
        bulk_f_opt_braceclobber => (r#"options[braceclobber]"#, r#"print $options[braceclobber]"#);
        bulk_f_opt_alwaystoend => (r#"options[alwaystoend]"#, r#"print $options[alwaystoend]"#);
    }
}

/// Seventh batch: **`zmodload` terminfo/termcap/pcre/files**, **`read -A`**, **`typeset -g` / `-n`**, **`pipestatus`**, bit ops, **`strftime`**, **`[[ -ot | -nt | -ef ]]`** in `mktemp`, **`coproc`**, table sizes **`${#parameters}`**, **RC quotes**, more **`$options`** (`autolist`, `histappend`, `glob`, …).
mod corpus_dash_fc_bulk_g {
    use super::*;

    parity_gap_tests! {
        bulk_g_zmodload_zsh_terminfo => (r#"zmodload zsh/terminfo"#, r##"zmodload zsh/terminfo 2>&1; print -r "ex=$?""##);
        bulk_g_zmodload_zsh_termcap => (r#"zmodload zsh/termcap"#, r##"zmodload zsh/termcap 2>&1; print -r "ex=$?""##);
        bulk_g_zmodload_zsh_pcre => (r#"zmodload zsh/pcre"#, r##"zmodload zsh/pcre 2>&1; print -r "ex=$?""##);
        bulk_g_zmodload_zsh_files => (r#"zmodload zsh/files"#, r##"zmodload zsh/files 2>&1; print -r "ex=$?""##);
        bulk_g_zmodload_zsh_sched_module => (r#"zmodload zsh/sched"#, r##"zmodload zsh/sched 2>&1; print -r "ex=$?""##);
        bulk_g_zmodload_zsh_watch => (r#"zmodload zsh/watch"#, r##"zmodload zsh/watch 2>&1; print -r "ex=$?""##);
        bulk_g_zmodload_zsh_db_gdbm => (r#"zmodload zsh/db_gdbm"#, r##"zmodload zsh/db_gdbm 2>&1; print -r "ex=$?""##);
        bulk_g_typeset_global_scalar => (r#"typeset -g"#, r#"typeset -g tgg_scalar=global_g; print $tgg_scalar"#);
        bulk_g_array_subscript_I_index => (r#"[(I) ] elem"#, r#"aig=(p q r); print ${aig[(I)q]}"#);
        bulk_g_brace_combo_two_by_two => (r#"X{a,b}Y{c}"#, r#"print -r X{a,b}Y{c}"#);
        bulk_g_read_capital_A_words => (r#"read -A"#, r#"read -A rag <<< 'pg qg rg'; print $rag[2]"#);
        bulk_g_pipestatus_three_cmds => (r#"pipestatus x3"#, r##"true|false|true; print -r "$pipestatus[1] $pipestatus[2] $pipestatus[3]""##);
        bulk_g_pipe_left_subshell_exit => (r#"( exit 3 ) |"#, r##"( exit 3 ) | true; print -r "$pipestatus[1] $pipestatus[2]""##);
        bulk_g_arith_bit_shl_assign => (r#"arith <<="#, r#"integer bshg=2; (( bshg <<= 3 )); print $bshg"#);
        bulk_g_arith_bit_shr_assign => (r#"arith >>="#, r#"integer bshr=32; (( bshr >>= 2 )); print $bshr"#);
        bulk_g_arith_bit_and => (r#"7 & 3"#, r#"print $(( 7 & 3 ))"#);
        bulk_g_arith_bit_or_pipe => (r#"4 | 2"#, r#"print $(( 4 | 2 ))"#);
        bulk_g_strftime_epoch_zero => (r#"strftime %s 0"#, r##"zmodload zsh/datetime 2>/dev/null; strftime %s 0; print stfg=$?"##);
        bulk_g_nameref_typeset_n => (r#"typeset -n"#, r#"typeset -n nrg=trg_g; trg_g=valn_g; print $nrg"#);
        bulk_g_mktemp_rel_path_glob => (r#"tmpd glob one file"#, r##"tdg=$(mktemp -d); mkdir -p $tdg/d_g; touch $tdg/d_g/f_g; ( builtin cd $tdg && print d_g/f_g ); ec=$?; command rm -rf $tdg; exit $ec"##);
        bulk_g_coproc_builtin_true => (r#"coproc true"#, r##"coproc true; print -r "cpid=$!""##);
        bulk_g_zsh_subshell_depth => (r#"ZSH_SUBSHELL"#, r#"print $ZSH_SUBSHELL"#);
        bulk_g_whence_p_missing_binary => (r#"whence -p missing"#, r##"whence -p __missing_bin_g__ 2>/dev/null; print wex=$?""##);
        bulk_g_hash_no_such_command => (r#"hash missing"#, r##"hash __no_such_cmd_g__ 2>/dev/null; print hex=$?""##);
        bulk_g_builtin_pwd_logical => (r#"pwd -L"#, r#"builtin pwd -L"#);
        bulk_g_command_pwd_builtin => (r#"command pwd"#, r#"command pwd"#);
        bulk_g_cd_quiet_flag_root => (r#"cd -q /"#, r##"builtin cd -q / 2>&1; print cqx=$?""##);
        bulk_g_count_parameters_assoc => (r#"${#parameters}"#, r#"print ${#parameters}"#);
        bulk_g_count_aliases_assoc => (r#"${#aliases}"#, r#"print ${#aliases}"#);
        bulk_g_count_functions_assoc => (r#"${#functions}"#, r#"print ${#functions}"#);
        bulk_g_lineno_two_prints => (r#"LINENO twice"#, r##"print -r "L1=$LINENO"; print -r "L2=$LINENO""##);
        bulk_g_cond_same_file_ef => (r#"[[ -ef ]]"#, r##"[[ /etc/hosts -ef /etc/hosts ]]; print efg=$?""##);
        bulk_g_cond_ot_nt_mktemp => (r#"[[ -ot -nt ]]"#, r##"tdc=$(mktemp -d); touch $tdc/og; touch $tdc/ng; [[ $tdc/og -ot $tdc/ng ]]; otw=$?; [[ $tdc/ng -nt $tdc/og ]]; ntw=$?; print -r "$otw $ntw"; command rm -rf $tdc"##);
        bulk_g_float_division_print => (r#"5.0/2"#, r#"print $(( 5.0 / 2 ))"#);
        bulk_g_typeset_F_and_E => (r#"typeset -F -E"#, r#"typeset -F ff1_g=2.5 -E ff2_g=3e0; print $ff1_g $ff2_g"#);
        bulk_g_array_sort_oa_flag => (r#"sort (oa)"#, r#"aog=(3 1 2); print ${(oa)aog}"#);
        bulk_g_octal_escape_dollar_quote => (r#"$'\\101'"#, r#"print -r $'\\101'"#);
        bulk_g_rcquotes_setopt_print => (r#"rcquotes"#, r##"setopt rcquotes; print 'it''s-rcg'"##);
        bulk_g_cond_and_chain_z => (r#"[[ && ]]"#, r#"[[ -z '' && -n x ]]; print czg=$?"#);
        bulk_g_arith_compare_chain => (r#"(( < && < ))"#, r#"(( 1 < 2 && 2 < 3 )); print chg=$?"#);
        bulk_g_export_scalar_word => (r#"export"#, r#"export exg_w=77; print $exg_w"#);
        bulk_g_readonly_array_typeset_ar => (r#"typeset -ar"#, r#"typeset -ar rar_g=(solo_g); print $rar_g[1]"#);
        bulk_g_int_div_trunc => (r#"5/2 int"#, r#"print $(( 5 / 2 ))"#);
        bulk_g_colon_ternary_arith => (r#"?: arith"#, r#"print $(( 0 ? 2 : 9 ))"#);
        bulk_g_assoc_unset_key_plus => (r#"unset 'A[k]'"#, r#"typeset -A aug=(kk vv); unset 'aug[kk]'; print ${+aug[kk]}"#);
        bulk_g_pad_left_l_colon => (r#"pad (l:5)"#, r#"print ${(l:5::0:)7}"#);
        bulk_g_pad_right_r_colon => (r#"pad (r:5)"#, r#"print ${(r:5::0:)7}"#);
        bulk_g_print_r_double_dashed => (r#"print -r -- --"#, r#"print -r -- '--lead'"#);
        bulk_g_terminfo_colors_key => (r#"terminfo colors"#, r#"print ${terminfo[colors]:-0}"#);
        bulk_g_plus_terminfo_bracket => (r#"$+terminfo"#, r#"print $+terminfo"#);
        bulk_g_array_union_pipe => (r#"a | b array"#, r#"ug1=(1 2 3); ug2=(2 9); print ${ug1:|ug2}"#);
        bulk_g_subscript_offset_Ie => (r#"[(Ie)]"#, r#"setopt extendedglob; asg=(foo bar baz); print ${asg[(Ie)f*]}"#);
        bulk_g_disable_builtin_hash_r => (r#"disable hash"#, r##"disable hash 2>&1; print dexh=$?; enable hash 2>&1; print enxh=$?"##);
        bulk_g_float_modulo => (r#"float mod"#, r#"print $(( 5.5 % 2 ))"#);
        bulk_g_string_length_dollar_quote_tabs => (r#"${#} tab word"#, r#"wug=$'x\tb'; print ${#wug}"#);
        bulk_g_opt_autolist => (r#"options[autolist]"#, r#"print $options[autolist]"#);
        bulk_g_opt_automenu => (r#"options[automenu]"#, r#"print $options[automenu]"#);
        bulk_g_opt_autonamedirs => (r#"options[autonamedirs]"#, r#"print $options[autonamedirs]"#);
        bulk_g_opt_autoparamkeys => (r#"options[autoparamkeys]"#, r#"print $options[autoparamkeys]"#);
        bulk_g_opt_autoresume => (r#"options[autoresume]"#, r#"print $options[autoresume]"#);
        bulk_g_opt_bashautolist => (r#"options[bashautolist]"#, r#"print $options[bashautolist]"#);
        bulk_g_opt_cshjunkiehistory => (r#"options[cshjunkiehistory]"#, r#"print $options[cshjunkiehistory]"#);
        bulk_g_opt_evallineno => (r#"options[evallineno]"#, r#"print $options[evallineno]"#);
        bulk_g_opt_forcefloat => (r#"options[forcefloat]"#, r#"print $options[forcefloat]"#);
        bulk_g_opt_glob => (r#"options[glob]"#, r#"print $options[glob]"#);
        bulk_g_opt_globalexport => (r#"options[globalexport]"#, r#"print $options[globalexport]"#);
        bulk_g_opt_globalrcs => (r#"options[globalrcs]"#, r#"print $options[globalrcs]"#);
        bulk_g_opt_globstarshort => (r#"options[globstarshort]"#, r#"print $options[globstarshort]"#);
        bulk_g_opt_hashall => (r#"options[hashall]"#, r#"print $options[hashall]"#);
        bulk_g_opt_hashexecutablesonly => (r#"options[hashexecutablesonly]"#, r#"print $options[hashexecutablesonly]"#);
        bulk_g_opt_histappend => (r#"options[histappend]"#, r#"print $options[histappend]"#);
        bulk_g_opt_histexpand => (r#"options[histexpand]"#, r#"print $options[histexpand]"#);
        bulk_g_opt_histlexwords => (r#"options[histlexwords]"#, r#"print $options[histlexwords]"#);
        bulk_g_opt_histnofunctions => (r#"options[histnofunctions]"#, r#"print $options[histnofunctions]"#);
        bulk_g_opt_incappendhistorytime => (r#"options[incappendhistorytime]"#, r#"print $options[incappendhistorytime]"#);
        bulk_g_opt_listambiguous => (r#"options[listambiguous]"#, r#"print $options[listambiguous]"#);
        bulk_g_opt_listbeep => (r#"options[listbeep]"#, r#"print $options[listbeep]"#);
        bulk_g_opt_listpacked => (r#"options[listpacked]"#, r#"print $options[listpacked]"#);
        bulk_g_opt_listtypes => (r#"options[listtypes]"#, r#"print $options[listtypes]"#);
        bulk_g_opt_localloops => (r#"options[localloops]"#, r#"print $options[localloops]"#);
        bulk_g_opt_log => (r#"options[log]"#, r#"print $options[log]"#);
        bulk_g_opt_kshautoload => (r#"options[kshautoload]"#, r#"print $options[kshautoload]"#);
        bulk_g_opt_kshtypeset => (r#"options[kshtypeset]"#, r#"print $options[kshtypeset]"#);
        bulk_g_opt_multifuncdef => (r#"options[multifuncdef]"#, r#"print $options[multifuncdef]"#);
        bulk_g_opt_mailpath => (r#"options[mailpath]"#, r#"print $options[mailpath]"#);
        bulk_g_opt_proxynext => (r#"options[proxynext]"#, r#"print $options[proxynext]"#);
    }
}

/// Eighth batch: remaining **`$options[…]`** keys not yet read in-file, plus **process substitution**, **`zcompile`**, **`zsh/mathfunc`**, **`sched` / `bindkey`**, **`#`-flags**, **`typeset -aU`**, **`getopts` errors**, **`[[ -v ]]`**, **`dirs -c`**, **RC / CSH-ish**, **`emulate csh`**, **`HISTCMD` / signals**.
mod corpus_dash_fc_bulk_h {
    use super::*;

    parity_gap_tests! {
        bulk_h_zmodload_zsh_compctl => (r#"zmodload zsh/compctl"#, r##"zmodload zsh/compctl 2>&1; print -r "ex=$?""##);
        bulk_h_zmodload_zsh_net_tcp => (r#"zmodload zsh/net/tcp"#, r##"zmodload zsh/net/tcp 2>&1; print -r "ex=$?""##);
        bulk_h_zmodload_zsh_param_private => (r#"zmodload zsh/param/private"#, r##"zmodload zsh/param/private 2>&1; print -r "ex=$?""##);
        bulk_h_zmodload_zsh_mathfunc_sin => (r#"mathfunc sin(0)"#, r##"zmodload zsh/mathfunc 2>/dev/null; print $(( sin(0) ))"##);
        bulk_h_arith_char_code_double_hash => (r#"##A"#, r#"print $(( ##A ))"#);
        bulk_h_cond_case_insensitive_pat => (r#"(#i) ="#, r##"setopt extendedglob; [[ ABC = (#i)abc ]]; print cih=$?"##);
        bulk_h_getopts_invalid_flag => (r#"getopts bad flag"#, r##"set -- -z; OPTIND=1; getopts 'a:' oh 2>/dev/null; print gbx=$?""##);
        bulk_h_printf_q_empty_arg => (r#"printf %q empty"#, r#"printf '%q\n' ''"#);
        bulk_h_mktemp_symlink_minus_L => (r#"[[ -L symlink ]]"#, r##"tdh=$(mktemp -d); touch $tdh/tgh; ln -s tgh $tdh/lgh; [[ -L $tdh/lgh ]]; print slh=$?; command rm -rf $tdh"##);
        bulk_h_zcompile_then_rm_zwc => (r#"zcompile tmp"#, r##"tfh=$(mktemp); print 'print ZCF_H' >$tfh; zcompile $tfh 2>&1; zceh=$?; command rm -f $tfh $tfh.zwc; print zceh"##);
        bulk_h_func_plus_funcfiletrace => (r#"funcfiletrace"#, r##"ffth() { print -r "fft=$#funcfiletrace"; }; ffth"##);
        bulk_h_percent_subst_capital_N => (r#"${(%):-%N}"#, r#"print ${(%):-%N}"#);
        bulk_h_read_delim_nul => (r#"read -d NUL"#, r##"read -d $'\\0' r0h <<< $'A\\0B'; print -r "$r0h""##);
        bulk_h_unset_array_second_elt => (r#"unset a[2]"#, r##"typeset -a uah=(xh yh zh); unset 'uah[2]'; print -r "$#uah $uah[1] $uah[3]"##);
        bulk_h_join_array_pipe_delim => (r#"join | array"#, r#"jah=( {1..3} ); print ${(j:|:)jah}"#);
        bulk_h_cond_executable_root_shell => (r#"[[ -x /bin/sh ]]"#, r##"[[ -x /bin/sh ]] || [[ -x /bin/bash ]]; print exsh=$?""##);
        bulk_h_cond_readable_hosts => (r#"[[ -r /etc/hosts ]]"#, r##"[[ -r /etc/hosts ]] || [[ -r /etc/hostname ]]; print rdh=$?""##);
        bulk_h_pipe_while_read_line => (r#"pipe | while read"#, r##"print wlh | while read -r wrh; do print -r "$wrh"; done"##);
        bulk_h_proc_subst_read_print => (r#"read < <(...)"#, r##"read -r lps < <(print psline_h); print -r "$lps""##);
        bulk_h_emulate_csh_minus_L => (r#"emulate csh"#, r##"emulate csh -L 2>&1; print emch=$?""##);
        bulk_h_unset_hyphen_f_fn => (r#"unset -f"#, r##"wth() { print x; }; unset -f wth; print ${+functions[wth]}"##);
        bulk_h_hash_f_rehash => (r#"hash -f"#, r#"hash -f; print hfh=$?"#);
        bulk_h_print_capital_E_no_escape => (r#"print -E"#, r#"print -E '*lit_h*'"#);
        bulk_h_typeset_integer_hex_16sharp => (r#"16#ff assign"#, r#"typeset -i xh=16#ff; print $xh"#);
        bulk_h_pushd_once_then_pop => (r#"pushd popd stack"#, r##"pushd /tmp >/dev/null 2>&1; print -r "ds=$#dirstack"; popd >/dev/null 2>&1; print popx=$?""##);
        bulk_h_dirs_clear_stack => (r#"dirs -c"#, r##"dirs -c 2>&1; print dch=$?""##);
        bulk_h_for_paren_brace_range => (r#"for ({1..2})"#, r#"for i ({1..2}); do print $i; done"#);
        bulk_h_case_glob_paren_pattern => (r#"case (p*)"#, r#"case ph in (p*) print hit_h;; esac"#);
        bulk_h_assoc_keys_M_matching => (r#"M on assoc keys"#, r#"typeset -A amh=(kx vx ky vy); print ${(Mk)amh:#k*}"#);
        bulk_h_ifs_split_comma_line => (r#"IFS , split"#, r#"lineh=a,b,c; IFS=,; aryh=(${(s:,:)lineh}); print $#aryh"#);
        bulk_h_bracket_v_unset_then_set => (r#"[[ -v ]]"#, r##"[[ -v vuh ]]; print v1=$?; vuh=1; [[ -v vuh ]]; print v2=$?""##);
        bulk_h_signals_bracket_name => (r#"signals[SIGINT]"#, r#"print ${signals[SIGINT]:-no_sig}"#);
        bulk_h_cd_chain_oldpwd => (r#"cd /; cd -"#, r##"cd /tmp; cd /; cd - >/dev/null; [[ $PWD = /tmp ]] || [[ $PWD = *tmp ]]; print cback=$?""##);
        bulk_h_test_posix_one_eq => (r#"test -eq"#, r#"test 1 -eq 1 && print t_ok_h"#);
        bulk_h_cond_or_string_equal => (r#"[[ -n || = ]]"#, r##"emptyh=; [[ -n $emptyh || xx = xx ]]; print orc=$?""##);
        bulk_h_print_HOST_scalar => (r#"HOST"#, r#"print ${HOST:-nohost}"#);
        bulk_h_print_LOGNAME_scalar => (r#"LOGNAME"#, r#"print ${LOGNAME:-nilog}"#);
        bulk_h_HISTCMD_integer => (r#"HISTCMD"#, r#"print $HISTCMD"#);
        bulk_h_zmodload_zsh_complete => (r#"zmodload zsh/complete"#, r##"zmodload zsh/complete 2>&1; print -r "ex=$?""##);
        bulk_h_zmodload_zsh_compwid => (r#"zmodload zsh/compwid"#, r##"zmodload zsh/compwid 2>&1; print -r "ex=$?""##);
        bulk_h_zmodload_zsh_zprof => (r#"zmodload zsh/zprof"#, r##"zmodload zsh/zprof 2>&1; print -r "ex=$?""##);
        bulk_h_typeset_array_Unique => (r#"typeset -aU"#, r#"typeset -aU auh=(x x y); print $auh"#);
        bulk_h_nullglob_star_N_in_tmp => (r#"nullglob *(N) argc"#, r##"tdn=$(mktemp -d); ( builtin cd $tdn && setopt nullglob && set -- *(N) && print argc=$# ); command rm -rf $tdn"##);
        bulk_h_bindkey_delete_byte => (r#"bindkey ^?"#, r##"bindkey '^?' 2>&1; print bkh=$?""##);
        bulk_h_sched_builtin_list => (r#"sched"#, r##"sched 2>&1; print sch=$?""##);
        bulk_h_param_e_double_expand => (r#"${(e):- }"#, r##"xeh=88; print ${(e):-v is $xeh}"##);
        bulk_h_cond_sized_file_hosts => (r#"[[ -s hosts ]]"#, r##"[[ -s /etc/hosts ]] || [[ -s /etc/hostname ]]; print szf=$?""##);
        bulk_h_unsetopt_errexit_after_false => (r#"unsetopt errexit"#, r##"setopt errexit; unsetopt errexit; false; print still_h"##);
        bulk_h_param_Q_unquote => (r#"${(Q) }"#, r#"wqh='"hi"'; print ${(Q)wqh}"#);
        bulk_h_qqq_triple_quote_param => (r#"qqq words"#, r#"wq3='two words'; print ${(qqq)wq3}"#);
        bulk_h_glob_plain_files_only_tmp => (r#"*(.) one file"#, r##"tfg=$(mktemp -d); touch $tfg/only_f; mkdir $tfg/subd; ( builtin cd $tfg && print *(.) ); command rm -rf $tfg"##);
        bulk_h_anon_fn_local_wipes_outer => (r#"() { local }"#, r##"lh_o=outer; () { local lh_o=inner; }; print ${lh_o}"##);
        bulk_h_print_TRY_BLOCK_ERROR => (r#"TRY_BLOCK_ERROR"#, r##"false; print -r "tbe=$TRY_BLOCK_ERROR""##);
        bulk_h_zformat_autoload_ping => (r#"zformat once"#, r##"autoload -Uz zformat 2>&1; zformat -f zfh '%s' ping; print -r "$zfh""##);
        bulk_h_arith_ternary_nested => (r#"?: nested"#, r#"print $(( 1 ? (0 ? 3 : 4) : 5 ))"#);
        bulk_h_scalar_hashhash_tail => (r#"##*/ tail"#, r#"svh=zz/yy/xx; print ${svh##*/}"#);
        bulk_h_print_dirstack_top => (r#"dirstack[1]"#, r#"print ${dirstack[1]:-no_ds}"#);
        bulk_h_setopt_warnnestedvar_noop => (r#"setopt warnnestedvar"#, r##"setopt warnnestedvar 2>&1; print wnx=$?"##);
        bulk_h_command_autoload_true => (r#"command autoload"#, r##"command autoload 2>&1; print acax=$?"##);
        bulk_h_false_true_colon_chain => (r#": : false true"#, r#":; :; false; true; print cch=$?"#);
        bulk_h_tilde_string_assign => (r#"~ in assignment"#, r##"th=~; print -r "tilde=${th:0:1}""##);
        bulk_h_logical_not_in_arith => (r#"! in (( ))"#, r#"print $(( !0 + !1 ))"#);
        bulk_h_array_insert_subscript => (r#"a[2]= mid"#, r##"typeset -a ins=(a c); ins[2]=b; print -r "$ins""##);
        bulk_h_hyphen_bracket_nzero => (r#"[[ -n 0 ]]"#, r#"[[ -n 0 ]]; print nz0=$?"#);
        bulk_h_zparseopts_end_of_opts => (r#"zparseopts --"#, r##"typeset -a zph=(); set -- -- -x; zparseopts -a zph - -- 2>&1; print -r "n=$#zph""##);
        bulk_h_opt_casematch => (r#"options[casematch]"#, r#"print $options[casematch]"#);
        bulk_h_opt_cshnullcmd => (r#"options[cshnullcmd]"#, r#"print $options[cshnullcmd]"#);
        bulk_h_opt_histignorealldups => (r#"options[histignorealldups]"#, r#"print $options[histignorealldups]"#);
        bulk_h_opt_localoptions => (r#"options[localoptions]"#, r#"print $options[localoptions]"#);
        bulk_h_opt_mailwarn => (r#"options[mailwarn]"#, r#"print $options[mailwarn]"#);
        bulk_h_opt_onecmd => (r#"options[onecmd]"#, r#"print $options[onecmd]"#);
        bulk_h_opt_physical => (r#"options[physical]"#, r#"print $options[physical]"#);
        bulk_h_opt_pipefail => (r#"options[pipefail]"#, r#"print $options[pipefail]"#);
        bulk_h_opt_posixjobs => (r#"options[posixjobs]"#, r#"print $options[posixjobs]"#);
        bulk_h_opt_printeightbit => (r#"options[printeightbit]"#, r#"print $options[printeightbit]"#);
        bulk_h_opt_promptvars => (r#"options[promptvars]"#, r#"print $options[promptvars]"#);
        bulk_h_opt_rcs => (r#"options[rcs]"#, r#"print $options[rcs]"#);
        bulk_h_opt_shfileexpansion => (r#"options[shfileexpansion]"#, r#"print $options[shfileexpansion]"#);
        bulk_h_opt_shortrepeat => (r#"options[shortrepeat]"#, r#"print $options[shortrepeat]"#);
        bulk_h_opt_shwordsplit => (r#"options[shwordsplit]"#, r#"print $options[shwordsplit]"#);
        bulk_h_opt_singlelinezle => (r#"options[singlelinezle]"#, r#"print $options[singlelinezle]"#);
        bulk_h_opt_stdin => (r#"options[stdin]"#, r#"print $options[stdin]"#);
        bulk_h_opt_trackall => (r#"options[trackall]"#, r#"print $options[trackall]"#);
    }
}

/// Ninth batch: **table sizes** (`${#commands}`, `${#builtins}`, …), **`ZSH_*` / `LANG` / `MACHTYPE`** reads,
/// extra **`zmodload`** probes (**`zsh/nearcolor`**, **`zsh/attr`**, **`zsh/clone`**), **`fc -p` / `fc -P`**, **`coproc` + `wait`**,
/// **`getopts`**, **`trap EXIT`**, background **`wait $!`**, **arith** (`**`, `[#16]`, `%`, xor), **expansion** (`(U)` / `(L)`, `${sv:n[:m]}`, `${ary:-1}`, `${ary:#pat}`, `(s:.:.)`, `(ok)`, `(Oa)`, `(u)`),
/// **`[[ -e / ]]`**, **`ulimit -n`**, **`compaudit`**, **`options[rc_expand_param]`**, and assorted builtins/parameters.
mod corpus_dash_fc_bulk_i {
    use super::*;

    parity_gap_tests! {
        bulk_i_zmodload_zsh_nearcolor => (r#"zmodload zsh/nearcolor"#, r##"zmodload zsh/nearcolor 2>&1; print -r "ex=$?""##);
        bulk_i_zmodload_zsh_attr => (r#"zmodload zsh/attr"#, r##"zmodload zsh/attr 2>&1; print -r "ex=$?""##);
        bulk_i_zmodload_zsh_clone => (r#"zmodload zsh/clone"#, r##"zmodload zsh/clone 2>&1; print -r "ex=$?""##);
        bulk_i_hash_num_commands => (r#"count commands"#, r#"print ${#commands}"#);
        bulk_i_hash_num_patchars => (r#"count patchars"#, r#"print ${#patchars}"#);
        bulk_i_hash_num_fpath => (r#"count fpath"#, r#"print ${#fpath}"#);
        bulk_i_hash_num_path => (r#"count path"#, r#"print ${#path}"#);
        bulk_i_hash_num_dis_builtins => (r#"count dis_builtins"#, r#"print ${#dis_builtins}"#);
        bulk_i_hash_num_builtins => (r#"count builtins"#, r#"print ${#builtins}"#);
        bulk_i_hash_num_widgets => (r#"count widgets"#, r#"print ${#widgets}"#);
        bulk_i_param_ZSH_EXEC_CONTEXT => (r#"ZSH_EXEC_CONTEXT"#, r#"print $ZSH_EXEC_CONTEXT"#);
        bulk_i_param_ZSH_ARGZERO => (r#"ZSH_ARGZERO"#, r#"print ${ZSH_ARGZERO:-no_zarg}"#);
        bulk_i_param_CPUTYPE => (r#"CPUTYPE"#, r#"print ${CPUTYPE:-nocpu}"#);
        bulk_i_param_HOSTTYPE => (r#"HOSTTYPE"#, r#"print ${HOSTTYPE:-nohostt}"#);
        bulk_i_param_PPID => (r#"PPID"#, r#"print $PPID"#);
        bulk_i_param_EGID => (r#"EGID"#, r#"print $EGID"#);
        bulk_i_param_GID => (r#"GID"#, r#"print $GID"#);
        bulk_i_param_SHLVL => (r#"SHLVL"#, r#"print $SHLVL"#);
        bulk_i_argv_subscript_slice => (r#"$@[2,-1]"#, r#"set -- a b c d; print $@[2,-1]"#);
        bulk_i_brace_sequence_descending => (r#"{10..1}"#, r#"print {10..1}"#);
        bulk_i_for_double_vars_pairs => (r#"for i j pairs"#, r#"for i j in 1 2 3 4; do print -r "$i-$j"; done"#);
        bulk_i_until_counter => (r#"until (( ))"#, r#"ui=0; until (( ui >= 2 )); do ui=$((ui+1)); done; print $ui"#);
        bulk_i_repeat_body_count => (r#"repeat 3"#, r#"integer ri=0; repeat 3; do ri=$((ri+1)); done; print $ri"#);
        bulk_i_typeset_float_precision_cap_f => (r#"typeset -F 6"#, r#"typeset -F 6 ffi_i=1.23456789; print $ffi_i"#);
        bulk_i_typeset_zero_pad_width => (r#"typeset -Z5"#, r#"typeset -Z5 -i zzi_i=7; print $zzi_i"#);
        bulk_i_typeset_integer_output_hex => (r#"typeset -i16"#, r#"typeset -i16 izi_i=15; print $izi_i"#);
        bulk_i_float_plus_eq => (r#"float +="#, r#"float fpi_i=1.5; fpi_i+=0.25; print $fpi_i"#);
        bulk_i_integer_plus_eq => (r#"integer +="#, r#"integer ii_i=2; ii_i+=7; print $ii_i"#);
        bulk_i_arith_exp_2_pow_10 => (r#"arith 2**10"#, r#"print $(( 2 ** 10 ))"#);
        bulk_i_arith_exp_7_pow_2 => (r#"arith 7**2"#, r#"print $(( 7 ** 2 ))"#);
        bulk_i_arith_output_base_16_hash => (r#"[#16]"#, r#"print $(( [#16] 255 ))"#);
        bulk_i_arith_output_base_8_hash => (r#"[#8]"#, r#"print $(( [#8] 64 ))"#);
        bulk_i_kill_zero_current_shell => (r#"kill -0 $$"#, r#"kill -0 $$; print k0i=$?"#);
        bulk_i_wait_no_child => (r#"wait no jobs"#, r#"wait 2>/dev/null; print wx_i=$?"#);
        bulk_i_coproc_exit_wait_status => (r#"coproc wait"#, r##"coproc exit 0; wait; print -r "cwx=$?""##);
        bulk_i_fc_push_pop_history_file => (r#"fc -p -P"#, r##"hi_i=$(mktemp); fc -p $hi_i 2>&1; fp1_i=$?; fc -P 2>/dev/null; fp2_i=$?; command rm -f $hi_i; print -r "$fp1_i $fp2_i""##);
        bulk_i_functrace_nested_depth => (r#"functrace depth"#, r##"outer_ft_i() { inner_ft_i() { print -r "ft_i=$#functrace"; }; inner_ft_i; }; outer_ft_i"##);
        bulk_i_funcstack_nested_depth => (r#"funcstack depth"#, r##"outer_fs_i() { inner_fs_i() { print -r "fs_i=$#funcstack"; }; inner_fs_i; }; outer_fs_i"##);
        bulk_i_readonly_unset_in_subshell => (r#"readonly unset"#, r##"readonly roi_i=1; ( unset roi_i ) 2>/dev/null; print -r "ur_i=$?""##);
        bulk_i_mktemp_executable_bit => (r#"chmod +x -x"#, r##"tx_i=$(mktemp); chmod +x $tx_i 2>/dev/null; [[ -x $tx_i ]]; print -r "xx_i=$?"; command rm -f $tx_i"##);
        bulk_i_compaudit_stdout_discard => (r#"compaudit"#, r##"compaudit >/dev/null 2>&1; print -r "ca_i=$?""##);
        bulk_i_command_subst_nested => (r#"nested $()"#, r##"nest_i() { print inner_i; }; print -r "$(nest_i)""##);
        bulk_i_heredoc_indented_strip => (r#"here <<-"#, r##"read x_i <<-EOI
	hi_line_i
EOI
print -r "x_i=$x_i""##);
        bulk_i_shwordsplit_sets_argc => (r#"shwordsplit argc"#, r##"setopt shwordsplit; wsi_i="a b c"; set -- $wsi_i; print -r "argci=$#""##);
        bulk_i_noglob_star_literal => (r#"noglob *"#, r##"setopt noglob; print -r *"##);
        bulk_i_printf_lower_hex => (r#"printf %x"#, r#"printf '%x\n' 255"#);
        bulk_i_expand_uppercase_flag_U => (r#"${(U) }"#, r#"mui_i=mixed; print ${(U)mui_i}"#);
        bulk_i_expand_lowercase_flag_L => (r#"${(L) }"#, r#"uli_i=MiXeD; print ${(L)uli_i}"#);
        bulk_i_pad_right_custom_char => (r#"pad (r:4::X:)"#, r#"print ${(r:4::X:)y}"#);
        bulk_i_array_lit_count_three => (r#"array #=3"#, r#"typeset -a aw3_i=(one two three); print $#aw3_i"#);
        bulk_i_pipestatus_after_false_true => (r#"pipestatus |"#, r##"false | true; print -r "$pipestatus[1] $pipestatus[2]""##);
        bulk_i_getopts_cluster_abc => (r#"getopts abc"#, r##"set -- -abc; OPTIND=1; getopts 'abc' o1_i; print -r "g1=$o1_i"; getopts 'abc' o2_i; print -r "g2=$o2_i""##);
        bulk_i_getopts_colon_optarg => (r#"getopts a:"#, r##"set -- -afoo_i; OPTIND=1; getopts 'a:' oa_i; print -r "$oa_i $OPTARG""##);
        bulk_i_glob_double_star_nested_tmp => (r#"** / in tmp"#, r##"tds_i=$(mktemp -d); mkdir -p $tds_i/a/b; touch $tds_i/a/b/c_i; ( builtin cd $tds_i && print a/**/c_i(N) ); command rm -rf $tds_i"##);
        bulk_i_background_wait_bang_statuses => (r#"wait $! bg"#, r##"true & wait $!; print -r "tw_i=$?"; false & wait $!; print -r "fw_i=$?""##);
        bulk_i_float_cap_E_scientific => (r#"typeset -E"#, r#"typeset -E ei_i=1.5e2; print $ei_i"#);
        bulk_i_arith_bit_xor => (r#"arith ^"#, r#"print $(( 5 ^ 3 ))"#);
        bulk_i_shift_two_keeps_first_of_rest => (r#"shift 2"#, r#"set -- a b c d; shift 2; print $1"#);
        bulk_i_arith_logical_or_in_parens => (r#"(( || ))"#, r#"(( 0 || 5 )); print aor_i=$?"#);
        bulk_i_eval_string_print => (r#"eval print"#, r#"eval "print ev_i=1""#);
        bulk_i_hash_reload_r => (r#"hash -r"#, r#"hash -r; print hri=$?"#);
        bulk_i_whence_verbose_builtin => (r#"whence -v print"#, r##"whence -v print 2>&1"##);
        bulk_i_arith_modulo_ten_three => (r#"10 % 3"#, r#"print $(( 10 % 3 ))"#);
        bulk_i_substring_scalar_slice => (r#"${var:n:m}"#, r#"svi_i=abcdef; print ${svi_i:2:3}"#);
        bulk_i_assoc_bracket_scalar => (r#"A[k]"#, r#"typeset -A aai_i=(ki vi); print ${aai_i[ki]}"#);
        bulk_i_IFS_length => (r#"${#IFS}"#, r#"print ${#IFS}"#);
        bulk_i_HISTCHARS_scalar => (r#"HISTCHARS"#, r#"print $HISTCHARS"#);
        bulk_i_KEYBOARD_HACK_default => (r#"KEYBOARD_HACK"#, r#"print ${KEYBOARD_HACK:-nilkh}"#);
        bulk_i_HISTCHARS_length => (r#"len HISTCHARS"#, r#"print ${#HISTCHARS}"#);
        bulk_i_WORDCHARS_length => (r#"len WORDCHARS"#, r#"print ${#WORDCHARS}"#);
        bulk_i_scheduled_events_count => (r#"#zsh_scheduled_events"#, r#"print ${#zsh_scheduled_events}"#);
        bulk_i_jobdirs_count => (r#"#jobdirs"#, r#"print ${#jobdirs}"#);
        bulk_i_watch_array_count => (r#"#watch"#, r#"print ${#watch}"#);
        bulk_i_cdpath_element_count => (r#"$#cdpath"#, r#"print $#cdpath"#);
        bulk_i_module_path_count => (r#"#module_path"#, r#"print ${#module_path}"#);
        bulk_i_signals_assoc_count => (r#"#signals"#, r#"print ${#signals}"#);
        bulk_i_reswords_assoc_count => (r#"#reswords"#, r#"print ${#reswords}"#);
        bulk_i_usergroups_defined_p => (r#"$+usergroups"#, r#"print $+usergroups"#);
        bulk_i_opt_rc_expand_param => (r#"options[rc_expand_param]"#, r#"print $options[rc_expand_param]"#);
        bulk_i_autoload_zmv => (r#"autoload zmv"#, r##"autoload -Uz zmv 2>&1; print -r "zmv_i=$?""##);
        bulk_i_param_LANG => (r#"LANG"#, r#"print ${LANG:-nil_LANG}"#);
        bulk_i_param_ZSH_PATCHLEVEL => (r#"ZSH_PATCHLEVEL"#, r#"print $ZSH_PATCHLEVEL"#);
        bulk_i_nullcmds_READNULL_and_NULL => (r#"NULL READNULL"#, r##"print -r "${NULLCMD:-N_nc}" "${READNULLCMD:-N_rd}""##);
        bulk_i_CHOST_and_MACHTYPE => (r#"CHOST MACHTYPE"#, r##"print -r "${CHOST:-}"; print -r "$MACHTYPE""##);
        bulk_i_commands_assoc_lookup_print => (r#"commands[print]"#, r##"print -r "${commands[print]:-noprintpath}""##);
        bulk_i_split_scalar_on_colon_flag => (r#"(s:.:.)"#, r#"sci_i=a.b.c; print ${(s.:.)sci_i}"#);
        bulk_i_assoc_sorted_keys_ok => (r#"(ok) assoc"#, r#"typeset -A oki_i=(bi 2 ai 1); print ${(ok)oki_i}"#);
        bulk_i_array_reversed_flag_Oa => (r#"(Oa) rev"#, r#"ari_i=(1 2 3); print ${(Oa)ari_i}"#);
        bulk_i_array_unique_flag_u => (r#"(u) uniq"#, r#"aui_i=(1 1 2); print ${(u)aui_i}"#);
        bulk_i_arith_grouped_factors => (r#"(( (1+2)*(3+4) ))"#, r#"print $(( (1+2)*(3+4) ))"#);
        bulk_i_arith_float_division => (r#"4 / 2.0"#, r#"print $(( 4 / 2.0 ))"#);
        bulk_i_logical_and_or_shortcircuit => (r#"&& ||"#, r##"true && print -r "Ai=1"; false || print -r "Bi=1""##);
        bulk_i_print_list_one_per_line => (r#"print -l"#, r#"print -l one_i two_i three_i"#);
        bulk_i_array_append_paren_elems => (r#"a+=( )"#, r#"typeset -a api_i=(1); api_i+=(2 3); print -r "${api_i[@]}""#);
        bulk_i_case_glob_branch => (r#"case * )"#, r#"case hi_i in hi*) print hit_i;; esac"#);
        bulk_i_hash_special_param_flags => (r#"$+tables"#, r##"print -r "$+parameters $+options $+builtins $+commands $+functions""##);
        bulk_i_trap_EXIT_prints => (r#"trap EXIT"#, r##"trap 'print -r Ti_exit' EXIT; :"##);
        bulk_i_ulimit_minus_n => (r#"ulimit -n"#, r##"print -r "ULi=$(ulimit -n)""##);
        bulk_i_filesystem_root_exists => (r#"[[ -e / ]]"#, r#"[[ -e / ]]; print eri=$?"#);
        bulk_i_array_negative_subscript => (r#"ary[-1]"#, r#"ani_i=(a b c); print ${ani_i[-1]}"#);
        bulk_i_array_filter_colon_num_pattern => (r#"${ary:#2}"#, r#"afi_i=(1 2 3); print ${afi_i:#2}"#);
        bulk_i_substring_from_offset_scalar => (r#"${sv:3}"#, r#"svo_i=abcdef; print ${svo_i:3}"#);
        bulk_i_default_colon_assign_REPLY => (r#"REPLY :="#, r#"unset REPLY; : ${REPLY:=rep_i}; print $REPLY"#);
        bulk_i_anonymous_function_call => (r#"() { }"#, r##"() { print -r anon_i; }"##);
        bulk_i_brace_comma_concat => (r#"{a,b}x"#, r#"print -r {a,b}_suf_i"#);
    }
}

/// Tenth batch: **`typeset -L`/`-R`/`-i2`/`-F2`/`-Z3`**, **radix `2#…`**, **`(f)`/`(z)` word breaking**, **`(j:|:)`**, **`(#b)`** (`extendedglob`), **`let`**, **multios writes**, **`localoptions`**,
/// **array sorts `(n)` / `(on)` / `(On)` / `(i)`**, **`${…//}` / `${…#}` / `${…%}`**, **glob in `[[ = ]]`**, **brace `{a..z..n}` steps**, **`cmp -s`**, **`[[ -k /tmp ]]`**, **`command -p`**, **`zmodload -L`**, **`emulate sh -c`**, **`$TRY_BLOCK_ERROR`**, **`$_`**, **`source` missing**, **`zstyle`/`bindkey` listings**, **`[[ =~ ]]`**, plus **`$options[bsdglob]`**, **`nohashdirs`**, **`errsilent`**.
mod corpus_dash_fc_bulk_j {
    use super::*;

    parity_gap_tests! {
        bulk_j_typeset_capital_L_pad => (r#"typeset -L 10"#, r##"typeset -L 10 lj_p=hi; print -r "[$lj_p]""##);
        bulk_j_typeset_capital_R_pad => (r#"typeset -R 10"#, r##"typeset -R 10 rj_p=hi; print -r "[$rj_p]""##);
        bulk_j_typeset_integer_base_two_display => (r#"typeset -i2"#, r#"typeset -i2 ij_tw=5; print $ij_tw"#);
        bulk_j_typeset_float_two_decimals => (r#"typeset -F2"#, r#"typeset -F2 fj_r=1.239; print $fj_r"#);
        bulk_j_typeset_Z3_width_integer => (r#"typeset -Z3 -i"#, r#"typeset -Z3 -i ij_z=7; print $ij_z"#);
        bulk_j_arith_literal_binary_prefix => (r#"2#1011"#, r#"print $(( 2#1011 ))"#);
        bulk_j_split_scalar_on_newlines_f_flag => (r#"split (f)"#, r##"vj_f=$'aj\nbj'; print -r "${(f)vj_f}""##);
        bulk_j_tokenize_words_z_flag => (r#"split (z)"#, r##"zj_w='xj  yj'; print -r "${(z)zj_w}""##);
        bulk_j_join_array_with_pipe_flag => (r#"join (j:|:)"#, r#"arj_j=( {1..3} ); print ${(j:|:)arj_j}"#);
        bulk_j_extendedglob_hash_b_capture => (r#"(#b) capture"#, r##"setopt extendedglob; [[ abcj = (#b)(a)(b*) ]]; print -r "$match""##);
        bulk_j_status_matches_dollar_question_false => (r#"status $?"#, r##"false; print -r "$status $?""##);
        bulk_j_pipestatus_entries_after_true => (r#"$#pipestatus true"#, r##"true; print -r "psnj=$#pipestatus""##);
        bulk_j_read_IFS_comma_two_fields => (r#"IFS , read"#, r##"IFS=,; read vj1 vj2 <<< 'pj,qj'; print -r "$vj1 $vj2""##);
        bulk_j_pad_left_dash_fill => (r#"pad (l::-:)"#, r#"print ${(l:4::-:)9}"#);
        bulk_j_read_herestring_one_line => (r#"read <<<"#, r##"read rj_hs <<< 'lin_j'; print -r "$rj_hs""##);
        bulk_j_multios_same_string_two_files => (r#"two > files"#, r##"tdm=$(mktemp -d); print -r samej >$tdm/aj >$tdm/bj; read -r lj1 <$tdm/aj; read -r lj2 <$tdm/bj; print -r "$lj1 $lj2"; command rm -rf $tdm"##);
        bulk_j_subshell_pwd_under_tmp => (r#"(cd /tmp)"#, r##"( builtin cd /tmp && print -r "subj=${PWD:t}" )""##);
        bulk_j_let_arith_assign => (r#"let"#, r#"let 'lj_s=2+3'; print $lj_s"#);
        bulk_j_function_return_status => (r#"return 9"#, r##"rj_fn() { return 9; }; rj_fn; print -r "rjex=$?""##);
        bulk_j_while_break_twice => (r#"while break"#, r##"ij_w=0; while (( ij_w < 5 )); do ij_w=$((ij_w+1)); [[ $ij_w -eq 2 ]] && break; done; print -r "$ij_w""##);
        bulk_j_while_continue_accum => (r#"while continue"#, r##"ij_c=0; sj_c=0; while (( ij_c < 4 )); do ij_c=$((ij_c+1)); [[ $ij_c -eq 2 ]] && continue; sj_c=$((sj_c+ij_c)); done; print -r "$sj_c""##);
        bulk_j_cond_isatty_stdin => (r#"[[ -t 0 ]]"#, r##"[[ -t 0 ]]; print -r "tj0=$?""##);
        bulk_j_cond_isatty_stdout => (r#"[[ -t 1 ]]"#, r##"[[ -t 1 ]]; print -r "tj1=$?""##);
        bulk_j_cond_name_pipe_dev_null => (r#"[[ -p /dev/null ]]"#, r##"[[ -p /dev/null ]]; print -r "pndj=$?""##);
        bulk_j_cond_readable_etc_hosts => (r#"[[ -r hosts ]]"#, r##"[[ -r /etc/hosts ]] || [[ -r /etc/hostname ]]; print -r "rhj=$?""##);
        bulk_j_cond_not_plain_file => (r#"[[ ! -e ]]"#, r##"[[ ! -e /__no_exist_j_path__ ]]; print -r "nej=$?""##);
        bulk_j_cond_logical_and_digits => (r#"[[ && -eq ]]"#, r##"[[ 1 -eq 1 && 2 -eq 2 ]]; print -r "andj=$?""##);
        bulk_j_cond_nul_and_nonempty => (r#"[[ -z -n ]]"#, r##"[[ -z '' && -n x ]]; print -r "znj=$?""##);
        bulk_j_alias_define_invoke_remove => (r#"alias cycle"#, r##"alias aj_pr='print ajv'; aj_pr; unalias aj_pr; print -r "has=${+aliases[aj_pr]}""##);
        bulk_j_array_subscript_range_slice => (r#"ary[2,4]"#, r##"arj_sl=(1 2 3 4); print -r "${arj_sl[2,4]}""##);
        bulk_j_print_OSTYPE_VENDOR_UID => (r#"OSTYPE VENDOR UID"#, r##"print -r "$OSTYPE $VENDOR $UID""##);
        bulk_j_count_modules_tables => (r#"#modules #loaded"#, r##"print -r "${#modules} ${#loaded_modules}""##);
        bulk_j_module_path_first => (r#"module_path[1]"#, r##"print -r "${module_path[1]:-nompath}""##);
        bulk_j_zmodload_list_silent => (r#"zmodload -L"#, r##"zmodload -L >/dev/null 2>&1; print -r "zLLj=$?""##);
        bulk_j_emulate_sh_one_cmd => (r#"emulate sh -c"#, r##"emulate sh -c 'print -r emj_sh'"##);
        bulk_j_localoptions_noglob_scoped => (r#"localoptions noglob"#, r##"setopt glob; loj_fn() { setopt localoptions; setopt noglob; print -r "in=${options[noglob]}"; }; loj_fn; print -r "out=${options[noglob]}""##);
        bulk_j_assoc_copy_via_kv_at => (r#"A copy @kv"#, r##"typeset -A asrcj=(kj vjv); typeset -A adstj=("${(@kv)asrcj}"); print -r "${adstj[kj]}""##);
        bulk_j_scalar_double_slash_subst => (r#"// repl"#, r##"svj_ds='aj1aj1'; print -r "${svj_ds//1/x}""##);
        bulk_j_array_onthefly_slash_subst => (r#"ary //"#, r##"arj_sl=(pj qj rj); print -r "${arj_sl//qj/xj}""##);
        bulk_j_remove_hash_shortest => (r#"# short"#, r##"svj_hs='ajXbj'; print -r "${svj_hs#*X}""##);
        bulk_j_remove_percent_shortest => (r#"% short"#, r##"svj_ts='aj.bj.cj'; print -r "${svj_ts%.*}""##);
        bulk_j_default_colon_minus_unset => (r#":- default"#, r##"unset vj_dm; print -r "${vj_dm:-defj}""##);
        bulk_j_alternate_colon_plus_set => (r#":+ alt"#, r##"vj_ap=sj; print -r "${vj_ap:+yesj}""##);
        bulk_j_name_reference_copy_argv => (r#"argv=( )"#, r##"argv=(tj1 tj2); print -r "$argv[1]""##);
        bulk_j_brace_range_step_three => (r#"{2..8..3}"#, r#"print {2..8..3}"#);
        bulk_j_brace_zero_padded_run => (r#"{01..03}"#, r#"print {01..03}"#);
        bulk_j_arith_pre_increment => (r#"++x"#, r##"integer ij_pi=1; print -r "$(( ++ij_pi ))""##);
        bulk_j_arith_post_decrement => (r#"x--"#, r##"integer ij_pd=3; print -r "$(( ij_pd-- )) $ij_pd""##);
        bulk_j_arith_float_times_int => (r#"float * int"#, r#"print $(( 3 * 2.0 ))"#);
        bulk_j_arith_bit_shift_left => (r#"<<"#, r#"print $(( 3 << 2 ))"#);
        bulk_j_arith_bit_shift_right => (r#">>"#, r#"print $(( 32 >> 2 ))"#);
        bulk_j_cmp_s_identical_hosts => (r#"cmp -s hosts"#, r##"cmp -s /etc/hosts /etc/hosts 2>/dev/null; print -r "cmj=$?""##);
        bulk_j_builtin_test_slash_exists => (r#"test -e /"#, r##"test -e /; print -r "tej=$?""##);
        bulk_j_cond_executable_sh_or_bash => (r#"[[ -x /bin/sh ]]"#, r##"[[ -x /bin/sh ]] || [[ -x /bin/bash ]]; print -r "exsj=$?""##);
        bulk_j_command_builtin_succeed => (r#"command true"#, r##"command true; print -r "ctj=$?""##);
        bulk_j_printf_unsigned_octal => (r#"printf %o"#, r#"printf '%o\n' 8"#);
        bulk_j_columns_lines_default_zero => (r#"COLUMNS LINES"#, r##"print -r "${COLUMNS:-0} ${LINES:-0}""##);
        bulk_j_prompt_eol_mark_param => (r#"PROMPT_EOL_MARK"#, r##"print -r "${PROMPT_EOL_MARK:-}""##);
        bulk_j_zstyle_list_ok => (r#"zstyle -L"#, r##"zstyle -L >/dev/null 2>&1; print -r "zsj=$?""##);
        bulk_j_bindkey_list_names_ok => (r#"bindkey -l"#, r##"bindkey -l >/dev/null 2>&1; print -r "bklj=$?""##);
        bulk_j_sort_array_numeric_n_flag => (r#"sort (n)"#, r##"arj_n=(3 1 2); print -r "${(n)arj_n}""##);
        bulk_j_sort_array_numeric_On_reverse => (r#"sort (On)"#, r##"arj_On=(3 1 2); print -r "${(On)arj_On}""##);
        bulk_j_sort_array_name_on_flag => (r#"sort (on)"#, r##"arj_on=(banana apple); print -r "${(on)arj_on}""##);
        bulk_j_sort_array_casefold_i_flag => (r#"sort (i)"#, r##"arj_i=(C a b); print -r "${(i)arj_i}""##);
        bulk_j_path_array_first_element => (r#"path[1]"#, r##"print -r "${path[1]:-nopath1}""##);
        bulk_j_ternary_arith_less_branch => (r#"?: lt"#, r#"print $(( 1 < 2 ? 30 : 40 ))"#);
        bulk_j_ternary_arith_greater_branch => (r#"?: gt"#, r#"print $(( 5 > 9 ? 1 : 2 ))"#);
        bulk_j_arith_float_less_than => (r#"float <"#, r##"(( 1.1 < 2.2 )); print -r "flj=$?""##);
        bulk_j_array_prepend_copy => (r#"a=(1 $a)"#, r##"arj_pr=(2 3); arj_pr=(1 $arj_pr); print -r "${arj_pr[@]}""##);
        bulk_j_set_doubledash_preserved => (r#"set -- --"#, r##"set -- -- -xj; print -r "$1""##);
        bulk_j_print_argv_zero_string => (r#"$0"#, r##"print -r "$0""##);
        bulk_j_zmodload_e_datetime_probe => (r#"zmodload -e"#, r##"zmodload -e zsh/datetime; print -r "zej=$?""##);
        bulk_j_typeset_export_uppercase_t => (r#"typeset -x (t)"#, r##"typeset -x exj_v=1; print -r "${(t)exj_v}""##);
        bulk_j_float_equals_integer_compare => (r#"2.0 == 2"#, r##"(( 2.0 == 2 )); print -r "feqj=$?""##);
        bulk_j_tilde_in_scalar => (r#"tilde assign"#, r##"tj_hd=~; print -r "${tj_hd:t}""##);
        bulk_j_scalar_head_colon_h => (r#":h head"#, r##"svj_bh=/xj/yj/zj; print -r "${svj_bh:h}""##);
        bulk_j_scalar_tail_colon_t => (r#":t tail"#, r##"svj_tt=/xj/yj/zj; print -r "${svj_tt:t}""##);
        bulk_j_PWD_colon_h_chop => (r#"PWD :h"#, r##"print -r "${PWD:h}""##);
        bulk_j_param_match_tilde_unquoted => (r#"$~pat"#, r##"patj=z; [[ z = $~patj ]]; print -r "pmj=$?""##);
        bulk_j_integer_division_negative => (r#"-7 / 3"#, r#"print $(( -7 / 3 ))"#);
        bulk_j_modulo_negative => (r#"-7 % 3"#, r#"print $(( -7 % 3 ))"#);
        bulk_j_string_eq_empty => (r#"'' = ''"#, r##"[[ '' = '' ]]; print -r "eqj=$?""##);
        bulk_j_brace_run_suffix_literal => (r#"{a..c}_"#, r#"print -r {a..c}_j"#);
        bulk_j_false_or_true_status => (r#"false || true"#, r##"false || true; print -r "foj=$?""##);
        bulk_j_true_and_false_status => (r#"true && false"#, r##"true && false; print -r "afj=$?""##);
        bulk_j_sticky_bit_tmpdir => (r#"[[ -k /tmp ]]"#, r##"[[ -k /tmp ]]; print -r "skj=$?""##);
        bulk_j_funcstack_depth_toplevel => (r#"$#funcstack"#, r##"print -r "$#funcstack""##);
        bulk_j_source_missing_file_status => (r#"source missing"#, r##"source /__src_missing_j__ 2>/dev/null; print -r "sxj=$?""##);
        bulk_j_try_block_error_after_true => (r#"TRY_BLOCK"#, r##"true; print -r "tbej=$TRY_BLOCK_ERROR""##);
        bulk_j_dollar_underscore_last_arg => (r#"$_"#, r##": lastj_arg; print -r "$_""##);
        bulk_j_cond_string_equals_glob_pattern => (r#"[[ = * ]]"#, r##"[[ abcj = *b* ]]; print -r "gitj=$?""##);
        bulk_j_LC_NUMERIC_param => (r#"LC_NUMERIC"#, r##"print -r "${LC_NUMERIC:-}""##);
        bulk_j_command_p_path_true => (r#"command -p"#, r##"command -p true; print -r "cpj=$?""##);
        bulk_j_dis_functions_count => (r#"#dis_functions"#, r##"print -r "${#dis_functions}""##);
        bulk_j_jobstates_plus => (r#"$+jobstates"#, r##"print -r "$+jobstates""##);
        bulk_j_zle_plus_param => (r#"$+ZLE"#, r##"print -r "$+ZLE""##);
        bulk_j_histchars_subscript_one => (r#"HISTCHARS[1]"#, r##"print -r "${HISTCHARS[1]}""##);
        bulk_j_opt_bsdglob => (r#"options[bsdglob]"#, r##"print -r "$options[bsdglob]""##);
        bulk_j_opt_nohashdirs => (r#"options[nohashdirs]"#, r##"print -r "$options[nohashdirs]""##);
        bulk_j_opt_errsilent => (r#"options[errsilent]"#, r##"print -r "$options[errsilent]""##);
        bulk_j_unsetopt_nohup_print => (r#"unsetopt nohup"#, r##"unsetopt nohup 2>/dev/null; print -r "uoj=$?""##);
        bulk_j_regex_paren_class_and => (r#"[[ =~ ]]"#, r##"[[ xyzj =~ '^x.*j$' ]]; print -r "rxj=$?""##);
    }
}

/// Eleventh batch: **`zmodload zsh/example`**, **`typeset -T`** tied scalar/array, **octal `typeset -i8` / `$(( 010 ))`**, **`read -d`**, **`${…/#…}` / `${…/%…}`**, **`(q)` / `(b)`**, **`for (( ))`**, **`(k)`** / range assign / sparse **`a[5]=`**, **bit `~`**, **`abs()`** after **`zsh/mathfunc`**, **`integer kcnt=${#${(f)$(typeset +i)}}`**, **`whence -c`**, **`[[ == glob ]]`**, **`if`/`elif`**, **`case` `|`**, anon **`local`**, **`rc_expand_param` + `${^a}`**, **`umask` / `limit` / `dirs`**, **`[[ -L ]]`**, **`print -C`**, **`jobs`**, **`[[ -c /dev/null ]]`**, **`getopts`** parse error, **`options[combininghooks]`**, **`dis_*` counts**, **`read -A`**, **`SHELL`**, **`$#funcfiletrace`**, and misc.
mod corpus_dash_fc_bulk_k {
    use super::*;

    parity_gap_tests! {
        bulk_k_zmodload_zsh_example => (r#"zmodload zsh/example"#, r##"zmodload zsh/example 2>&1; print -r "k_ex=$?""##);
        bulk_k_substitute_hash_prefix => (r#"${var/#a/A}"#, r##"sk_pf=abc; print -r "${sk_pf/#a/A}""##);
        bulk_k_substitute_percent_suffix => (r#"${var%.x/_}"#, r##"sk_sf=file.x; print -r "${sk_sf/%.x/_ok}""##);
        bulk_k_quote_flag_q_metachars => (r#"${(q) }"#, r##"qk_v='a*b'; print -r "${(q)qk_v}""##);
        bulk_k_backslash_flag_b => (r#"${(b) }"#, r##"bk_v=$'a\tb'; print -r "${(b)bk_v}""##);
        bulk_k_for_arithmetic_c_style => (r#"for (( ))"#, r##"for (( ik_fc=1; ik_fc<=3; ik_fc++ )); do print -r "kf=$ik_fc"; done"##);
        bulk_k_arith_postincrement_line => (r#"(( x++ ))"#, r##"(( kk_pi=0 )); (( kk_pi++ )); print -r "$kk_pi""##);
        bulk_k_typeset_integer_output_oct => (r#"typeset -i8"#, r#"typeset -i8 kk_io=12; print -r "$kk_io""#);
        bulk_k_read_delim_x_herestring => (r#"read -d x"#, r##"read -d x kk_rk <<< 'axbx'; print -r "rk=$kk_rk""##);
        bulk_k_cond_symlink_mktemp => (r#"[[ -L ]]"#, r##"tdl=$(mktemp -d); touch $tdl/tk; ln -sf tk $tdl/lk; [[ -L $tdl/lk ]]; print -r "slk=$?"; command rm -rf $tdl"##);
        bulk_k_assoc_keys_only_k_flag => (r#"${(k)A}"#, r##"typeset -A kk_am=(kk1 vk1); print -r "${(k)kk_am}""##);
        bulk_k_array_range_assign_two => (r#"a[1,2]="#, r##"ak_sl=(1 2); ak_sl[1,2]=(x y); print -r "${ak_sl[@]}""##);
        bulk_k_shift_too_far_status => (r#"shift 99"#, r##"set -- a b c; shift 99 2>/dev/null; print -r "shk=$?""##);
        bulk_k_arith_power_right_assoc => (r#"7**2**2"#, r##"print -r "$(( 7 ** 2 ** 2 ))""##);
        bulk_k_whence_c_builtin => (r#"whence -c print"#, r##"whence -c print 2>&1"##);
        bulk_k_printf_style_g_float => (r#"printf %g"#, r##"printf '%g\n' 3.14159"##);
        bulk_k_command_v_builtin => (r#"command -v print"#, r##"command -v print 2>&1"##);
        bulk_k_string_equal_glob_dq_star => (r#"[[ == pattern ]]"#, r##"[[ abc_k == a* ]]; print -r "sqk=$?""##);
        bulk_k_if_elif_else_chain => (r#"if elif"#, r##"if false; then print bad_k; elif true; then print ok_kelif; else print no_k; fi"##);
        bulk_k_brace_group_subshell => (r#"{ group; }"#, r##"{ print -r grp_k; }"##);
        bulk_k_case_pipe_pattern => (r#"case a|b"#, r##"case XX_k in X*|*Y_k) print miss;; XX_k|YY_k) print hit_kcp;; esac"##);
        bulk_k_anon_function_local => (r#"( ) { local }"#, r##"() { local lk_k=42; print -r "$lk_k"; }""##);
        bulk_k_colon_minus_assign => (r#":-"#, r##"unset uk_ca; : ${uk_ca:=905_k}; print -r "$uk_ca""##);
        bulk_k_typeset_tied_scalar_array => (r#"typeset -T"#, r##"typeset -T TK_s arK_t; arK_t=(x y z); print -r "tsk=$TK_s nK=$#arK_t""##);
        bulk_k_arith_bit_not_tilde => (r#"~255"#, r##"print -r "$(( ~255 ))""##);
        bulk_k_mathfunc_abs_after_load => (r#"mathfunc abs"#, r##"zmodload zsh/mathfunc 2>/dev/null; print -r "$(( abs(-5) ))""##);
        bulk_k_integer_count_typeset_plus_i => (r#"typeset +i #lines"#, r##"integer kk_il=${#${(f)$(typeset +i 2>/dev/null)}}; print -r "$kk_il""##);
        bulk_k_param_FCEDIT => (r#"FCEDIT"#, r##"FCEDIT=fe_k; print -r "${FCEDIT:-}""##);
        bulk_k_module_aliases_plus => (r#"$+module_aliases"#, r##"print -r "$+module_aliases""##);
        bulk_k_count_dis_aliases => (r#"#dis_aliases"#, r##"print -r "${#dis_aliases}""##);
        bulk_k_count_dis_functions => (r#"#dis_functions"#, r##"print -r "${#dis_functions}""##);
        bulk_k_options_combininghooks => (r#"options[combininghooks]"#, r##"print -r "$options[combininghooks]""##);
        bulk_k_rc_expand_caret_array => (r#"${^ary}"#, r##"setopt rc_expand_param; ak_rc=(p q); print -r "${^ak_rc}""##);
        bulk_k_SECONDS_reset_nop => (r#"SECONDS"#, r##"SECONDS=0; :; print -r "$SECONDS""##);
        bulk_k_umask_print_numeric => (r#"umask"#, r##"print -r "$(umask)""##);
        bulk_k_limit_soft_noarg => (r#"limit"#, r##"limit 2>/dev/null; print -r "lmk=$?""##);
        bulk_k_dirs_builtin_ok => (r#"dirs"#, r##"dirs 2>/dev/null; print -r "drk=$?""##);
        bulk_k_builtin_test_string_eq => (r#"test str ="#, r##"test 'a_k' = 'a_k' && print -r teqk"##);
        bulk_k_dual_bracket_invert_file => (r#"[[ ! -e ]]"#, r##"[[ ! -e /__no_k_file__ ]]; print -r "nek=$?""##);
        bulk_k_sparse_array_subscript => (r#"a[5]="#, r##"ak_sp=(); ak_sp[5]=hi_k; print -r "$#ak_sp $ak_sp[5]""##);
        bulk_k_printf_percent_q_line => (r#"printf %q line"#, r##"printf '%q\n' 'two words_k'"##);
        bulk_k_double_bracket_str_ne => (r#"[[ != ]]"#, r##"[[ abc != def ]]; print -r "nek2=$?""##);
        bulk_k_arith_logical_and_ints => (r#"&& in (( ))"#, r##"print -r "$(( 1 && 0 )) $(( 1 && 1 ))""##);
        bulk_k_arith_logical_or_ints => (r#"\|\| in (( ))"#, r##"print -r "$(( 0 || 0 )) $(( 0 || 1 ))""##);
        bulk_k_nested_arith_parens => (r#"(( (1+2)*(4+5) ))"#, r##"print -r "$(( (1+2)*(4+5) ))""##);
        bulk_k_float_compare_ge => (r#">=" float"#, r##"(( 2.5 >= 2.5 )); print -r "fgek=$?""##);
        bulk_k_float_compare_le => (r#"<=" float"#, r##"(( 1.0 <= 2.0 )); print -r "flek=$?""##);
        bulk_k_ternary_nested_arith => (r#"?: nested"#, r##"print -r "$(( 1 ? (0 ? 9 : 8) : 7 ))""##);
        bulk_k_division_negative_trunc => (r#"11 / -3"#, r##"print -r "$(( 11 / -3 ))""##);
        bulk_k_modulo_positive => (r#"100 % 7"#, r##"print -r "$(( 100 % 7 ))""##);
        bulk_k_char_code_double_hash_x => (r#"##x"#, r##"print -r "$(( ##x ))""##);
        bulk_k_parameter_PWD_colon_t => (r#"PWD :t"#, r##"print -r "${PWD:t}""##);
        bulk_k_scalar_dirname_basename_combo => (r#":h :t"#, r##"sk_p=/one_k/two_k/three_k; print -r "${sk_p:h:t}""##);
        bulk_k_array_first_last_subscripts => (r#"[1] [-1]"#, r##"ak_fl=(u v w); print -r "$ak_fl[1] $ak_fl[-1]""##);
