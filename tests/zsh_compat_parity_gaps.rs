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
//! `corpus_dash_fc_bulk_h`, `corpus_dash_fc_bulk_i`, `corpus_dash_fc_bulk_j`, `corpus_dash_fc_bulk_k`, `corpus_dash_fc_bulk_l`,
//! `corpus_dash_fc_bulk_m`, `corpus_dash_fc_bulk_n`, `corpus_dash_fc_bulk_o`, `corpus_dash_fc_bulk_p`,
//! `corpus_dash_fc_bulk_q`, `corpus_dash_fc_bulk_r`, `corpus_dash_fc_bulk_s`, `corpus_dash_fc_bulk_t`, `corpus_dash_fc_bulk_u`, `corpus_dash_fc_bulk_v`,
//! `corpus_dash_fc_bulk_w` … `corpus_dash_fc_bulk_ak`, rounds 1–100: `corpus_dash_fc_bulk_al` … `corpus_dash_fc_bulk_eg` (48 zsh-probed `-fc` pins/round). Pass/fail is **stdout + exit** only (see `assert_parity`).

#![allow(non_snake_case)]

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
            z.stdout, r.stdout, z.stderr, r.stderr, z.exit, r.exit
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
        bulk_k_glob_plain_files_qual_dot_tmp => (r#"*(.) tmp"#, r##"tgk=$(mktemp -d); touch $tgk/only_kf; mkdir $tgk/dk; ( builtin cd $tgk && print *(.) ); eck=$?; command rm -rf $tgk; exit $eck"##);
        bulk_k_print_capital_C_three_columns => (r#"print -C 3"#, r#"print -C 3 1 2 3 4 5 6"#);
        bulk_k_repeat_builtin_twice => (r#"repeat 2"#, r##"repeat 2; do print -r rep_k; done"##);
        bulk_k_while_read_from_pipe_line => (r#"print | read"#, r##"print line_k | while read -r rk_wl; do print -r "$rk_wl"; done"##);
        bulk_k_parameter_module_path_last => (r#"module_path[-1]"#, r##"print -r "${module_path[-1]:-nomp}""##);
        bulk_k_read_null_device_empty => (r#"read < /dev/null"#, r##"read -r rk_null </dev/null; print -r "len=${#rk_null}""##);
        bulk_k_float_int_product => (r#"3.0 * 2"#, r##"print -r "$(( 3.0 * 2 ))""##);
        bulk_k_literal_octal_ten => (r#"010"#, r##"print -r "$(( 010 ))""##);
        bulk_k_test_bang_nonexistent => (r#"test ! -e"#, r##"test ! -e /__nope_k__; print -r "tnk=$?""##);
        bulk_k_setopt_unsetopt_clobber_cycle => (r#"noclobber cycle"#, r##"setopt noclobber; unsetopt noclobber; print -r "$options[clobber]""##);
        bulk_k_enable_hash_builtin => (r#"enable hash"#, r##"enable hash 2>/dev/null; print -r "ehk=$?""##);
        bulk_k_read_capital_A_split => (r#"read -A"#, r##"read -A Ark_r <<< 'pk qk'; print -r "$Ark_r[2]""##);
        bulk_k_param_SHELL_scalar => (r#"SHELL"#, r##"print -r "${SHELL:-nil_sh}""##);
        bulk_k_funcfiletrace_len_top => (r#"$#funcfiletrace"#, r##"print -r "$#funcfiletrace""##);
        bulk_k_jobs_builtin_none => (r#"jobs"#, r##"jobs -p 2>/dev/null; print -r "jbk=$?""##);
        bulk_k_char_dev_null_stream => (r#"[[ -c /dev/null ]]"#, r##"[[ -c /dev/null ]]; print -r "cdevk=$?""##);
        bulk_k_getopts_bad_option_status => (r#"getopts err"#, r##"set -- -bz; OPTIND=1; getopts 'a:' ogk 2>/dev/null; print -r "gok=$?""##);
        bulk_k_unset_multiple_names => (r#"unset a b"#, r##"typeset uvk_a=1 uvk_b=2; unset uvk_a uvk_b; print -r "${+uvk_a}${+uvk_b}""##);
        bulk_k_empty_array_count => (r#"empty array"#, r##"ak_e=(); print -r "$#ak_e""##);
        bulk_k_bang_command_negate => (r#"! false"#, r##"! false; print -r "nfk=$?""##);
        bulk_k_colon_chain_status => (r#": false true"#, r##"false; true; print -r "cck=$?""##);
        bulk_k_string_eq_empty_brackets => (r#"[[ '' ]]"#, r##"[[ '' == '' ]]; print -r "eek=$?""##);
        bulk_k_cond_identical_strings => (r#"[[ x = x ]]"#, r##"[[ zz_k = zz_k ]]; print -r "idk=$?""##);
        bulk_k_arith_eq_zero_one => (r#"== 0 (( ))"#, r##"(( 0 == 0 )); print -r "eq0k=$?""##);
        bulk_k_arith_ne_compare => (r#"!= (( ))"#, r##"(( 1 != 2 )); print -r "nek3=$?""##);
        bulk_k_print_hyphen_n_dash => (r#"print - --"#, r##"print -r -- '-lead_k'"##);
        bulk_k_readonly_assign_in_subshell => (r#"readonly in ( )"#, r##"readonly rk_k=1; ( rk_k=2 ) 2>/dev/null; print -r "irk=$?""##);
        bulk_k_bindkey_default_zero => (r#"bindkey ^@"#, r##"bindkey '^@' 2>/dev/null; print -r "bk0k=$?""##);
        bulk_k_zstyle_s_get => (r#"zstyle -g"#, r##"zstyle -g ZSK_reply '*' 2>/dev/null; print -r "zsgk=$?""##);
        bulk_k_float_div_one => (r#"1.0/1"#, r##"print -r "$(( 1.0 / 1 ))""##);
        bulk_k_integer_minus_times => (r#"-3 * 4"#, r##"print -r "$(( -3 * 4 ))""##);
        bulk_k_assoc_subscript_bracket_key => (r#"A['x']"#, r##"typeset -A Ak_bq=(x vx); print -r "${Ak_bq[x]}""##);
        bulk_k_array_append_bracket_plus => (r#"a+=""#, r##"ak_ap=(1); ak_ap+=(2); print -r "${#ak_ap}""##);
        bulk_k_scalar_length_hash => (r#"${#scalar}"#, r##"sk_len=abcd; print -r "${#sk_len}""##);
        bulk_k_glob_star_D_dotfiles_tmp => (r#"*(D) dot"#, r##"tdk=$(mktemp -d); touch $tdk/.hid_k $tdk/vis_k; ( builtin cd $tdk && print *(D) ); command rm -rf $tdk"##);
        bulk_k_extendedglob_negate_class => (r#"^(pat)"#, r##"setopt extendedglob; [[ xyz =~ '^[^x]*$' ]]; print -r "nxk=$?""##);
        bulk_k_assoc_values_only_capital_v => (r#"${(v)A}"#, r##"typeset -A Ak_vv=(k1 v1 k2 v2); print -r "${(v)Ak_vv}""##);
        bulk_k_whitespace_trim_once => (r#"space trim"#, r##"setopt extendedglob; swk='   mid'; print -r "${swk##[[:space:]]#}""##);
        bulk_k_exec_dot_slash_path => (r#"[[ -x path ]]"#, r##"tex=$(mktemp); printf '#!/bin/sh\necho tx\n' >$tex; chmod +x $tex; [[ -x $tex ]]; print -r "txk=$?"; command rm -f $tex"##);
        bulk_k_rematch_pcre_off_still_regex => (r#"=~ off pcre"#, r##"unsetopt rematchpcre 2>/dev/null; [[ ab =~ '^a.b$' ]]; print -r "rmk=$?""##);
        bulk_k_parameter_histchars_len => (r#"${#HISTCHARS}"#, r##"print -r "${#HISTCHARS}""##);
        bulk_k_options_kshoptionprint => (r#"options[kshoptionprint]"#, r##"print -r "$options[kshoptionprint]""##);
        bulk_k_options_kshzerosubscript => (r#"options[kshzerosubscript]"#, r##"print -r "$options[kshzerosubscript]""##);
        bulk_k_options_loopinline => (r#"options[loopinline]"#, r##"print -r "$options[loopinline]""##);
        bulk_k_options_overstrike => (r#"options[overstrike]"#, r##"print -r "$options[overstrike]""##);
        bulk_k_options_promptbang => (r#"options[promptbang]"#, r##"print -r "$options[promptbang]""##);
        bulk_k_options_promptcr => (r#"options[promptcr]"#, r##"print -r "$options[promptcr]""##);
        bulk_k_options_promptpercent => (r#"options[promptpercent]"#, r##"print -r "$options[promptpercent]""##);
        bulk_k_options_promptsp => (r#"options[promptsp]"#, r##"print -r "$options[promptsp]""##);
        bulk_k_options_transientrprompt => (r#"options[transientrprompt]"#, r##"print -r "$options[transientrprompt]""##);
        bulk_k_ulimit_soft_f_max => (r#"ulimit -f"#, r##"print -r "$(ulimit -f)""##);
    }
}

/// Twelfth batch: env (**`TERM`**, **`ZDOTDIR`**, **`VISUAL`**, **`EMACS`**), **`zmodload zsh/deltochar`** (often fails on macOS), **regex capture** (`$MATCH`, `$MBEGIN`/`$MEND`), **`typeset` width / radix / float**, **`(oL)`** / **`(s.|.)`**, **`${…/pat/repl}`** on scalar & array,
/// **extendedglob `ab#c`**, **`zparseopts`**, **`read -t 0`**, **`trap '' INT`**, **`(P)`** indirection, **`(%)` prompt flags**, **`(c)`** line split, **`(Mk)`** assoc match keys, **mathfunc `int`/`ceil`/`hypot`**, **glob `*(^/)`**, **`builtin cd`** to **`/`**, **`typeset -g`**, **`#dis_*` counts**, **`${(ok)options}` first key**, **`schedules`**, **`strftime`** with **`zsh/datetime`**, and assorted **`$options[…]`** / conditions.
mod corpus_dash_fc_bulk_l {
    use super::*;

    parity_gap_tests! {
        bulk_l_zmodload_zsh_deltochar => (r#"zmodload zsh/deltochar"#, r##"zmodload zsh/deltochar 2>&1; print -r "dex=$?""##);
        bulk_l_param_TERM => (r#"TERM"#, r##"print -r "${TERM:-noterm}""##);
        bulk_l_param_ZDOTDIR => (r#"ZDOTDIR"#, r##"print -r "${ZDOTDIR:-nil_zd}""##);
        bulk_l_param_VISUAL => (r#"VISUAL"#, r##"print -r "${VISUAL:-nil_vis}""##);
        bulk_l_param_EMACS => (r#"EMACS"#, r##"print -r "${EMACS:-nil_em}""##);
        bulk_l_typeset_integer_hex_assign => (r#"typeset -i 16#"#, r##"typeset -i xl_h=16#ff; print -r "$xl_h""##);
        bulk_l_printf_percent_b_escape => (r#"printf %b"#, r##"printf '%b\n' '\141'"##);
        bulk_l_arith_literal_base_36 => (r#"36#zz"#, r##"print -r "$(( 36#zz ))""##);
        bulk_l_typeset_Z5_negative_int => (r#"typeset -Z5 -i "-""#, r##"typeset -Z5 -i xl_n=-3; print -r "$xl_n""##);
        bulk_l_array_sort_by_length_oL => (r#"sort (oL)"#, r##"al_oL=(aaa bb c); print -r "${(oL)al_oL}""##);
        bulk_l_split_scalar_on_pipe_char => (r#"(s.|.)"#, r##"sl_pi='a|b|c'; print -r "${(s.|.)sl_pi}""##);
        bulk_l_scalar_slash_replace_once => (r#"#/repl"#, r##"sl_sr='ap1'; print -r "${sl_sr/p1/P1}""##);
        bulk_l_array_slash_replace_elt => (r#"ary /p/P"#, r##"al_sr=(ap b cp); print -r "${al_sr/p/P}""##);
        bulk_l_extendedglob_hash_quantifier => (r#"ab#c"#, r##"setopt extendedglob; [[ abc = ab#c ]]; print -r "hq=$?""##);
        bulk_l_options_aliasing => (r#"options[aliasing]"#, r##"print -r "$options[aliasing]""##);
        bulk_l_options_histnomultiline => (r#"options[histnomultiline]"#, r##"print -r "$options[histnomultiline]""##);
        bulk_l_options_localpatterns => (r#"options[localpatterns]"#, r##"print -r "$options[localpatterns]""##);
        bulk_l_cond_writeable_tmp => (r#"[[ -w /tmp ]]"#, r##"[[ -w /tmp ]]; print -r "wtk=$?""##);
        bulk_l_pad_left_width_six => (r#"(l:6)"#, r##"print -r "[${(l:6)hi}]""##);
        bulk_l_param_indirect_P_flag => (r#"(P) indir"#, r##"nm_l=PWD; print -r "${(P)nm_l}""##);
        bulk_l_typeset_F5_float_width => (r#"typeset -F5"#, r##"typeset -F5 fl_l=2.5; print -r "$fl_l""##);
        bulk_l_arith_print_hex_ff => (r#"$((16#ff))"#, r##"print -r "$(( 16#ff ))""##);
        bulk_l_arith_comma_assign_pair => (r#"(( a= , b= ))"#, r##"(( al_ca=1, bl_cb=2 )); print -r "$al_ca $bl_cb""##);
        bulk_l_arith_ior_assign => (r#"|= (( ))"#, r##"integer il_ia=5; (( il_ia |= 2 )); print -r "$il_ia""##);
        bulk_l_cond_owned_mktemp => (r#"[[ -O file ]]"#, r##"tfo=$(mktemp); [[ -O $tfo ]]; print -r "owk=$?"; command rm -f $tfo"##);
        bulk_l_read_timeout_zero_herestring => (r#"read -t 0"#, r##"read -t 0 -r rl_rt0 <<< hi_l; print -r "ex=$? ln=$rl_rt0""##);
        bulk_l_zparseopts_minus_x_flag => (r#"zparseopts x"#, r##"typeset -a al_zp=(); set -- -x; zparseopts -a al_zp x; print -r "n=$#al_zp v=$al_zp""##);
        bulk_l_param_OLDPWD_default => (r#"OLDPWD"#, r##"print -r "${OLDPWD:-nil_op}""##);
        bulk_l_count_dis_patchars => (r#"#dis_patchars"#, r##"print -r "${#dis_patchars}""##);
        bulk_l_count_dis_reswords => (r#"#dis_reswords"#, r##"print -r "${#dis_reswords}""##);
        bulk_l_brace_embed_prefix_suffix => (r#"pre_{u,v}_suf"#, r##"print -r pre_{u,v}_sx""##);
        bulk_l_quote_flag_qq_words => (r#"${(qq) }"#, r##"wl_qq='a b'; print -r "${(qq)wl_qq}""##);
        bulk_l_chars_split_flag_c_newlines => (r#"${(c) }"#, r##"vl_c=$'a\nb'; print -r "${(c)vl_c}""##);
        bulk_l_rematch_MATCH_substring => (r#"$MATCH"#, r##"[[ abc_l =~ b ]]; print -r "$MATCH""##);
        bulk_l_rematch_MBEGIN_MEND => (r#"MBEGIN MEND"#, r##"[[ xyx_l =~ x ]]; print -r "$MBEGIN $MEND""##);
        bulk_l_builtin_cd_root_basename => (r#"cd / :t"#, r##"builtin cd /; print -r "${PWD:t}""##);
        bulk_l_glob_non_directory_files => (r#"*(^/)"#, r##"td_nd=$(mktemp -d); touch $td_nd/fl_nd; mkdir $td_nd/dr_nd; ( builtin cd $td_nd && print *(^/) ); command rm -rf $td_nd"##);
        bulk_l_mathfunc_int_ceil_hypot => (r#"int ceil hypot"#, r##"zmodload zsh/mathfunc 2>/dev/null; print -r "$(( int(2.2) )) $(( ceil(1.1) )) $(( hypot(6,8) ))""##);
        bulk_l_ARGV_tied_to_argv => (r#"ARGV argv"#, r##"argv=(zl_one); print -r "${argv[1]}_${ARGV[1]:-na}""##);
        bulk_l_trap_ignore_INT => (r#"trap INT"#, r##"trap '' INT; print -r trap_ign_ok""##);
        bulk_l_test_n_empty_true => (r#"test -n \"\""#, r##"test -n ''; print -r "tnz=$?""##);
        bulk_l_test_z_nonempty_false => (r#"test -z word"#, r##"test -z 'hi'; print -r "tzn=$?""##);
        bulk_l_arith_chain_le_all => (r#"<= chain"#, r##"(( 1 <= 2 && 2 <= 3 )); print -r "lec=$?""##);
        bulk_l_param_CDPATH_default => (r#"CDPATH"#, r##"print -r "${CDPATH:-empty_cd}""##);
        bulk_l_strftime_year_epoch_datetime => (r#"strftime %Y"#, r##"zmodload zsh/datetime 2>/dev/null; strftime %Y 0 >/dev/null; print -r "stfl=$?""##);
        bulk_l_unset_IFS_plus_test => (r#"unset IFS"#, r##"unset IFS; print -r "${IFS+ifs_set}""##);
        bulk_l_typeset_global_in_function => (r#"typeset -g"#, r##"fn_gl() { typeset -g vl_gl=9; }; fn_gl; print -r "$vl_gl""##);
        bulk_l_typeset_integer_binary_radix => (r#"typeset -i2"#, r##"typeset -i2 il_b=4; print -r "$il_b""##);
        bulk_l_float_compare_gt => (r#"float >"#, r##"(( 2.2 > 2.1 )); print -r "fgt=$?""##);
        bulk_l_case_insensitive_pattern_flag => (r#"(#i) ="#, r##"setopt extendedglob; [[ Ab_l = (#i)ab_l ]]; print -r "cik=$?""##);
        bulk_l_cond_PWD_rooted_slash => (r#"PWD /*"#, r##"[[ $PWD = /* ]]; print -r "prk=$?""##);
        bulk_l_print_LINENO_top => (r#"LINENO"#, r##"print -r "LNO=$LINENO""##);
        bulk_l_fc_list_range_status => (r#"fc -l"#, r##"fc -l 1 1 2>/dev/null; print -r "fca=$?""##);
        bulk_l_history_line_count => (r#"#history"#, r##"print -r "${#history}""##);
        bulk_l_plus_historywords => (r#"$+historywords"#, r##"print -r "$+historywords""##);
        bulk_l_subscript_I_pattern_match => (r#"[(I)pat]"#, r##"al_I=(ax_l bx_l cx_l); print -r "${al_I[(I)x*]}""##);
        bulk_l_brace_range_zero_three => (r#"{0..3}"#, r##"print -r {0..3}""##);
        bulk_l_getopts_exhaust_second_call => (r#"getopts done"#, r##"set -- -a foo_l; OPTIND=1; getopts 'a:' og1_l; getopts 'a:' og2_l 2>/dev/null; print -r "gex=$?""##);
        bulk_l_setopt_nounset_print => (r#"setopt nounset"#, r##"setopt nounset 2>/dev/null; print -r "$options[nounset]""##);
        bulk_l_PS3_prompt_var => (r#"PS3"#, r##"PS3='p3l'; print -r "$PS3""##);
        bulk_l_RANDOM_after_seed => (r#"RANDOM="#, r##"RANDOM=99; print -r "$RANDOM""##);
        bulk_l_schedules_table_count => (r#"#schedules"#, r##"print -r "${#schedules}""##);
        bulk_l_zmodload_e_zsh_files => (r#"zmodload -e files"#, r##"zmodload -e zsh/files; print -r "efls=$?""##);
        bulk_l_percent_expand_PS4 => (r#"${(%)PS4}"#, r##"PS4='+'; print -r "${(%)PS4}""##);
        bulk_l_assoc_Mk_filter_keys => (r#"(Mk)"#, r##"typeset -A Amk_l=(Key_l vl_k other oth); print -r "${(Mk)Amk_l:#K*}""##);
        bulk_l_cond_logical_or_in_test => (r#"[[ \|\| ]]"#, r##"[[ x = y || a = a ]]; print -r "lor=$?""##);
        bulk_l_double_paren_bit_and => (r#"& (( ))"#, r##"print -r "$(( 7 & 3 ))""##);
        bulk_l_double_paren_bit_or => (r#"| arith"#, r##"print -r "$(( 1 | 2 ))""##);
        bulk_l_string_ord_le_two_digits => (r#"-lt two"#, r##"[[ 01 -lt 2 ]]; print -r "ltk=$?""##);
        bulk_l_umask_minus_S_symbolic => (r#"umask -S"#, r##"print -r "$(umask -S)""##);
        bulk_l_print_commands_assoc_some_key => (r#"commands[ls]"#, r##"print -r "${commands[ls]:-no_ls}""##);
        bulk_l_builtin_whence_p_ls => (r#"whence -p ls"#, r##"whence -p ls 2>/dev/null"##);
        bulk_l_print_first_sorted_option_key => (r#"(ok)options[1]"#, r##"olk=(${(ok)options}); print -r "${olk[1]}""##);
        bulk_l_array_Omega_reverse_numeric => (r#"(On) dup"#, r##"al_On2=(9 1 4); print -r "${(On)al_On2}""##);
        bulk_l_print_epochrealtime_plus => (r#"$+EPOCHREALTIME"#, r##"print -r "$+EPOCHREALTIME""##);
        bulk_l_eval_arith_print => (r#"eval arith"#, r##"eval 'print -r $(( 4 + 5 ))'"##);
        bulk_l_typeset_unique_array_flag => (r#"typeset -aU"#, r##"typeset -aU al_U=(z z y); print -r "$al_U""##);
        bulk_l_scalar_anchor_replace_all => (r#"//#/"#, r##"sl_aa='ax/x/x'; print -r "${sl_aa//\/_}""##);
        bulk_l_array_join_newline_j_flag => (r#"join newline"#, r##"al_jn=(u v); print -r "${(j:
    :)al_jn}""##);
        bulk_l_disable_builtin_where => (r#"disable where"#, r##"disable where 2>/dev/null; print -r "dwh=$?"; enable where 2>/dev/null; print -r "ewh=$?""##);
        bulk_l_unset_pattern_minus_m => (r#"unset -m"#, r##"unset -m 'uvp_l_*' 2>/dev/null; typeset uvp_l_x=1 uvp_l_y=2; unset -m 'uvp_l_*'; print -r "${+uvp_l_x}${+uvp_l_y}""##);
        bulk_l_hyphen_paren_subshell_exit => (r#"( subshell )"#, r##"( exit 4 ); print -r "sex=$?""##);
        bulk_l_command_true_then_print => (r#"command true"#, r##"command true; print -r no_ex_l"##);
    }
}

/// Thirteenth batch: **`zsh -fc` / `zsh -c` startup surface** (`$+SHINSTDIN`, `ZSH_SCRIPT`, top-level
/// `$#`, `$+ZSH_EXECUTION_STRING`), **named-directory** `${(D)PWD}`, **`source` / `.` on `/dev/null`**,
/// **`ZSH_EVAL_CONTEXT`** (toplevel line, **subshell**, **function**, **anonymous function**),
/// non-interactive / **no-ZLE** probes (`[[ -v PS1 ]]`, `[[ -o zle ]]`, `$+LINES` / `$+COLUMNS`, `[[ -t 1 ]]`),
/// **brace no-op group**, **`$_` after `:`**, **array subslice assign**, **`posixargzero`**, **`command -p`**,
/// **`read -k0`**, **`emulate -L`**, **`local` shadowing**, **`fc -l`** on empty history, **`zle -l`** errors,
/// **`braceccl`**, **here-string `read`**, **`print -r --`**, **`unset`**, **`coproc`**, **`whence -p`**,
/// **`nullglob`** empty-dir **`(*)`**, **`exec {fd}`** read, **`pushd -q`**, **`$*` / `$@`**, **`typeset -r`**,
/// **arith `**`**, **lexicographic `[[ … < … ]]`**, **`while` zero iterations**, **`assoc` `(ok)` keys**, **`printf` width**.
mod corpus_dash_fc_bulk_m {
    use super::*;

    parity_gap_tests! {
        bulk_m_fc_plus_SHINSTDIN => (r#"$+SHINSTDIN (-fc)"#, r##"print -r "shin=$+SHINSTDIN""##);
        bulk_m_fc_ZSH_SCRIPT_default => (r#"ZSH_SCRIPT (-fc)"#, r##"print -r "zs=${ZSH_SCRIPT-unset}""##);
        bulk_m_fc_argc_top_level => (r#"$# argv (-fc)"#, r##"print -r "argc=$#""##);
        bulk_m_fc_named_dir_DPWD_home => (r#"hash -d + (D)PWD"#, r##"hash -d hm_m=$HOME; builtin cd $HOME; print -r "${(D)PWD}""##);
        bulk_m_fc_source_dev_null => (r#"source /dev/null"#, r##"source /dev/null; print -r "sc=$?""##);
        bulk_m_fc_dot_dev_null => (r#". /dev/null"#, r##". /dev/null; print -r "dc=$?""##);
        bulk_m_fc_ZSH_EVAL_CONTEXT_subshell => (r#"ZSH_EVAL_CONTEXT subshell"#, r##"print -r "top=$ZSH_EVAL_CONTEXT"; ( print -r "sub=$ZSH_EVAL_CONTEXT" )"##);
        bulk_m_fc_ZSH_EVAL_CONTEXT_function => (r#"ZSH_EVAL_CONTEXT function"#, r##"fn_ec_m() { print -r "fn=$ZSH_EVAL_CONTEXT" }; fn_ec_m"##);
        bulk_m_fc_ZSH_EVAL_CONTEXT_anon_fn => (r#"ZSH_EVAL_CONTEXT anon"#, r##"() { print -r "an=$ZSH_EVAL_CONTEXT" }"##);
        bulk_m_fc_empty_brace_group => (r#"{ :; } no-op"#, r##"{ :; }; print -r brk_ok_m"##);
        bulk_m_fc_cond_v_PS1 => (r#"[[ -v PS1 ]]"#, r##"[[ -v PS1 ]]; print -r "vps=$?""##);
        bulk_m_fc_plus_PS1 => (r#"$+PS1"#, r##"print -r "p1p=$+PS1""##);
        bulk_m_fc_plus_LINES_COLUMNS => (r#"$+LINES $+COLUMNS"#, r##"print -r "lc=$+LINES $+COLUMNS""##);
        bulk_m_fc_cond_t_stdout_tty => (r#"[[ -t 1 ]]"#, r##"[[ -t 1 ]]; print -r "tty1=$?""##);
        bulk_m_fc_us_score_after_colon => (r#"$_ after :"#, r##":; print -r "us=$_""##);
        bulk_m_fc_array_subslice_assign => (r#"a[2]=( ) insert"#, r##"typeset -a am_ss=(p q r s); am_ss[2]=(x y); print -r "n=$#am_ss e3=$am_ss[3]""##);
        bulk_m_fc_setopt_posixargzero => (r#"posixargzero"#, r##"print -r "paz=$options[posixargzero]""##);
        bulk_m_fc_command_p_true => (r#"command -p true"#, r##"command -p true; print -r cmdp_ok_m"##);
        bulk_m_fc_read_k_zero_devnull => (r#"read -k0 /dev/null"#, r##"read -k 0 -u0 2>/dev/null; print -r "rk=$?""##);
        bulk_m_fc_emulate_L_zsh => (r#"emulate -L zsh"#, r##"emulate -L zsh; print -r emL_ok_m"##);
        bulk_m_fc_local_hides_global => (r#"local vs global"#, r##"vg_m=outer; fn_lv() { local vg_m=inner; print -r "in=$vg_m"; }; fn_lv; print -r "out=$vg_m""##);
        bulk_m_fc_fc_list_empty_hist => (r#"fc -l no hist"#, r##"fc -l -1 -1 2>/dev/null; print -r "fcl=$?""##);
        bulk_m_fc_zle_list_no_tty => (r#"zle -l -L"#, r##"zle -l 2>/dev/null; print -r "zllx=$?""##);
        bulk_m_fc_brace_ccl_lowercase => (r#"braceccl {a-c}"#, r##"setopt braceccl; print -r {a-c}"##);
        bulk_m_fc_read_herestring_word => (r#"read <<<"#, r##"read -r rs_m <<< 'hs_word_m'; print -r "$rs_m""##);
        bulk_m_fc_print_r_double_dash => (r#"print -r --"#, r##"print -r -- '--dd_m'"##);
        bulk_m_fc_unset_then_plus_param => (r#"unset + v"#, r##"typeset uv_m=1; unset uv_m; print -r "up=${+uv_m}""##);
        bulk_m_fc_coproc_placeholder => (r#"coproc : nohang"#, r##"coproc : 2>/dev/null; print -r "cpx=$?""##);
        bulk_m_fc_whence_p_sh => (r#"whence -p sh"#, r##"whence -p sh 2>/dev/null | head -1"##);
        bulk_m_fc_setopt_nounset_status => (r#"set -u option read"#, r##"setopt nounset; print -r "nou=$options[nounset]""##);
        bulk_m_fc_true_semicolon_chain => (r#":; true; print"#, r##":; true; print -r semi_m"##);
        bulk_m_fc_parameter_star_join => (r#"$* join"#, r##"set -- 'a b' c; print -r "$*""##);
        bulk_m_fc_parameter_at_second => (r#"$@[2] words"#, r##"set -- x 'y z'; print -r "n=$# e2=$@[2]""##);
        bulk_m_fc_typeset_r_readonly_scalar => (r#"typeset -r"#, r##"typeset -r rm_m=ro; ( rm_m=bad ) 2>/dev/null; print -r "rmv=$rm_m""##);
        bulk_m_fc_arith_pow_two_caret => (r#"arith ^^"#, r##"print -r "$(( 3 ** 2 ))""##);
        bulk_m_fc_nullglob_empty_dir_star => (r#"nullglob * empty"#, r##"setopt nullglob; td_ng=$(mktemp -d); ( builtin cd $td_ng && files_m=(*) && print -r "nf=$#files_m" ); command rm -rf $td_ng"##);
        bulk_m_fc_cond_exists_eq_dir => (r#"[[ -e . ]]"#, r##"[[ -e . ]]; print -r "edot=$?""##);
        bulk_m_fc_hash_named_cd_tilde => (r#"cd ~name"#, r##"hash -d nm2_m=/tmp; cd ~nm2_m; print -r "tpwd=${PWD:t}""##);
        bulk_m_fc_pushd_quiet_tmp => (r#"pushd -q /tmp"#, r##"pushd -q /tmp 2>/dev/null; print -r "pdq=$?"; popd >/dev/null 2>&1"##);
        bulk_m_fc_ZSH_EXECUTION_STRING_plus => (r#"$+ZSH_EXECUTION_STRING"#, r##"print -r "zes=$+ZSH_EXECUTION_STRING""##);
        bulk_m_fc_cond_no_zle_running => (r#"[[ -o zle ]]"#, r##"[[ -o zle ]]; print -r "nozle=$?""##);
        bulk_m_fc_array_keys_hash_sorted => (r#"assoc (ok)"#, r##"typeset -A Ak_m=(k2 v2 k1 v1); print -r "${(ok)Ak_m}""##);
        bulk_m_fc_sprintf_width_star => (r#"sprintf %*d"#, r##"print -r "$(printf '%*d' 4 7)""##);
        bulk_m_fc_exec_fd_read_file => (r#"exec {fd}<file"#, r##"tf_fd=$(mktemp); print -r line_m >$tf_fd; exec {fd_m}<$tf_fd; read -r rd_m <&$fd_m; print -r "$rd_m"; exec {fd_m}>&-; command rm -f $tf_fd"##);
        bulk_m_fc_double_bracket_str_lt => (r#"[[ '10' < '2' ]]"#, r##"[[ '10' < '2' ]]; print -r "slt=$?""##);
        bulk_m_fc_while_false_once => (r#"while false once"#, r##"n_w=0; while false; do (( n_w++ )); done; print -r "nw=$n_w""##);
    }
}

/// Fourteenth batch: **`-fc` script semantics** — **`$(< file)`** (“read file”), **`ARGC`** (dash-c argv count),
/// **`OPTIND`**, **`IFS` / `LANG` / `LC_ALL`** probes, **pipeline `print | while read`**, **boolean `&&` / `||`**, **nested subshells**,
/// **`getopts` `-v:`**, **`set +A`** ksh array form, **`typeset -h`**, **`float` `+=`**, **associative key `\[[k]]`**, **`case` `|` branches**,
/// **`for` `break` / `continue`**, **single-char `[[ … = ? ]]`**, **`wait`** invalid pid, **`cd .`**, **`print -n` concat**, **`command -v`**, **`$+(functions|commands)[…]`**, **`umask`** numeric, **`[[ -o noexec]]`**, **`LINENO`**, **here-doc `read`**.
mod corpus_dash_fc_bulk_n {
    use super::*;

    parity_gap_tests! {
        bulk_n_fc_read_paren_less_file => (r#"$(< file)"#, r##"tf_n=$(mktemp); print -r qf_n >$tf_n; print -r "sub=$(<$tf_n)"; command rm -f $tf_n"##);
        bulk_n_fc_ARGC_matches_argc => (r#"ARGC"#, r##"print -r "ARGC=$ARGC argc=$#""##);
        bulk_n_fc_OPTIND_default => (r#"OPTIND start"#, r##"print -r "oi=$OPTIND""##);
        bulk_n_fc_IFS_set_plus => (r#"$+IFS"#, r##"print -r "ifsp=$+IFS""##);
        bulk_n_fc_LANG_LC_ALL => (r#"LANG LC_ALL"#, r##"print -r "lang=${LANG:-nog} lc=${LC_ALL:-noa}""##);
        bulk_n_fc_pipe_while_read => (r#"print | while read"#, r##"print -r pipe_ln | while IFS= read -r wr_n; do print -r "w=$wr_n"; done"##);
        bulk_n_fc_logic_or_after_and => (r#"&& || short"#, r##"true && false || print -r or_ok_n"##);
        bulk_n_fc_nested_subshell_print => (r#"( ( ) )"#, r##"( ( print -r dblsub_n ) )"##);
        bulk_n_fc_subshell_assign_hidden => (r#"subshell \$+"#, r##"( xv_n=1 ); print -r "hidden=$+xv_n""##);
        bulk_n_fc_getopts_v_colon => (r#"getopts v:"#, r##"set -- -v vn_arg; OPTIND=1; getopts "v:" go_n; print -r "o=$go_n a=$OPTARG""##);
        bulk_n_fc_set_plus_A_array => (r#"set +A ary"#, r##"set +A ary_n a b c; print -r "cn=$#ary_n t=$ary_n[3]""##);
        bulk_n_fc_typeset_hide_attr_h => (r#"typeset -h"#, r##"typeset -h hid_n=8; print -r "hv=$hid_n""##);
        bulk_n_fc_float_plus_assign => (r#"float +="#, r##"float fn_n=1.0; (( fn_n += 0.5 )); print -r "$fn_n""##);
        bulk_n_fc_assoc_bracket_key => (r#"A[[k]]"#, r##"typeset -A Ab_n; Ab_n[\[k\]]=bk_n; print -r "${Ab_n[[k]]}""##);
        bulk_n_fc_case_pipe_branch => (r#"case a|b"#, r##"case b_n in a|b_n) print -r dual_case;; *) print -r bad_case;; esac"##);
        bulk_n_fc_for_break => (r#"for break"#, r##"for br_n in 1; do break; print -r never_br; done; print -r broke_n"##);
        bulk_n_fc_for_continue => (r#"for continue"#, r##"for cn_n in 1 2; do [[ $cn_n == 1 ]] && continue; print -r "cx=$cn_n"; done"##);
        bulk_n_fc_cond_single_char_glob => (r#"[[ = ? ]]"#, r##"[[ z_n = ? ]]; print -r "qg=$?""##);
        bulk_n_fc_wait_bad_pid => (r#"wait 999999"#, r##"wait 999999 2>/dev/null; print -r "wx=$?""##);
        bulk_n_fc_cd_dot_status => (r#"cd ."#, r##"builtin cd .; print -r "cdd=$?""##);
        bulk_n_fc_print_n_concat => (r#"print -n"#, r##"print -n pre_n; print -r suf_n"##);
        bulk_n_fc_command_v_builtin => (r#"command -v print"#, r##"command -v print 2>/dev/null"##);
        bulk_n_fc_plus_functions_typo => (r#"$+functions[nodef]"#, r##"print -r "nf=$+functions[fn_typo_nonexistent_n]""##);
        bulk_n_fc_plus_commands_cat => (r#"$+commands[cat]"#, r##"print -r "cc=$+commands[cat]""##);
        bulk_n_fc_umask_octal => (r#"umask num"#, r##"print -r "$(umask)""##);
        bulk_n_fc_cond_o_noexec => (r#"[[ -o noexec ]]"#, r##"[[ -o noexec ]]; print -r "nex=$?""##);
        bulk_n_fc_LINENO_advances => (r#"LINENO"#, r##"l1_n=$LINENO
l2_n=$LINENO
print -r "delta=$(( l2_n - l1_n ))""##);
        bulk_n_fc_read_heredoc_line => (r#"read <<"#, r##"read -r hd_n <<HDN
hdline_n
HDN
print -r "$hd_n""##);
        bulk_n_fc_until_once_break => (r#"until break"#, r##"until true; do print -r once_ut; break; done"##);
        bulk_n_fc_arith_max_two => (r#"max(( ))"#, r##"print -r "$(( 3 > 2 ? 3 : 2 ))""##);
        bulk_n_fc_scalar_colon_question => (r#"${x?}"#, r##"xn_msg_n=okq; print -r "${xn_msg_n?fail}"##);
        bulk_n_fc_array_reverse_Omega_cap => (r#"${(Om)}"#, r##"am_om=(z y x); print -r "${(Om)am_om}""##);
        bulk_n_fc_glob_qual_N_one_file => (r#"*(N) one"#, r##"setopt nullglob; td_one=$(mktemp -d); touch $td_one/onlyf; ( builtin cd $td_one && print -r "nf=$#*(N)" ); command rm -rf $td_one"##);
    }
}

/// Fifteenth batch: **`-fc`** — **`argv[-1]`**, **arith precedence**, **nested brace expansion**, **`command cat` + heredoc**,
/// **process substitution** `read < <(…)`, **associative `(ov)`**, **`[[ =~ ]]` + `$MATCH`**, **`print -P`** + **`promptsubst`**,
/// **`noglob` literal `*`**, **`typeset -i` `010`**, **`[[ -gt ]]`**, **post-increment `$(( i++ ))`**, **`dirstack[1]`** default,
/// **empty array count**, **float compare chain**, **array prepend**, **`eval` arith**, **`if !`**, **`ARGV`/`argv`**, **`pipestatus`** after `|`,
/// **`zstyle -T`**, **`${(%)PS1}`**, **`(j:,:)` join**, **`[[ -o trackall ]]`**, **`mktemp -t`**, **`chmod` + `[[ -x ]]`**, **nested `if`**, **`typeset +m`**,
/// **`$?` after `: `**, **`#schedules`**, **`zmodload -Re`** failure, **`trap '' USR1`**.
mod corpus_dash_fc_bulk_o {
    use super::*;

    parity_gap_tests! {
        bulk_o_fc_argv_last_negative => (r#"argv[-1]"#, r##"set -- a_o b_o c_o; print -r "$argv[-1]""##);
        bulk_o_fc_arith_mul_precedence => (r#"2+3*4"#, r##"print -r "$(( 2 + 3 * 4 ))""##);
        bulk_o_fc_brace_nested_pairs => (r#"x{1,2}y{3,4}"#, r##"print -r x{1,2}y{3,4}"##);
        bulk_o_fc_command_cat_heredoc => (r#"command cat <<"#, r##"command cat <<HD_O
hc_line_o
HD_O"##);
        bulk_o_fc_proc_subst_cat_read => (r#"cat < <( )"#, r##"read -r lr_o < <( print -r proc_so ); print -r "$lr_o""##);
        bulk_o_fc_assoc_sorted_values_ov => (r#"(ov) assoc"#, r##"typeset -A Aov_o=(zk va zi vb); print -r "${(ov)Aov_o}""##);
        bulk_o_fc_regex_match_MATCH_var => (r#"=~ MATCH"#, r##"[[ str_oob =~ oob ]]; print -r "$MATCH""##);
        bulk_o_fc_print_P_cond_yes => (r#"print -P %(\?.)"#, r##"setopt promptsubst; true; print -P '%(?.y_p.n_p)'"##);
        bulk_o_fc_noglob_literal_star => (r#"noglob *literal"#, r##"noglob print -r '*.no_glob_expand_o_zz'"##);
        bulk_o_fc_integer_leading_zero_ten => (r#"typeset -i 010"#, r##"typeset -i io_o=010; print -r "$io_o""##);
        bulk_o_fc_cond_int_gt => (r#"-gt"#, r##"[[ 7 -gt 3 ]]; print -r "igt=$?""##);
        bulk_o_fc_arith_postincrement_print => (r#"i++ (( ))"#, r##"integer ip_o=0; print -r "$(( ip_o++ )) post=$ip_o""##);
        bulk_o_fc_dirstack_sub_empty => (r#"dirstack[1]"#, r##"print -r "${dirstack[1]:-ds_empty_o}""##);
        bulk_o_fc_empty_array_hash_count => (r#"${#empty ary}"#, r##"ea_o=(); print -r "$#ea_o""##);
        bulk_o_fc_float_compare_chain => (r#"float < &&"#, r##"(( 0.5 < 1.5 && 1.5 < 2.5 )); print -r "flt=$?""##);
        bulk_o_fc_array_prepend_spread => (r#"a=(1 pre)"#, r##"ap_o=(mid); ap_o=(first $ap_o); print -r "$ap_o[1]-$ap_o[2]""##);
        bulk_o_fc_eval_arith_seven_eight => (r#"eval arith"#, r##"eval 'print -r $((7 * 8))'"##);
        bulk_o_fc_if_bang_false => (r#"if ! false"#, r##"if ! false; then print -r if_neg_o; fi"##);
        bulk_o_fc_ARGV_alias_argv => (r#"ARGV argv"#, r##"argv=(only_o); print -r "a1=$ARGV[1]""##);
        bulk_o_fc_pipestatus_two_cmds => (r#"pipestatus"#, r##"false | true; print -r "${pipestatus[1]} ${pipestatus[2]}""##);
        bulk_o_fc_zstyle_T_default => (r#"zstyle -T"#, r##"zstyle -T ':completion_o:*' foo_o; print -r "zst=$?""##);
        bulk_o_fc_percent_expand_PS1_simple => (r#"${(%)PS1}"#, r##"PS1='> '; setopt promptsubst; print -r "${(%)PS1}""##);
        bulk_o_fc_array_join_comma_j => (r#"(j:,:) join"#, r##"aj_o=(u v w); print -r "${(j:,:)aj_o}""##);
        bulk_o_fc_cond_o_trackall => (r#"[[ -o trackall ]]"#, r##"[[ -o trackall ]]; print -r "tka=$?""##);
        bulk_o_fc_mktemp_template => (r#"mktemp tmp.XXX"#, r##"mt_o=$(mktemp -t tmp_o.XXXXXX); [[ -f $mt_o ]]; print -r "mkf=$?"; command rm -f $mt_o"##);
        bulk_o_fc_execbit_mktemp => (r#"chmod +x [[ -x ]]"#, r##"tx_o=$(mktemp); printf '#!/bin/sh\necho x\n' >$tx_o; chmod +x $tx_o; [[ -x $tx_o ]]; print -r "xxo=$?"; command rm -f $tx_o"##);
        bulk_o_fc_nested_if_else => (r#"if if"#, r##"if true; then if false; then print -r bad_ni; else print -r ok_ni; fi; fi"##);
        bulk_o_fc_typeset_plus_m_one => (r#"typeset +m"#, r##"typeset one_o_nm=1; typeset +m one_o_nm"##);
        bulk_o_fc_status_after_colon_chain => (r#"$? :"#, r##"false; true; :; print -r "stc=$?""##);
        bulk_o_fc_schedules_count => (r#"#schedules"#, r##"print -r "nsch=${#schedules}""##);
        bulk_o_fc_zmodload_R_e_dummy => (r#"zmodload -Re"#, r##"zmodload -Re zsh/none_such_mod_o 2>/dev/null; print -r "zre=$?""##);
        bulk_o_fc_trap_usr1_dummy => (r#"trap USR1"#, r##"trap '' USR1; print -r trap_usr_ok_o"##);
    }
}

/// Sixteenth batch: **`-fc` hook arrays** (`zshexit_functions`, `periodic_functions`, `preexec_functions`),
/// **`HISTFILE` unset**, **`TMOUT` / `REPORTTIME`** defaults, **anonymous function `$1`**, **`select` one shot**,
/// **`zstyle -d`**, **`print -u2`** + stdout, **`zcompile`** empty file, **compound `[[ … && ( … || … ) ]]`**,
/// **signed `(( … ))` chain**, **`unalias -a`**, **`bindkey -N` / `-D`**, **`dirs -p` line count**, **`printf` octal** byte.
mod corpus_dash_fc_bulk_p {
    use super::*;

    parity_gap_tests! {
        bulk_p_fc_zshexit_functions_count => (r#"#zshexit_functions"#, r##"print -r "zex=${#zshexit_functions}""##);
        bulk_p_fc_periodic_functions_count => (r#"#periodic_functions"#, r##"print -r "pdc=${#periodic_functions}""##);
        bulk_p_fc_preexec_functions_count => (r#"#preexec_functions"#, r##"print -r "pex=${#preexec_functions}""##);
        bulk_p_fc_anon_fn_first_arg => (r#"anon $1"#, r##"() { print -r "$1"; } arg_p_one"##);
        bulk_p_fc_histfile_unset_hyphen => (r#"HISTFILE unset (-fc)"#, r##"unset HISTFILE 2>/dev/null; print -r "${HISTFILE-hist_unset_p}""##);
        bulk_p_fc_TMOUT_default => (r#"TMOUT"#, r##"print -r "tmo=${TMOUT:-nil_tm_p}""##);
        bulk_p_fc_REPORTTIME_default => (r#"REPORTTIME"#, r##"print -r "rpt=${REPORTTIME:-nil_rt_p}""##);
        bulk_p_fc_select_first_word => (r#"select break"#, r##"select sp_p in wx wy; do print -r "sel=$sp_p"; break; done"##);
        bulk_p_fc_zstyle_delete_style => (r#"zstyle -d"#, r##"zstyle -d ':bulk_p_del:*' missing_style_p 2>/dev/null; print -r "zsd=$?""##);
        bulk_p_fc_print_u2_then_r => (r#"print -u2"#, r##"print -u2 err_u2_p_line 2>/dev/null; print -r after_u2_p"##);
        bulk_p_fc_zcompile_empty_file => (r#"zcompile empty"#, r##"zf_p=$(mktemp); : >$zf_p; zcompile $zf_p 2>/dev/null; print -r "zce=$?"; command rm -f $zf_p $zf_p.zwc"##);
        bulk_p_fc_cond_and_paren_or => (r#"[[ && ( || ) ]]"#, r##"[[ 1 -eq 1 && ( 2 -eq 2 || 3 -eq 9 ) ]]; print -r "cpa=$?""##);
        bulk_p_fc_arith_signed_compare_chain => (r#"(( -2 < -1 ))"#, r##"(( -2 < -1 && -1 < 0 )); print -r "sgc=$?""##);
        bulk_p_fc_unalias_all_count => (r#"unalias -a"#, r##"unalias -a 2>/dev/null; print -r "nac=${#aliases}""##);
        bulk_p_fc_bindkey_new_map_roundtrip => (r#"bindkey -N -D"#, r##"bindkey -N kp_map_p 2>/dev/null; bkn=$?; bindkey -D kp_map_p 2>/dev/null; bkd=$?; print -r "n=$bkn d=$bkd""##);
        bulk_p_fc_dirs_p_line_count => (r#"#dirs -p lines"#, r##"dp_p=(${(f)"$(dirs -p)"}); print -r "dpl=$#dp_p""##);
        bulk_p_fc_printf_octal_byte_a => (r#"printf \\141"#, r##"printf -v op_p '\141'; print -r "$op_p""##);
    }
}

/// Seventeenth batch: **history / temp defaults** (`HISTORY_IGNORE`, `HISTIGNORE`, `TMPPREFIX`),
/// **`:=` default-assign**, **arith `(( ary[2]=… ))`** on a seeded array, **`trap '' DEBUG`**, **`zstyle -t`**,
/// **`##` char code** in `(( ))`, **`[[ ! false ]]`,** **`case *pat`**, C-style **`for ((;;))`**, **`read` with `IFS=`**,
/// **`||` short-circuit status**, **`${(%):-%#}`**, **`typeset -F1`**, **`emulate -L sh`** line.
mod corpus_dash_fc_bulk_q {
    use super::*;

    parity_gap_tests! {
        bulk_q_fc_history_ignore_default => (r#"HISTORY_IGNORE"#, r##"print -r "hue=${HISTORY_IGNORE-unset_hu_q}""##);
        bulk_q_fc_histignore_default => (r#"HISTIGNORE"#, r##"print -r "hi=${HISTIGNORE-unset_hi_q}""##);
        bulk_q_fc_tmp_prefix_default => (r#"TMPPREFIX"#, r##"print -r "tp=${TMPPREFIX-nil_tp_q}""##);
        bulk_q_fc_typeset_F1_round => (r#"typeset -F1"#, r##"typeset -F1 fq_q_types=2.25; print -r "$fq_q_types""##);
        bulk_q_fc_colon_eq_assign => (r#":= assign"#, r##"unset v_col_q 2>/dev/null; : "${v_col_q:=eq_assign_q}"; print -r "$v_col_q""##);
        bulk_q_fc_arith_array_subscript_assign => (r#"(( ary[2]= ))"#, r##"typeset -a ary_q=(px_q); (( ary_q[2]=9 )); print -r "${ary_q[1]}-${ary_q[2]}""##);
        bulk_q_fc_trap_debug_noop => (r#"trap DEBUG"#, r##"trap '' DEBUG; print -r dbg_trap_ok_q"##);
        bulk_q_fc_zstyle_bool_return => (r#"zstyle -t"#, r##"zstyle ':bulk_q_zst:*' bq true; zstyle -t ':bulk_q_zst:*' bq; print -r "zqt=$?""##);
        bulk_q_fc_arith_char_code_x => (r#"(( ## x ))"#, r##"print -r "$(( ##x ))""##);
        bulk_q_fc_cond_bang_false => (r#"[[ ! false ]]"#, r##"[[ ! false ]]; print -r "bnf=$?""##);
        bulk_q_fc_case_glob_star => (r#"case *pat"#, r##"case cc_q_xyz in *q_x*) print -r case_glob_ok;; esac"##);
        bulk_q_fc_for_c_style_increment => (r#"for (( ;; ))"#, r##"integer iq_q=0; for (( iq_q=0; iq_q<3; iq_q++ )); do :; done; print -r "iloop=$iq_q""##);
        bulk_q_fc_read_ifs_colon_split => (r#"IFS=: read"#, r##"IFS=: read -r rq_a rq_b <<< 'left_q:right_q'; print -r "$rq_a=$rq_b""##);
        bulk_q_fc_logic_or_short_circuit => (r#"false \|\| true \$?"#, r##"false || true; print -r "lor=$?""##);
        bulk_q_fc_percent_prompt_hash => (r#"${(%):-%#}"#, r##"print -r "ph=${(%):-%#}""##);
        bulk_q_fc_emulate_L_sh_line => (r#"emulate -L sh"#, r##"emulate -L sh; print -r emulate_sh_line_q"##);
    }
}

/// Eighteenth batch: **`MAIL` default**, **`unset 'ary[i]'`**, **`source <( )`**, **`if` / `elif`**, **`until` counter**,
/// **`&&` / `||` list precedence**, **`(kv)` assoc pairs**, **`( : & )` / `$!`**, **`(#i)`** in `[[ … == … ]]`, **`printf %x`**,
/// **`typeset -Z2 -i`**, **`function` keyword**, **`cmp -s`**, **`[`** single-bracket test.
mod corpus_dash_fc_bulk_r {
    use super::*;

    parity_gap_tests! {
        bulk_r_fc_mail_default => (r#"MAIL"#, r##"print -r "ml=${MAIL-nil_ml_r}""##);
        bulk_r_fc_unset_array_middle_elt => (r#"unset ary[2]"#, r##"typeset -a ar_r=(e1_r e2_r e3_r); unset 'ar_r[2]'; print -r "n=$#ar_r t1=${ar_r[1]} t3=${ar_r[3]}""##);
        bulk_r_fc_source_proc_subst => (r#"source <( )"#, r##"source <( print -r 'print -r line_src_r' )"##);
        bulk_r_fc_if_elif_true_branch => (r#"if elif"#, r##"if false; then print -r bad_if_r; elif true; then print -r ok_elif_r; fi"##);
        bulk_r_fc_until_integer_equals_two => (r#"until -eq 2"#, r##"integer ur_r=0; until [[ $ur_r -eq 2 ]]; do ur_r=$(( ur_r + 1 )); done; print -r "ur=$ur_r""##);
        bulk_r_fc_and_or_precedence => (r#"&& \|\| status"#, r##"true && false || true; print -r "aoc=$?""##);
        bulk_r_fc_assoc_kv_pairs => (r#"(kv) assoc"#, r##"typeset -A Akr_r=(kr_a va_r kb_r vb_r); print -r "${(kv)Akr_r}""##);
        bulk_r_fc_background_null_wait_bang => (r#"( : & ) \$!"#, r##"( : & ); print -r "bg=$!""##);
        bulk_r_fc_case_insensitive_eq_pattern => (r#"(#i) =="#, r##"setopt extendedglob; [[ abc_r_xy == (#i)AbC_r_xY ]]; print -r "cie=$?""##);
        bulk_r_fc_printf_hex_byte_lowercase => (r#"printf %x 255"#, r##"print -r "$(printf '%x' 255)""##);
        bulk_r_fc_typeset_Z2_integer_seven => (r#"typeset -Z2 -i"#, r##"typeset -Z2 -i zi_r=7; print -r "$zi_r""##);
        bulk_r_fc_function_keyword_def => (r#"function { }"#, r##"function fn_kw_r { print -r fn_kw_r_body; }; fn_kw_r"##);
        bulk_r_fc_cmp_equal_two_files => (r#"cmp -s"#, r##"tc1_r=$(mktemp); tc2_r=$(mktemp); print -r same_r >$tc1_r; print -r same_r >$tc2_r; cmp -s $tc1_r $tc2_r; print -r "cmp=$?"; command rm -f $tc1_r $tc2_r"##);
        bulk_r_fc_bracket_builtin_eq => (r#"[ 3 -eq 3 ]"#, r##"[ 3 -eq 3 ]; print -r "brk=$?""##);
    }
}

/// Nineteenth batch: **`read` EOF** from `/dev/null`, **`[[ -r ]]`**, **`:+` / `:-` when unset**, **`(( \|\| ))`** status,
/// **array assign `"${(@)…}`**, **`typeset -i16`**, **`typeset -F2` / `-E2`**, **`while [[ $i -lt … ]]`**, **`case` `[:…:]` class**,
/// **command substitution `basename`**, **`[[ -z ${unset:-} ]]`**, **`//` global replace**, **`(s.:.)` split**, **`typeset -L3`** truncate.
mod corpus_dash_fc_bulk_s {
    use super::*;

    parity_gap_tests! {
        bulk_s_fc_read_eof_devnull => (r#"read </dev/null"#, r##"read -r em_s </dev/null; print -r "ex=$?""##);
        bulk_s_fc_cond_readable_devnull => (r#"[[ -r /dev/null ]]"#, r##"[[ -r /dev/null ]]; print -r "rr=$?""##);
        bulk_s_fc_param_colon_plus_dash_unset => (r#":+ :-"#, r##"unset xs_s; print -r "alt=${xs_s:+alt}${xs_s:-dash}""##);
        bulk_s_fc_arith_logical_or_zero_five => (r#"(( 0 \| 5 ))"#, r##"(( 0 || 5 )); print -r "lg=$?""##);
        bulk_s_fc_array_copy_glob_at => (r#"\"\${(@)ary}\""#, r##"typeset -a cp_s=(u v); typeset -a ac_s; ac_s=("${(@)cp_s}"); print -r "$#ac_s ${ac_s[2]}""##);
        bulk_s_fc_typeset_integer_16_ff => (r#"typeset -i16 255"#, r##"typeset -i16 hx_s=255; print -r "$hx_s""##);
        bulk_s_fc_typeset_F2_roundtrip => (r#"typeset -F2"#, r##"typeset -F2 ff_s=1.996; print -r "$ff_s""##);
        bulk_s_fc_while_lt_integer_two => (r#"while \$i -lt"#, r##"integer ws_s; while [[ $ws_s -lt 2 ]]; do ws_s=$(( ws_s + 1 )); done; print -r "w=$ws_s""##);
        bulk_s_fc_case_posix_char_class => (r#"case \[:\]"#, r##"case z_s in [[:alpha:]]_s) print -r cls;; *) print -r no;; esac"##);
        bulk_s_fc_basename_cmd_subst => (r#"basename"#, r##"print -r "$(basename /a/b/c_s_file)""##);
        bulk_s_fc_cond_z_empty_default_expand => (r#"[[ -z \${:- } ]]"#, r##"[[ -z ${unset_z_s:-} ]]; print -r "iz=$?""##);
        bulk_s_fc_scalar_double_slash_replace => (r#"// replace"#, r##"ss_s=axax; print -r "${ss_s//x/_}""##);
        bulk_s_fc_split_scalar_on_colon_s_flag => (r#"(s.:.)"#, r##"col_s=a.b.c; v_s=(${(s.:.)col_s}); print -r "$#v_s ${v_s[2]}""##);
        bulk_s_fc_typeset_L3_left_trunc => (r#"typeset -L3"#, r##"typeset -L3 ls_s=abcdef; print -r "$ls_s""##);
        bulk_s_fc_typeset_E2_float_round => (r#"typeset -E2"#, r##"typeset -E2 ef_s=9.876; print -r "$ef_s""##);
    }
}

/// Twentieth batch: **`dirname`**, **`typeset -R4`**, **array spread concat**, **`(k)`** assoc keys, **`${(%):-%d}`**,
/// **arith `(( var = cmp ))`**, **`[[ =~ ]]`**, **`(( && ))`**, **`:=` assign**, **`!` arith**, **`{a..c}suf`**, **`(ie)`** subscript,
/// **`(j:\\n:)` join**, **subshell exit / `local` scope**, **`$status` chain**, **`read` + `${#…}`** from temp file.
mod corpus_dash_fc_bulk_t {
    use super::*;

    parity_gap_tests! {
        bulk_t_fc_dirname_cmd_subst => (r#"dirname"#, r##"print -r "$(dirname /x/y/z_t_end)""##);
        bulk_t_fc_typeset_R4_right_pad => (r#"typeset -R4"#, r##"typeset -R4 rs_t=ab; print -r "[$rs_t]""##);
        bulk_t_fc_array_concat_spread => (r#"ary=(\$a \$b)"#, r##"typeset -a a1_t=(1 2); typeset -a a2_t=(3); typeset -a m_t; m_t=($a1_t $a2_t); print -r "$#m_t $m_t[3]""##);
        bulk_t_fc_assoc_keys_k_modifier => (r#"(k) assoc"#, r##"typeset -A At_t=(ka va kb vb); print -r "${(k)At_t}""##);
        bulk_t_fc_percent_prompt_d_cwd => (r#"\${(%):-%d}"#, r##"print -r "cwd_t=${(%):-%d}""##);
        bulk_t_fc_arith_assign_from_compare => (r#"(( n = 3 < 5 ))"#, r##"integer it_t; (( it_t = 3 < 5 )); print -r "it=$it_t""##);
        bulk_t_fc_cond_regex_match => (r#"=~ \^x"#, r##"[[ xyz_t =~ ^x..t$ ]]; print -r "rm=$?""##);
        bulk_t_fc_arith_double_and => (r#"(( 1 && 1 ))"#, r##"print -r "$(( 1 && 1 ))""##);
        bulk_t_fc_colon_eq_assign_blank => (r#": \":=\""#, r##"unset xa_t; : "${xa_t:=asg_t}"; print -r "$xa_t""##);
        bulk_t_fc_arith_logical_not_zero => (r#"!0 arith"#, r##"print -r "$(( !0 ))""##);
        bulk_t_fc_brace_alpha_range_suffix => (r#"{a..c}"#, r##"ARY_t=( {a..c}_t ); print -r "$#ARY_t $ARY_t[2]""##);
        bulk_t_fc_array_ie_first_match => (r#"(ie)"#, r##"ary_t=(xa xb xc); print -r "${ary_t[(ie)x*]}""##);
        bulk_t_fc_join_newline_j_flag => (r#"(j:\\n:)"#, r##"a_t=(p q); print -r "${(j:
    :)a_t}""##);
        bulk_t_fc_subshell_exit_code => (r#"( exit N )"#, r##"( exit 3 ); print -r "sx=$?""##);
        bulk_t_fc_subshell_local_hides => (r#"local in ( )"#, r##"( typeset x_sc_t=1; print -r "in=$x_sc_t" ); print -r "out=${+x_sc_t}""##);
        bulk_t_fc_status_after_false_true => (r#"\$status chain"#, r##"false; true; print -r "st=$status""##);
        bulk_t_fc_read_len_temp_file => (r#"read <file \${#}"#, r##"f_ln_t=$(mktemp); print -r abcd >$f_ln_t; read -r ln_t <$f_ln_t; print -r "${#ln_t}"; command rm -f $f_ln_t"##);
    }
}

/// Twenty-first batch: **`(o)` / `(on)` array sort**, **`#` / `##` strip**, **comma `(( ++, ++ ))`**, **`printf %04x`**, **`[[ -n \$empty ]]`**,
/// **`<<=`**, **`**` power**, **`\${#assoc}`**, **`\$@[-1]`**, **`(U)`**, **`//` / `/` replace**, **`~` arith**, **`2#` literal**, **`getopts`**, **`[[ -nt ]]`** vs `/dev/null`.
mod corpus_dash_fc_bulk_u {
    use super::*;

    parity_gap_tests! {
        bulk_u_fc_array_sort_o_modifier => (r#"(o) sort"#, r##"typeset -a s_u=(b a c); print -r "${(o)s_u}""##);
        bulk_u_fc_scalar_hash_shortest_prefix => (r#"# shortest prefix"#, r##"st_u=fofoo; print -r "${st_u#foo}""##);
        bulk_u_fc_scalar_hash_longest_prefix => (r#"## longest prefix"#, r##"st_u=fofoo; print -r "${st_u##foo}""##);
        bulk_u_fc_arith_comma_postincrement_twice => (r#"(( ++, ++ ))"#, r##"integer iu_u=0; (( iu_u++, iu_u++ )); print -r "$iu_u""##);
        bulk_u_fc_printf_lowercase_x_width => (r#"printf %04x"#, r##"print -r "$(printf '%04x' 10)""##);
        bulk_u_fc_cond_n_empty_param => (r#"[[ -n \$empty ]]"#, r##"eu_u=; [[ -n $eu_u ]]; print -r "nz=$?""##);
        bulk_u_fc_arith_shl_assign => (r#"<<="#, r##"integer iu_u=1; (( iu_u <<= 2 )); print -r "$iu_u""##);
        bulk_u_fc_arith_caret_power => (r#"\*\*"#, r##"print -r "$(( 3 ** 2 ))""##);
        bulk_u_fc_assoc_hash_count => (r#"${#assoc}"#, r##"typeset -A Bu_u=(ku vu); print -r "${#Bu_u}""##);
        bulk_u_fc_argv_minus_one_subscript => (r#"argv[-1]"#, r##"set -- a_u b_u; print -r "$@[-1]""##);
        bulk_u_fc_param_flag_uppercase => (r#"(U)"#, r##"low_u=abc; print -r "${(U)low_u}""##);
        bulk_u_fc_scalar_slash_slash_replace_all => (r#"// /y"#, r##"aaa_u=xxx; print -r "${aaa_u//x/y}""##);
        bulk_u_fc_arith_bitwise_not => (r#"~"#, r##"print -r "$(( ~7 ))""##);
        bulk_u_fc_arith_binary_radix_2 => (r#"2\#"#, r##"print -r "$(( 2#101 ))""##);
        bulk_u_fc_getopts_minus_a => (r#"getopts a"#, r##"set -- -a; OPTIND=1; getopts "a" go_u; print -r "o=$go_u""##);
        bulk_u_fc_array_sort_on_numeric => (r#"(on)"#, r##"typeset -a nu_u=(3 1 2); print -r "${(on)nu_u}""##);
        bulk_u_fc_cond_newer_than_devnull => (r#"-nt"#, r##"tf_u=$(mktemp); print -r x >$tf_u; [[ $tf_u -nt /dev/null ]]; print -r "nt=$?"; command rm -f $tf_u"##);
    }
}

/// Twenty-second batch: **`%` / `%%` suffix strip**, **`(i)` / `(I)`** on arrays, **`[[ - ]]`** (`-x`), **`16#` / `0x`**, **`(w)` words**,
/// **arith compare-assign**, shifts, **`&` / `^=`**, **`+=` / `+=()`**, nested **`${${…:+…}:-…}`**, **`[[ = ]]`** / **`&&`**, **multi-assign concat**,
/// **`read -d ''`**, **`whence -c` + slice**, **`++` prefix** in `(( ))`.
mod corpus_dash_fc_bulk_v {
    use super::*;

    parity_gap_tests! {
        bulk_v_fc_scalar_pct_short_suffix => (r#"% suffix"#, r##"sv_v=foobarx; print -r "${sv_v%x}""##);
        bulk_v_fc_scalar_pct_long_suffix => (r#"%% suffix"#, r##"sv_v=foobarx; print -r "${sv_v%%x}""##);
        bulk_v_fc_array_sort_ci_flag => (r#"(i) sort"#, r##"typeset -a av_v=(m z a); print -r "${(i)av_v}""##);
        bulk_v_fc_array_capital_I_subscript => (r#"(I) sub"#, r##"typeset -a av_v=(a b c); print -r "${av_v[(I)b]}""##);
        bulk_v_fc_cond_executable_bin_sh => (r#"[[ -x /bin/sh ]]"#, r##"[[ -x /bin/sh ]]; print -r "xx=$?""##);
        bulk_v_fc_arith_radix_16_ff => (r#"16\#ff"#, r##"print -r "$(( 16#ff ))""##);
        bulk_v_fc_param_flag_w_words => (r#"(w) words"#, r##"words_v='a b c'; print -r "${(w)words_v}""##);
        bulk_v_fc_arith_assign_gt_zero => (r#"(( n = 1>0 ))"#, r##"integer iv_v; (( iv_v=1>0 )); print -r "$iv_v""##);
        bulk_v_fc_arith_shift_right => (r#">>"#, r##"print -r "$(( 9 >> 2 ))""##);
        bulk_v_fc_arith_bitwise_and => (r#"& (( ))"#, r##"print -r "$(( 2#1010 & 2#1100 ))""##);
        bulk_v_fc_array_append_plus_eq => (r#"+= elts"#, r##"typeset -a avp_v=(q); avp_v+=(r s); print -r "$#avp_v ${avp_v[-1]}""##);
        bulk_v_fc_array_plus_eq_empty => (r#"+=()"#, r##"unset rv_v; rv_v=(1); rv_v+=(); print -r "n=$#rv_v""##);
        bulk_v_fc_nested_colon_plus_dash => (r#"\${\${:+}:-}"#, r##"unset osv_v; print -r "u=${${osv_v:+alt}:-def}"
osv_v=; print -r "e=${${osv_v:+alt}:-def}"
osv_v=x; print -r "s=${${osv_v:+alt}:-def}""##);
        bulk_v_fc_cond_string_single_eq => (r#"[[ = ]]"#, r##"[[ y_v = y_v ]]; print -r "eq=$?""##);
        bulk_v_fc_adjacent_param_concat => (r#"a=1 b=2 concat"#, r##"a_v=1 b_v=2; print -r "$a_v$b_v""##);
        bulk_v_fc_read_dash_d_empty_delim => (r#"read -d \"\""#, r##"printf '%s' a_v b_v | IFS= read -d '' x_v; print -r "${#x_v}""##);
        bulk_v_fc_whence_builtin_colon_head => (r#"whence -c :"#, r##"w_v=$(whence -c :); print -r "${w_v:0:12}""##);
        bulk_v_fc_arith_prefix_increment_print => (r#"++prefix"#, r##"integer iu_v=0; print -r "$(( ++iu_v )) post=$iu_v""##);
        bulk_v_fc_cond_and_and_integers => (r#"[[ && -eq ]]"#, r##"[[ 1 -eq 1 && 2 -eq 2 ]]; print -r "ac=$?""##);
        bulk_v_fc_arith_literal_hex_0x10 => (r#"0x10"#, r##"print -r "$(( 0x10 ))""##);
        bulk_v_fc_arith_xor_assign => (r#"\^="#, r##"xor_v=1; (( xor_v ^= 3 )); print -r "$xor_v""##);
    }
}

/// Twenty-third batch: **`[[ ! -e ]]`**, **string `<`**, **`-eq`**, **array `[i,j]`**, **comma `(( ))`**
/// (value + exit), **`command`**, **`unsetopt xtrace` / `$options`**, nested **`$(…)`**, **`=~`**, **`builtin print`**,
/// **`[[ -r ]]`**, **`%` arith**, **integer `/`**, **`${:-}`**, **`read` + `<<<`**, **`setopt nonomatch`**, **`[[ -L ]]`** on `/dev/stdin`,
/// **`[[ -s ]]`** on a temp file, **`(R)*` assoc subscript** with **`(k)`**, **arith `&&`** in `$(( ))`.
mod corpus_dash_fc_bulk_w {
    use super::*;

    parity_gap_tests! {
        bulk_w_fc_cond_not_exists => (r#"[[ ! -e ]]"#, r##"[[ ! -e /nonexistent_zfc_bulk_w ]]; print -r "ne=$?""##);
        bulk_w_fc_cond_string_lt => (r#"[[ < ]]"#, r##"[[ a_wz < b_wz ]]; print -r "lt=$?""##);
        bulk_w_fc_cond_int_eq => (r#"-eq"#, r##"[[ 1 -eq 1 ]]; print -r "eq=$?""##);
        bulk_w_fc_array_subscript_range => (r#"[i,j]"#, r##"ary_w=(u v w x); print -r "${ary_w[2,3]}""##);
        bulk_w_fc_arith_comma_value => (r#"$(( , ))"#, r##"print -r "cm=$(( 1 , 2 ))""##);
        bulk_w_fc_arith_comma_last_false => (r#"(( , 0 )) exit"#, r##"(( 1 , 0 )); print -r "cz=$?""##);
        bulk_w_fc_command_builtin => (r#"command"#, r##"command true; print -r "ct=$?""##);
        bulk_w_fc_unsetopt_xtrace_table => (r#"unsetopt xtrace"#, r##"unsetopt xtrace; print -r "xo=$options[xtrace]""##);
        bulk_w_fc_nested_cmd_subst => (r#"\$( \$( ))"#, r##"print -r "nest=$(print -r inner_wz)""##);
        bulk_w_fc_regex_match => (r#"=~"#, r##"[[ y_wz =~ ^y ]]; print -r "rx=$?""##);
        bulk_w_fc_builtin_print_r => (r#"builtin print"#, r##"builtin print -r bul_wz""##);
        bulk_w_fc_cond_readable_hosts => (r#"-r /etc/hosts"#, r##"[[ -r /etc/hosts ]]; print -r "rd=$?""##);
        bulk_w_fc_arith_mod => (r#"\%"#, r##"print -r "$(( 7 % 3 ))""##);
        bulk_w_fc_arith_int_div => (r#"typeset -i /"#, r##"typeset -i zi_w=5/2; print -r "$zi_w""##);
        bulk_w_fc_param_dash_colon_default => (r#"\${:-}"#, r##"print -r "${:-deflit_wz}""##);
        bulk_w_fc_read_string_redirect => (r#"read <<<"#, r##"IFS=: read -r a_wz _ <<< "p_wz:q_wz"; print -r "$a_wz""##);
        bulk_w_fc_setopt_nonnomatch_option => (r#"nonomatch"#, r##"unset NO_NOMATCH 2>/dev/null; setopt nonomatch 2>/dev/null; print -r "nn=$options[nomatch]""##);
        bulk_w_fc_cond_symlink_stdin => (r#"[[ -L /dev/stdin ]]"#, r##"[[ -L /dev/stdin ]]; print -r "ln=$?""##);
        bulk_w_fc_cond_nonempty_file => (r#"[[ -s ]]"#, r##"tf_wz=$(mktemp); print -r xy >$tf_wz; [[ -s $tf_wz ]]; print -r "sg=$?"; command rm -f $tf_wz"##);
        bulk_w_fc_assoc_reverse_glob_sub => (r#"(k)[(R)*]"#, r##"typeset -A Aw_wz=(k_wz v_wz); print -r "R=${(k)Aw_wz[(R)*]}""##);
        bulk_w_fc_arith_comma_assign => (r#"(( x=, ))"#, r##"integer xw_wz; print -r "$(( xw_wz=3, xw_wz+1 ))""##);
        bulk_w_fc_arith_logical_and_values => (r#"$(( && ))"#, r##"print -r "$(( 1 && 2 )) $(( 0 && 2 ))""##);
    }
}

/// Twenty-fourth batch: **`!` after `;`**, **`@kv` small assoc**, **`(s.:.)` / `(s:|:)`**, **`(Ws:x:)`**, **unary `**` precedences**,
/// **`funstack` count**, **comma `++` in `(( ))`**, **array `[-1]`**, **`! false` after `true`**, **`**` right-assoc chain**, **`/` truncation**,
/// **`(j:|:)`∘`(s:|:)`**, **`${:-}`**, **`[[ -b ]]`**, **`!=` in `$(( ))`**, **`for` words**, **`unset 'ary[i]'`**, **paren arith**, **`//` replace**.
mod corpus_dash_fc_bulk_x {
    use super::*;

    parity_gap_tests! {
        bulk_x_fc_bang_true_after_false => (r#"false; ! true"#, r##"false; ! true; print -r "ex=$?""##);
        bulk_x_fc_assoc_kv_at_small => (r#"@kv typeset -A"#, r##"typeset -A Hx_x=(kx_x vx_x); print -r "${(@kv)Hx_x}""##);
        bulk_x_fc_split_scalar_s_dot => (r#"(s.:.)"#, r##"sx_x=p:q:r; print -r "${(s.:.)sx_x}""##);
        bulk_x_fc_arith_unary_pow_chain => (r#"- ** **"#, r##"print -r "$(( -3 ** 2 )) $(( (-3) ** 2 ))""##);
        bulk_x_fc_word_split_custom_sep_x => (r#"(Ws:x:)"#, r##"wx_x='a x b x c'; print -r "${(Ws:x:)wx_x}""##);
        bulk_x_fc_funstack_depth => (r#"$#funstack fn"#, r##"fun_x() { print -r "fs=$#funstack"; }; fun_x"##);
        bulk_x_fc_arith_comma_postincr_twice => (r#"(( ++ , ++ ))"#, r##"integer nx_x=0; (( nx_x++, nx_x++ )); print -r "$nx_x""##);
        bulk_x_fc_array_minus_one_subscript => (r#"[-1]"#, r##"ary_x=(a_x b_x c_x); print -r "${ary_x[-1]}""##);
        bulk_x_fc_bang_false_after_true => (r#"true; ! false"#, r##"true; ! false; print -r "en=$?""##);
        bulk_x_fc_arith_pow_right_assoc => (r#"2**3**2"#, r##"print -r "$(( 2 ** 3 ** 2 ))""##);
        bulk_x_fc_split_scalar_s_pipe => (r#"(s:|:)"#, r##"sx2_x='|a|b|'; print -r "${(s:|:)sx2_x}""##);
        bulk_x_fc_arith_div_trunc => (r#"5 / -2"#, r##"print -r "$(( 5/-2 ))""##);
        bulk_x_fc_join_pipe_of_split_pipe => (r#"(j:|:)(s:|:)"#, r##"sx3_x=p:q; print -r "${(j:|:)${(s:|:)sx3_x}}""##);
        bulk_x_fc_arith_pow_left_assoc_chain => (r#"1**2**3"#, r##"print -r "$(( 1 ** 2 ** 3 ))""##);
        bulk_x_fc_param_dash_colon_empty => (r#"hi\${:-}"#, r##"print -r "hi${:-}""##);
        bulk_x_fc_cond_block_dev_zero => (r#"[[ -b /dev/zero ]]"#, r##"[[ -b /dev/zero ]]; print -r "blk=$?""##);
        bulk_x_fc_arith_bang_eq => (r#"!="#, r##"print -r "$(( 2!=3 )) $(( 2!=2 ))""##);
        bulk_x_fc_for_word_list => (r#"for words"#, r##"for x_x in 1_x 2_x; do print -r "$x_x"; done"##);
        bulk_x_fc_unset_array_subscript_sparse => (r#"unset ary[i]"#, r##"typeset -a uy_x=(q_x w_x e_x); unset 'uy_x[2]'; print -r "$#uy_x ${uy_x[1]}${uy_x[3]}""##);
        bulk_x_fc_arith_paren_mul => (r#"(2+3)*4"#, r##"print -r "$(( (2+3)*4 ))""##);
        bulk_x_fc_scalar_slash_slash_replace => (r#"// /_"#, r##"sx_x='  trim  '; print -r "${sx_x// /_}""##);
        bulk_x_fc_arith_pow_mul_chain => (r#"2**3*2"#, r##"print -r "$(( 2**3*2 ))""##);
    }
}

/// Twenty-fifth batch: **arith `<=` / `>=` / `!`**, **shifts** (`<<`, chained), **`[[` glob `==`**,
/// **sparse / range subscripts**, **`|` / `&` / `&=`**, **`%` strip**, **`while false`**, **`until` + `break`**, **`-p`**, **`< <( )`**, **`%` modulo sign**,
/// **ternary in `$(( ))`**, **`-i8`**, **`#` char code**, **lex `<`**, **`zmodload`**, **`(k)` assoc**, **`read` EOF**, **`:t`**, **`$*[i,-1]`**, **`zsh/datetime`** (optional load).
mod corpus_dash_fc_bulk_y {
    use super::*;

    parity_gap_tests! {
        bulk_y_fc_arith_print_le_two => (r#"$(( <= ))"#, r##"print -r "$(( 1 <= 2 )) $(( 2 <= 1 ))""##);
        bulk_y_fc_arith_shift_three => (r#"1<<3"#, r##"print -r "$(( 1 << 3 ))""##);
        bulk_y_fc_cond_glob_rhs_star => (r#"[[ == * ]]"#, r##"[[ pfx_y1_y == pfx_y*_y ]]; print -r "gm=$?""##);
        bulk_y_fc_array_sparse_index_three => (r#"a[3]="#, r##"typeset -a ay_y=(9); ay_y[3]=hi_y; print -r "${ay_y[3]} $#ay_y""##);
        bulk_y_fc_array_range_slice => (r#"[i,-1]"#, r##"ary_y=(a_y b_y c_y); print -r "${ary_y[2,-1]}""##);
        bulk_y_fc_arith_logical_or_zeros_and_val => (r#"|| 0 5"#, r##"print -r "$(( 0 || 0 )) $(( 0 || 5 ))""##);
        bulk_y_fc_arith_postdecrement_stmt => (r#"(( -- ))"#, r##"integer im_y=5; (( im_y-- )); print -r "$im_y""##);
        bulk_y_fc_scalar_percent_shortest => (r#"% pat"#, r##"sv_y=abc_y; print -r "${sv_y%b*}""##);
        bulk_y_fc_arith_bit_or_and => (r#"| & \$(( ))"#, r##"print -r "$(( 3|5 )) $(( 3&5 ))""##);
        bulk_y_fc_arith_print_le_and_chain => (r#"$(( <= && ))"#, r##"print -r "$(( 1 <= 2 && 2 <= 3 ))""##);
        bulk_y_fc_cond_z_empty_unset => (r#"[[ -z \$unset ]]"#, r##"unset e_y; [[ -z $e_y ]]; print -r "zz=$?""##);
        bulk_y_fc_while_false_zero_iter => (r#"while false"#, r##"while false; do print -r no_y; done; print -r "wd=$?""##);
        bulk_y_fc_until_increment_break => (r#"until break"#, r##"integer iu_y=0; until [[ $iu_y -eq 1 ]]; do iu_y=$((iu_y+1)); print -r u_y; break; done"##);
        bulk_y_fc_arith_logical_not_zero_nonzero => (r#"!0 !5"#, r##"print -r "$(( !0 )) $(( !5 ))""##);
        bulk_y_fc_cond_pipe_fd1 => (r#"-p /dev/fd/1"#, r##"[[ -p /dev/fd/1 ]]; print -r "pp=$?""##);
        bulk_y_fc_read_proc_subst => (r#"read < <( )"#, r##"read -r ry_y < <(print -r pipe_y); print -r "$ry_y""##);
        bulk_y_fc_arith_mod_signed => (r#"%% -9"#, r##"print -r "$(( 9%4 )) $(( -9%4 ))""##);
        bulk_y_fc_arith_print_ge_pair => (r#"$(( >= ))"#, r##"print -r "$(( 4 >= 4 )) $(( 3 >= 4 ))""##);
        bulk_y_fc_arith_less_than_vars => (r#"(( < vars ))"#, r##"integer ac_y=1 bc_y=2; (( ac_y<bc_y )); print -r "cmp=$?""##);
        bulk_y_fc_cond_same_file_ef_twice => (r#"-ef self"#, r##"file_y=$(mktemp); print -r z >"$file_y"; [[ "$file_y" -ef "$file_y" ]]; print -r "ef=$?"; command rm -f "$file_y""##);
        bulk_y_fc_arith_ternary_two_branches => (r#"\?:"#, r##"print -r "$(( 0 ? 9 : 8 )) $(( 1 ? 7 : 6 ))""##);
        bulk_y_fc_typeset_integer_octal_two => (r#"typeset -i8"#, r##"typeset -i8 oy_y=010; print -r "$oy_y""##);
        bulk_y_fc_arith_char_code_a => (r#"# char"#, r##"print -r "$(( #a ))""##);
        bulk_y_fc_cond_lex_lt_strings => (r#"[[ < strings ]]"#, r##"[[ abc_y < abd_y ]]; print -r "lt=$?""##);
        bulk_y_fc_zmodload_datetime_side_effect => (r#"zmodload datetime"#, r##"zmodload zsh/datetime 2>/dev/null; print -r "zm=$?""##);
        bulk_y_fc_assoc_keys_k_flag => (r#"(k) typeset -A"#, r##"typeset -A map_y=(ky_y vy_y); print -r "${(k)map_y}""##);
        bulk_y_fc_read_null_eof => (r#"read </dev/null"#, r##"read -r _y < /dev/null; print -r "rd=$?""##);
        bulk_y_fc_scalar_basename_colon_t => (r#":t"#, r##"sy_y=foo_y/bar_y; print -r "${sy_y:t}""##);
        bulk_y_fc_positional_star_slice => (r#"\$*[i,-1]"#, r##"set -- a_y b_y c_y; print -r "${*[2,-1]}""##);
        bulk_y_fc_arith_shift_left_chain => (r#"<< chained"#, r##"print -r "$(( 1 << 2 << 1 ))""##);
        bulk_y_fc_arith_bitand_assign => (r#"&= (( ))"#, r##"integer ixor_y=15; (( ixor_y &= 7 )); print -r "$ixor_y""##);
    }
}

/// Twenty-sixth batch: **arith `>` / `!` / `<<` / `>>`**, **`let`**, **`[[` `=` glob**, **`if :`**, **`printf`∘`read`**, **array copy+append**, **`(v)` / `(kv)`**, **post/prefix `++`**, **`#` / `%%`**, **`$( )`**, **`[[ -r / -O ]]`**, **overwrite `>f` + `-s`**, **`/=` / `%=`**, **`-i16`**, **`${PWD:n:m}`**, **signed `>>`**, **empty `$#ary`**, **`(j:,)∘(z)`**, **`(Ae)`**, **`true`/`false` in `$(( ))`**, **`.` = `.*`**, **function shadows `true`**, **`(ok)` empty scalar**, **`[[ -R ]]`**.
mod corpus_dash_fc_bulk_z {
    use super::*;

    parity_gap_tests! {
        bulk_z_fc_arith_print_gt_pair => (r#"$(( > ))"#, r##"print -r "$(( 2 > 1 )) $(( 1 > 2 ))""##);
        bulk_z_fc_let_assign => (r#"let"#, r##"let 'iz_z=3+4'; print -r "$iz_z""##);
        bulk_z_fc_cond_single_eq_glob => (r#"[[ = glob ]]"#, r##"[[ foo_z = fo* ]]; print -r "seq=$?""##);
        bulk_z_fc_if_colon => (r#"if :"#, r##"if :; then print -r if_z_ok; fi"##);
        bulk_z_fc_printf_pipe_read => (r#"printf|read"#, r##"printf '%d' 9 | read -r pz_z; print -r "$pz_z""##);
        bulk_z_fc_array_copy_append_elt => (r#"( )=( @ 3rd )"#, r##"typeset -a az1_z=(1_z 2_z); az1_z=( "${az1_z[@]}" 3_z ); print -r "$#az1_z ${az1_z[3]}""##);
        bulk_z_fc_assoc_values_v => (r#"(v) typeset -A"#, r##"typeset -A Az_z=(kz_vz wz_xz); print -r "${(v)Az_z}""##);
        bulk_z_fc_assoc_kv_at => (r#"@kv"#, r##"typeset -A Az2_z=(kz2_v2 wz2_x2); print -r "${(kv)Az2_z}""##);
        bulk_z_fc_arith_postincr_mid_print => (r#"post++ mid"#, r##"integer zz_z=9; print -r "x$(( zz_z++ )) y$(( zz_z ))""##);
        bulk_z_fc_scalar_hash_star_prefix => (r#"#*"#, r##"sz_z=abc; print -r "${sz_z#*b}""##);
        bulk_z_fc_scalar_pct_longest_before_underscore => (r#"%%_*"#, r##"sz2_z=ab_cd; print -r "${sz2_z%%_*}""##);
        bulk_z_fc_arith_prefix_incr_zero => (r#"++ on 0"#, r##"integer zz2_z=0; print -r "$(( ++zz2_z )) $zz2_z""##);
        bulk_z_fc_cmd_subst_print_r => (r#"$()"#, r##"rz_z=$(print -r subz); print -r "<$rz_z>""##);
        bulk_z_fc_cond_readable_root => (r#"-r /"#, r##"[[ -r / ]]; print -r "rr=$?""##);
        bulk_z_fc_cond_owned_tmp => (r#"-O /tmp"#, r##"[[ -O /tmp ]]; print -r "ow=$?""##);
        bulk_z_fc_redir_overwrite_nonempty => (r#">> -s"#, r##"tf_z=$(mktemp); print -r old >"$tf_z"; print -r new >"$tf_z"; [[ -s $tf_z ]]; print -r "sg=$?"; command rm -f "$tf_z""##);
        bulk_z_fc_arith_shr_chain => (r#">> >>"#, r##"print -r "$(( 4 >> 1 >> 1 ))""##);
        bulk_z_fc_arith_mod_assign => (r#"%= (( ))"#, r##"integer mz_z=40; (( mz_z %= 7 )); print -r "$mz_z""##);
        bulk_z_fc_arith_shift_zero_left => (r#"0<<"#, r##"print -r "$(( 0<<1 ))""##);
        bulk_z_fc_typeset_integer_hex_radix => (r#"typeset -i16"#, r##"typeset -i16 hx_z=255; print -r "$hx_z""##);
        bulk_z_fc_arith_div_assign => (r#"/= (( ))"#, r##"integer div_z=10; (( div_z /= 3 )); print -r "$div_z""##);
        bulk_z_fc_arith_bool_not_not => (r#"!!"#, r##"print -r "$(( !!7 )) $(( !!0 ))""##);
        bulk_z_fc_scalar_slice_offset_width => (r#"\${PWD:0:3}"#, r##"print -r "${PWD:0:3}""##);
        bulk_z_fc_arith_signed_shr => (r#"neg >>"#, r##"integer ar_z=-5; print -r "$(( ar_z >> 1 ))""##);
        bulk_z_fc_array_empty_count => (r#"\$# empty ()"#, r##"emptyarr_z=(); print -r "n=${#emptyarr_z}""##);
        bulk_z_fc_join_comma_of_z_split => (r#"(j:,:)(z)"#, r##"ws_z='a b c'; print -r "${(j:,:)${(z)ws_z}}""##);
        bulk_z_fc_array_assign_expand_empty => (r#"(Ae)="#, r##"typeset -a az_ez=(1); print -r "${(Ae)az_ez=}""##);
        bulk_z_fc_arith_bool_names_sum => (r#"true+false"#, r##"print -r "$(( true + false ))""##);
        bulk_z_fc_cond_dot_eq_dotstar => (r#". = .*"#, r##"[[ . = .* ]]; print -r "dp=$?""##);
        bulk_z_fc_function_shadows_builtin_true => (r#"true ()"#, r##"true () { print -r real_z; }; true; print -r after_true_z"##);
        bulk_z_fc_param_ok_empty_scalar => (r#"(ok) empty"#, r##"optsv_z=; print -r "${(ok)optsv_z}""##);
        bulk_z_fc_cond_readable_real_id => (r#"-R"#, r##"[[ -R /etc/hosts 2>/dev/null ]]; print -r "rdh=$?""##);
    }
}

/// Twenty-seventh batch (after `z`): **`typeset -l`**, **`[[ -a ]]` / `-h`**, **`for (( ;; ))`**, **`read -t`**, **bit `&`**, **`-nt`**, **`(U)`**, **arrays `@` / `+=` / `unset`**, **`(j)`∘`(s:,:)`**, **arith `~` / `^`**, **`unfunction`**, **`whence -p`+`read`**, **binary `2#`**, **`extendedglob`**, **`typeset -F2`**, **`hash -r`**, **`builtin` / `command`**, **`**`**, **ternary**, **`#?`**, **`(%)`**, **`$#` in function**, **`${:-}`**, **`${PWD:h}`**, **`(k)` / `(om)`**, **`(s:_:)`**, **`$''` length**, **`[[ == /* ]]`**, **`-H` + `${+}`**, **`=~`**, **`%` / `**` combo**, **`(ie)`**.
mod corpus_dash_fc_bulk_aa {
    use super::*;

    parity_gap_tests! {
        bulk_aa_fc_typeset_lower_el => (r#"typeset -l"#, r##"typeset -l lx_aa=Hi_aa; print -r "$lx_aa""##);
        bulk_aa_fc_cond_access_exists => (r#"-a file"#, r##"[[ -a /etc/hosts ]]; print -r "aa=$?""##);
        bulk_aa_fc_cond_symlink => (r#"-h /dev/stdin"#, r##"[[ -h /dev/stdin ]]; print -r "hh=$?""##);
        bulk_aa_fc_for_c_style_twice => (r#"for (( ))"#, r##"for (( iaa=0; iaa<2; iaa++ )); do print -r "i$iaa"; done"##);
        bulk_aa_fc_read_timeout_zero => (r#"read -t 0"#, r##"read -t 0 < /dev/null; print -r "rt=$?""##);
        bulk_aa_fc_arith_bit_and_seven => (r#"& 7"#, r##"print -r "$(( 15 & 7 ))""##);
        bulk_aa_fc_cond_newer_than => (r#"-nt"#, r##"tf1_aa=$(mktemp); tf2_aa=$(mktemp); print -r x >"$tf1_aa"; sleep 0; print -r y >"$tf2_aa"; [[ "$tf1_aa" -nt "$tf2_aa" ]]; print -r "nt=$?"; command rm -f "$tf1_aa" "$tf2_aa""##);
        bulk_aa_fc_param_flag_upper => (r#"(U)"#, r##"low_aa=abc_aa; print -r "${(U)low_aa}""##);
        bulk_aa_fc_array_unset_then_assign => (r#"unset ary"#, r##"unset pa_aa; pa_aa=(x y); print -r "${pa_aa[1]}${pa_aa[-1]}""##);
        bulk_aa_fc_array_at_flag => (r#"@ join words"#, r##"pa2_aa=(u v); print -r "${(@)pa2_aa}""##);
        bulk_aa_fc_join_space_of_split_comma => (r#"(j: )(s:,:)"#, r##"csv_aa=a,b,c; print -r "${(j: :)${(s:,:)csv_aa}}""##);
        bulk_aa_fc_arith_bit_not_neg_one => (r#"~"#, r##"integer neg_aa=-1; print -r "$(( ~neg_aa ))""##);
        bulk_aa_fc_arith_paren_xor => (r#"\^ |"#, r##"print -r "$(( (1|2)^(2|4) ))""##);
        bulk_aa_fc_builtin_unfunction_missing => (r#"unfunction"#, r##"unfunction zsh_main 2>/dev/null; print -r "uf=$?""##);
        bulk_aa_fc_whence_p_read_len => (r#"whence -p sh"#, r##"whence -p sh 2>/dev/null | read -r wp_aa; print -r "${#wp_aa}""##);
        bulk_aa_fc_arith_binary_literal_add => (r#"2#"#, r##"print -r "$(( 2#11 + 1 ))""##);
        bulk_aa_fc_cond_extendedglob_star => (r#"extglob *"#, r##"setopt extendedglob; [[ st123_aa == *123_aa ]]; print -r "ex=$?""##);
        bulk_aa_fc_typeset_float_two_places => (r#"typeset -F2"#, r##"typeset -F2 ff_aa=1.25; print -r "$ff_aa""##);
        bulk_aa_fc_builtin_hash_rehash => (r#"hash -r"#, r##"hash -r 2>/dev/null; print -r "hr=$?""##);
        bulk_aa_fc_builtin_print => (r#"builtin print"#, r##"builtin print -r bi_aa"##);
        bulk_aa_fc_command_print => (r#"command print"#, r##"command print -r cmd_aa"##);
        bulk_aa_fc_arith_pow_star_three => (r#"** 3"#, r##"integer xy_aa=2; print -r "$(( xy_aa ** 3 ))""##);
        bulk_aa_fc_arith_ternary_gt => (r#"? :"#, r##"print -r "$(( (3>-2) ? 1 : 0 ))""##);
        bulk_aa_fc_scalar_hash_one_char => (r#"#?"#, r##"ua_aa=4; print -r "${ua_aa#?}""##);
        bulk_aa_fc_array_append_plus_eq => (r#"+= tail"#, r##"unset ua_aa; ua_aa=(p q); ua_aa+=(r); print -r "$ua_aa[-1]""##);
        bulk_aa_fc_prompt_percent_hash => (r#"(%)#"#, r##"print -r "${(%)#}""##);
        bulk_aa_fc_function_arg_count_three => (r#"\$# 3"#, r##"c3_aa() { print -r "$#"; }; c3_aa one two three"##);
        bulk_aa_fc_join_newline_array => (r#"(j:\\n:)"#, r##"ab_aa=(l m); print -r "${(j:
    :)ab_aa}""##);
        bulk_aa_fc_param_equals_default_empty => (r#"\${=:-}"#, r##"print -r "${=:-}""##);
        bulk_aa_fc_dirname_plus_pwd => (r#":hPWD"#, r##"print -r "${PWD:h}$PWD""##);
        bulk_aa_fc_assoc_sorted_keys => (r#"(k) assoc"#, r##"typeset -A Aa2_aa=(mk_az vz_az); print -r "${(k)Aa2_aa}""##);
        bulk_aa_fc_array_sort_om => (r#"(om)"#, r##"aom_aa=(2 1 3); print -r "${(om)aom_aa}""##);
        bulk_aa_fc_arith_pow_zero => (r#"** 0"#, r##"print -r "$(( 2 ** 0 ))""##);
        bulk_aa_fc_split_scalar_underscore => (r#"(s:_:)"#, r##"sf2_aa=ab_cd_ef; print -r "${(s:_:)sf2_aa}""##);
        bulk_aa_fc_arith_pow_square => (r#"** 2"#, r##"integer mma_aa=9; print -r "$(( mma_aa ** 2 ))""##);
        bulk_aa_fc_cond_n_dollar_empty_quoting => (r#"-n \$''"#, r##"[[ -n $'' ]]; print -r "nq=$?""##);
        bulk_aa_fc_string_dollar_quote_len => (r#"\$'\\n' len"#, r##"line_aa=$'line1\nline2'; print -r "${#line_aa}""##);
        bulk_aa_fc_prompt_percent_d => (r#"(%)d"#, r##"print -r "${(%)d}""##);
        bulk_aa_fc_cond_abs_path_pattern => (r#"== /*"#, r##"[[ /bin/sh == /* ]]; print -r "abs=$?""##);
        bulk_aa_fc_typeset_hide_and_plus => (r#"typeset -H"#, r##"typeset -H hid_aa=h_val; print -r "${+hid_aa}""##);
        bulk_aa_fc_cond_regex_match => (r#"=~"#, r##"[[ foo =~ ^f ]]; print -r "rx=$?""##);
        bulk_aa_fc_arith_mod_and_pow_mix => (r#"% **"#, r##"print -r "$(( 127 % 10 )) $(( 5 ** 3 % 10 ))""##);
        bulk_aa_fc_array_ie_first_glob => (r#"(ie)"#, r##"aie_aa=(x1_aa x2_aa x3_aa); print -r "${aie_aa[(ie)x*]}""##);
    }
}

/// Twenty-eighth batch: **ternary `$(( ))`**, **`[[ -w / -x / -d ]]`**, **`typeset -E` arith**, **`+=` compound**, **`(i)`** sort,
/// **hex `&`**, **`(pj:\t:)`**, **`[ -eq ]`**, **`$ZSH_PATCHLEVEL`**, **`printf`+`read`**, **logical `$(( ))`**, **`//` squeeze**, **`&&`/`,` in `(( ))`**,
/// **`echo -n` / `basename`**, **`trap ''`**, **`${var:n}`** (unset + set), **`-i5`**, **shift-and-mask / `|`**, **array subscripts in arith**, **`set --` order**,
/// **`(I)`**, **`IFS=;` `<<<`**, **`**` sum**, **`--` post-decrement**, **`<`**, **`${:?}`**, **`a##` / `extendedglob`**, **`(@M)…:#`**, **`(OA)`**, **`unalias -m`**.
mod corpus_dash_fc_bulk_ab {
    use super::*;

    parity_gap_tests! {
        bulk_ab_fc_arith_ternary_ge => (r#"\? >="#, r##"print -r "$(( 3 >= 2 ? 5 : 9 ))""##);
        bulk_ab_fc_cond_writable_tmp => (r#"-w /tmp"#, r##"[[ -w /tmp ]]; print -r "wr=$?""##);
        bulk_ab_fc_cond_executable_sh => (r#"-x /bin/sh"#, r##"[[ -x /bin/sh ]]; print -r "xx=$?""##);
        bulk_ab_fc_float_div_in_arith => (r#"typeset -E /"#, r##"typeset -E ee_ab=1.5; print -r "$(( ee_ab / 2 ))""##);
        bulk_ab_fc_arith_plus_eq_mul => (r#"+= *"#, r##"integer ia_ab=3; (( ia_ab += 2 * 3 )); print -r "$ia_ab""##);
        bulk_ab_fc_array_sort_ci_modifier => (r#"(i) sort"#, r##"abmix_ab=(B_ab a_ab); print -r "${(i)abmix_ab}""##);
        bulk_ab_fc_arith_hex_and_mask => (r#"0x &"#, r##"print -r "$(( 0xFF & 0x0F ))""##);
        bulk_ab_fc_cond_is_dir_bin => (r#"-d /bin"#, r##"[[ -d /bin ]]; print -r "dd=$?""##);
        bulk_ab_fc_join_tab_array => (r#"(pj:\\t:)"#, r##"tw_ab=(x_ab y_ab); print -r "${(pj:\t:)tw_ab}""##);
        bulk_ab_fc_posix_bracket_eq => (r#"[ -eq ]"#, r##"[ 1 -eq 1 ]; print -r "pq=$?""##);
        bulk_ab_fc_zsh_patchlevel_or_nil => (r#"ZSH_PATCHLEVEL"#, r##"print -r "${ZSH_PATCHLEVEL:-nil}""##);
        bulk_ab_fc_read_after_printf_two_lines => (r#"printf|read"#, r##"printf '%s\n%s' a_ab b_ab | IFS= read -r ln_ab; print -r "${#ln_ab}""##);
        bulk_ab_fc_arith_ge_and_le => (r#"&& in \$(( ))"#, r##"print -r "$(( 6 >= 6 && 1 <= 2 ))""##);
        bulk_ab_fc_scalar_slash_slash_remove_space => (r#"//  del"#, r##"sz_ab='  x_ab  '; print -r "${sz_ab// /}""##);
        bulk_ab_fc_arith_and_comma_postincr => (r#"&& ++ ,"#, r##"integer ib_ab=1; (( ib_ab && ib_ab++, ib_ab++ )); print -r "$ib_ab""##);
        bulk_ab_fc_builtin_echo_dash_n => (r#"echo -n"#, r##"builtin echo -n e_ab; print -r z"##);
        bulk_ab_fc_basename_command_subst => (r#"basename"#, r##"print -r $(basename /x/y/z_ab)"##);
        bulk_ab_fc_trap_empty_sigint => (r#"trap '''' INT"#, r##"trap '' INT 2>/dev/null; print -r "tr=$?""##);
        bulk_ab_fc_scalar_slice_unset => (r#"\${v:3} unset"#, r##"print -r "${str_ab:3}""##);
        bulk_ab_fc_scalar_slice_offset => (r#"\${v:3} set"#, r##"str_ab=012345_ab; print -r "${str_ab:3}""##);
        bulk_ab_fc_typeset_int_radix_five => (r#"typeset -i5"#, r##"typeset -i5 iv_ab=17; print -r "$iv_ab""##);
        bulk_ab_fc_arith_shl_and_mask => (r#"<< &"#, r##"print -r "$(( 9 << 1 & 15 ))""##);
        bulk_ab_fc_arith_shl_or_one => (r#"<< \|"#, r##"print -r "$(( 1 << 2 | 1 ))""##);
        bulk_ab_fc_arith_array_cells_sum => (r#"ary[i] +"#, r##"vars_ab=(10 20); print -r "$(( vars_ab[1] + vars_ab[2] ))""##);
        bulk_ab_fc_arith_div_after_pair_assign => (r#"(( , / ))"#, r##"(( x_ab=8, y_ab=2 )); print -r "$(( x_ab/y_ab ))""##);
        bulk_ab_fc_set_swap_two_words => (r#"set -- swap"#, r##"set -- one_ab two_ab; print -r "$2$1""##);
        bulk_ab_fc_array_capital_I_match => (r#"(I) idx"#, r##"ary_ab=(a_ab b_ab c_ab); print -r "${ary_ab[(I)b_ab]}""##);
        bulk_ab_fc_read_ifs_semicolon_string => (r#"IFS=;"#, r##"IFS=\; read -r r1_ab r2_ab <<< "a_ab;b_ab"; print -r "$r1_ab$r2_ab""##);
        bulk_ab_fc_arith_sum_of_squares => (r#"**2 sum"#, r##"print -r "$(( 3**2 + 4**2 ))""##);
        bulk_ab_fc_arith_decrement_zero => (r#"--0"#, r##"integer id_ab=0; (( id_ab-- )); print -r "$id_ab""##);
        bulk_ab_fc_arith_lt_negative => (r#"< 0"#, r##"print -r "$(( -1 < 0 ))""##);
        bulk_ab_fc_param_colon_err_when_unset => (r#"\${:?\}"#, r##"unset fail_ab; : ${fail_ab:?ok_ab} 2>/dev/null; print -r "qe=$?""##);
        bulk_ab_fc_cond_eq_glob_hash_plain => (r#"a## plain"#, r##"[[ aa_ab == a##_ab ]]; print -r "hq=$?""##);
        bulk_ab_fc_cond_extglob_hash => (r#"a## extglob"#, r##"setopt extendedglob; [[ aa_ab == a##_ab ]]; print -r "hq2=$?""##);
        bulk_ab_fc_array_match_exclude_pattern => (r#"( @M ) :#"#, r##"path_ab=(/ /tmp); print -r "${(@M)path_ab:#/no_such_ab}""##);
        bulk_ab_fc_array_sort_OA_reverse => (r#"(OA)"#, r##"rev_ab=(3 1 2); print -r "${(OA)rev_ab}""##);
        bulk_ab_fc_unalias_m_empty_pattern => (r#"unalias -m"#, r##"unalias -m '' 2>/dev/null; print -r "ua=$?""##);
    }
}

/// Twenty-ninth batch: **signed int `/` / `%`**, **`-=` / `|=` / `^=`**, **conds `!=` / `!` / `-z` / `-e` / `-c`**, **`(n)`/`(o)`/`(oa)`/`(eu)`/`(L@)`**, **`dirname`**, **`**`**, **multi `[[ ]]`**, **bit shifts**, **`export`**, **`pushd`/`popd`**, **`$ZSH_NAME`**, **`//` replace**, **nested `?:`**, **`(Ie)`**, **arith subscript `0+2`**, **`(%)?`**, **`dirs`**, **`whence -v`**, **`wc`**, **`-ot`**, **`||` in `[ ]`**, **`-Z2 -i`**, **`%` / `#` strips**, **`(qq)`**, **nested array assign**, **`${:-}`**, **fn `return`**, **`cd .`**, **`pipestatus`**, **`(j: /:)`**, **octal `010`**, **`read -u0 -t0`**, **`typeset -A`**, **`bindkey|head`**, **chained `>=`/`==`**, **`-ne`**, **`(S)`**.
mod corpus_dash_fc_bulk_ac {
    use super::*;

    parity_gap_tests! {
        bulk_ac_fc_arith_signed_div_trunc => (r#"-7/2"#, r##"print -r "$(( -7 / 2 ))""##);
        bulk_ac_fc_arith_minus_eq => (r#"-="#, r##"integer ix_ac=5; (( ix_ac -= 3 )); print -r "$ix_ac""##);
        bulk_ac_fc_cond_z_unset_param => (r#"[[ -z ]]"#, r##"unset em_ac; [[ -z $em_ac ]]; print -r "ze=$?""##);
        bulk_ac_fc_cond_string_ne => (r#"[[ != ]]"#, r##"[[ foo_ac != bar_ac ]]; print -r "ne=$?""##);
        bulk_ac_fc_cond_bang_false => (r#"[[ ! false ]]"#, r##"[[ ! false ]]; print -r "bnf=$?""##);
        bulk_ac_fc_arith_bit_and => (r#"31 & 18"#, r##"print -r "$(( 31 & 18 ))""##);
        bulk_ac_fc_array_sort_numeric_n => (r#"(n)"#, r##"na_ac=(10 2 3); print -r "${(n)na_ac}""##);
        bulk_ac_fc_array_sort_o_modifier => (r#"(o)"#, r##"os_ac=(b_ac a_ac); print -r "${(o)os_ac}""##);
        bulk_ac_fc_dirname_command_subst => (r#"dirname"#, r##"print -r $(dirname /a/b/c_ac)"##);
        bulk_ac_fc_arith_pow_scalar_var => (r#"** scalar"#, r##"iy_ac=2; print -r "$(( iy_ac ** iy_ac ))""##);
        bulk_ac_fc_cond_int_rel_and_chain => (r#"-gt && -lt"#, r##"[[ 9 -gt 8 && 7 -lt 8 ]]; print -r "gl=$?""##);
        bulk_ac_fc_arith_shift_wide => (r#"16-bit >>"#, r##"print -r "$(( (1<<16)>>15 ))""##);
        bulk_ac_fc_export_roundtrip => (r#"export"#, r##"export EX_ac=e_ac; print -r "$EX_ac""##);
        bulk_ac_fc_pushd_popd_status => (r#"pushd popd"#, r##"pushd /tmp 2>/dev/null; popd >/dev/null; print -r "pd=$?""##);
        bulk_ac_fc_array_lower_at => (r#"(L@)"#, r##"ary_lc_ac=(X_ac y_ac); print -r "${(L@)ary_lc_ac}""##);
        bulk_ac_fc_param_zsh_name => (r#"ZSH_NAME"#, r##"print -r "${ZSH_NAME}""##);
        bulk_ac_fc_scalar_slash_slash_underscore_to_hyphen => (r#"//_ -"#, r##"sc_ac='a_b_c_ac'; print -r "${sc_ac//_/-}""##);
        bulk_ac_fc_arith_ior_assign => (r#"|= (( ))"#, r##"integer ia_ac=1; (( ia_ac |= 4 )); print -r "$ia_ac""##);
        bulk_ac_fc_arith_ixor_assign => (r#"^= (( ))"#, r##"integer ib_ac=7; (( ib_ac ^= 2 )); print -r "$ib_ac""##);
        bulk_ac_fc_arith_nested_ternary => (r#"?: nested"#, r##"print -r "$(( 0 ? 1 : (2 ? 3 : 4) ))""##);
        bulk_ac_fc_array_ie_last_match => (r#"(Ie)"#, r##"rv_ac=(9 8 1); print -r "${rv_ac[(Ie)9*]}""##);
        bulk_ac_fc_array_subscript_arith_sum => (r#"ary[0+2]"#, r##"ary_sub_ac=(p q r); print -r "${ary_sub_ac[0+2]}""##);
        bulk_ac_fc_prompt_percent_qmark => (r#"(%)?"#, r##"print -r "${(%)?}""##);
        bulk_ac_fc_builtin_dirs_status => (r#"dirs"#, r##"dirs 2>/dev/null; print -r "dr=$?""##);
        bulk_ac_fc_whence_v_read_slice => (r#"whence -v"#, r##"whence -v : 2>/dev/null | read -r wv_ac; print -r "${wv_ac:0:8}""##);
        bulk_ac_fc_wc_c_herestring => (r#"wc -c <<<"#, r##"wc -c <<< hi_ac 2>/dev/null | tr -d ' '"##);
        bulk_ac_fc_arith_mod_negative => (r#"% neg"#, r##"integer im_ac=-3; print -r "$(( im_ac % 2 ))""##);
        bulk_ac_fc_arith_bang_eq_nums => (r#"!="#, r##"print -r "$(( 1 != 0 ))""##);
        bulk_ac_fc_cond_exists_dev_null => (r#"-e"#, r##"[[ -e /dev/null ]]; print -r "en=$?""##);
        bulk_ac_fc_arith_eq_chain_print => (r#"=="#, r##"print -r "$(( 1==1 )) $(( 0==1 ))""##);
        bulk_ac_fc_cond_older_than_self => (r#"-ot"#, r##"tf_ac=$(mktemp); : >"$tf_ac"; [[ "$tf_ac" -ot "$tf_ac" ]]; print -r "ot=$?"; command rm -f "$tf_ac""##);
        bulk_ac_fc_cond_or_posix_int => (r#"[ \|\| ]"#, r##"[[ 1 -eq 1 || 0 -eq 1 ]]; print -r "or=$?""##);
        bulk_ac_fc_typeset_zero_pad_two => (r#"typeset -Z2 -i"#, r##"typeset -Z2 -i zi_ac=3; print -r "$zi_ac""##);
        bulk_ac_fc_scalar_pct_strip_short_suffix => (r#"%-"#, r##"st_ac=prefix_ac-suf_ac; print -r "${st_ac%-*}""##);
        bulk_ac_fc_scalar_hash_strip_long_prefix => (r#"##*"#, r##"st2_ac=pre_ac_suf_ac; print -r "${st2_ac#*_}""##);
        bulk_ac_fc_param_qq_quote => (r#"(qq)"#, r##"lit_ac='a b'; print -r "${(qq)lit_ac}""##);
        bulk_ac_fc_arith_signed_div_again => (r#"-9/4"#, r##"integer neg2_ac=-9; print -r "$(( neg2_ac / 4 ))""##);
        bulk_ac_fc_array_element_array_count => (r#"a[1]=( )"#, r##"unset mix_ac; mix_ac=(1 2); mix_ac[1]=(a b); print -r "$#mix_ac""##);
        bulk_ac_fc_param_colon_minus_literal => (r#"\${:-}"#, r##"print -r "${:-subst_ac}""##);
        bulk_ac_fc_cond_star_middle => (r#"* mid *"#, r##"[[ xyz_ac == *y* ]]; print -r "sy=$?""##);
        bulk_ac_fc_function_return_two => (r#"return 2"#, r##"fun_ac() { return 2; }; fun_ac; print -r "rs=$?""##);
        bulk_ac_fc_arith_nested_logical_or => (r#"\|\| nest"#, r##"print -r "$(( 0 || (0 || 7) ))""##);
        bulk_ac_fc_builtin_cd_dot => (r#"cd ."#, r##"cd . 2>/dev/null; print -r "cd=$?""##);
        bulk_ac_fc_pipestatus_false_true => (r#"pipestatus"#, r##"false | true; print -r "${pipestatus[1]} ${pipestatus[2]}""##);
        bulk_ac_fc_array_unique_eu => (r#"(eu)"#, r##"ary_eq_ac=(1 1 2); print -r "${(eu)ary_eq_ac}""##);
        bulk_ac_fc_join_space_slash => (r#"(j: /:)"#, r##"wa_ac=(x_ac y_ac); print -r "${(j: /:)wa_ac}""##);
        bulk_ac_fc_arith_octal_010 => (r#"010"#, r##"integer io_ac=010; print -r "$(( io_ac ))""##);
        bulk_ac_fc_cond_char_special => (r#"-c"#, r##"[[ -c /dev/null ]]; print -r "ch=$?""##);
        bulk_ac_fc_read_u0_t0 => (r#"read -u0 -t0"#, r##"read -u 0 -t 0 2>/dev/null; print -r "rd=$?""##);
        bulk_ac_fc_array_sort_oa_alpha => (r#"(oa)"#, r##"ord_ac=(d b a c); print -r "${(oa)ord_ac}""##);
        bulk_ac_fc_assoc_key_lookup => (r#"typeset -A ="#, r##"unset assoc_ac; typeset -A assoc_ac=(k1_ac v1_ac); print -r "${assoc_ac[k1_ac]}""##);
        bulk_ac_fc_arith_paren_add_div => (r#"(+)"#, r##"print -r "$(( (8+8) / 4 ))""##);
        bulk_ac_fc_bindkey_l_head_read => (r#"bindkey -l"#, r##"bindkey -l 2>/dev/null | head -1 | read -r bk_ac; print -r "${+bk_ac}""##);
        bulk_ac_fc_arith_ge_eq_chain => (r#">= =="#, r##"print -r "$(( 2 >= 1 == 1 ))""##);
        bulk_ac_fc_cond_int_ne => (r#"-ne"#, r##"[[ 3 -ne 4 ]]; print -r "nei=$?""##);
        bulk_ac_fc_param_s_shortest_inner => (r#"(S)#*"#, r##"szg_ac=gg_hi; print -r "${(S)szg_ac#*_}""##);
    }
}

mod corpus_dash_fc_bulk_ad {
    use super::*;

    parity_gap_tests! {
        bulk_ad_fc_arith_lt_chain => (r#"1<2<3"#, r##"print -r "$(( 1<2<3 ))""##);
        bulk_ad_fc_cond_sock_devnull => (r#"-S /dev/null"#, r##"[[ -S /dev/null 2>/dev/null ]]; print -r "sk=$?""##);
        bulk_ad_fc_function_colon_body => (r#"function : body"#, r##"logical_ad() { :; }; logical_ad; print -r ok_ad"##);
        bulk_ad_fc_array_unique => (r#"typeset -aU"#, r##"typeset -aU ad_ary=(a_ad b_ad a_ad); print -r "$#ad_ary""##);
        bulk_ad_fc_arith_logical_mix => (r#"|| + ||"#, r##"print -r "$(( (1||0)+(0||2) ))""##);
        bulk_ad_fc_glob_prefix_star => (r#"= b*"#, r##"[[ bar_ad = b* ]]; print -r "gm=$?""##);
        bulk_ad_fc_mktemp_rmdir => (r#"rmdir mktemp"#, r##"d_ad=$(mktemp -d 2>/dev/null); rmdir "$d_ad" 2>/dev/null; print -r "rd=$?""##);
        bulk_ad_fc_command_p_true => (r#"command -p true"#, r##"command -p true 2>/dev/null; print -r "cp=$?""##);
        bulk_ad_fc_function_integer_local => (r#"integer in function"#, r##"fn_ad() { integer loc_ad=3; print -r "$loc_ad"; }; fn_ad"##);
        bulk_ad_fc_array_subscript_one_ksharrays_off => (r#"ary[1] ksharrays"#, r##"unsetopt ksharrays 2>/dev/null; ary_ad=(x_ad); ary_ad[1]=y_ad; print -r "$ary_ad[1]""##);
        bulk_ad_fc_arith_hex_shift => (r#"0x10 <<"#, r##"print -r "$(( 0x10 << 1 ))""##);
        bulk_ad_fc_cond_sticky_tmp => (r#"-G /tmp"#, r##"[[ -G /tmp 2>/dev/null ]]; print -r "sg=$?""##);
        bulk_ad_fc_typeset_base_three => (r#"typeset -i3"#, r##"typeset -i3 ad_i=5; print -r "$ad_i""##);
        bulk_ad_fc_grep_hosts => (r#"grep /etc/hosts"#, r##"grep -q . /etc/hosts 2>/dev/null; print -r "gq=$?""##);
        bulk_ad_fc_arith_neg_zero_eq => (r#"-0=="#, r##"print -r "$(( -0 == 0 ))""##);
        bulk_ad_fc_param_at_hyphen_empty => (r#"${@-} empty"#, r##"[[ "${@-}" = "" ]]; print -r "ar=$?""##);
        bulk_ad_fc_arith_ge_self => (r#"3>=3"#, r##"print -r "$(( 3>=3 ))""##);
        bulk_ad_fc_integer_sub_assign_expr => (r#"-=2+1"#, r##"integer ia_ad=6; (( ia_ad-=2+1 )); print -r "$ia_ad""##);
        bulk_ad_fc_scalar_hash_short => (r#"#*."#, r##"st_ad=foo.bar.ad; print -r "${st_ad#*.}""##);
        bulk_ad_fc_scalar_hash_long => (r#"##*."#, r##"st2_ad=foo.bar.ad; print -r "${st2_ad##*.}""##);
        bulk_ad_fc_array_subscript_range => (r#"[1,-1]"#, r##"unsetop_ad=(a_ad b_ad); print -r "${unsetop_ad[1,-1]}""##);
        bulk_ad_fc_array_subscript_w_word => (r#"(w)2"#, r##"ary_sp_ad=(a_ad b_ad); print -r "${ary_sp_ad[(w)2]}""##);
        bulk_ad_fc_modifier_W_words => (r#"(W) spaces"#, r##"ws_ad="  hello  world  "; print -r "${(W)ws_ad}""##);
        bulk_ad_fc_glob_alt_unset_extglob => (r#"qua*(r|s) no extglob"#, r##"[[ quarter_ad = qua*(r|s)_ad ]]; print -r "alt=$?""##);
        bulk_ad_fc_glob_alt_set_extglob => (r#"qua*(r|s) extglob"#, r##"setopt extendedglob; [[ quarter_ad = qua*(r|s)_ad ]]; print -r "alt2=$?""##);
        bulk_ad_fc_arith_shift_combo => (r#"1<<30>>29"#, r##"print -r "$(( 1<<30>>29 ))""##);
        bulk_ad_fc_disown_noarg => (r#"disown"#, r##"disown 2>/dev/null; print -r "di=$?""##);
        bulk_ad_fc_arith_literal_456 => (r#"456 literal"#, r##"print -r "$((456))""##);
        bulk_ad_fc_arith_double_bang => (r#"!!2"#, r##"print -r "$(( !!2 ))""##);
    }
}

mod corpus_dash_fc_bulk_ae {
    use super::*;

    parity_gap_tests! {
        bulk_ae_fc_arith_bor_band_mix => (r#"& \| arith"#, r##"print -r "$(( 1 & 2 | 3 ))""##);
        bulk_ae_fc_cond_block_devnull => (r#"-b /dev/null"#, r##"[[ -b /dev/null 2>/dev/null ]]; print -r "bb=$?""##);
        bulk_ae_fc_typeset_lower => (r#"typeset -l"#, r##"typeset -l low_ae=XYZ; print -r "$low_ae""##);
        bulk_ae_fc_array_I_y_subscript => (r#"(I)y"#, r##"ary_i_ae=(x y z); print -r "${ary_i_ae[(I)y]}""##);
        bulk_ae_fc_arith_pow_right_assoc => (r#"2**3**2"#, r##"print -r "$(( 2**3**2 ))""##);
        bulk_ae_fc_join_pipe_sep => (r#"(j:\|:)"#, r##"ary_j_ae=(p q r); print -r "${(j:|:)ary_j_ae}""##);
        bulk_ae_fc_arith_bool_sum => (r#"(>0)+"#, r##"print -r "$(( (1>0)+(0>1) ))""##);
        bulk_ae_fc_cond_regex_eq_tilde => (r#"=~ ^z"#, r##"[[ z_ae =~ ^z ]]; print -r "rx=$?""##);
        bulk_ae_fc_param_colon_equals_assign => (r#"\${:=}"#, r##"unset n_ae; : ${n_ae:=9}; print -r "$n_ae""##);
        bulk_ae_fc_assoc_bracket_lookup => (r#"A [k]"#, r##"typeset -A ae_ass=(one_ae 1); print -r "${ae_ass[one_ae]}""##);
        bulk_ae_fc_arith_shift_lr_chain => (r#">><< chain"#, r##"print -r "$(( 15>>2<<1 ))""##);
        bulk_ae_fc_builtin_echo_word => (r#"builtin echo"#, r##"builtin echo ae_builtin"##);
        bulk_ae_fc_param_pwd_tail => (r#"PWD##*/"#, r##"print -r "${PWD##*/}""##);
        bulk_ae_fc_arith_div_neg_trunc => (r#"5/-2"#, r##"print -r "$(( 5/-2 ))""##);
        bulk_ae_fc_function_print_status => (r#"fn print"#, r##"fc_ae() { print -r inner_ae; }; fc_ae; print -r "af=$?""##);
        bulk_ae_fc_prompt_percent_hash => (r#"(%)#"#, r##"print -r "${(%)#}""##);
        bulk_ae_fc_arith_pow_zero => (r#"1**0"#, r##"print -r "$(( 1 ** 0 ))""##);
        bulk_ae_fc_array_neg_range_slice => (r#"[-2,-1]"#, r##"ary_rev_ae=(9 8 7); print -r "${ary_rev_ae[-2,-1]}""##);
        bulk_ae_fc_arith_tilde_mask_u8 => (r#"~ << mask"#, r##"print -r "$(( ~(~0<<~0) & 255 ))""##);
        bulk_ae_fc_join_comma_sep => (r#"(j:,:)"#, r##"ae_csv=(1 2 3); print -r "${(j:,:)ae_csv}""##);
        bulk_ae_fc_arith_mod_negative => (r#"-5%3"#, r##"print -r "$(( -5 % 3 ))""##);
        bulk_ae_fc_cond_and_int_compare => (r#"&& -gt -1"#, r##"[[ 9 -eq 9 && 0 -gt -1 ]]; print -r "ac=$?""##);
        bulk_ae_fc_param_qq_pwd => (r#"(qq)PWD"#, r##"print -r "${(qq)PWD}""##);
        bulk_ae_fc_param_qqq_spaces => (r#"(qqq)"#, r##"spa_ae="a b"; print -r "${(qqq)spa_ae}""##);
        bulk_ae_fc_scalar_slash_first_underscore => (r#"/_ /%"#, r##"string_ae=abc_def; print -r "${string_ae/_/%}""##);
        bulk_ae_fc_cond_hyphen_default_empty => (r#"-n \${:-}"#, r##"unset em_ae; [[ -n ${em_ae:-} ]]; print -r "ne=$?""##);
        bulk_ae_fc_array_Ue_unique => (r#"(Ue)"#, r##"dup_ae=(a a b); print -r "${(Ue)dup_ae}""##);
        bulk_ae_fc_arith_hex_and => (r#"0xFF&"#, r##"print -r "$(( 0xFF & 0x0F ))""##);
        bulk_ae_fc_cond_executable_sh => (r#"-x /bin/sh"#, r##"[[ -x /bin/sh ]]; print -r "xx=$?""##);
        bulk_ae_fc_prompt_percent_tilde => (r#"(%)~"#, r##"print -r "${(%)~}""##);
    }
}

mod corpus_dash_fc_bulk_af {
    use super::*;

    parity_gap_tests! {
        bulk_af_fc_arith_bit_xor => (r#"^ arith"#, r##"print -r "$(( 9^5 ))""##);
        bulk_af_fc_arith_lt_chain_345 => (r#"3<4<5"#, r##"print -r "$(( 3<4<5 ))""##);
        bulk_af_fc_glob_infix_stars => (r#"*o*af"#, r##"[[ foo_af = *o*af ]]; print -r "star=$?""##);
        bulk_af_fc_param_ifs_length => (r#"\${#IFS}"#, r##"print -r "${#IFS}""##);
        bulk_af_fc_arith_paren_neg_sq => (r#"(-3)**2"#, r##"print -r "$(( (-3)**2 ))""##);
        bulk_af_fc_array_sort_on_numeric => (r#"(on)"#, r##"ary_af=(9 1 8); print -r "${(on)ary_af}""##);
        bulk_af_fc_arith_logical_and_nums => (r#"1&&2"#, r##"print -r "$(( 1 && 2 ))""##);
        bulk_af_fc_arith_logical_or_zero => (r#"0\|\|3"#, r##"print -r "$(( 0 || 3 ))""##);
        bulk_af_fc_printf_pipe_wc_bytes => (r#"printf wc"#, r##"printf "%s" qqq_af | /usr/bin/wc -c | /usr/bin/tr -d "[:space:]""##);
        bulk_af_fc_arith_unset_plus_five => (r#"unset + 5"#, r##"unset zero_af; print -r "$(( zero_af + 5 ))""##);
        bulk_af_fc_typeset_plus_i_scalar => (r#"typeset +i"#, r##"typeset +i int_af=7; print -r "$int_af""##);
        bulk_af_fc_param_L_lower => (r#"(L)"#, r##"UP_af=MiXeD_af; print -r "${(L)UP_af}""##);
        bulk_af_fc_param_colon_tail => (r#":t"#, r##"st_af=/x/y/z_af; print -r "${st_af:t}""##);
        bulk_af_fc_param_colon_head => (r#":h"#, r##"st2_af=/a/b/c_af; print -r "${st2_af:h}""##);
        bulk_af_fc_arith_pow_negative_exp => (r#"4**-2"#, r##"print -r "$(( 4 ** -2 ))""##);
        bulk_af_fc_array_sort_oe_every => (r#"(oe)"#, r##"ary_s_af=(d b a c); print -r "${(oe)ary_s_af}""##);
        bulk_af_fc_arith_ternary_true => (r#"1?2:3"#, r##"print -r "$(( 1 ? 2 : 3 ))""##);
        bulk_af_fc_arith_ternary_false => (r#"0?2:3"#, r##"print -r "$(( 0 ? 2 : 3 ))""##);
        bulk_af_fc_cond_regex_path => (r#"/tmp =~"#, r##"[[ /tmp =~ ^/ ]]; print -r "rxd=$?""##);
        bulk_af_fc_arith_mod_neg_divisor => (r#"9%-4"#, r##"print -r "$(( 9 % -4 ))""##);
        bulk_af_fc_cond_symlink_root => (r#"-L /"#, r##"[[ -L / ]]; print -r "sy=$?""##);
        bulk_af_fc_function_return_three => (r#"return 3"#, r##"fnr_af() { return 3; }; fnr_af >/dev/null; print -r "$?""##);
        bulk_af_fc_arith_assign_in_void => (r#": \$(( = ))"#, r##": $(( q_af = 4 + 1 )); print -r "$q_af""##);
        bulk_af_fc_param_c_byte => (r#"(c)"#, r##"char_af=A; print -r "${(c)char_af}""##);
        bulk_af_fc_cond_sticky_bit_tmp => (r#"-k /tmp"#, r##"[[ -k /tmp ]]; print -r "sk=$?""##);
        bulk_af_fc_arith_shift_i63 => (r#"1<<63>>63"#, r##"print -r "$(( 1<<63>>63 ))""##);
        bulk_af_fc_typeset_float_Z2 => (r#"-F 2"#, r##"typeset -F 2 fl_af=3.4; print -r "$fl_af""##);
        bulk_af_fc_typeset_float_Z1 => (r#"-F 1"#, r##"typeset -F 1 fl2_af=9.96; print -r "$fl2_af""##);
        bulk_af_fc_arith_plus_plus_chain => (r#"+++5"#, r##"print -r "$(( +++5 ))""##);
        bulk_af_fc_unsetopt_shwordsplit => (r#"shwordsplit"#, r##"unsetopt shwordsplit 2>/dev/null; print -r "sw=$?""##);
        bulk_af_fc_param_V_escape => (r#"(V) tab"#, r##"vis_af=$'a\tb'; print -r "${(V)vis_af}""##);
        bulk_af_fc_array_range_subscript => (r#"[2,3]"#, r##"seq_af=(aa bb cc dd); print -r "${seq_af[2,3]}""##);
        bulk_af_fc_arith_binary_shift => (r#"0b101<<"#, r##"print -r "$(( 0b101 << 1 ))""##);
        bulk_af_fc_cond_readable_devnull => (r#"-r /dev/null"#, r##"[[ -r /dev/null ]]; print -r "rd=$?""##);
        bulk_af_fc_arith_div_neg_three => (r#"7/-3"#, r##"print -r "$(( 7 / -3 ))""##);
    }
}

mod corpus_dash_fc_bulk_ag {
    use super::*;

    parity_gap_tests! {
        bulk_ag_fc_arith_hex_plus_octal => (r#"0x10+0o10"#, r##"print -r "$(( 0x10 + 0o10 ))""##);
        bulk_ag_fc_arith_double_bang_sum => (r#"!!0+!!1"#, r##"print -r "$(( !!0 + !!1 ))""##);
        bulk_ag_fc_cond_fifo_stdin => (r#"-p /dev/fd/0"#, r##"[[ -p /dev/fd/0 ]]; print -r "fd=$?""##);
        bulk_ag_fc_cond_setuid_devnull => (r#"-u /dev/null"#, r##"[[ -u /dev/null ]]; print -r "ur=$?""##);
        bulk_ag_fc_array_index_assign => (r#"ary[2]="#, r##"ag_ary=(a b); ag_ary[2]=cc; print -r "$ag_ary[2]""##);
        bulk_ag_fc_arith_mod_small => (r#"8%3"#, r##"print -r "$(( 8 % 3 ))""##);
        bulk_ag_fc_glob_paren_alternation => (r#"(b|z) extglob"#, r##"setopt extendedglob; [[ bcd_ag = (b|z)cd_ag ]]; print -r "eg=$?""##);
        bulk_ag_fc_arith_eq_lt_chain => (r#"1==1<2"#, r##"print -r "$(( 1 == 1 < 2 ))""##);
        bulk_ag_fc_array_bash_slice => (r#"[@]:1:1"#, r##"typeset -a ag_pos=(1 2); print -r "${ag_pos[@]:1:1}""##);
        bulk_ag_fc_param_colon_hyphen_default => (r#"\${:-}"#, r##"unset x_ag; x_ag="${x_ag:-def_ag}"; print -r "$x_ag""##);
        bulk_ag_fc_arith_pow_eq => (r#"2**10==1024"#, r##"print -r "$(( 2**10 == 1024 ))""##);
        bulk_ag_fc_scalar_slash_global_double => (r#"//--/\|"#, r##"ag_str="a--b--c"; print -r "${ag_str//--/|}""##);
        bulk_ag_fc_arith_neg_pow_prec => (r#"-2**4"#, r##"print -r "$(( -2 ** 4 ))""##);
        bulk_ag_fc_arith_paren_neg_pow => (r#"-(2**4)"#, r##"print -r "$(( - (2 ** 4) ))""##);
        bulk_ag_fc_glob_dot_txt => (r#"*.txt"#, r##"[[ file_ag.txt = *.txt ]]; print -r "gx=$?""##);
        bulk_ag_fc_glob_bracket_class => (r#"[Aa]*"#, r##"[[ Ag_Text = [Aa]* ]]; print -r "gb=$?""##);
        bulk_ag_fc_prompt_percent_digit => (r#"(%)5"#, r##"print -r "${(%)5}""##);
        bulk_ag_fc_cond_or_posix => (r#"\|\| -eq"#, r##"[[ 0 -eq 0 || 1 -eq 1 ]]; print -r "or2=$?""##);
        bulk_ag_fc_arith_int_div => (r#"12/5"#, r##"print -r "$(( 12 / 5 ))""##);
        bulk_ag_fc_arith_float_div => (r#"12.0/5"#, r##"print -r "$(( 12.0 / 5 ))""##);
        bulk_ag_fc_array_r_subscript => (r#"(r)0"#, r##"ary_z_ag=(9 0 3); print -r "${ary_z_ag[(r)0]}""##);
        bulk_ag_fc_typeset_integer_base16 => (r#"-i16"#, r##"typeset -i16 hex_ag=255; print -r "$hex_ag""##);
        bulk_ag_fc_arith_shift_mask_byte => (r#"<<8)-1"#, r##"print -r "$(( (1<<8) - 1 ))""##);
        bulk_ag_fc_cond_samefile_strings => (r#"-ef"#, r##"[[ same_ag -ef same_ag ]]; print -r "ef=$?""##);
        bulk_ag_fc_status_false_then_true => (r#"false true $?"#, r##"false; true; print -r "$?""##);
        bulk_ag_fc_subshell_local => (r#"() var"#, r##"( subshell_ag=7; print -r "$subshell_ag" )"##);
        bulk_ag_fc_arith_pow_right_stack => (r#"5**2**2"#, r##"print -r "$(( 5 ** 2 ** 2 ))""##);
        bulk_ag_fc_join_newline_char => (r#"(j:\n:)"#, r##"ag_lines=(L1 L2); print -r "${(j:\n:)ag_lines}""##);
        bulk_ag_fc_prompt_percent_percent => (r#"(%%)"#, r##"ag_str2=ok; print -r "${(%%)ag_str2}""##);
        bulk_ag_fc_eval_single_quote => (r#"eval"#, r##"ev_ag=ok_ev; eval 'print -r $ev_ag'"##);
        bulk_ag_fc_typeset_zero_pad_width => (r#"-Z 4"#, r##"typeset -Z 4 zp_ag=12; print -r "$zp_ag""##);
        bulk_ag_fc_arith_band_signed => (r#"1&-1&1"#, r##"print -r "$(( 1 & -1 & 1 ))""##);
        bulk_ag_fc_glob_infix_y_star => (r#"*y*"#, r##"[[ xyz_ag = *y* ]]; print -r "ym=$?""##);
        bulk_ag_fc_array_i_casefold_sort => (r#"(i) sort"#, r##"mix_ag=(B_ag a_ag C_ag); print -r "${(i)mix_ag}""##);
        bulk_ag_fc_arith_mul_add_prec => (r#"3*4+5"#, r##"print -r "$(( 3 * 4 + 5 ))""##);
        bulk_ag_fc_arith_radix_16 => (r#"16#FF"#, r##"print -r "$(( 16#FF ))""##);
        bulk_ag_fc_arith_radix_2 => (r#"2#1010"#, r##"print -r "$(( 2#1010 ))""##);
        bulk_ag_fc_arith_pow_zero_scalar => (r#"9**0"#, r##"print -r "$(( 9**0 ))""##);
    }
}

mod corpus_dash_fc_bulk_ah {
    use super::*;

    parity_gap_tests! {
        bulk_ah_fc_row_001 => (r#"bulk ah 001"#, r##"print -r "$(( 1<<16 | 2 ))""##);
        bulk_ah_fc_row_002 => (r#"bulk ah 002"#, r##"print -r "$(( 1 || 0 && 0 ))""##);
        bulk_ah_fc_row_003 => (r#"bulk ah 003"#, r##"print -r "$(( 0 && 1 || 1 ))""##);
        bulk_ah_fc_row_004 => (r#"bulk ah 004"#, r##"ah_x=2; ah_y=3; print -r $(( ah_x * ah_y + 1 ))"##);
        bulk_ah_fc_row_005 => (r#"bulk ah 005"#, r##"print -r "$(( (ah_a=7) ))"; unset ah_a"##);
        bulk_ah_fc_row_006 => (r#"bulk ah 006"#, r##"[[ -z "" ]]; print -r "ez=$?""##);
        bulk_ah_fc_row_007 => (r#"bulk ah 007"#, r##"[[ -n one_ah ]]; print -r "nz=$?""##);
        bulk_ah_fc_row_008 => (r#"bulk ah 008"#, r##"[[ abc_ah < def_ah ]]; print -r "lt=$?""##);
        bulk_ah_fc_row_009 => (r#"bulk ah 009"#, r##"[[ xyz_ah > aaa_ah ]]; print -r "gt=$?""##);
        bulk_ah_fc_row_010 => (r#"bulk ah 010"#, r##"ah_emp=(u v); print -r "len$#ah_emp""##);
        bulk_ah_fc_row_011 => (r#"bulk ah 011"#, r##"ah_pair=(m n); print -r "$#ah_pair""##);
        bulk_ah_fc_row_012 => (r#"bulk ah 012"#, r##"typeset ah_scalar=42; print -r "$ah_scalar""##);
        bulk_ah_fc_row_013 => (r#"bulk ah 013"#, r##"ah_aa=(aa bb); print -r "$ah_aa[1]""##);
        bulk_ah_fc_row_014 => (r#"bulk ah 014"#, r##"line_ah="/usr/local/bin/foo"; print -r "${line_ah:5:4}""##);
        bulk_ah_fc_row_015 => (r#"bulk ah 015"#, r##"line_ah="/usr/local/bin/foo"; print -r "${line_ah: -8}""##);
        bulk_ah_fc_row_016 => (r#"bulk ah 016"#, r##"pad_ah=hi; print -r "${(l:5::-:)pad_ah}""##);
        bulk_ah_fc_row_017 => (r#"bulk ah 017"#, r##"ah_suf=foo.bar; print -r "${ah_suf%.bar}""##);
        bulk_ah_fc_row_018 => (r#"bulk ah 018"#, r##"ah_suf2=foo.bar.baz; print -r "${ah_suf2%.*}""##);
        bulk_ah_fc_row_019 => (r#"bulk ah 019"#, r##"ah_pre=foo.bar; print -r "${ah_pre#foo.}""##);
        bulk_ah_fc_row_020 => (r#"bulk ah 020"#, r##"integer ah_iv=5; print -r "$(( ah_iv++ ))""##);
        bulk_ah_fc_row_021 => (r#"bulk ah 021"#, r##"integer ah_id=10; ah_id=$(( ah_id / 3 )); print -r "$ah_id""##);
        bulk_ah_fc_row_022 => (r#"bulk ah 022"#, r##"print -r "$(( ah_neg=~2 ))"; unset ah_neg"##);
        bulk_ah_fc_row_023 => (r#"bulk ah 023"#, r##"setopt extendedglob; [[ case_ah = (#i)CASE_ah ]]; print -r "ci=$?""##);
        bulk_ah_fc_row_024 => (r#"bulk ah 024"#, r##"[[ 001 -eq 1 ]]; print -r "eq0=$?""##);
        bulk_ah_fc_row_025 => (r#"bulk ah 025"#, r##"[[ 007 -eq 7 ]]; print -r "oct7=$?""##);
        bulk_ah_fc_row_026 => (r#"bulk ah 026"#, r##"print -r "$(( 07 + 01 ))""##);
        bulk_ah_fc_row_027 => (r#"bulk ah 027"#, r##"print -r "$(( +++---+++-5 ))""##);
        bulk_ah_fc_row_028 => (r#"bulk ah 028"#, r##"print -r "$(( 123456 % 789 ))""##);
        bulk_ah_fc_row_029 => (r#"bulk ah 029"#, r##"[[ ./x_ah = */x_ah ]]; print -r "dp=$?""##);
        bulk_ah_fc_row_030 => (r#"bulk ah 030"#, r##"[[ / = / ]]; print -r "ro=$?""##);
        bulk_ah_fc_row_031 => (r#"bulk ah 031"#, r##"ah_sort=(c A b); print -r "${(i)ah_sort}""##);
        bulk_ah_fc_row_032 => (r#"bulk ah 032"#, r##"ah_os=(z a); print -r "${(o)ah_os}""##);
        bulk_ah_fc_row_033 => (r#"bulk ah 033"#, r##"ah_oa=(9 100 3); print -r "${(Oa)ah_oa}""##);
        bulk_ah_fc_row_034 => (r#"bulk ah 034"#, r##"ah_oi=(9 100 3); print -r "${(Oi)ah_oi}""##);
        bulk_ah_fc_row_035 => (r#"bulk ah 035"#, r##"ah_dup=(1 1 2 3 2); print -r "${(u)ah_dup}""##);
        bulk_ah_fc_row_036 => (r#"bulk ah 036"#, r##"ah_ue=(x x y); print -r "${(Ue)ah_ue}""##);
        bulk_ah_fc_row_037 => (r#"bulk ah 037"#, r##"ah_qq="x y"; print -r "${(qq)ah_qq}""##);
        bulk_ah_fc_row_038 => (r#"bulk ah 038"#, r##"bits_ah=x; ah_flag=0; [[ $bits_ah = *y* ]]; ah_flag=1; print -r "$ah_flag""##);
        bulk_ah_fc_row_039 => (r#"bulk ah 039"#, r##"print -r "$(print -r nested_ah)""##);
        bulk_ah_fc_row_040 => (r#"bulk ah 040"#, r##"arrp_ah=(p); arrp_ah+=q; print -r "${#arrp_ah} ${arrp_ah[2]}""##);
        bulk_ah_fc_row_041 => (r#"bulk ah 041"#, r##"unset ah_nv; [[ -v ah_nv ]]; print -r "vn=$?""##);
        bulk_ah_fc_row_042 => (r#"bulk ah 042"#, r##"ah_nv=1; [[ -v ah_nv ]]; print -r "vt=$?""##);
        bulk_ah_fc_row_043 => (r#"bulk ah 043"#, r##"function ah_f { return 4; }; ah_f; print -r "fr=$?""##);
        bulk_ah_fc_row_044 => (r#"bulk ah 044"#, r##"print -r "$(( 1<<62 == 4<<60 ))""##);
        bulk_ah_fc_row_045 => (r#"bulk ah 045"#, r##"print -r "$(( ~(0b1111) & 0xff ))""##);
        bulk_ah_fc_row_046 => (r#"bulk ah 046"#, r##"print -r "$(( 0b10101 ^ 0b01010 ))""##);
        bulk_ah_fc_row_047 => (r#"bulk ah 047"#, r##"ah_m=(1 2 3); unset "ah_m[2]"; print -r "$#ah_m ${ah_m[1]} ${ah_m[3]}""##);
        bulk_ah_fc_row_048 => (r#"bulk ah 048"#, r##"ah_at=(1 2); print -r "${(@)ah_at}""##);
        bulk_ah_fc_row_049 => (r#"bulk ah 049"#, r##"read -r ah_r < /dev/null; print -r "$?""##);
        bulk_ah_fc_row_050 => (r#"bulk ah 050"#, r##"[[ ! -e /no_such_ah_path_zz ]]; print -r "ne=$?""##);
        bulk_ah_fc_row_051 => (r#"bulk ah 051"#, r##"cd / 2>/dev/null; print -r "$PWD""##);
        bulk_ah_fc_row_052 => (r#"bulk ah 052"#, r##"[[ -d /tmp ]]; print -r "dt=$?""##);
        bulk_ah_fc_row_053 => (r#"bulk ah 053"#, r##"[[ -L /dev/fd/0 ]]; print -r "lf=$?""##);
        bulk_ah_fc_row_054 => (r#"bulk ah 054"#, r##"print -r "${(%)3}""##);
        bulk_ah_fc_row_055 => (r#"bulk ah 055"#, r##"print -r "$(( (3<4) + (4<3) ))""##);
        bulk_ah_fc_row_056 => (r#"bulk ah 056"#, r##"print -r "$(( (-1)**3 ))""##);
        bulk_ah_fc_row_057 => (r#"bulk ah 057"#, r##"str_ah="abc"; print -r "${str_ah/ /_}""##);
        bulk_ah_fc_row_058 => (r#"bulk ah 058"#, r##"str2_ah="foo.bar.foo"; print -r "${str2_ah//foo/baz}""##);
        bulk_ah_fc_row_059 => (r#"bulk ah 059"#, r##"typeset ah_assoc_dummy=1; print -r "${+ah_assoc_dummy}""##);
        bulk_ah_fc_row_060 => (r#"bulk ah 060"#, r##"integer ah_i27=27; ah_msg=msg; out_ah="${ah_i27#${ah_msg}}"; print -r "$out_ah""##);
        bulk_ah_fc_row_061 => (r#"bulk ah 061"#, r##"print -r "$(( 0<= 1 <= 1 ))""##);
        bulk_ah_fc_row_062 => (r#"bulk ah 062"#, r##"print -r "$(( 6>= 5 >= 1 ))""##);
        bulk_ah_fc_row_063 => (r#"bulk ah 063"#, r##"[[ "" = "" ]]; print -r "es=$?""##);
        bulk_ah_fc_row_064 => (r#"bulk ah 064"#, r##"[[ "x" != "y" ]]; print -r "nq=$?""##);
        bulk_ah_fc_row_065 => (r#"bulk ah 065"#, r##"print -r "v=${ah_unset-9}""##);
        bulk_ah_fc_row_066 => (r#"bulk ah 066"#, r##"unset ah_a; ah_a+=(z); print -r "${ah_a[1]}""##);
        bulk_ah_fc_row_067 => (r#"bulk ah 067"#, r##": "${(@)ah_empty_sub}"; print -r "ae=$?""##);
        bulk_ah_fc_row_068 => (r#"bulk ah 068"#, r##"print -r "$(( 1**2**3 ))""##);
        bulk_ah_fc_row_069 => (r#"bulk ah 069"#, r##"print -r "$(( 1 ? 0 ? 3 : 4 : 5 ))""##);
        bulk_ah_fc_row_070 => (r#"bulk ah 070"#, r##"print -r "$(( 0 ? 1 : 2 ? 3 : 4 ))""##);
        bulk_ah_fc_row_071 => (r#"bulk ah 071"#, r##"print -r "$(( 0xdead & 0xff ))""##);
        bulk_ah_fc_row_072 => (r#"bulk ah 072"#, r##"typeset -F 3 ah_flt=1.234; print -r "$ah_flt""##);
        bulk_ah_fc_row_073 => (r#"bulk ah 073"#, r##"print -r "$(( 1000000 >> 8 ))""##);
        bulk_ah_fc_row_074 => (r#"bulk ah 074"#, r##"ah_path=/a/b/c; print -r "${(s:/:)ah_path}""##);
        bulk_ah_fc_row_075 => (r#"bulk ah 075"#, r##"print -r "$(( 42 & 255 ))""##);
        bulk_ah_fc_row_076 => (r#"bulk ah 076"#, r##"typeset -Z5 z5_ah=7; print -r "$z5_ah""##);
        bulk_ah_fc_row_077 => (r#"bulk ah 077"#, r##"[[ shorter_ah = *hort* ]]; print -r "mid=$?""##);
        bulk_ah_fc_row_078 => (r#"bulk ah 078"#, r##"ah_up=mixed_CASE_ah; print -r "${(U)ah_up}""##);
        bulk_ah_fc_row_079 => (r#"bulk ah 079"#, r##"print -r "$(( 0 ** 0 ))""##);
        bulk_ah_fc_row_080 => (r#"bulk ah 080"#, r##"print -r "$(( 1>>31<<31 ))""##);
        bulk_ah_fc_row_081 => (r#"bulk ah 081"#, r##"fn_lc(){ typeset LC_ALL=C; print -r "$LC_ALL"; }; fn_lc"##);
        bulk_ah_fc_row_082 => (r#"bulk ah 082"#, r##"print -r "$(( 0o377 ))""##);
        bulk_ah_fc_row_083 => (r#"bulk ah 083"#, r##"setopt extendedglob; [[ yep_ah = (#b)y(e)p_ah ]]; print -r "bt=$?""##);
        bulk_ah_fc_row_084 => (r#"bulk ah 084"#, r##"print -r "$#path""##);
        bulk_ah_fc_row_085 => (r#"bulk ah 085"#, r##"integer mn_ah=-100; print -r "$(( mn_ah ** 2 ))""##);
        bulk_ah_fc_row_086 => (r#"bulk ah 086"#, r##"ah_l=AbC; print -r "${(L)ah_l}""##);
        bulk_ah_fc_row_087 => (r#"bulk ah 087"#, r##"ary_rev=(9 8); print -r "${ary_rev[-1]}""##);
        bulk_ah_fc_row_088 => (r#"bulk ah 088"#, r##"print -r "$(( 1 != 0 != 0 ))""##);
        bulk_ah_fc_row_089 => (r#"bulk ah 089"#, r##"[[ -N /dev/null ]]; print -r "nzf=$?""##);
        bulk_ah_fc_row_090 => (r#"bulk ah 090"#, r##"[[ -O /dev/null ]]; print -r "ow=$?""##);
        bulk_ah_fc_row_091 => (r#"bulk ah 091"#, r##"dirs 2>/dev/null; print -r "drs=$?""##);
        bulk_ah_fc_row_092 => (r#"bulk ah 092"#, r##"print -r "$(( 99 >= 100 ? 1 : 0 ))""##);
        bulk_ah_fc_row_093 => (r#"bulk ah 093"#, r##"typeset -l lo_ty=MiX; print -r "$lo_ty""##);
        bulk_ah_fc_row_094 => (r#"bulk ah 094"#, r##"ah_flines=$'a\nb'; print -r "${(F)ah_flines}""##);
        bulk_ah_fc_row_095 => (r#"bulk ah 095"#, r##"unsetopt globdots 2>/dev/null; print -r "gd=$?""##);
        bulk_ah_fc_row_096 => (r#"bulk ah 096"#, r##"print -r "$(( 0x1ffffffff & 3 ))""##);
        bulk_ah_fc_row_097 => (r#"bulk ah 097"#, r##"ah_cseq=(1 2 3 4); print -r "${ah_cseq[2,-1]}""##);
        bulk_ah_fc_row_098 => (r#"bulk ah 098"#, r##"float ah_of=0.25; print -r "$(( ah_of + 0.25 ))""##);
        bulk_ah_fc_row_099 => (r#"bulk ah 099"#, r##"print -r "$(( 10#99 ))""##);
        bulk_ah_fc_row_100 => (r#"bulk ah 100"#, r##"ah_zero=42; print -r "${(0)ah_zero}""##);
        bulk_ah_fc_row_101 => (r#"bulk ah 101"#, r##"setopt braceexpand; print -r {a,b}_ah"##);
        bulk_ah_fc_row_102 => (r#"bulk ah 102"#, r##"unsetopt multios 2>/dev/null; print -r "mo=$?""##);
        bulk_ah_fc_row_103 => (r#"bulk ah 103"#, r##"print -r "$(( (1|2) * (3&1) ))""##);
        bulk_ah_fc_row_104 => (r#"bulk ah 104"#, r##"[[ z_string_ah = z* ]]; print -r "vars=$?""##);
        bulk_ah_fc_row_105 => (r#"bulk ah 105"#, r##"print -r "$(( 2 ** (1+1) ))""##);
        bulk_ah_fc_row_106 => (r#"bulk ah 106"#, r##"ah_qs=(ab); print -r "${(q)ah_qs}""##);
        bulk_ah_fc_row_107 => (r#"bulk ah 107"#, r##"typeset -Z3 z3_ah=4; print -r "$z3_ah""##);
        bulk_ah_fc_row_108 => (r#"bulk ah 108"#, r##"print -r "$(( 1<<2<<1 ))""##);
        bulk_ah_fc_row_109 => (r#"bulk ah 109"#, r##"print -r "$(( 4 ** (1+0+1) ))""##);
        bulk_ah_fc_row_110 => (r#"bulk ah 110"#, r##"print -r "$(( 0x80000000 >> 31 ))""##);
        bulk_ah_fc_row_111 => (r#"bulk ah 111"#, r##"[[ -a /dev/null ]]; print -r "ap=$?""##);
        bulk_ah_fc_row_112 => (r#"bulk ah 112"#, r##"print -r "$(( 0x7fffffff % 3 ))""##);
        bulk_ah_fc_row_113 => (r#"bulk ah 113"#, r##"typeset -A ah_kv1=(solo val); print -r "${(kv)ah_kv1}""##);
        bulk_ah_fc_row_114 => (r#"bulk ah 114"#, r##"print -r "$(( -7 % 4 ))""##);
        bulk_ah_fc_row_115 => (r#"bulk ah 115"#, r##"print -r "$(( 2#111 == 7 ))""##);
        bulk_ah_fc_row_116 => (r#"bulk ah 116"#, r##"fn_add(){ integer s=0; s=$(( s + $1 + $2 )); print -r "$s"; }; fn_add 3 4"##);
        bulk_ah_fc_row_117 => (r#"bulk ah 117"#, r##"print -r "$(( 1_000 + 2_000 ))""##);
        bulk_ah_fc_row_118 => (r#"bulk ah 118"#, r##"[[ 1 -eq 1 ]] && [[ 2 -eq 2 ]]; print -r "ch=$?""##);
        bulk_ah_fc_row_119 => (r#"bulk ah 119"#, r##"print -r "$(( 0b1 == 1 ))""##);
        bulk_ah_fc_row_120 => (r#"bulk ah 120"#, r##"ah_ax=(1); ah_ax+=2; print -r "${ah_ax[2]}""##);
        bulk_ah_fc_row_121 => (r#"bulk ah 121"#, r##"typeset -E 2 e2_ah=4000; print -r "$e2_ah""##);
        bulk_ah_fc_row_122 => (r#"bulk ah 122"#, r##"print -r $(( ##a ))"##);
        bulk_ah_fc_row_123 => (r#"bulk ah 123"#, r##"print -r "$(( 6 / 2 / 3 ))""##);
        bulk_ah_fc_row_124 => (r#"bulk ah 124"#, r##"shift_ah=(first second third); print -r "$shift_ah[2,-1]""##);
        bulk_ah_fc_row_125 => (r#"bulk ah 125"#, r##"ah_rep=zzz; print -r "${ah_rep//z/+}""##);
        bulk_ah_fc_row_126 => (r#"bulk ah 126"#, r##"print -r "$(( !0 + !0 ))""##);
        bulk_ah_fc_row_127 => (r#"bulk ah 127"#, r##"[[ -e /dev/null || -e /tmp ]]; print -r "eo=$?""##);
        bulk_ah_fc_row_128 => (r#"bulk ah 128"#, r##"builtin print raw_ah"##);
        bulk_ah_fc_row_129 => (r#"bulk ah 129"#, r##"print -r "$(( 3 < 4 ? 10 : 20 ))""##);
        bulk_ah_fc_row_130 => (r#"bulk ah 130"#, r##"print -r "$(( [#10]42 ))""##);
        bulk_ah_fc_row_131 => (r#"bulk ah 131"#, r##"print -r "$(( 1<<3 == 2**3 ))""##);
        bulk_ah_fc_row_132 => (r#"bulk ah 132"#, r##"typeset -F1 ah_cmp=1.05; print -r "$(( ah_cmp > 1 ))""##);
        bulk_ah_fc_row_133 => (r#"bulk ah 133"#, r##"print -r "$(( 0b1111 ))""##);
        bulk_ah_fc_row_134 => (r#"bulk ah 134"#, r##"print -r "$(( (3>2) + (2>3) ))""##);
        bulk_ah_fc_row_135 => (r#"bulk ah 135"#, r##"[[ word_ah =~ w ]]; print -r "rxw=$?""##);
        bulk_ah_fc_row_136 => (r#"bulk ah 136"#, r##"export AH_EX=port; print -r "$AH_EX"; unset AH_EX"##);
        bulk_ah_fc_row_137 => (r#"bulk ah 137"#, r##"ah_flines2=$'l1\nl2'; print -r "${(@f)ah_flines2}""##);
        bulk_ah_fc_row_138 => (r#"bulk ah 138"#, r##"print -r "$(( 12 & 10 ))""##);
        bulk_ah_fc_row_139 => (r#"bulk ah 139"#, r##"print -r "$(( (1+2)*(3+4) ))""##);
        bulk_ah_fc_row_140 => (r#"bulk ah 140"#, r##"ah_mix=aBc; print -r "${(L)${(U)ah_mix}}""##);
        bulk_ah_fc_row_141 => (r#"bulk ah 141"#, r##"typeset -h hv_ah=1; print -r "${+hv_ah}""##);
        bulk_ah_fc_row_142 => (r#"bulk ah 142"#, r##"print -r "$(( 100 / 20 / 5 ))""##);
        bulk_ah_fc_row_143 => (r#"bulk ah 143"#, r##"[[ zero_ah = <-> ]]; print -r "dg=$?""##);
        bulk_ah_fc_row_144 => (r#"bulk ah 144"#, r##"print -r "$(( 0x80 >> 4 ))""##);
    }
}

mod corpus_dash_fc_bulk_ai {
    use super::*;

    parity_gap_tests! {
        bulk_ai_fc_row_001 => (r#"bulk ai 001"#, r##"print -r "$(( $# * 0 + 5 ))""##);
        bulk_ai_fc_row_002 => (r#"bulk ai 002"#, r##"ai_scalar=9; print -r "$(( ai_scalar * ai_scalar ))""##);
        bulk_ai_fc_row_003 => (r#"bulk ai 003"#, r##"typeset -i2 ai_bin=5; print -r "$ai_bin""##);
        bulk_ai_fc_row_004 => (r#"bulk ai 004"#, r##"print -r "$(( (5<3) ? 9 : 8 ))""##);
        bulk_ai_fc_row_005 => (r#"bulk ai 005"#, r##"print -r "$(( 2 ** 3 ** 2 ))""##);
        bulk_ai_fc_row_006 => (r#"bulk ai 006"#, r##"ai_arr=(5); print -r "${ai_arr[1]}""##);
        bulk_ai_fc_row_007 => (r#"bulk ai 007"#, r##"print -r $(( ##Z ))"##);
        bulk_ai_fc_row_008 => (r#"bulk ai 008"#, r##"[[ -t 0 ]]; print -r "it=$?""##);
        bulk_ai_fc_row_009 => (r#"bulk ai 009"#, r##"[[ -t 1 ]]; print -r "ot=$?""##);
        bulk_ai_fc_row_010 => (r#"bulk ai 010"#, r##"print -r "${(e):-3+4}""##);
        bulk_ai_fc_row_011 => (r#"bulk ai 011"#, r##"ai_h=$'\\x41'; print -r "$ai_h""##);
        bulk_ai_fc_row_012 => (r#"bulk ai 012"#, r##"print -r "$(( 0xffFF ))""##);
        bulk_ai_fc_row_013 => (r#"bulk ai 013"#, r##"print -r "$(( 1000 % 7 ))""##);
        bulk_ai_fc_row_014 => (r#"bulk ai 014"#, r##"[[ . = . ]]; print -r "de=$?""##);
        bulk_ai_fc_row_015 => (r#"bulk ai 015"#, r##"[[ ai -lt bj ]]; print -r "lj=$?""##);
        bulk_ai_fc_row_016 => (r#"bulk ai 016"#, r##"typeset -i16 ai_hex=0x0f0f; print -r "$ai_hex""##);
        bulk_ai_fc_row_017 => (r#"bulk ai 017"#, r##"ai_oct=8; print -r "${(0)ai_oct}""##);
        bulk_ai_fc_row_018 => (r#"bulk ai 018"#, r##"unset ai_pl; print -r "${ai_pl:+yes}""##);
        bulk_ai_fc_row_019 => (r#"bulk ai 019"#, r##"unset ai_pc; : ${ai_pc::=defai}; print -r "$ai_pc""##);
        bulk_ai_fc_row_020 => (r#"bulk ai 020"#, r##"print -r "$(( (1+2)**3 ))""##);
        bulk_ai_fc_row_021 => (r#"bulk ai 021"#, r##"ai_kv=(k v); print -r "${ai_kv[(w)2]}""##);
        bulk_ai_fc_row_022 => (r#"bulk ai 022"#, r##"word_ai="  two  tokens  "; print -r "${(z)word_ai}""##);
        bulk_ai_fc_row_023 => (r#"bulk ai 023"#, r##"print -r "$(( 3 >> 1 << 2 ))""##);
        bulk_ai_fc_row_024 => (r#"bulk ai 024"#, r##"typeset -aU ai_u=(a b a); print -r "${#ai_u}""##);
        bulk_ai_fc_row_025 => (r#"bulk ai 025"#, r##"print -r "$(( 0 || 0 || 7 ))""##);
        bulk_ai_fc_row_026 => (r#"bulk ai 026"#, r##"print -r "$(( 9 & 6 ^ 3 ))""##);
        bulk_ai_fc_row_027 => (r#"bulk ai 027"#, r##"[[ / = /* ]]; print -r "rs=$?""##);
        bulk_ai_fc_row_028 => (r#"bulk ai 028"#, r##"setopt extendedglob; [[ kite_ai = *i*e ]]; print -r "ms=$?""##);
        bulk_ai_fc_row_029 => (r#"bulk ai 029"#, r##"fn_ai(){ integer x=1; integer y=2; print -r "$(( x+y ))"; }; fn_ai"##);
        bulk_ai_fc_row_030 => (r#"bulk ai 030"#, r##"print -r "$(printf %d 65)""##);
        bulk_ai_fc_row_031 => (r#"bulk ai 031"#, r##"ai_cap="hello ai"; print -r "${(C)ai_cap}""##);
        bulk_ai_fc_row_032 => (r#"bulk ai 032"#, r##"ai_collapse="  hi  "; print -r "${(W)ai_collapse}""##);
        bulk_ai_fc_row_033 => (r#"bulk ai 033"#, r##"ai_tri=(1 2 3); print -r "${ai_tri[(Ie)2]}""##);
        bulk_ai_fc_row_034 => (r#"bulk ai 034"#, r##"ai_neg=(3 2 1); print -r "${ai_neg[(I)1]}""##);
        bulk_ai_fc_row_035 => (r#"bulk ai 035"#, r##"print -r "$(( 3 <|> 5 ))""##);
        bulk_ai_fc_row_036 => (r#"bulk ai 036"#, r##"builtin echo -n ai_noeol; print -r"##);
        bulk_ai_fc_row_037 => (r#"bulk ai 037"#, r##"[[ -x /usr/bin/true ]]; print -r "xtr=$?""##);
        bulk_ai_fc_row_038 => (r#"bulk ai 038"#, r##"[[ -e /dev/zero ]]; print -r "ezr=$?""##);
        bulk_ai_fc_row_039 => (r#"bulk ai 039"#, r##"print -r "$(( 24 % 5 % 3 ))""##);
        bulk_ai_fc_row_040 => (r#"bulk ai 040"#, r##"d_ai=$(mktemp -d); rmdir "$d_ai"; print -r "oktmp=$?""##);
        bulk_ai_fc_row_041 => (r#"bulk ai 041"#, r##"print -r "$(( (1>0) + (0>0) + (-1>0) ))""##);
        bulk_ai_fc_row_042 => (r#"bulk ai 042"#, r##"typeset +i ai_pi=4; print -r "$ai_pi""##);
        bulk_ai_fc_row_043 => (r#"bulk ai 043"#, r##"print -r "$(( 0b010101 ))""##);
        bulk_ai_fc_row_044 => (r#"bulk ai 044"#, r##"float f1=1.25; float f2=2; print -r "$(( f1 * f2 ))""##);
        bulk_ai_fc_row_045 => (r#"bulk ai 045"#, r##"print -r "$(( 1 && 0 || 2 && 3 ))""##);
        bulk_ai_fc_row_046 => (r#"bulk ai 046"#, r##"ai_ary_sub=(a b c); print -r "${ai_ary_sub[@]:1}""##);
        bulk_ai_fc_row_047 => (r#"bulk ai 047"#, r##"print -r "${${ai_nested:-in}:+out}"; ai_nested=''"##);
        bulk_ai_fc_row_048 => (r#"bulk ai 048"#, r##"print -r "${${ai_nested2:+skip}:-fallback}"; unset ai_nested2"##);
        bulk_ai_fc_row_049 => (r#"bulk ai 049"#, r##"print -r "$(( 4 ** (2**2) ))""##);
        bulk_ai_fc_row_050 => (r#"bulk ai 050"#, r##"readonly ro_ai=fix; print -r "$ro_ai""##);
        bulk_ai_fc_row_051 => (r#"bulk ai 051"#, r##"typeset -r ai_rd=9; print -r "$ai_rd""##);
        bulk_ai_fc_row_052 => (r#"bulk ai 052"#, r##"print -r "$(( 1 , 2 , 3 ))""##);
        bulk_ai_fc_row_053 => (r#"bulk ai 053"#, r##"true; print -r "$?""##);
        bulk_ai_fc_row_054 => (r#"bulk ai 054"#, r##"UPAI=XY; print -r "${(L)UPAI}""##);
        bulk_ai_fc_row_055 => (r#"bulk ai 055"#, r##"lowai=ab; print -r "${(u)lowai}""##);
        bulk_ai_fc_row_056 => (r#"bulk ai 056"#, r##"print -r "$(( - ( - ( -5 ) ) ))""##);
        bulk_ai_fc_row_057 => (r#"bulk ai 057"#, r##"[[ "" != x ]]; print -r "nem=$?""##);
        bulk_ai_fc_row_058 => (r#"bulk ai 058"#, r##"[[ 3 -ge 3 ]]; print -r "gem=$?""##);
        bulk_ai_fc_row_059 => (r#"bulk ai 059"#, r##"print -r "${(%)2}""##);
        bulk_ai_fc_row_060 => (r#"bulk ai 060"#, r##"[[ -w /tmp ]]; print -r "wt=$?""##);
        bulk_ai_fc_row_061 => (r#"bulk ai 061"#, r##"[[ -s /dev/null ]]; print -r "sz=$?""##);
        bulk_ai_fc_row_062 => (r#"bulk ai 062"#, r##"print -r "$(( 11 ** 2 % 50 ))""##);
        bulk_ai_fc_row_063 => (r#"bulk ai 063"#, r##"ai_seq=(Q R S); print -r "${ai_seq[1,2]}""##);
        bulk_ai_fc_row_064 => (r#"bulk ai 064"#, r##"print -r "$(( 2#1000 ))""##);
        bulk_ai_fc_row_065 => (r#"bulk ai 065"#, r##"print -r "$(( 8#10 ))""##);
        bulk_ai_fc_row_066 => (r#"bulk ai 066"#, r##"print -r "$(( 16#abc ))""##);
        bulk_ai_fc_row_067 => (r#"bulk ai 067"#, r##"print -r "$(( 12#9b ))""##);
        bulk_ai_fc_row_068 => (r#"bulk ai 068"#, r##"setopt extendedglob; [[ tag_ai = (#m)[a-z]##_ai ]]; print -r "mh=$?""##);
        bulk_ai_fc_row_069 => (r#"bulk ai 069"#, r##"print -r "$(( 1<=1 && 2<=3 ))""##);
        bulk_ai_fc_row_070 => (r#"bulk ai 070"#, r##"print -r $(( ##b ))"##);
        bulk_ai_fc_row_071 => (r#"bulk ai 071"#, r##"typeset +L ai_L=AbCd; print -r "$ai_L""##);
        bulk_ai_fc_row_072 => (r#"bulk ai 072"#, r##"unsetopt typesetsilent 2>/dev/null; print -r "ts=$?""##);
        bulk_ai_fc_row_073 => (r#"bulk ai 073"#, r##"print -r "$(( 2#101010 ))""##);
        bulk_ai_fc_row_074 => (r#"bulk ai 074"#, r##"ai_stack=(d e f g); print -r "${ai_stack[2,3]}""##);
        bulk_ai_fc_row_075 => (r#"bulk ai 075"#, r##"print -r "$(( 42 ))""##);
        bulk_ai_fc_row_076 => (r#"bulk ai 076"#, r##"print -r "$(( 0x101 >> 4 ))""##);
        bulk_ai_fc_row_077 => (r#"bulk ai 077"#, r##"print -r "$(( 5**0**3 ))""##);
        bulk_ai_fc_row_078 => (r#"bulk ai 078"#, r##"[[ ABC = [A-Z]## ]]; print -r "uc=$?""##);
        bulk_ai_fc_row_079 => (r#"bulk ai 079"#, r##"setopt extendedglob; [[ mix123 = [[:digit:]]# ]]; print -r "dg2=$?""##);
        bulk_ai_fc_row_080 => (r#"bulk ai 080"#, r##"print -r "$(( #char_ai ))"; char_ai=*"##);
        bulk_ai_fc_row_081 => (r#"bulk ai 081"#, r##"ai_multi="A B"; ai_multi=("${(z)ai_multi}"); print -r "${(L)ai_multi}""##);
        bulk_ai_fc_row_082 => (r#"bulk ai 082"#, r##"print -r "$(( 1 || -1 ))""##);
        bulk_ai_fc_row_083 => (r#"bulk ai 083"#, r##"unsetopt errexit 2>/dev/null; print -r "er=$?""##);
        bulk_ai_fc_row_084 => (r#"bulk ai 084"#, r##"print -r "$(( 9 <> 5 ))""##);
        bulk_ai_fc_row_085 => (r#"bulk ai 085"#, r##"fnr2(){ return 0; }; fnr2; print -r "zr=$?""##);
        bulk_ai_fc_row_086 => (r#"bulk ai 086"#, r##"[[ -z "" ]]; print -r "ze2=$?""##);
        bulk_ai_fc_row_087 => (r#"bulk ai 087"#, r##"ai_assoc1=(alice bob); print -r "${ai_assoc1[(w)1]}""##);
        bulk_ai_fc_row_088 => (r#"bulk ai 088"#, r##"print -r "$(( 128 >> 4 >> 1 ))""##);
        bulk_ai_fc_row_089 => (r#"bulk ai 089"#, r##"print -r "$(( ~2 & 7 ))""##);
        bulk_ai_fc_row_090 => (r#"bulk ai 090"#, r##"print -r "$(( 2 | 4 | 8 ))""##);
        bulk_ai_fc_row_091 => (r#"bulk ai 091"#, r##"typeset ai_ts; ai_ts=$(date +%s 2>/dev/null); print -r "${+ai_ts}""##);
        bulk_ai_fc_row_092 => (r#"bulk ai 092"#, r##"print -r "$(( 1000003 % 97 ))""##);
        bulk_ai_fc_row_093 => (r#"bulk ai 093"#, r##"[[ bot_ai = *t*ai ]]; print -r "bm=$?""##);
        bulk_ai_fc_row_094 => (r#"bulk ai 094"#, r##"print -r "$(( 1 + 2 * 3 - 4 ))""##);
        bulk_ai_fc_row_095 => (r#"bulk ai 095"#, r##"ai_prepend=(x y); ai_prepend=(a b "${(@)ai_prepend}"); print -r "${(j::)ai_prepend}""##);
        bulk_ai_fc_row_096 => (r#"bulk ai 096"#, r##"print -r "$(( 2 ** (1+2) + 1 ))""##);
        bulk_ai_fc_row_097 => (r#"bulk ai 097"#, r##"unset LANG LC_ALL; print -r "${LANG:-C}""##);
        bulk_ai_fc_row_098 => (r#"bulk ai 098"#, r##"print -r "$(( 63 & 31 | 15 ))""##);
        bulk_ai_fc_row_099 => (r#"bulk ai 099"#, r##"typeset -aS ai_single="x y"; print -r "$#ai_single $ai_single[2]""##);
        bulk_ai_fc_row_100 => (r#"bulk ai 100"#, r##"print -r "$(( (9>8)>>(1<0) ))""##);
        bulk_ai_fc_row_101 => (r#"bulk ai 101"#, r##"ai_tup=(10 20); print -r "${ai_tup[(w)1]}""##);
        bulk_ai_fc_row_102 => (r#"bulk ai 102"#, r##"print -r "$(( 4 <> 4 ))""##);
        bulk_ai_fc_row_103 => (r#"bulk ai 103"#, r##"[[ AAA_ai =~ ^A+ ]]; print -r "pl=$?""##);
        bulk_ai_fc_row_104 => (r#"bulk ai 104"#, r##"ai_q=simple; print -r "${(q)ai_q}""##);
        bulk_ai_fc_row_105 => (r#"bulk ai 105"#, r##"print -r "$(( 1>>63 ))""##);
        bulk_ai_fc_row_106 => (r#"bulk ai 106"#, r##"print -r "$(( 0 - 1 == -1 ))""##);
        bulk_ai_fc_row_107 => (r#"bulk ai 107"#, r##"typeset -F2 ai_fsum=3.33; print -r "$ai_fsum""##);
        bulk_ai_fc_row_108 => (r#"bulk ai 108"#, r##"print -r "$(( 0x7fffffff & 0 ))""##);
        bulk_ai_fc_row_109 => (r#"bulk ai 109"#, r##"ai_rev=(9 8 7); print -r "${ai_rev[-3,-2]}""##);
        bulk_ai_fc_row_110 => (r#"bulk ai 110"#, r##"print -r "$(( 12 + 34 ))""##);
        bulk_ai_fc_row_111 => (r#"bulk ai 111"#, r##"[[ bee_ai = *ee* ]]; print -r "sub=$?""##);
        bulk_ai_fc_row_112 => (r#"bulk ai 112"#, r##"print -r "$(( 2 ** 20 >> 10 ))""##);
        bulk_ai_fc_row_113 => (r#"bulk ai 113"#, r##"print -r "$(( (1<<5)-1 ))""##);
        bulk_ai_fc_row_114 => (r#"bulk ai 114"#, r##"print -r "$(( 100 / 25 ))""##);
        bulk_ai_fc_row_115 => (r#"bulk ai 115"#, r##"[[ -h /dev/stdin ]]; print -r "hs=$?""##);
        bulk_ai_fc_row_116 => (r#"bulk ai 116"#, r##"print -r "${(L)@}"; set -- MIXED_arg"##);
        bulk_ai_fc_row_117 => (r#"bulk ai 117"#, r##"print -r "$(( 0xabc & 0xf0 ))""##);
        bulk_ai_fc_row_118 => (r#"bulk ai 118"#, r##"integer ai_coalesce; : ${ai_coalesce::=$(( 2+3 ))}; print -r "$ai_coalesce""##);
        bulk_ai_fc_row_119 => (r#"bulk ai 119"#, r##"print -r "$(( 2*2*2*2 ))""##);
        bulk_ai_fc_row_120 => (r#"bulk ai 120"#, r##"fn_ret5(){ return 5; }; fn_ret5 2>/dev/null; print -r "$?""##);
        bulk_ai_fc_row_121 => (r#"bulk ai 121"#, r##"typeset -i8 ai_octin=10; print -r "$ai_octin""##);
        bulk_ai_fc_row_122 => (r#"bulk ai 122"#, r##"print -r $(( ##n ))"##);
        bulk_ai_fc_row_123 => (r#"bulk ai 123"#, r##"unsetopt bgnice 2>/dev/null; print -r "bn=$?""##);
        bulk_ai_fc_row_124 => (r#"bulk ai 124"#, r##"print -r "$(( (1==1)+(0==1) ))""##);
        bulk_ai_fc_row_125 => (r#"bulk ai 125"#, r##"str_ai="abc.def.ghi"; print -r "${str_ai:r}""##);
        bulk_ai_fc_row_126 => (r#"bulk ai 126"#, r##"str_ai2="abc.def.ghi"; print -r "${str_ai2:e}""##);
        bulk_ai_fc_row_127 => (r#"bulk ai 127"#, r##"print -r "$(( 1 && (0 || 1) ))""##);
        bulk_ai_fc_row_128 => (r#"bulk ai 128"#, r##"ai_e=(u v w); print -r "${(e)ai_e}""##);
        bulk_ai_fc_row_129 => (r#"bulk ai 129"#, r##"print -r "$(( 15 ^ 9 ))""##);
        bulk_ai_fc_row_130 => (r#"bulk ai 130"#, r##"print -r "${(F)ai_par}"; ai_par=$'p\nq'"##);
        bulk_ai_fc_row_131 => (r#"bulk ai 131"#, r##"print -r "$(( 4 % 2 == 0 ))""##);
        bulk_ai_fc_row_132 => (r#"bulk ai 132"#, r##"[[ v1_ai -ef v1_ai ]]; print -r "ef2=$?""##);
        bulk_ai_fc_row_133 => (r#"bulk ai 133"#, r##"print -r "$(( 5 ** 2 % 7 ))""##);
        bulk_ai_fc_row_134 => (r#"bulk ai 134"#, r##"ai_slice=abcdefghij; print -r "$ai_slice[3,7]""##);
        bulk_ai_fc_row_135 => (r#"bulk ai 135"#, r##"print -r "$(( 2#101 & 2#010 ))""##);
        bulk_ai_fc_row_136 => (r#"bulk ai 136"#, r##"setopt extendedglob; [[ foo_ai = (#i)FOO_ai ]]; print -r "ci2=$?""##);
        bulk_ai_fc_row_137 => (r#"bulk ai 137"#, r##"print -r "$(( 0x10001 % 256 ))""##);
        bulk_ai_fc_row_138 => (r#"bulk ai 138"#, r##"unset AI_X; typeset +x AI_X; AI_X=hid; print -r "${+AI_X}""##);
        bulk_ai_fc_row_139 => (r#"bulk ai 139"#, r##"print -r "$(( 8 +--- 3 ))""##);
        bulk_ai_fc_row_140 => (r#"bulk ai 140"#, r##"ai_ww=(i j k); print -r "${ai_ww[(w)-1]}""##);
        bulk_ai_fc_row_141 => (r#"bulk ai 141"#, r##"print -r "$(( 3>2>1 ))""##);
        bulk_ai_fc_row_142 => (r#"bulk ai 142"#, r##"print -r $(( ##9 ))"##);
        bulk_ai_fc_row_143 => (r#"bulk ai 143"#, r##": $(( ai_void=6 )); print -r "$ai_void""##);
        bulk_ai_fc_row_144 => (r#"bulk ai 144"#, r##"print -r "$(( 1<<0 ))""##);
        bulk_ai_fc_row_145 => (r#"bulk ai 145"#, r##"[[ -n /dev/null ]]; print -r "nnf=$?""##);
        bulk_ai_fc_row_146 => (r#"bulk ai 146"#, r##"print -r "$(( 72 / 8 / 3 ))""##);
        bulk_ai_fc_row_147 => (r#"bulk ai 147"#, r##"typeset -Z2 zi_ai=4; print -r "$zi_ai""##);
        bulk_ai_fc_row_148 => (r#"bulk ai 148"#, r##"print -r "$(( ~(255) & 0xff ))""##);
    }
}

mod corpus_dash_fc_bulk_aj {
    use super::*;

    parity_gap_tests! {
        bulk_aj_fc_row_001 => (r#"bulk aj 001"#, r##"arr_aj=(x y z); print -r "${arr_aj[(R)y]}""##);
        bulk_aj_fc_row_002 => (r#"bulk aj 002"#, r##"arr_aj=(x y z); print -r "${arr_aj[(r)y]}""##);
        bulk_aj_fc_row_003 => (r#"bulk aj 003"#, r##"arr_aj=(1 2 3); print -r "${arr_aj[(Ie)2]}""##);
        bulk_aj_fc_row_004 => (r#"bulk aj 004"#, r##"arr_aj=(1 2 3); print -r "${arr_aj[(i)2]}""##);
        bulk_aj_fc_row_005 => (r#"bulk aj 005"#, r##"arr_aj=(a b c d); print -r "${arr_aj[2,3]}""##);
        bulk_aj_fc_row_006 => (r#"bulk aj 006"#, r##"arr_aj=(1 2 3); print -r "${arr_aj[1,-1]}""##);
        bulk_aj_fc_row_007 => (r#"bulk aj 007"#, r##"unset y_aj; : ${y_aj::=defaj}; print -r "$y_aj""##);
        bulk_aj_fc_row_008 => (r#"bulk aj 008"#, r##"unset x_aj; print -r "${${x_aj:-fb}}""##);
        bulk_aj_fc_row_009 => (r#"bulk aj 009"#, r##"unset x_aj2; print -r "${${x_aj2:+set}:-unset}""##);
        bulk_aj_fc_row_010 => (r#"bulk aj 010"#, r##"str_aj=abc.def; print -r "${str_aj:r}:${str_aj:e}""##);
        bulk_aj_fc_row_011 => (r#"bulk aj 011"#, r##"[[ 42 = <-> ]]; print -r "ng=$?""##);
        bulk_aj_fc_row_012 => (r#"bulk aj 012"#, r##"[[ abc = <-> ]]; print -r "na=$?""##);
        bulk_aj_fc_row_013 => (r#"bulk aj 013"#, r##"[[ host_aj = ##host_aj ]]; print -r "pf=$?""##);
        bulk_aj_fc_row_014 => (r#"bulk aj 014"#, r##"[[ host_aj = host_aj## ]]; print -r "sf=$?""##);
        bulk_aj_fc_row_015 => (r#"bulk aj 015"#, r##"setopt extendedglob; [[ abc_aj = [a-z]## ]]; print -r "rp=$?""##);
        bulk_aj_fc_row_016 => (r#"bulk aj 016"#, r##"setopt extendedglob; [[ abc_aj = (#i)ABC_aj ]]; print -r "ci=$?""##);
        bulk_aj_fc_row_017 => (r#"bulk aj 017"#, r##"[[ /etc/hosts -ef /etc/hosts ]]; print -r "ef=$?""##);
        bulk_aj_fc_row_018 => (r#"bulk aj 018"#, r##"[[ /etc/hosts -nt /tmp ]]; print -r "nt=$?""##);
        bulk_aj_fc_row_019 => (r#"bulk aj 019"#, r##"[[ /tmp -ot /etc/hosts ]]; print -r "ot=$?""##);
        bulk_aj_fc_row_020 => (r#"bulk aj 020"#, r##"print -r "$(( 1_000 + 2_000 ))""##);
        bulk_aj_fc_row_021 => (r#"bulk aj 021"#, r##"print -r $(( ##a ))"##);
        bulk_aj_fc_row_022 => (r#"bulk aj 022"#, r##"(( x_aj = 5#101 )); print -r "$x_aj""##);
        bulk_aj_fc_row_023 => (r#"bulk aj 023"#, r##"print -r "$(( 5#101 ))""##);
        bulk_aj_fc_row_024 => (r#"bulk aj 024"#, r##"print -r "$(( 12#9b ))""##);
        bulk_aj_fc_row_025 => (r#"bulk aj 025"#, r##"print -r "$(( 0b101010 ))""##);
        bulk_aj_fc_row_026 => (r#"bulk aj 026"#, r##"print -r "$(( 2 ** 3 ** 2 ))""##);
        bulk_aj_fc_row_027 => (r#"bulk aj 027"#, r##"print -r "$(( 9 & 6 ^ 3 ))""##);
        bulk_aj_fc_row_028 => (r#"bulk aj 028"#, r##"print -r "$(( 128 >> 4 >> 1 ))""##);
        bulk_aj_fc_row_029 => (r#"bulk aj 029"#, r##"print -r "$(( ~(255) & 0xff ))""##);
        bulk_aj_fc_row_030 => (r#"bulk aj 030"#, r##"print -r "$(( 0xabc & 0xf0 ))""##);
        bulk_aj_fc_row_031 => (r#"bulk aj 031"#, r##"float f1_aj=1.5 f2_aj=2; print -r "$(( f1_aj * f2_aj ))""##);
        bulk_aj_fc_row_032 => (r#"bulk aj 032"#, r##"typeset -F2 f_aj=3.14159; print -r "$f_aj""##);
        bulk_aj_fc_row_033 => (r#"bulk aj 033"#, r##"typeset -Z5 z_aj=7; print -r "$z_aj""##);
        bulk_aj_fc_row_034 => (r#"bulk aj 034"#, r##"typeset -Z2 zi_aj=4; print -r "$zi_aj""##);
        bulk_aj_fc_row_035 => (r#"bulk aj 035"#, r##"typeset -E2 e_aj=4000; print -r "$e_aj""##);
        bulk_aj_fc_row_036 => (r#"bulk aj 036"#, r##"typeset -i8 o_aj=10; print -r "$o_aj""##);
        bulk_aj_fc_row_037 => (r#"bulk aj 037"#, r##"typeset -i16 x_aj=0xff; print -r "$x_aj""##);
        bulk_aj_fc_row_038 => (r#"bulk aj 038"#, r##"typeset +L L_aj=AbCd; print -r "$L_aj""##);
        bulk_aj_fc_row_039 => (r#"bulk aj 039"#, r##"typeset +U U_aj=xy; print -r "$U_aj""##);
        bulk_aj_fc_row_040 => (r#"bulk aj 040"#, r##"word_aj="a  b   c"; print -r "${(w)word_aj}""##);
        bulk_aj_fc_row_041 => (r#"bulk aj 041"#, r##"print -r "${(%)3}""##);
        bulk_aj_fc_row_042 => (r#"bulk aj 042"#, r##"o_aj=8; print -r "${(0)o_aj}""##);
        bulk_aj_fc_row_043 => (r#"bulk aj 043"#, r##"s_aj=barfooxyz; print -r "${s_aj[(i)foo]}""##);
        bulk_aj_fc_row_044 => (r#"bulk aj 044"#, r##"x_aj=foo; print -r "${x_aj:s/foo/bar/}""##);
        bulk_aj_fc_row_045 => (r#"bulk aj 045"#, r##"x_aj=foofoo; print -r "${x_aj//foo/bar}""##);
        bulk_aj_fc_row_046 => (r#"bulk aj 046"#, r##"typeset -A h_aj; h_aj[k]=v; print -r "${(k)h_aj}""##);
        bulk_aj_fc_row_047 => (r#"bulk aj 047"#, r##"typeset -A h_aj2; h_aj2[a]=1 h_aj2[b]=2; print -r "${(kv)h_aj2}""##);
        bulk_aj_fc_row_048 => (r#"bulk aj 048"#, r##"typeset -aU u_aj=(a b a); print -r "${#u_aj}""##);
        bulk_aj_fc_row_049 => (r#"bulk aj 049"#, r##"fn_aj(){ print -r $1 $2; }; fn_aj a b""##);
        bulk_aj_fc_row_050 => (r#"bulk aj 050"#, r##"local x_aj=1; fn_aj(){ local x_aj=2; print -r $x_aj; }; fn_aj""##);
        bulk_aj_fc_row_051 => (r#"bulk aj 051"#, r##"print -r "${(e):-3+4}""##);
        bulk_aj_fc_row_052 => (r#"bulk aj 052"#, r##"print -r "${(P)x_aj}"; x_aj=HOME"##);
        bulk_aj_fc_row_053 => (r#"bulk aj 053"#, r##"print -r "${(q+)x_aj}"; x_aj=hi"##);
        bulk_aj_fc_row_054 => (r#"bulk aj 054"#, r##"print -r "${(qq)x_aj}"; x_aj=hi"##);
        bulk_aj_fc_row_055 => (r#"bulk aj 055"#, r##"unset y_aj; print -r "${y_aj:-def}""##);
        bulk_aj_fc_row_056 => (r#"bulk aj 056"#, r##"y_aj=set; print -r "${y_aj:+yes}""##);
        bulk_aj_fc_row_057 => (r#"bulk aj 057"#, r##"print -r "${(j:,:)a_aj}"; a_aj=(x y z)"##);
        bulk_aj_fc_row_058 => (r#"bulk aj 058"#, r##"print -r "${(s.:.)x_aj}"; x_aj=a.b.c"##);
        bulk_aj_fc_row_059 => (r#"bulk aj 059"#, r##"print -r "${(f)x_aj}"; x_aj=$'a\nb\nc'"##);
        bulk_aj_fc_row_060 => (r#"bulk aj 060"#, r##"print -r "${(ps:\n:)x_aj}"; x_aj=$'a\nb'"##);
        bulk_aj_fc_row_061 => (r#"bulk aj 061"#, r##"typeset -h x_aj=1; print -r "${+x_aj}""##);
        bulk_aj_fc_row_062 => (r#"bulk aj 062"#, r##"print -r "${(b)x_aj}"; x_aj=hi"##);
        bulk_aj_fc_row_063 => (r#"bulk aj 063"#, r##"print -r "${(A)x_aj}"; x_aj=1 2"##);
        bulk_aj_fc_row_064 => (r#"bulk aj 064"#, r##"print -r "${(aa)x_aj}"; x_aj=(1 2)"##);
        bulk_aj_fc_row_065 => (r#"bulk aj 065"#, r##"print -r "${pipestatus}"; true | true"##);
        bulk_aj_fc_row_066 => (r#"bulk aj 066"#, r##"print -r "${pipestatus[1]}"; true | false"##);
        bulk_aj_fc_row_067 => (r#"bulk aj 067"#, r##"print -r "${status}"; (exit 7)"##);
        bulk_aj_fc_row_068 => (r#"bulk aj 068"#, r##"print -r "${+functions[fn_aj]}"; fn_aj() {}"##);
        bulk_aj_fc_row_069 => (r#"bulk aj 069"#, r##"emulate -L zsh; print -r $?"##);
        bulk_aj_fc_row_070 => (r#"bulk aj 070"#, r##"[[ -o interactive ]]; print -r $?"##);
        bulk_aj_fc_row_071 => (r#"bulk aj 071"#, r##"print -r "$(( 1 && (0 || 1) ))""##);
        bulk_aj_fc_row_072 => (r#"bulk aj 072"#, r##"print -r "$(( (1>0) + (0>0) ))""##);
        bulk_aj_fc_row_073 => (r#"bulk aj 073"#, r##"print -r "$(( 3 > 2 > 1 ))""##);
        bulk_aj_fc_row_074 => (r#"bulk aj 074"#, r##"print -r "$(( 4 % 2 == 0 ))""##);
        bulk_aj_fc_row_075 => (r#"bulk aj 075"#, r##"print -r "$(( 0 - 1 == -1 ))""##);
        bulk_aj_fc_row_076 => (r#"bulk aj 076"#, r##"print -r "$(( 1<<0 ))""##);
        bulk_aj_fc_row_077 => (r#"bulk aj 077"#, r##"print -r "$(( 72 / 8 / 3 ))""##);
        bulk_aj_fc_row_078 => (r#"bulk aj 078"#, r##"print -r "$(( 100 / 20 / 5 ))""##);
        bulk_aj_fc_row_079 => (r#"bulk aj 079"#, r##"print -r "$(( 24 % 5 % 3 ))""##);
        bulk_aj_fc_row_080 => (r#"bulk aj 080"#, r##"print -r "$(( 2#101 & 2#010 ))""##);
        bulk_aj_fc_row_081 => (r#"bulk aj 081"#, r##"print -r "$(( 8#10 ))""##);
        bulk_aj_fc_row_082 => (r#"bulk aj 082"#, r##"print -r "$(( 16#abc ))""##);
        bulk_aj_fc_row_083 => (r#"bulk aj 083"#, r##"print -r "$(( 2#1000 ))""##);
        bulk_aj_fc_row_084 => (r#"bulk aj 084"#, r##"print -r "$(( 0x80 >> 4 ))""##);
        bulk_aj_fc_row_085 => (r#"bulk aj 085"#, r##"print -r "$(( 5 ** 2 % 7 ))""##);
        bulk_aj_fc_row_086 => (r#"bulk aj 086"#, r##"print -r "$(( (9>8)>>(1<0) ))""##);
        bulk_aj_fc_row_087 => (r#"bulk aj 087"#, r##"print -r "$(( 15 ^ 9 ))""##);
        bulk_aj_fc_row_088 => (r#"bulk aj 088"#, r##"print -r "$(( 2 | 4 | 8 ))""##);
        bulk_aj_fc_row_089 => (r#"bulk aj 089"#, r##"print -r "$(( ~2 & 7 ))""##);
        bulk_aj_fc_row_090 => (r#"bulk aj 090"#, r##"print -r "$(( 0 || 0 || 7 ))""##);
        bulk_aj_fc_row_091 => (r#"bulk aj 091"#, r##"print -r "$(( 1 || -1 ))""##);
        bulk_aj_fc_row_092 => (r#"bulk aj 092"#, r##"print -r "$(( 3 <|> 5 ))""##);
        bulk_aj_fc_row_093 => (r#"bulk aj 093"#, r##"print -r "$(( 3 <> 5 ))""##);
        bulk_aj_fc_row_094 => (r#"bulk aj 094"#, r##"[[ -a /etc/hosts ]]; print -r "a=$?""##);
        bulk_aj_fc_row_095 => (r#"bulk aj 095"#, r##"[[ -b /dev/null ]]; print -r "b=$?""##);
        bulk_aj_fc_row_096 => (r#"bulk aj 096"#, r##"[[ -c /dev/null ]]; print -r "c=$?""##);
        bulk_aj_fc_row_097 => (r#"bulk aj 097"#, r##"[[ -u /etc/hosts ]]; print -r "u=$?""##);
        bulk_aj_fc_row_098 => (r#"bulk aj 098"#, r##"[[ -g / ]]; print -r "g=$?""##);
        bulk_aj_fc_row_099 => (r#"bulk aj 099"#, r##"[[ -k /tmp ]]; print -r "k=$?""##);
        bulk_aj_fc_row_100 => (r#"bulk aj 100"#, r##"[[ bee_aj = *ee* ]]; print -r "sub=$?""##);
        bulk_aj_fc_row_101 => (r#"bulk aj 101"#, r##"[[ AAA_aj =~ ^A+ ]]; print -r "pl=$?""##);
        bulk_aj_fc_row_102 => (r#"bulk aj 102"#, r##"setopt extendedglob; [[ tag_aj = (#m)[a-z]##_aj ]]; print -r "mh=$?""##);
        bulk_aj_fc_row_103 => (r#"bulk aj 103"#, r##"setopt extendedglob; [[ mix123_aj = [[:digit:]]# ]]; print -r "dg=$?""##);
        bulk_aj_fc_row_104 => (r#"bulk aj 104"#, r##"print -r "${(L)@}"; set -- MIXED_arg"##);
        bulk_aj_fc_row_105 => (r#"bulk aj 105"#, r##"ai_slice_aj=abcdefghij; print -r "$ai_slice_aj[3,7]""##);
        bulk_aj_fc_row_106 => (r#"bulk aj 106"#, r##"str_aj2="abc.def.ghi"; print -r "${str_aj2:r}""##);
        bulk_aj_fc_row_107 => (r#"bulk aj 107"#, r##"str_aj3="abc.def.ghi"; print -r "${str_aj3:e}""##);
        bulk_aj_fc_row_108 => (r#"bulk aj 108"#, r##"print -r "${(L)${(U)aj_mix}}"; aj_mix=aBc"##);
        bulk_aj_fc_row_109 => (r#"bulk aj 109"#, r##"print -r "${(c)#a_aj}"; a_aj=(abc def)"##);
        bulk_aj_fc_row_110 => (r#"bulk aj 110"#, r##"print -r "${(w)#x_aj}"; x_aj="a b c d""##);
        bulk_aj_fc_row_111 => (r#"bulk aj 111"#, r##"print -r "${#x_aj}"; x_aj=hello"##);
        bulk_aj_fc_row_112 => (r#"bulk aj 112"#, r##"print -r "${#a_aj}"; a_aj=(a b c d)"##);
        bulk_aj_fc_row_113 => (r#"bulk aj 113"#, r##"typeset -A m_aj=(a 1 b 2 c 3); print -r "${#m_aj}""##);
        bulk_aj_fc_row_114 => (r#"bulk aj 114"#, r##"print -r "${(t)x_aj}"; x_aj=hello"##);
        bulk_aj_fc_row_115 => (r#"bulk aj 115"#, r##"unset y_aj; print -r "${+y_aj}""##);
        bulk_aj_fc_row_116 => (r#"bulk aj 116"#, r##"x_aj=hello; print -r "${+x_aj}""##);
        bulk_aj_fc_row_117 => (r#"bulk aj 117"#, r##"print -r "${(u)a_aj}"; a_aj=(a b a c b a)"##);
        bulk_aj_fc_row_118 => (r#"bulk aj 118"#, r##"print -r "${(o)a_aj}"; a_aj=(c a b)"##);
        bulk_aj_fc_row_119 => (r#"bulk aj 119"#, r##"print -r "${(O)a_aj}"; a_aj=(c a b)"##);
        bulk_aj_fc_row_120 => (r#"bulk aj 120"#, r##"print -r "${(on)a_aj}"; a_aj=(10 2 100 1)"##);
    }
}

mod corpus_dash_fc_bulk_ak {
    use super::*;

    parity_gap_tests! {
        bulk_ak_fc_row_001 => (r#"bulk ak 001"#, r##"[[ -v x_ak ]]; print -r $?; x_ak=1"##);
        bulk_ak_fc_row_002 => (r#"bulk ak 002"#, r##"unset y_ak; [[ -v y_ak ]]; print -r $?"##);
        bulk_ak_fc_row_003 => (r#"bulk ak 003"#, r##"[[ -h /dev/stdin ]]; print -r "hs=$?""##);
        bulk_ak_fc_row_004 => (r#"bulk ak 004"#, r##"[[ -p /dev/fd/0 ]]; print -r "ps=$?""##);
        bulk_ak_fc_row_005 => (r#"bulk ak 005"#, r##"[[ -O /etc/hosts ]]; print -r "os=$?""##);
        bulk_ak_fc_row_006 => (r#"bulk ak 006"#, r##"[[ -G / ]]; print -r "gs=$?""##);
        bulk_ak_fc_row_007 => (r#"bulk ak 007"#, r##"[[ v1_ak -ef v1_ak ]]; print -r "ef=$?""##);
        bulk_ak_fc_row_008 => (r#"bulk ak 008"#, r##"setopt extendedglob; [[ foo_ak = (#b)oo ]]; print -r "bb=$?""##);
        bulk_ak_fc_row_009 => (r#"bulk ak 009"#, r##"setopt extendedglob; [[ foo_ak = (#s)fo ]]; print -r "ss=$?""##);
        bulk_ak_fc_row_010 => (r#"bulk ak 010"#, r##"setopt extendedglob; [[ foo_ak = fo(#e) ]]; print -r "ee=$?""##);
        bulk_ak_fc_row_011 => (r#"bulk ak 011"#, r##"integer i_ak=5; (( i_ak |= 3 )); print -r "$i_ak""##);
        bulk_ak_fc_row_012 => (r#"bulk ak 012"#, r##"integer i_ak=5; (( i_ak &= 3 )); print -r "$i_ak""##);
        bulk_ak_fc_row_013 => (r#"bulk ak 013"#, r##"integer i_ak=5; (( i_ak ^= 3 )); print -r "$i_ak""##);
        bulk_ak_fc_row_014 => (r#"bulk ak 014"#, r##"integer i_ak=5; (( i_ak <<= 1 )); print -r "$i_ak""##);
        bulk_ak_fc_row_015 => (r#"bulk ak 015"#, r##"integer i_ak=5; (( i_ak >>= 1 )); print -r "$i_ak""##);
        bulk_ak_fc_row_016 => (r#"bulk ak 016"#, r##"print -r "$(( true ))""##);
        bulk_ak_fc_row_017 => (r#"bulk ak 017"#, r##"print -r "$(( false ))""##);
        bulk_ak_fc_row_018 => (r#"bulk ak 018"#, r##"print -r "$(( ##Z ))""##);
        bulk_ak_fc_row_019 => (r#"bulk ak 019"#, r##"print -r "$(( ##b ))""##);
        bulk_ak_fc_row_020 => (r#"bulk ak 020"#, r##"print -r "$(( 0b1111 ))""##);
        bulk_ak_fc_row_021 => (r#"bulk ak 021"#, r##"print -r "$(( 0xffFF ))""##);
        bulk_ak_fc_row_022 => (r#"bulk ak 022"#, r##"typeset -F1 cmp_ak=1.05; print -r "$(( cmp_ak > 1 ))""##);
        bulk_ak_fc_row_023 => (r#"bulk ak 023"#, r##"typeset -R4 r_ak=hi; print -r "$r_ak""##);
        bulk_ak_fc_row_024 => (r#"bulk ak 024"#, r##"typeset -aS ary_ak="x y"; print -r "$#ary_ak $ary_ak[2]""##);
        bulk_ak_fc_row_025 => (r#"bulk ak 025"#, r##"typeset +i pi_ak=4; print -r "$pi_ak""##);
        bulk_ak_fc_row_026 => (r#"bulk ak 026"#, r##"arr_ak=(1); arr_ak[1]+=2; print -r "${arr_ak[1]}""##);
        bulk_ak_fc_row_027 => (r#"bulk ak 027"#, r##"arr_ak=(a b c); print -r "${arr_ak[@]:1}""##);
        bulk_ak_fc_row_028 => (r#"bulk ak 028"#, r##"arr_ak=(9 8 7); print -r "${arr_ak[-3,-2]}""##);
        bulk_ak_fc_row_029 => (r#"bulk ak 029"#, r##"print -r "${(j:-:)a_ak}"; a_ak=(p q r)"##);
        bulk_ak_fc_row_030 => (r#"bulk ak 030"#, r##"print -r "${(j::)a_ak}"; a_ak=(x y)"##);
        bulk_ak_fc_row_031 => (r#"bulk ak 031"#, r##"print -r "${(b)x_ak}"; x_ak=hi"##);
        bulk_ak_fc_row_032 => (r#"bulk ak 032"#, r##"print -r "${(w)word_ak}"; word_ak="a  b   c""##);
        bulk_ak_fc_row_033 => (r#"bulk ak 033"#, r##"print -r "${(W)word_ak}"; word_ak="  hi  ""##);
        bulk_ak_fc_row_034 => (r#"bulk ak 034"#, r##"word_ak=$'l1\nl2'; print -r "${(@f)word_ak}""##);
        bulk_ak_fc_row_035 => (r#"bulk ak 035"#, r##"print -r "${(z)word_ak}"; word_ak="a b c""##);
        bulk_ak_fc_row_036 => (r#"bulk ak 036"#, r##"x_ak=a1a2; pat_ak=a; print -r "${x_ak//pat_ak/repl}""##);
        bulk_ak_fc_row_037 => (r#"bulk ak 037"#, r##"x_ak=abc; pat_ak=a; print -r "${x_ak/#pat_ak/repl}""##);
        bulk_ak_fc_row_038 => (r#"bulk ak 038"#, r##"x_ak=abc; pat_ak=c; print -r "${x_ak/%pat_ak/repl}""##);
        bulk_ak_fc_row_039 => (r#"bulk ak 039"#, r##"print -r "${PWD:h}""##);
        bulk_ak_fc_row_040 => (r#"bulk ak 040"#, r##"pushd /tmp >/dev/null; popd >/dev/null; print -r $?"##);
        bulk_ak_fc_row_041 => (r#"bulk ak 041"#, r##"set -- a b c; shift; print -r "$1""##);
        bulk_ak_fc_row_042 => (r#"bulk ak 042"#, r##"set -- a b c; shift 2; print -r $#"##);
        bulk_ak_fc_row_043 => (r#"bulk ak 043"#, r##"fn_ak(){ typeset -a la_ak=(x y); print -r "${#la_ak}"; }; fn_ak"##);
        bulk_ak_fc_row_044 => (r#"bulk ak 044"#, r##"print -r "${(L)${(U)mix_ak}}"; mix_ak=aBc"##);
        bulk_ak_fc_row_045 => (r#"bulk ak 045"#, r##"typeset -A h_ak; h_ak=(k v); print -r "${h_ak[(R)v]}""##);
        bulk_ak_fc_row_046 => (r#"bulk ak 046"#, r##"typeset -A h_ak2; h_ak2=(a 1 b 2); print -r "${h_ak2[(r)2]}""##);
        bulk_ak_fc_row_047 => (r#"bulk ak 047"#, r##"typeset -A h_ak3; h_ak3=(x 1 y 2); print -r "${(k)h_ak3}""##);
        bulk_ak_fc_row_048 => (r#"bulk ak 048"#, r##"typeset -A h_ak4; h_ak4=(x 1 y 2); print -r "${(v)h_ak4}""##);
        bulk_ak_fc_row_049 => (r#"bulk ak 049"#, r##"typeset -A h_ak5; h_ak5=(x 1 y 2); print -r "${(kv)h_ak5}""##);
        bulk_ak_fc_row_050 => (r#"bulk ak 050"#, r##"typeset -A h_ak6; h_ak6=(x 1 y 2); print -r "${(@Mk)h_ak6}""##);
        bulk_ak_fc_row_051 => (r#"bulk ak 051"#, r##"typeset -A h_ak7; h_ak7=(x 1 y 2); print -r "${(@Mv)h_ak7}""##);
        bulk_ak_fc_row_052 => (r#"bulk ak 052"#, r##"print -r "${(@on)nums_ak}"; nums_ak=(10 2 100)"##);
        bulk_ak_fc_row_053 => (r#"bulk ak 053"#, r##"print -r "${(@oa)nums_ak}"; nums_ak=(10 2 100)"##);
        bulk_ak_fc_row_054 => (r#"bulk ak 054"#, r##"print -r "${(@eu)nums_ak}"; nums_ak=(a A b B)"##);
        bulk_ak_fc_row_055 => (r#"bulk ak 055"#, r##"print -r "${(@L)nums_ak}"; nums_ak=(a B c)"##);
        bulk_ak_fc_row_056 => (r#"bulk ak 056"#, r##"print -r "${arr_ak[(w)2]}"; arr_ak=(a b c)"##);
        bulk_ak_fc_row_057 => (r#"bulk ak 057"#, r##"print -r "${arr_ak[@]:1:2}"; arr_ak=(a b c)"##);
        bulk_ak_fc_row_058 => (r#"bulk ak 058"#, r##"arr_ak=(1 2); arr_ak+=3; print -r "${arr_ak[@]}""##);
        bulk_ak_fc_row_059 => (r#"bulk ak 059"#, r##"unset x_ak; typeset +x x_ak=hid; print -r "${+x_ak}""##);
        bulk_ak_fc_row_060 => (r#"bulk ak 060"#, r##"print -r "${(e):-3+4}""##);
        bulk_ak_fc_row_061 => (r#"bulk ak 061"#, r##"print -r "${(q+)x_ak}"; x_ak=hi"##);
        bulk_ak_fc_row_062 => (r#"bulk ak 062"#, r##"print -r "${(pj:-:)a_ak}"; a_ak=(p q r)"##);
        bulk_ak_fc_row_063 => (r#"bulk ak 063"#, r##"print -r "${(pj::)a_ak}"; a_ak=(x y)"##);
        bulk_ak_fc_row_064 => (r#"bulk ak 064"#, r##"print -r "${(s.:.)x_ak}"; x_ak=a.b.c"##);
        bulk_ak_fc_row_065 => (r#"bulk ak 065"#, r##"print -r "${(ps:\n:)x_ak}"; x_ak=$'a\nb'"##);
        bulk_ak_fc_row_066 => (r#"bulk ak 066"#, r##"print -r "${(f)x_ak}"; x_ak=$'a\nb\nc'"##);
        bulk_ak_fc_row_067 => (r#"bulk ak 067"#, r##"print -r "${(A)x_ak}"; x_ak=1 2"##);
        bulk_ak_fc_row_068 => (r#"bulk ak 068"#, r##"print -r "${(aa)x_ak}"; x_ak=(1 2)"##);
        bulk_ak_fc_row_069 => (r#"bulk ak 069"#, r##"print -r "${(c)#a_ak}"; a_ak=(abc def)"##);
        bulk_ak_fc_row_070 => (r#"bulk ak 070"#, r##"print -r "${(w)#x_ak}"; x_ak="a b c""##);
        bulk_ak_fc_row_071 => (r#"bulk ak 071"#, r##"[[ -z "" ]]; print -r "ze=$?""##);
        bulk_ak_fc_row_072 => (r#"bulk ak 072"#, r##"[[ -n /dev/null ]]; print -r "nn=$?""##);
        bulk_ak_fc_row_073 => (r#"bulk ak 073"#, r##"[[ zero_ak = <-> ]]; print -r "dg=$?""##);
        bulk_ak_fc_row_074 => (r#"bulk ak 074"#, r##"print -r "$(( 1 , 2 , 3 ))""##);
        bulk_ak_fc_row_075 => (r#"bulk ak 075"#, r##"print -r "$(( ##n ))""##);
        bulk_ak_fc_row_076 => (r#"bulk ak 076"#, r##"print -r "$(( ##9 ))""##);
        bulk_ak_fc_row_077 => (r#"bulk ak 077"#, r##"print -r "$(( 2#1010 ))""##);
        bulk_ak_fc_row_078 => (r#"bulk ak 078"#, r##"print -r "$(( 8#17 ))""##);
        bulk_ak_fc_row_079 => (r#"bulk ak 079"#, r##"print -r "$(( 16#FF ))""##);
        bulk_ak_fc_row_080 => (r#"bulk ak 080"#, r##"print -r "$(( 0xDEAD ))""##);
        bulk_ak_fc_row_081 => (r#"bulk ak 081"#, r##"print -r "$(( -(-(-5)) ))""##);
        bulk_ak_fc_row_082 => (r#"bulk ak 082"#, r##"print -r "$(( 3>2 ? 10 : 20 ))""##);
        bulk_ak_fc_row_083 => (r#"bulk ak 083"#, r##"print -r "$(( 5 ** 0 ** 3 ))""##);
        bulk_ak_fc_row_084 => (r#"bulk ak 084"#, r##"print -r "$(( 11 ** 2 % 50 ))""##);
        bulk_ak_fc_row_085 => (r#"bulk ak 085"#, r##"print -r "$(( 1000003 % 97 ))""##);
        bulk_ak_fc_row_086 => (r#"bulk ak 086"#, r##"print -r "$(( 63 & 31 | 15 ))""##);
        bulk_ak_fc_row_087 => (r#"bulk ak 087"#, r##"print -r "$(( 0x7fffffff & 0 ))""##);
        bulk_ak_fc_row_088 => (r#"bulk ak 088"#, r##"print -r "$(( 1<<63 ))""##);
        bulk_ak_fc_row_089 => (r#"bulk ak 089"#, r##"print -r "$(( 4 <> 4 ))""##);
        bulk_ak_fc_row_090 => (r#"bulk ak 090"#, r##"[[ bee_ak = *ee* ]]; print -r "bm=$?""##);
        bulk_ak_fc_row_091 => (r#"bulk ak 091"#, r##"[[ ABC_ak =~ ^[A-Z]+ ]]; print -r "rx=$?""##);
        bulk_ak_fc_row_092 => (r#"bulk ak 092"#, r##"setopt extendedglob; [[ tag_ak = (#m)[a-z]##_ak ]]; print -r "mh=$?""##);
        bulk_ak_fc_row_093 => (r#"bulk ak 093"#, r##"setopt extendedglob; [[ mix123_ak = [[:digit:]]# ]]; print -r "dg2=$?""##);
        bulk_ak_fc_row_094 => (r#"bulk ak 094"#, r##"print -r "${(L)@}"; set -- MIXED_arg"##);
        bulk_ak_fc_row_095 => (r#"bulk ak 095"#, r##"ai_slice_ak=abcdefghij; print -r "$ai_slice_ak[3,7]""##);
        bulk_ak_fc_row_096 => (r#"bulk ak 096"#, r##"str_ak2="abc.def.ghi"; print -r "${str_ak2:r}""##);
        bulk_ak_fc_row_097 => (r#"bulk ak 097"#, r##"str_ak3="abc.def.ghi"; print -r "${str_ak3:e}""##);
        bulk_ak_fc_row_098 => (r#"bulk ak 098"#, r##"print -r "${(u)a_ak}"; a_ak=(a b a c)"##);
        bulk_ak_fc_row_099 => (r#"bulk ak 099"#, r##"print -r "${(o)a_ak}"; a_ak=(c a b)"##);
        bulk_ak_fc_row_100 => (r#"bulk ak 100"#, r##"print -r "${(O)a_ak}"; a_ak=(c a b)"##);
        bulk_ak_fc_row_101 => (r#"bulk ak 101"#, r##"print -r "${(on)a_ak}"; a_ak=(10 2 100)"##);
        bulk_ak_fc_row_102 => (r#"bulk ak 102"#, r##"print -r "${(On)a_ak}"; a_ak=(10 2 100)"##);
        bulk_ak_fc_row_103 => (r#"bulk ak 103"#, r##"print -r "${(oa)a_ak}"; a_ak=(10 2 100)"##);
        bulk_ak_fc_row_104 => (r#"bulk ak 104"#, r##"print -r "${(eu)a_ak}"; a_ak=(a A b)"##);
        bulk_ak_fc_row_105 => (r#"bulk ak 105"#, r##"print -r "${(L@)a_ak}"; a_ak=(a B c)"##);
        bulk_ak_fc_row_106 => (r#"bulk ak 106"#, r##"arr_ak=(a b c); print -r "${arr_ak[(I)b]}""##);
        bulk_ak_fc_row_107 => (r#"bulk ak 107"#, r##"arr_ak=(a b c); print -r "${arr_ak[(i)b]}""##);
        bulk_ak_fc_row_108 => (r#"bulk ak 108"#, r##"arr_ak=(a b c); print -r "${arr_ak[(Ie)b]}""##);
        bulk_ak_fc_row_109 => (r#"bulk ak 109"#, r##"arr_ak=(a b c); print -r "${arr_ak[(ie)b]}""##);
        bulk_ak_fc_row_110 => (r#"bulk ak 110"#, r##"print -r "${arr_ak[(r)b]}"; arr_ak=(a b c b)"##);
        bulk_ak_fc_row_111 => (r#"bulk ak 111"#, r##"print -r "${arr_ak[(R)b]}"; arr_ak=(a b c b)"##);
        bulk_ak_fc_row_112 => (r#"bulk ak 112"#, r##"print -r "${arr_ak[2,-1]}"; arr_ak=(1 2 3 4)"##);
        bulk_ak_fc_row_113 => (r#"bulk ak 113"#, r##"print -r "${arr_ak[1,3]}"; arr_ak=(1 2 3 4 5)"##);
        bulk_ak_fc_row_114 => (r#"bulk ak 114"#, r##"print -r "${arr_ak[(w)-1]}"; arr_ak=(a b c)"##);
        bulk_ak_fc_row_115 => (r#"bulk ak 115"#, r##"word_ak="  hi  "; print -r "${(W)word_ak}""##);
        bulk_ak_fc_row_116 => (r#"bulk ak 116"#, r##"print -r "${(j:,:)a_ak}"; a_ak=(x y z)"##);
        bulk_ak_fc_row_117 => (r#"bulk ak 117"#, r##"print -r "${(pj:,:)a_ak}"; a_ak=(x y z)"##);
        bulk_ak_fc_row_118 => (r#"bulk ak 118"#, r##"print -r "${(F)a_ak}"; a_ak=$'p\nq'"##);
        bulk_ak_fc_row_119 => (r#"bulk ak 119"#, r##"print -r "${(e)a_ak}"; a_ak=$'3+4'"##);
        bulk_ak_fc_row_120 => (r#"bulk ak 120"#, r##"print -r "${(P)x_ak}"; x_ak=HOME"##);
    }
}



mod corpus_dash_fc_bulk_al {
    use super::*;

    parity_gap_tests! {
        bulk_al_fc_row_001 => (r#"bulk al 001"#, r###"print -r $(( !0 ))"###);
        bulk_al_fc_row_002 => (r#"bulk al 002"#, r###"integer n=5; (( n += 2 )); print -r $n"###);
        bulk_al_fc_row_003 => (r#"bulk al 003"#, r###"integer n=5; (( n -= 1 )); print -r $n"###);
        bulk_al_fc_row_004 => (r#"bulk al 004"#, r###"integer n=5; (( n *= 2 )); print -r $n"###);
        bulk_al_fc_row_005 => (r#"bulk al 005"#, r###"integer n=5; (( n |= 3 )); print -r $n"###);
        bulk_al_fc_row_006 => (r#"bulk al 006"#, r###"integer n=5; (( n &= 3 )); print -r $n"###);
        bulk_al_fc_row_007 => (r#"bulk al 007"#, r###"print -r $(( true ))"###);
        bulk_al_fc_row_008 => (r#"bulk al 008"#, r###"print -r $(( false ))"###);
        bulk_al_fc_row_009 => (r#"bulk al 009"#, r###"[[ -e / ]]; print -r $?"###);
        bulk_al_fc_row_010 => (r#"bulk al 010"#, r###"[[ -d /tmp ]]; print -r $?"###);
        bulk_al_fc_row_011 => (r#"bulk al 011"#, r###"[[ -f /etc/hosts ]]; print -r $?"###);
        bulk_al_fc_row_012 => (r#"bulk al 012"#, r###"[[ -r /etc/hosts ]]; print -r $?"###);
        bulk_al_fc_row_013 => (r#"bulk al 013"#, r###"[[ -w /tmp ]]; print -r $?"###);
        bulk_al_fc_row_014 => (r#"bulk al 014"#, r###"[[ -x /bin/sh ]]; print -r $?"###);
        bulk_al_fc_row_015 => (r#"bulk al 015"#, r###"[[ 42 = <-> ]]; print -r $?"###);
        bulk_al_fc_row_016 => (r#"bulk al 016"#, r###"[[ abc = <-> ]]; print -r $?"###);
        bulk_al_fc_row_017 => (r#"bulk al 017"#, r####"[[ host = ##host ]]; print -r $?"####);
        bulk_al_fc_row_018 => (r#"bulk al 018"#, r###"[[ -v x ]]; print -r $?; x=1"###);
        bulk_al_fc_row_019 => (r#"bulk al 019"#, r###"unset y; [[ -v y ]]; print -r $?"###);
        bulk_al_fc_row_020 => (r#"bulk al 020"#, r###"setopt extendedglob; [[ abc = (#i)ABC ]]; print -r $?"###);
        bulk_al_fc_row_021 => (r#"bulk al 021"#, r###"setopt extendedglob; [[ foo = (#b)oo ]]; print -r $?"###);
        bulk_al_fc_row_022 => (r#"bulk al 022"#, r###"[[ abc = a* ]]; print -r $?"###);
        bulk_al_fc_row_023 => (r#"bulk al 023"#, r###"[[ abc =~ ^a ]]; print -r $?"###);
        bulk_al_fc_row_024 => (r#"bulk al 024"#, r###"[[ -z '' ]]; print -r $?"###);
        bulk_al_fc_row_025 => (r#"bulk al 025"#, r###"[[ -n abc ]]; print -r $?"###);
        bulk_al_fc_row_026 => (r#"bulk al 026"#, r###"typeset -i n=10; print -r $n"###);
        bulk_al_fc_row_027 => (r#"bulk al 027"#, r###"typeset -l n=AbC; print -r $n"###);
        bulk_al_fc_row_028 => (r#"bulk al 028"#, r###"typeset -u n=xy; print -r $n"###);
        bulk_al_fc_row_029 => (r#"bulk al 029"#, r###"typeset -Z5 n=7; print -r $n"###);
        bulk_al_fc_row_030 => (r#"bulk al 030"#, r###"typeset -a a=(x y); print -r ${a[2]}"###);
        bulk_al_fc_row_031 => (r#"bulk al 031"#, r###"arr=(1 2); arr+=3; print -r ${arr[@]}"###);
        bulk_al_fc_row_032 => (r#"bulk al 032"#, r###"arr=(1); arr[1]+=2; print -r ${arr[1]}"###);
        bulk_al_fc_row_033 => (r#"bulk al 033"#, r###"unset v; print -r ${v:-def}"###);
        bulk_al_fc_row_034 => (r#"bulk al 034"#, r###"v=set; print -r ${v:+yes}"###);
        bulk_al_fc_row_035 => (r#"bulk al 035"#, r###"unset v; : ${v::=def}; print -r $v"###);
        bulk_al_fc_row_036 => (r#"bulk al 036"#, r###"print -r ${PWD:h}"###);
        bulk_al_fc_row_037 => (r#"bulk al 037"#, r###"print -r ${PWD:t}"###);
        bulk_al_fc_row_038 => (r#"bulk al 038"#, r###"true | true; print -r $?"###);
        bulk_al_fc_row_039 => (r#"bulk al 039"#, r###"true | false; print -r $?"###);
        bulk_al_fc_row_040 => (r#"bulk al 040"#, r###"print -r ${pipestatus[1]}; true | false"###);
        bulk_al_fc_row_041 => (r#"bulk al 041"#, r###"print -r ${#pipestatus}; true | true | true"###);
        bulk_al_fc_row_042 => (r#"bulk al 042"#, r###"set -- a b c; shift; print -r $1"###);
        bulk_al_fc_row_043 => (r#"bulk al 043"#, r###"set -- a b c; shift 2; print -r $#"###);
        bulk_al_fc_row_044 => (r#"bulk al 044"#, r###"fn(){ print -r $1; }; fn x"###);
        bulk_al_fc_row_045 => (r#"bulk al 045"#, r###"fn(){ local x=2; print -r $x; }; fn"###);
        bulk_al_fc_row_046 => (r#"bulk al 046"#, r###"fn(){ return 2; }; fn; print -r $?"###);
        bulk_al_fc_row_047 => (r#"bulk al 047"#, r###"print -r ${(q)x}; x=hi"###);
        bulk_al_fc_row_048 => (r#"bulk al 048"#, r###"print -r ${(qq)x}; x=hi"###);
    }
}

mod corpus_dash_fc_bulk_am {
    use super::*;

    parity_gap_tests! {
        bulk_am_fc_row_001 => (r#"bulk am 001"#, r###"[[ -v x ]]; print -r $?; x=1"###);
        bulk_am_fc_row_002 => (r#"bulk am 002"#, r###"unset y; [[ -v y ]]; print -r $?"###);
        bulk_am_fc_row_003 => (r#"bulk am 003"#, r###"setopt extendedglob; [[ abc = (#i)ABC ]]; print -r $?"###);
        bulk_am_fc_row_004 => (r#"bulk am 004"#, r###"setopt extendedglob; [[ foo = (#b)oo ]]; print -r $?"###);
        bulk_am_fc_row_005 => (r#"bulk am 005"#, r###"[[ abc = a* ]]; print -r $?"###);
        bulk_am_fc_row_006 => (r#"bulk am 006"#, r###"[[ abc =~ ^a ]]; print -r $?"###);
        bulk_am_fc_row_007 => (r#"bulk am 007"#, r###"[[ -z '' ]]; print -r $?"###);
        bulk_am_fc_row_008 => (r#"bulk am 008"#, r###"[[ -n abc ]]; print -r $?"###);
        bulk_am_fc_row_009 => (r#"bulk am 009"#, r###"typeset -i n=10; print -r $n"###);
        bulk_am_fc_row_010 => (r#"bulk am 010"#, r###"typeset -l n=AbC; print -r $n"###);
        bulk_am_fc_row_011 => (r#"bulk am 011"#, r###"typeset -u n=xy; print -r $n"###);
        bulk_am_fc_row_012 => (r#"bulk am 012"#, r###"typeset -Z5 n=7; print -r $n"###);
        bulk_am_fc_row_013 => (r#"bulk am 013"#, r###"typeset -a a=(x y); print -r ${a[2]}"###);
        bulk_am_fc_row_014 => (r#"bulk am 014"#, r###"arr=(1 2); arr+=3; print -r ${arr[@]}"###);
        bulk_am_fc_row_015 => (r#"bulk am 015"#, r###"arr=(1); arr[1]+=2; print -r ${arr[1]}"###);
        bulk_am_fc_row_016 => (r#"bulk am 016"#, r###"unset v; print -r ${v:-def}"###);
        bulk_am_fc_row_017 => (r#"bulk am 017"#, r###"v=set; print -r ${v:+yes}"###);
        bulk_am_fc_row_018 => (r#"bulk am 018"#, r###"unset v; : ${v::=def}; print -r $v"###);
        bulk_am_fc_row_019 => (r#"bulk am 019"#, r###"print -r ${PWD:h}"###);
        bulk_am_fc_row_020 => (r#"bulk am 020"#, r###"print -r ${PWD:t}"###);
        bulk_am_fc_row_021 => (r#"bulk am 021"#, r###"true | true; print -r $?"###);
        bulk_am_fc_row_022 => (r#"bulk am 022"#, r###"true | false; print -r $?"###);
        bulk_am_fc_row_023 => (r#"bulk am 023"#, r###"print -r ${pipestatus[1]}; true | false"###);
        bulk_am_fc_row_024 => (r#"bulk am 024"#, r###"print -r ${#pipestatus}; true | true | true"###);
        bulk_am_fc_row_025 => (r#"bulk am 025"#, r###"set -- a b c; shift; print -r $1"###);
        bulk_am_fc_row_026 => (r#"bulk am 026"#, r###"set -- a b c; shift 2; print -r $#"###);
        bulk_am_fc_row_027 => (r#"bulk am 027"#, r###"fn(){ print -r $1; }; fn x"###);
        bulk_am_fc_row_028 => (r#"bulk am 028"#, r###"fn(){ local x=2; print -r $x; }; fn"###);
        bulk_am_fc_row_029 => (r#"bulk am 029"#, r###"fn(){ return 2; }; fn; print -r $?"###);
        bulk_am_fc_row_030 => (r#"bulk am 030"#, r###"print -r ${(q)x}; x=hi"###);
        bulk_am_fc_row_031 => (r#"bulk am 031"#, r###"print -r ${(qq)x}; x=hi"###);
        bulk_am_fc_row_032 => (r#"bulk am 032"#, r###"x=hi; print -r ${(q-)x}"###);
        bulk_am_fc_row_033 => (r#"bulk am 033"#, r###"x=hi; print -r ${(q+)x}"###);
        bulk_am_fc_row_034 => (r#"bulk am 034"#, r###"print -r ${(w)w}; w=a b c"###);
        bulk_am_fc_row_035 => (r#"bulk am 035"#, r###"print -r ${(u)a}; a=(a a b)"###);
        bulk_am_fc_row_036 => (r#"bulk am 036"#, r###"print -r ${(o)a}; a=(c b a)"###);
        bulk_am_fc_row_037 => (r#"bulk am 037"#, r###"print -r ${(j:,:)a}; a=(x y)"###);
        bulk_am_fc_row_038 => (r#"bulk am 038"#, r###"arr=(a b c); print -r ${arr[(I)b]}"###);
        bulk_am_fc_row_039 => (r#"bulk am 039"#, r###"arr=(a b c); print -r ${arr[(R)b]}"###);
        bulk_am_fc_row_040 => (r#"bulk am 040"#, r###"arr=(9 8 7); print -r ${arr[-2,-1]}"###);
        bulk_am_fc_row_041 => (r#"bulk am 041"#, r###"typeset -A h; h=(k v); print -r ${(k)h}"###);
        bulk_am_fc_row_042 => (r#"bulk am 042"#, r###"typeset -A h; h=(a 1 b 2); print -r ${(kv)h}"###);
        bulk_am_fc_row_043 => (r#"bulk am 043"#, r###"print -r ${+options}"###);
        bulk_am_fc_row_044 => (r#"bulk am 044"#, r###"print -r ${+parameters}"###);
        bulk_am_fc_row_045 => (r#"bulk am 045"#, r###"print -r ${+aliases}"###);
        bulk_am_fc_row_046 => (r#"bulk am 046"#, r###"print -r ${+functions}"###);
        bulk_am_fc_row_047 => (r#"bulk am 047"#, r###"print -r $ZSH_NAME"###);
        bulk_am_fc_row_048 => (r#"bulk am 048"#, r###"print -r ${ZSH_VERSION%%.*}"###);
    }
}

mod corpus_dash_fc_bulk_an {
    use super::*;

    parity_gap_tests! {
        bulk_an_fc_row_001 => (r#"bulk an 001"#, r###"unset v; : ${v::=def}; print -r $v"###);
        bulk_an_fc_row_002 => (r#"bulk an 002"#, r###"print -r ${PWD:h}"###);
        bulk_an_fc_row_003 => (r#"bulk an 003"#, r###"print -r ${PWD:t}"###);
        bulk_an_fc_row_004 => (r#"bulk an 004"#, r###"true | true; print -r $?"###);
        bulk_an_fc_row_005 => (r#"bulk an 005"#, r###"true | false; print -r $?"###);
        bulk_an_fc_row_006 => (r#"bulk an 006"#, r###"print -r ${pipestatus[1]}; true | false"###);
        bulk_an_fc_row_007 => (r#"bulk an 007"#, r###"print -r ${#pipestatus}; true | true | true"###);
        bulk_an_fc_row_008 => (r#"bulk an 008"#, r###"set -- a b c; shift; print -r $1"###);
        bulk_an_fc_row_009 => (r#"bulk an 009"#, r###"set -- a b c; shift 2; print -r $#"###);
        bulk_an_fc_row_010 => (r#"bulk an 010"#, r###"fn(){ print -r $1; }; fn x"###);
        bulk_an_fc_row_011 => (r#"bulk an 011"#, r###"fn(){ local x=2; print -r $x; }; fn"###);
        bulk_an_fc_row_012 => (r#"bulk an 012"#, r###"fn(){ return 2; }; fn; print -r $?"###);
        bulk_an_fc_row_013 => (r#"bulk an 013"#, r###"print -r ${(q)x}; x=hi"###);
        bulk_an_fc_row_014 => (r#"bulk an 014"#, r###"print -r ${(qq)x}; x=hi"###);
        bulk_an_fc_row_015 => (r#"bulk an 015"#, r###"x=hi; print -r ${(q-)x}"###);
        bulk_an_fc_row_016 => (r#"bulk an 016"#, r###"x=hi; print -r ${(q+)x}"###);
        bulk_an_fc_row_017 => (r#"bulk an 017"#, r###"print -r ${(w)w}; w=a b c"###);
        bulk_an_fc_row_018 => (r#"bulk an 018"#, r###"print -r ${(u)a}; a=(a a b)"###);
        bulk_an_fc_row_019 => (r#"bulk an 019"#, r###"print -r ${(o)a}; a=(c b a)"###);
        bulk_an_fc_row_020 => (r#"bulk an 020"#, r###"print -r ${(j:,:)a}; a=(x y)"###);
        bulk_an_fc_row_021 => (r#"bulk an 021"#, r###"arr=(a b c); print -r ${arr[(I)b]}"###);
        bulk_an_fc_row_022 => (r#"bulk an 022"#, r###"arr=(a b c); print -r ${arr[(R)b]}"###);
        bulk_an_fc_row_023 => (r#"bulk an 023"#, r###"arr=(9 8 7); print -r ${arr[-2,-1]}"###);
        bulk_an_fc_row_024 => (r#"bulk an 024"#, r###"typeset -A h; h=(k v); print -r ${(k)h}"###);
        bulk_an_fc_row_025 => (r#"bulk an 025"#, r###"typeset -A h; h=(a 1 b 2); print -r ${(kv)h}"###);
        bulk_an_fc_row_026 => (r#"bulk an 026"#, r###"print -r ${+options}"###);
        bulk_an_fc_row_027 => (r#"bulk an 027"#, r###"print -r ${+parameters}"###);
        bulk_an_fc_row_028 => (r#"bulk an 028"#, r###"print -r ${+aliases}"###);
        bulk_an_fc_row_029 => (r#"bulk an 029"#, r###"print -r ${+functions}"###);
        bulk_an_fc_row_030 => (r#"bulk an 030"#, r###"print -r $ZSH_NAME"###);
        bulk_an_fc_row_031 => (r#"bulk an 031"#, r###"print -r ${ZSH_VERSION%%.*}"###);
        bulk_an_fc_row_032 => (r#"bulk an 032"#, r###"whence -w print"###);
        bulk_an_fc_row_033 => (r#"bulk an 033"#, r###"command -v true"###);
        bulk_an_fc_row_034 => (r#"bulk an 034"#, r###"emulate -L zsh; print -r $?"###);
        bulk_an_fc_row_035 => (r#"bulk an 035"#, r###"alias za='echo z'; za; unalias za 2>/dev/null"###);
        bulk_an_fc_row_036 => (r#"bulk an 036"#, r###"read -r line <<< 'one two'; print -r $line"###);
        bulk_an_fc_row_037 => (r#"bulk an 037"#, r###"cat <<< 'herestring'"###);
        bulk_an_fc_row_038 => (r#"bulk an 038"#, r###"echo hello 2>/dev/null"###);
        bulk_an_fc_row_039 => (r#"bulk an 039"#, r###"printf '%s\n' a b c | head -1"###);
        bulk_an_fc_row_040 => (r#"bulk an 040"#, r###"true && echo yes"###);
        bulk_an_fc_row_041 => (r#"bulk an 041"#, r###"false || echo yes"###);
        bulk_an_fc_row_042 => (r#"bulk an 042"#, r###"(exit 3); print -r $?"###);
        bulk_an_fc_row_043 => (r#"bulk an 043"#, r###"print -r ${status}; (exit 4)"###);
        bulk_an_fc_row_044 => (r#"bulk an 044"#, r###"print -r $(( 1_000 + 1 ))"###);
        bulk_an_fc_row_045 => (r#"bulk an 045"#, r###"print -r $(( 5#101 ))"###);
        bulk_an_fc_row_046 => (r#"bulk an 046"#, r###"print -r $(( 0b1111 ))"###);
        bulk_an_fc_row_047 => (r#"bulk an 047"#, r###"print -r $(( 2 ** 3 ** 2 ))"###);
        bulk_an_fc_row_048 => (r#"bulk an 048"#, r###"float f=1.5; print -r $(( f * 2 ))"###);
    }
}

mod corpus_dash_fc_bulk_ao {
    use super::*;

    parity_gap_tests! {
        bulk_ao_fc_row_001 => (r#"bulk ao 001"#, r###"print -r ${(u)a}; a=(a a b)"###);
        bulk_ao_fc_row_002 => (r#"bulk ao 002"#, r###"print -r ${(o)a}; a=(c b a)"###);
        bulk_ao_fc_row_003 => (r#"bulk ao 003"#, r###"print -r ${(j:,:)a}; a=(x y)"###);
        bulk_ao_fc_row_004 => (r#"bulk ao 004"#, r###"arr=(a b c); print -r ${arr[(I)b]}"###);
        bulk_ao_fc_row_005 => (r#"bulk ao 005"#, r###"arr=(a b c); print -r ${arr[(R)b]}"###);
        bulk_ao_fc_row_006 => (r#"bulk ao 006"#, r###"arr=(9 8 7); print -r ${arr[-2,-1]}"###);
        bulk_ao_fc_row_007 => (r#"bulk ao 007"#, r###"typeset -A h; h=(k v); print -r ${(k)h}"###);
        bulk_ao_fc_row_008 => (r#"bulk ao 008"#, r###"typeset -A h; h=(a 1 b 2); print -r ${(kv)h}"###);
        bulk_ao_fc_row_009 => (r#"bulk ao 009"#, r###"print -r ${+options}"###);
        bulk_ao_fc_row_010 => (r#"bulk ao 010"#, r###"print -r ${+parameters}"###);
        bulk_ao_fc_row_011 => (r#"bulk ao 011"#, r###"print -r ${+aliases}"###);
        bulk_ao_fc_row_012 => (r#"bulk ao 012"#, r###"print -r ${+functions}"###);
        bulk_ao_fc_row_013 => (r#"bulk ao 013"#, r###"print -r $ZSH_NAME"###);
        bulk_ao_fc_row_014 => (r#"bulk ao 014"#, r###"print -r ${ZSH_VERSION%%.*}"###);
        bulk_ao_fc_row_015 => (r#"bulk ao 015"#, r###"whence -w print"###);
        bulk_ao_fc_row_016 => (r#"bulk ao 016"#, r###"command -v true"###);
        bulk_ao_fc_row_017 => (r#"bulk ao 017"#, r###"emulate -L zsh; print -r $?"###);
        bulk_ao_fc_row_018 => (r#"bulk ao 018"#, r###"alias za='echo z'; za; unalias za 2>/dev/null"###);
        bulk_ao_fc_row_019 => (r#"bulk ao 019"#, r###"read -r line <<< 'one two'; print -r $line"###);
        bulk_ao_fc_row_020 => (r#"bulk ao 020"#, r###"cat <<< 'herestring'"###);
        bulk_ao_fc_row_021 => (r#"bulk ao 021"#, r###"echo hello 2>/dev/null"###);
        bulk_ao_fc_row_022 => (r#"bulk ao 022"#, r###"printf '%s\n' a b c | head -1"###);
        bulk_ao_fc_row_023 => (r#"bulk ao 023"#, r###"true && echo yes"###);
        bulk_ao_fc_row_024 => (r#"bulk ao 024"#, r###"false || echo yes"###);
        bulk_ao_fc_row_025 => (r#"bulk ao 025"#, r###"(exit 3); print -r $?"###);
        bulk_ao_fc_row_026 => (r#"bulk ao 026"#, r###"print -r ${status}; (exit 4)"###);
        bulk_ao_fc_row_027 => (r#"bulk ao 027"#, r###"print -r $(( 1_000 + 1 ))"###);
        bulk_ao_fc_row_028 => (r#"bulk ao 028"#, r###"print -r $(( 5#101 ))"###);
        bulk_ao_fc_row_029 => (r#"bulk ao 029"#, r###"print -r $(( 0b1111 ))"###);
        bulk_ao_fc_row_030 => (r#"bulk ao 030"#, r###"print -r $(( 2 ** 3 ** 2 ))"###);
        bulk_ao_fc_row_031 => (r#"bulk ao 031"#, r###"float f=1.5; print -r $(( f * 2 ))"###);
        bulk_ao_fc_row_032 => (r#"bulk ao 032"#, r###"typeset -F2 f=3.14; print -r $f"###);
        bulk_ao_fc_row_033 => (r#"bulk ao 033"#, r###"[[ /etc/hosts -nt /tmp ]]; print -r $?"###);
        bulk_ao_fc_row_034 => (r#"bulk ao 034"#, r###"[[ /tmp -ot /etc/hosts ]]; print -r $?"###);
        bulk_ao_fc_row_035 => (r#"bulk ao 035"#, r###"[[ /etc/hosts -ef /etc/hosts ]]; print -r $?"###);
        bulk_ao_fc_row_036 => (r#"bulk ao 036"#, r####"setopt extendedglob; [[ abc = [a-z]## ]]; print -r $?"####);
        bulk_ao_fc_row_037 => (r#"bulk ao 037"#, r###"print -r ${(L)${(U)m}}; m=aBc"###);
        bulk_ao_fc_row_038 => (r#"bulk ao 038"#, r###"print -r ${#x}; x=hello"###);
        bulk_ao_fc_row_039 => (r#"bulk ao 039"#, r###"print -r ${#a}; a=(a b c)"###);
        bulk_ao_fc_row_040 => (r#"bulk ao 040"#, r###"print -r ${(c)#a}; a=(ab cd)"###);
        bulk_ao_fc_row_041 => (r#"bulk ao 041"#, r###"print -r ${(b)x}; x=hi"###);
        bulk_ao_fc_row_042 => (r#"bulk ao 042"#, r###"print -r ${(e):-2+2}"###);
        bulk_ao_fc_row_043 => (r#"bulk ao 043"#, r###"print -r ${(P)r}; r=HOME"###);
        bulk_ao_fc_row_044 => (r#"bulk ao 044"#, r###"print -r ${(on)n}; n=(10 2 1)"###);
        bulk_ao_fc_row_045 => (r#"bulk ao 045"#, r###"print -r ${(eu)n}; n=(a A b)"###);
        bulk_ao_fc_row_046 => (r#"bulk ao 046"#, r###"typeset -aU u=(a a b); print -r ${#u}"###);
        bulk_ao_fc_row_047 => (r#"bulk ao 047"#, r###"typeset -h hv=1; print -r ${+hv}"###);
        bulk_ao_fc_row_048 => (r#"bulk ao 048"#, r###"x=a1a2; p=a; print -r ${x//p/r}"###);
    }
}

mod corpus_dash_fc_bulk_ap {
    use super::*;

    parity_gap_tests! {
        bulk_ap_fc_row_001 => (r#"bulk ap 001"#, r###"command -v true"###);
        bulk_ap_fc_row_002 => (r#"bulk ap 002"#, r###"emulate -L zsh; print -r $?"###);
        bulk_ap_fc_row_003 => (r#"bulk ap 003"#, r###"alias za='echo z'; za; unalias za 2>/dev/null"###);
        bulk_ap_fc_row_004 => (r#"bulk ap 004"#, r###"read -r line <<< 'one two'; print -r $line"###);
        bulk_ap_fc_row_005 => (r#"bulk ap 005"#, r###"cat <<< 'herestring'"###);
        bulk_ap_fc_row_006 => (r#"bulk ap 006"#, r###"echo hello 2>/dev/null"###);
        bulk_ap_fc_row_007 => (r#"bulk ap 007"#, r###"printf '%s\n' a b c | head -1"###);
        bulk_ap_fc_row_008 => (r#"bulk ap 008"#, r###"true && echo yes"###);
        bulk_ap_fc_row_009 => (r#"bulk ap 009"#, r###"false || echo yes"###);
        bulk_ap_fc_row_010 => (r#"bulk ap 010"#, r###"(exit 3); print -r $?"###);
        bulk_ap_fc_row_011 => (r#"bulk ap 011"#, r###"print -r ${status}; (exit 4)"###);
        bulk_ap_fc_row_012 => (r#"bulk ap 012"#, r###"print -r $(( 1_000 + 1 ))"###);
        bulk_ap_fc_row_013 => (r#"bulk ap 013"#, r###"print -r $(( 5#101 ))"###);
        bulk_ap_fc_row_014 => (r#"bulk ap 014"#, r###"print -r $(( 0b1111 ))"###);
        bulk_ap_fc_row_015 => (r#"bulk ap 015"#, r###"print -r $(( 2 ** 3 ** 2 ))"###);
        bulk_ap_fc_row_016 => (r#"bulk ap 016"#, r###"float f=1.5; print -r $(( f * 2 ))"###);
        bulk_ap_fc_row_017 => (r#"bulk ap 017"#, r###"typeset -F2 f=3.14; print -r $f"###);
        bulk_ap_fc_row_018 => (r#"bulk ap 018"#, r###"[[ /etc/hosts -nt /tmp ]]; print -r $?"###);
        bulk_ap_fc_row_019 => (r#"bulk ap 019"#, r###"[[ /tmp -ot /etc/hosts ]]; print -r $?"###);
        bulk_ap_fc_row_020 => (r#"bulk ap 020"#, r###"[[ /etc/hosts -ef /etc/hosts ]]; print -r $?"###);
        bulk_ap_fc_row_021 => (r#"bulk ap 021"#, r####"setopt extendedglob; [[ abc = [a-z]## ]]; print -r $?"####);
        bulk_ap_fc_row_022 => (r#"bulk ap 022"#, r###"print -r ${(L)${(U)m}}; m=aBc"###);
        bulk_ap_fc_row_023 => (r#"bulk ap 023"#, r###"print -r ${#x}; x=hello"###);
        bulk_ap_fc_row_024 => (r#"bulk ap 024"#, r###"print -r ${#a}; a=(a b c)"###);
        bulk_ap_fc_row_025 => (r#"bulk ap 025"#, r###"print -r ${(c)#a}; a=(ab cd)"###);
        bulk_ap_fc_row_026 => (r#"bulk ap 026"#, r###"print -r ${(b)x}; x=hi"###);
        bulk_ap_fc_row_027 => (r#"bulk ap 027"#, r###"print -r ${(e):-2+2}"###);
        bulk_ap_fc_row_028 => (r#"bulk ap 028"#, r###"print -r ${(P)r}; r=HOME"###);
        bulk_ap_fc_row_029 => (r#"bulk ap 029"#, r###"print -r ${(on)n}; n=(10 2 1)"###);
        bulk_ap_fc_row_030 => (r#"bulk ap 030"#, r###"print -r ${(eu)n}; n=(a A b)"###);
        bulk_ap_fc_row_031 => (r#"bulk ap 031"#, r###"typeset -aU u=(a a b); print -r ${#u}"###);
        bulk_ap_fc_row_032 => (r#"bulk ap 032"#, r###"typeset -h hv=1; print -r ${+hv}"###);
        bulk_ap_fc_row_033 => (r#"bulk ap 033"#, r###"x=a1a2; p=a; print -r ${x//p/r}"###);
        bulk_ap_fc_row_034 => (r#"bulk ap 034"#, r###"for i in 1 2 3; do (( i == 2 )) && continue; print -r $i; done"###);
        bulk_ap_fc_row_035 => (r#"bulk ap 035"#, r###"while :; do break; print -r n; done; print -r after"###);
        bulk_ap_fc_row_036 => (r#"bulk ap 036"#, r###"case w in (a|b) echo ab ;; *) echo star ;; esac"###);
        bulk_ap_fc_row_037 => (r#"bulk ap 037"#, r###"if [[ -n '' ]]; then echo y; else echo n; fi"###);
        bulk_ap_fc_row_038 => (r#"bulk ap 038"#, r###"print -r ${argv[1]}; set -- p q"###);
        bulk_ap_fc_row_039 => (r#"bulk ap 039"#, r###"print -r $ARGC; set -- a b"###);
        bulk_ap_fc_row_040 => (r#"bulk ap 040"#, r###"print -r ${dirstack[1]:-empty}"###);
        bulk_ap_fc_row_041 => (r#"bulk ap 041"#, r###"print -r ${+pipestatus}"###);
        bulk_ap_fc_row_042 => (r#"bulk ap 042"#, r###"print -r ${+history}"###);
        bulk_ap_fc_row_043 => (r#"bulk ap 043"#, r###"print -r ${+commands}"###);
        bulk_ap_fc_row_044 => (r#"bulk ap 044"#, r###"print -r ${+builtins}"###);
        bulk_ap_fc_row_045 => (r#"bulk ap 045"#, r###"print -r ${+widgets}"###);
        bulk_ap_fc_row_046 => (r#"bulk ap 046"#, r###"print -r ${+terminfo}"###);
        bulk_ap_fc_row_047 => (r#"bulk ap 047"#, r###"print -r ${+modules}"###);
        bulk_ap_fc_row_048 => (r#"bulk ap 048"#, r###"print -r ${+patchars}"###);
    }
}

mod corpus_dash_fc_bulk_aq {
    use super::*;

    parity_gap_tests! {
        bulk_aq_fc_row_001 => (r#"bulk aq 001"#, r###"[[ /etc/hosts -nt /tmp ]]; print -r $?"###);
        bulk_aq_fc_row_002 => (r#"bulk aq 002"#, r###"[[ /tmp -ot /etc/hosts ]]; print -r $?"###);
        bulk_aq_fc_row_003 => (r#"bulk aq 003"#, r###"[[ /etc/hosts -ef /etc/hosts ]]; print -r $?"###);
        bulk_aq_fc_row_004 => (r#"bulk aq 004"#, r####"setopt extendedglob; [[ abc = [a-z]## ]]; print -r $?"####);
        bulk_aq_fc_row_005 => (r#"bulk aq 005"#, r###"print -r ${(L)${(U)m}}; m=aBc"###);
        bulk_aq_fc_row_006 => (r#"bulk aq 006"#, r###"print -r ${#x}; x=hello"###);
        bulk_aq_fc_row_007 => (r#"bulk aq 007"#, r###"print -r ${#a}; a=(a b c)"###);
        bulk_aq_fc_row_008 => (r#"bulk aq 008"#, r###"print -r ${(c)#a}; a=(ab cd)"###);
        bulk_aq_fc_row_009 => (r#"bulk aq 009"#, r###"print -r ${(b)x}; x=hi"###);
        bulk_aq_fc_row_010 => (r#"bulk aq 010"#, r###"print -r ${(e):-2+2}"###);
        bulk_aq_fc_row_011 => (r#"bulk aq 011"#, r###"print -r ${(P)r}; r=HOME"###);
        bulk_aq_fc_row_012 => (r#"bulk aq 012"#, r###"print -r ${(on)n}; n=(10 2 1)"###);
        bulk_aq_fc_row_013 => (r#"bulk aq 013"#, r###"print -r ${(eu)n}; n=(a A b)"###);
        bulk_aq_fc_row_014 => (r#"bulk aq 014"#, r###"typeset -aU u=(a a b); print -r ${#u}"###);
        bulk_aq_fc_row_015 => (r#"bulk aq 015"#, r###"typeset -h hv=1; print -r ${+hv}"###);
        bulk_aq_fc_row_016 => (r#"bulk aq 016"#, r###"x=a1a2; p=a; print -r ${x//p/r}"###);
        bulk_aq_fc_row_017 => (r#"bulk aq 017"#, r###"for i in 1 2 3; do (( i == 2 )) && continue; print -r $i; done"###);
        bulk_aq_fc_row_018 => (r#"bulk aq 018"#, r###"while :; do break; print -r n; done; print -r after"###);
        bulk_aq_fc_row_019 => (r#"bulk aq 019"#, r###"case w in (a|b) echo ab ;; *) echo star ;; esac"###);
        bulk_aq_fc_row_020 => (r#"bulk aq 020"#, r###"if [[ -n '' ]]; then echo y; else echo n; fi"###);
        bulk_aq_fc_row_021 => (r#"bulk aq 021"#, r###"print -r ${argv[1]}; set -- p q"###);
        bulk_aq_fc_row_022 => (r#"bulk aq 022"#, r###"print -r $ARGC; set -- a b"###);
        bulk_aq_fc_row_023 => (r#"bulk aq 023"#, r###"print -r ${dirstack[1]:-empty}"###);
        bulk_aq_fc_row_024 => (r#"bulk aq 024"#, r###"print -r ${+pipestatus}"###);
        bulk_aq_fc_row_025 => (r#"bulk aq 025"#, r###"print -r ${+history}"###);
        bulk_aq_fc_row_026 => (r#"bulk aq 026"#, r###"print -r ${+commands}"###);
        bulk_aq_fc_row_027 => (r#"bulk aq 027"#, r###"print -r ${+builtins}"###);
        bulk_aq_fc_row_028 => (r#"bulk aq 028"#, r###"print -r ${+widgets}"###);
        bulk_aq_fc_row_029 => (r#"bulk aq 029"#, r###"print -r ${+terminfo}"###);
        bulk_aq_fc_row_030 => (r#"bulk aq 030"#, r###"print -r ${+modules}"###);
        bulk_aq_fc_row_031 => (r#"bulk aq 031"#, r###"print -r ${+patchars}"###);
        bulk_aq_fc_row_032 => (r#"bulk aq 032"#, r###"print -r ${+reswords}"###);
        bulk_aq_fc_row_033 => (r#"bulk aq 033"#, r###"print -r ${+dis_aliases}"###);
        bulk_aq_fc_row_034 => (r#"bulk aq 034"#, r###"print -r ${+dis_functions}"###);
        bulk_aq_fc_row_035 => (r#"bulk aq 035"#, r###"print -r ${+parameters[(I)PATH]}"###);
        bulk_aq_fc_row_036 => (r#"bulk aq 036"#, r###"arr=(a b c d); print -r ${arr[2,3]}"###);
        bulk_aq_fc_row_037 => (r#"bulk aq 037"#, r###"arr=(1 2 3); print -r ${arr[1,-1]}"###);
        bulk_aq_fc_row_038 => (r#"bulk aq 038"#, r###"s=barfooxyz; print -r ${s[(i)foo]}"###);
        bulk_aq_fc_row_039 => (r#"bulk aq 039"#, r###"typeset -A h; h=(k v); print -r ${h[(R)v]}"###);
        bulk_aq_fc_row_040 => (r#"bulk aq 040"#, r###"typeset -A h; h=(a 1 b 2); print -r ${h[(r)2]}"###);
        bulk_aq_fc_row_041 => (r#"bulk aq 041"#, r###"print -r $(( 9 & 6 ^ 3 ))"###);
        bulk_aq_fc_row_042 => (r#"bulk aq 042"#, r###"print -r $(( 128 >> 2 ))"###);
        bulk_aq_fc_row_043 => (r#"bulk aq 043"#, r###"print -r $(( ~(255) & 0xff ))"###);
        bulk_aq_fc_row_044 => (r#"bulk aq 044"#, r###"print -r $(( 3 <|> 5 ))"###);
        bulk_aq_fc_row_045 => (r#"bulk aq 045"#, r###"print -r $(( 3 <> 5 ))"###);
        bulk_aq_fc_row_046 => (r#"bulk aq 046"#, r###"(( 5#11 )); print -r $?"###);
        bulk_aq_fc_row_047 => (r#"bulk aq 047"#, r###"integer n=5; (( n ^= 3 )); print -r $n"###);
        bulk_aq_fc_row_048 => (r#"bulk aq 048"#, r###"integer n=5; (( n <<= 1 )); print -r $n"###);
    }
}

mod corpus_dash_fc_bulk_ar {
    use super::*;

    parity_gap_tests! {
        bulk_ar_fc_row_001 => (r#"bulk ar 001"#, r###"for i in 1 2 3; do (( i == 2 )) && continue; print -r $i; done"###);
        bulk_ar_fc_row_002 => (r#"bulk ar 002"#, r###"while :; do break; print -r n; done; print -r after"###);
        bulk_ar_fc_row_003 => (r#"bulk ar 003"#, r###"case w in (a|b) echo ab ;; *) echo star ;; esac"###);
        bulk_ar_fc_row_004 => (r#"bulk ar 004"#, r###"if [[ -n '' ]]; then echo y; else echo n; fi"###);
        bulk_ar_fc_row_005 => (r#"bulk ar 005"#, r###"print -r ${argv[1]}; set -- p q"###);
        bulk_ar_fc_row_006 => (r#"bulk ar 006"#, r###"print -r $ARGC; set -- a b"###);
        bulk_ar_fc_row_007 => (r#"bulk ar 007"#, r###"print -r ${dirstack[1]:-empty}"###);
        bulk_ar_fc_row_008 => (r#"bulk ar 008"#, r###"print -r ${+pipestatus}"###);
        bulk_ar_fc_row_009 => (r#"bulk ar 009"#, r###"print -r ${+history}"###);
        bulk_ar_fc_row_010 => (r#"bulk ar 010"#, r###"print -r ${+commands}"###);
        bulk_ar_fc_row_011 => (r#"bulk ar 011"#, r###"print -r ${+builtins}"###);
        bulk_ar_fc_row_012 => (r#"bulk ar 012"#, r###"print -r ${+widgets}"###);
        bulk_ar_fc_row_013 => (r#"bulk ar 013"#, r###"print -r ${+terminfo}"###);
        bulk_ar_fc_row_014 => (r#"bulk ar 014"#, r###"print -r ${+modules}"###);
        bulk_ar_fc_row_015 => (r#"bulk ar 015"#, r###"print -r ${+patchars}"###);
        bulk_ar_fc_row_016 => (r#"bulk ar 016"#, r###"print -r ${+reswords}"###);
        bulk_ar_fc_row_017 => (r#"bulk ar 017"#, r###"print -r ${+dis_aliases}"###);
        bulk_ar_fc_row_018 => (r#"bulk ar 018"#, r###"print -r ${+dis_functions}"###);
        bulk_ar_fc_row_019 => (r#"bulk ar 019"#, r###"print -r ${+parameters[(I)PATH]}"###);
        bulk_ar_fc_row_020 => (r#"bulk ar 020"#, r###"arr=(a b c d); print -r ${arr[2,3]}"###);
        bulk_ar_fc_row_021 => (r#"bulk ar 021"#, r###"arr=(1 2 3); print -r ${arr[1,-1]}"###);
        bulk_ar_fc_row_022 => (r#"bulk ar 022"#, r###"s=barfooxyz; print -r ${s[(i)foo]}"###);
        bulk_ar_fc_row_023 => (r#"bulk ar 023"#, r###"typeset -A h; h=(k v); print -r ${h[(R)v]}"###);
        bulk_ar_fc_row_024 => (r#"bulk ar 024"#, r###"typeset -A h; h=(a 1 b 2); print -r ${h[(r)2]}"###);
        bulk_ar_fc_row_025 => (r#"bulk ar 025"#, r###"print -r $(( 9 & 6 ^ 3 ))"###);
        bulk_ar_fc_row_026 => (r#"bulk ar 026"#, r###"print -r $(( 128 >> 2 ))"###);
        bulk_ar_fc_row_027 => (r#"bulk ar 027"#, r###"print -r $(( ~(255) & 0xff ))"###);
        bulk_ar_fc_row_028 => (r#"bulk ar 028"#, r###"print -r $(( 3 <|> 5 ))"###);
        bulk_ar_fc_row_029 => (r#"bulk ar 029"#, r###"print -r $(( 3 <> 5 ))"###);
        bulk_ar_fc_row_030 => (r#"bulk ar 030"#, r###"(( 5#11 )); print -r $?"###);
        bulk_ar_fc_row_031 => (r#"bulk ar 031"#, r###"integer n=5; (( n ^= 3 )); print -r $n"###);
        bulk_ar_fc_row_032 => (r#"bulk ar 032"#, r###"integer n=5; (( n <<= 1 )); print -r $n"###);
        bulk_ar_fc_row_033 => (r#"bulk ar 033"#, r###"integer n=5; (( n >>= 1 )); print -r $n"###);
        bulk_ar_fc_row_034 => (r#"bulk ar 034"#, r###"integer n=5; (( n /= 2 )); print -r $n"###);
        bulk_ar_fc_row_035 => (r#"bulk ar 035"#, r###"integer n=5; (( n %= 3 )); print -r $n"###);
        bulk_ar_fc_row_036 => (r#"bulk ar 036"#, r###"typeset -i8 n=10; print -r $n"###);
        bulk_ar_fc_row_037 => (r#"bulk ar 037"#, r###"typeset -i16 n=255; print -r $n"###);
        bulk_ar_fc_row_038 => (r#"bulk ar 038"#, r###"typeset -E2 n=4000; print -r $n"###);
        bulk_ar_fc_row_039 => (r#"bulk ar 039"#, r###"typeset -R4 n=hi; print -r $n"###);
        bulk_ar_fc_row_040 => (r#"bulk ar 040"#, r###"typeset +L n=Ab; print -r $n"###);
        bulk_ar_fc_row_041 => (r#"bulk ar 041"#, r###"typeset +U n=xy; print -r $n"###);
        bulk_ar_fc_row_042 => (r#"bulk ar 042"#, r###"typeset +i n=4; print -r $n"###);
        bulk_ar_fc_row_043 => (r#"bulk ar 043"#, r###"export EX=1; print -r $EX; unset EX"###);
        bulk_ar_fc_row_044 => (r#"bulk ar 044"#, r###"readonly ro=5; print -r $ro"###);
        bulk_ar_fc_row_045 => (r#"bulk ar 045"#, r###"print -r ${${v:-fb}}; unset v"###);
        bulk_ar_fc_row_046 => (r#"bulk ar 046"#, r###"print -r ${${v:+set}:-unset}; unset v"###);
        bulk_ar_fc_row_047 => (r#"bulk ar 047"#, r###"word=$'l1\nl2'; print -r ${(@f)word}"###);
        bulk_ar_fc_row_048 => (r#"bulk ar 048"#, r###"word=  hi  ; print -r ${(W)word}"###);
    }
}

mod corpus_dash_fc_bulk_as {
    use super::*;

    parity_gap_tests! {
        bulk_as_fc_row_001 => (r#"bulk as 001"#, r###"print -r ${+dis_functions}"###);
        bulk_as_fc_row_002 => (r#"bulk as 002"#, r###"print -r ${+parameters[(I)PATH]}"###);
        bulk_as_fc_row_003 => (r#"bulk as 003"#, r###"arr=(a b c d); print -r ${arr[2,3]}"###);
        bulk_as_fc_row_004 => (r#"bulk as 004"#, r###"arr=(1 2 3); print -r ${arr[1,-1]}"###);
        bulk_as_fc_row_005 => (r#"bulk as 005"#, r###"s=barfooxyz; print -r ${s[(i)foo]}"###);
        bulk_as_fc_row_006 => (r#"bulk as 006"#, r###"typeset -A h; h=(k v); print -r ${h[(R)v]}"###);
        bulk_as_fc_row_007 => (r#"bulk as 007"#, r###"typeset -A h; h=(a 1 b 2); print -r ${h[(r)2]}"###);
        bulk_as_fc_row_008 => (r#"bulk as 008"#, r###"print -r $(( 9 & 6 ^ 3 ))"###);
        bulk_as_fc_row_009 => (r#"bulk as 009"#, r###"print -r $(( 128 >> 2 ))"###);
        bulk_as_fc_row_010 => (r#"bulk as 010"#, r###"print -r $(( ~(255) & 0xff ))"###);
        bulk_as_fc_row_011 => (r#"bulk as 011"#, r###"print -r $(( 3 <|> 5 ))"###);
        bulk_as_fc_row_012 => (r#"bulk as 012"#, r###"print -r $(( 3 <> 5 ))"###);
        bulk_as_fc_row_013 => (r#"bulk as 013"#, r###"(( 5#11 )); print -r $?"###);
        bulk_as_fc_row_014 => (r#"bulk as 014"#, r###"integer n=5; (( n ^= 3 )); print -r $n"###);
        bulk_as_fc_row_015 => (r#"bulk as 015"#, r###"integer n=5; (( n <<= 1 )); print -r $n"###);
        bulk_as_fc_row_016 => (r#"bulk as 016"#, r###"integer n=5; (( n >>= 1 )); print -r $n"###);
        bulk_as_fc_row_017 => (r#"bulk as 017"#, r###"integer n=5; (( n /= 2 )); print -r $n"###);
        bulk_as_fc_row_018 => (r#"bulk as 018"#, r###"integer n=5; (( n %= 3 )); print -r $n"###);
        bulk_as_fc_row_019 => (r#"bulk as 019"#, r###"typeset -i8 n=10; print -r $n"###);
        bulk_as_fc_row_020 => (r#"bulk as 020"#, r###"typeset -i16 n=255; print -r $n"###);
        bulk_as_fc_row_021 => (r#"bulk as 021"#, r###"typeset -E2 n=4000; print -r $n"###);
        bulk_as_fc_row_022 => (r#"bulk as 022"#, r###"typeset -R4 n=hi; print -r $n"###);
        bulk_as_fc_row_023 => (r#"bulk as 023"#, r###"typeset +L n=Ab; print -r $n"###);
        bulk_as_fc_row_024 => (r#"bulk as 024"#, r###"typeset +U n=xy; print -r $n"###);
        bulk_as_fc_row_025 => (r#"bulk as 025"#, r###"typeset +i n=4; print -r $n"###);
        bulk_as_fc_row_026 => (r#"bulk as 026"#, r###"export EX=1; print -r $EX; unset EX"###);
        bulk_as_fc_row_027 => (r#"bulk as 027"#, r###"readonly ro=5; print -r $ro"###);
        bulk_as_fc_row_028 => (r#"bulk as 028"#, r###"print -r ${${v:-fb}}; unset v"###);
        bulk_as_fc_row_029 => (r#"bulk as 029"#, r###"print -r ${${v:+set}:-unset}; unset v"###);
        bulk_as_fc_row_030 => (r#"bulk as 030"#, r###"word=$'l1\nl2'; print -r ${(@f)word}"###);
        bulk_as_fc_row_031 => (r#"bulk as 031"#, r###"word=  hi  ; print -r ${(W)word}"###);
        bulk_as_fc_row_032 => (r#"bulk as 032"#, r###"print -r ${(z)word}; word=a b c"###);
        bulk_as_fc_row_033 => (r#"bulk as 033"#, r###"print -r ${(F)x}; x=$'p\nq'"###);
        bulk_as_fc_row_034 => (r#"bulk as 034"#, r###"print -r ${(A)x}; x=1 2"###);
        bulk_as_fc_row_035 => (r#"bulk as 035"#, r###"print -r ${(aa)x}; x=(1 2)"###);
        bulk_as_fc_row_036 => (r#"bulk as 036"#, r###"print -r ${(%)2}"###);
        bulk_as_fc_row_037 => (r#"bulk as 037"#, r###"o=8; print -r ${(0)o}"###);
        bulk_as_fc_row_038 => (r#"bulk as 038"#, r###"str=abc.def; print -r ${str:r}"###);
        bulk_as_fc_row_039 => (r#"bulk as 039"#, r###"str=abc.def; print -r ${str:e}"###);
        bulk_as_fc_row_040 => (r#"bulk as 040"#, r###"[[ -h /dev/stdin ]]; print -r $?"###);
        bulk_as_fc_row_041 => (r#"bulk as 041"#, r###"[[ -p /dev/fd/0 ]]; print -r $?"###);
        bulk_as_fc_row_042 => (r#"bulk as 042"#, r###"[[ -O /etc/hosts ]]; print -r $?"###);
        bulk_as_fc_row_043 => (r#"bulk as 043"#, r###"[[ -G / ]]; print -r $?"###);
        bulk_as_fc_row_044 => (r#"bulk as 044"#, r###"[[ -a /etc/hosts ]]; print -r $?"###);
        bulk_as_fc_row_045 => (r#"bulk as 045"#, r###"[[ bee = *ee* ]]; print -r $?"###);
        bulk_as_fc_row_046 => (r#"bulk as 046"#, r###"[[ 1 -eq 1 ]]; print -r $?"###);
        bulk_as_fc_row_047 => (r#"bulk as 047"#, r###"[[ 1 -ne 2 ]]; print -r $?"###);
        bulk_as_fc_row_048 => (r#"bulk as 048"#, r###"[[ 3 -lt 5 ]]; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_at {
    use super::*;

    parity_gap_tests! {
        bulk_at_fc_row_001 => (r#"bulk at 001"#, r###"integer n=5; (( n >>= 1 )); print -r $n"###);
        bulk_at_fc_row_002 => (r#"bulk at 002"#, r###"integer n=5; (( n /= 2 )); print -r $n"###);
        bulk_at_fc_row_003 => (r#"bulk at 003"#, r###"integer n=5; (( n %= 3 )); print -r $n"###);
        bulk_at_fc_row_004 => (r#"bulk at 004"#, r###"typeset -i8 n=10; print -r $n"###);
        bulk_at_fc_row_005 => (r#"bulk at 005"#, r###"typeset -i16 n=255; print -r $n"###);
        bulk_at_fc_row_006 => (r#"bulk at 006"#, r###"typeset -E2 n=4000; print -r $n"###);
        bulk_at_fc_row_007 => (r#"bulk at 007"#, r###"typeset -R4 n=hi; print -r $n"###);
        bulk_at_fc_row_008 => (r#"bulk at 008"#, r###"typeset +L n=Ab; print -r $n"###);
        bulk_at_fc_row_009 => (r#"bulk at 009"#, r###"typeset +U n=xy; print -r $n"###);
        bulk_at_fc_row_010 => (r#"bulk at 010"#, r###"typeset +i n=4; print -r $n"###);
        bulk_at_fc_row_011 => (r#"bulk at 011"#, r###"export EX=1; print -r $EX; unset EX"###);
        bulk_at_fc_row_012 => (r#"bulk at 012"#, r###"readonly ro=5; print -r $ro"###);
        bulk_at_fc_row_013 => (r#"bulk at 013"#, r###"print -r ${${v:-fb}}; unset v"###);
        bulk_at_fc_row_014 => (r#"bulk at 014"#, r###"print -r ${${v:+set}:-unset}; unset v"###);
        bulk_at_fc_row_015 => (r#"bulk at 015"#, r###"word=$'l1\nl2'; print -r ${(@f)word}"###);
        bulk_at_fc_row_016 => (r#"bulk at 016"#, r###"word=  hi  ; print -r ${(W)word}"###);
        bulk_at_fc_row_017 => (r#"bulk at 017"#, r###"print -r ${(z)word}; word=a b c"###);
        bulk_at_fc_row_018 => (r#"bulk at 018"#, r###"print -r ${(F)x}; x=$'p\nq'"###);
        bulk_at_fc_row_019 => (r#"bulk at 019"#, r###"print -r ${(A)x}; x=1 2"###);
        bulk_at_fc_row_020 => (r#"bulk at 020"#, r###"print -r ${(aa)x}; x=(1 2)"###);
        bulk_at_fc_row_021 => (r#"bulk at 021"#, r###"print -r ${(%)2}"###);
        bulk_at_fc_row_022 => (r#"bulk at 022"#, r###"o=8; print -r ${(0)o}"###);
        bulk_at_fc_row_023 => (r#"bulk at 023"#, r###"str=abc.def; print -r ${str:r}"###);
        bulk_at_fc_row_024 => (r#"bulk at 024"#, r###"str=abc.def; print -r ${str:e}"###);
        bulk_at_fc_row_025 => (r#"bulk at 025"#, r###"[[ -h /dev/stdin ]]; print -r $?"###);
        bulk_at_fc_row_026 => (r#"bulk at 026"#, r###"[[ -p /dev/fd/0 ]]; print -r $?"###);
        bulk_at_fc_row_027 => (r#"bulk at 027"#, r###"[[ -O /etc/hosts ]]; print -r $?"###);
        bulk_at_fc_row_028 => (r#"bulk at 028"#, r###"[[ -G / ]]; print -r $?"###);
        bulk_at_fc_row_029 => (r#"bulk at 029"#, r###"[[ -a /etc/hosts ]]; print -r $?"###);
        bulk_at_fc_row_030 => (r#"bulk at 030"#, r###"[[ bee = *ee* ]]; print -r $?"###);
        bulk_at_fc_row_031 => (r#"bulk at 031"#, r###"[[ 1 -eq 1 ]]; print -r $?"###);
        bulk_at_fc_row_032 => (r#"bulk at 032"#, r###"[[ 1 -ne 2 ]]; print -r $?"###);
        bulk_at_fc_row_033 => (r#"bulk at 033"#, r###"[[ 3 -lt 5 ]]; print -r $?"###);
        bulk_at_fc_row_034 => (r#"bulk at 034"#, r###"[[ 5 -le 5 ]]; print -r $?"###);
        bulk_at_fc_row_035 => (r#"bulk at 035"#, r###"[[ 5 -gt 3 ]]; print -r $?"###);
        bulk_at_fc_row_036 => (r#"bulk at 036"#, r###"[[ 5 -ge 5 ]]; print -r $?"###);
        bulk_at_fc_row_037 => (r#"bulk at 037"#, r###"[[ -o nullglob ]]; print -r $?"###);
        bulk_at_fc_row_038 => (r#"bulk at 038"#, r###"unsetopt extendedglob 2>/dev/null; [[ -o extendedglob ]]; print -r $?"###);
        bulk_at_fc_row_039 => (r#"bulk at 039"#, r###"setopt extendedglob; [[ -o extendedglob ]]; print -r $?"###);
        bulk_at_fc_row_040 => (r#"bulk at 040"#, r###"[[ -o no_extendedglob ]]; print -r $?"###);
        bulk_at_fc_row_041 => (r#"bulk at 041"#, r###"print -r $(( 1 , 2 , 3 ))"###);
        bulk_at_fc_row_042 => (r#"bulk at 042"#, r###"print -r $(( 3 < 5 ? 1 : 0 ))"###);
        bulk_at_fc_row_043 => (r#"bulk at 043"#, r###"print -r $(( 0xff & 0x0f ))"###);
        bulk_at_fc_row_044 => (r#"bulk at 044"#, r###"print -r $(( 1 << 4 ))"###);
        bulk_at_fc_row_045 => (r#"bulk at 045"#, r###"print -r $(( 16 >> 2 ))"###);
        bulk_at_fc_row_046 => (r#"bulk at 046"#, r###"print -r $(( -1 >> 1 ))"###);
        bulk_at_fc_row_047 => (r#"bulk at 047"#, r###"print -r $(( 8#17 ))"###);
        bulk_at_fc_row_048 => (r#"bulk at 048"#, r###"print -r $(( 16#ff ))"###);
    }
}

mod corpus_dash_fc_bulk_au {
    use super::*;

    parity_gap_tests! {
        bulk_au_fc_row_001 => (r#"bulk au 001"#, r###"print -r ${(F)x}; x=$'p\nq'"###);
        bulk_au_fc_row_002 => (r#"bulk au 002"#, r###"print -r ${(A)x}; x=1 2"###);
        bulk_au_fc_row_003 => (r#"bulk au 003"#, r###"print -r ${(aa)x}; x=(1 2)"###);
        bulk_au_fc_row_004 => (r#"bulk au 004"#, r###"print -r ${(%)2}"###);
        bulk_au_fc_row_005 => (r#"bulk au 005"#, r###"o=8; print -r ${(0)o}"###);
        bulk_au_fc_row_006 => (r#"bulk au 006"#, r###"str=abc.def; print -r ${str:r}"###);
        bulk_au_fc_row_007 => (r#"bulk au 007"#, r###"str=abc.def; print -r ${str:e}"###);
        bulk_au_fc_row_008 => (r#"bulk au 008"#, r###"[[ -h /dev/stdin ]]; print -r $?"###);
        bulk_au_fc_row_009 => (r#"bulk au 009"#, r###"[[ -p /dev/fd/0 ]]; print -r $?"###);
        bulk_au_fc_row_010 => (r#"bulk au 010"#, r###"[[ -O /etc/hosts ]]; print -r $?"###);
        bulk_au_fc_row_011 => (r#"bulk au 011"#, r###"[[ -G / ]]; print -r $?"###);
        bulk_au_fc_row_012 => (r#"bulk au 012"#, r###"[[ -a /etc/hosts ]]; print -r $?"###);
        bulk_au_fc_row_013 => (r#"bulk au 013"#, r###"[[ bee = *ee* ]]; print -r $?"###);
        bulk_au_fc_row_014 => (r#"bulk au 014"#, r###"[[ 1 -eq 1 ]]; print -r $?"###);
        bulk_au_fc_row_015 => (r#"bulk au 015"#, r###"[[ 1 -ne 2 ]]; print -r $?"###);
        bulk_au_fc_row_016 => (r#"bulk au 016"#, r###"[[ 3 -lt 5 ]]; print -r $?"###);
        bulk_au_fc_row_017 => (r#"bulk au 017"#, r###"[[ 5 -le 5 ]]; print -r $?"###);
        bulk_au_fc_row_018 => (r#"bulk au 018"#, r###"[[ 5 -gt 3 ]]; print -r $?"###);
        bulk_au_fc_row_019 => (r#"bulk au 019"#, r###"[[ 5 -ge 5 ]]; print -r $?"###);
        bulk_au_fc_row_020 => (r#"bulk au 020"#, r###"[[ -o nullglob ]]; print -r $?"###);
        bulk_au_fc_row_021 => (r#"bulk au 021"#, r###"unsetopt extendedglob 2>/dev/null; [[ -o extendedglob ]]; print -r $?"###);
        bulk_au_fc_row_022 => (r#"bulk au 022"#, r###"setopt extendedglob; [[ -o extendedglob ]]; print -r $?"###);
        bulk_au_fc_row_023 => (r#"bulk au 023"#, r###"[[ -o no_extendedglob ]]; print -r $?"###);
        bulk_au_fc_row_024 => (r#"bulk au 024"#, r###"print -r $(( 1 , 2 , 3 ))"###);
        bulk_au_fc_row_025 => (r#"bulk au 025"#, r###"print -r $(( 3 < 5 ? 1 : 0 ))"###);
        bulk_au_fc_row_026 => (r#"bulk au 026"#, r###"print -r $(( 0xff & 0x0f ))"###);
        bulk_au_fc_row_027 => (r#"bulk au 027"#, r###"print -r $(( 1 << 4 ))"###);
        bulk_au_fc_row_028 => (r#"bulk au 028"#, r###"print -r $(( 16 >> 2 ))"###);
        bulk_au_fc_row_029 => (r#"bulk au 029"#, r###"print -r $(( -1 >> 1 ))"###);
        bulk_au_fc_row_030 => (r#"bulk au 030"#, r###"print -r $(( 8#17 ))"###);
        bulk_au_fc_row_031 => (r#"bulk au 031"#, r###"print -r $(( 16#ff ))"###);
        bulk_au_fc_row_032 => (r#"bulk au 032"#, r###"print -r $(( 2#1010 ))"###);
        bulk_au_fc_row_033 => (r#"bulk au 033"#, r###"print -r $(( 0b1010 ))"###);
        bulk_au_fc_row_034 => (r#"bulk au 034"#, r###"typeset -F1 c=1.05; print -r $(( c > 1 ))"###);
        bulk_au_fc_row_035 => (r#"bulk au 035"#, r###"print -r $(( 4 % 2 == 0 ))"###);
        bulk_au_fc_row_036 => (r#"bulk au 036"#, r###"print -r $(( 0 - 1 == -1 ))"###);
        bulk_au_fc_row_037 => (r#"bulk au 037"#, r###"print -r $(( 72 / 8 / 3 ))"###);
        bulk_au_fc_row_038 => (r#"bulk au 038"#, r###"print -r $(( 24 % 5 % 3 ))"###);
        bulk_au_fc_row_039 => (r#"bulk au 039"#, r###"print -r $(( 2 | 4 | 8 ))"###);
        bulk_au_fc_row_040 => (r#"bulk au 040"#, r###"print -r $(( 15 ^ 9 ))"###);
        bulk_au_fc_row_041 => (r#"bulk au 041"#, r###"print -r $(( 0 || 0 || 7 ))"###);
        bulk_au_fc_row_042 => (r#"bulk au 042"#, r###"print -r $(( 1 || -1 ))"###);
        bulk_au_fc_row_043 => (r#"bulk au 043"#, r###"print -r $(( (1>0) + (0>0) ))"###);
        bulk_au_fc_row_044 => (r#"bulk au 044"#, r###"print -r $(( 3 > 2 > 1 ))"###);
        bulk_au_fc_row_045 => (r#"bulk au 045"#, r###"print -r $(( (9>8)>>(1<0) ))"###);
        bulk_au_fc_row_046 => (r#"bulk au 046"#, r###"print -r $(( 5 ** 2 % 7 ))"###);
        bulk_au_fc_row_047 => (r#"bulk au 047"#, r###"print -r $(( 11 ** 2 % 50 ))"###);
        bulk_au_fc_row_048 => (r#"bulk au 048"#, r###"print -r $(( 100 / 20 / 5 ))"###);
    }
}
mod corpus_dash_fc_bulk_av {
    use super::*;

    parity_gap_tests! {
        bulk_av_fc_row_001 => (r#"bulk av 001"#, r###"[[ 5 -gt 3 ]]; print -r $?"###);
        bulk_av_fc_row_002 => (r#"bulk av 002"#, r###"[[ 5 -ge 5 ]]; print -r $?"###);
        bulk_av_fc_row_003 => (r#"bulk av 003"#, r###"[[ -o nullglob ]]; print -r $?"###);
        bulk_av_fc_row_004 => (r#"bulk av 004"#, r###"unsetopt extendedglob 2>/dev/null; [[ -o extendedglob ]]; print -r $?"###);
        bulk_av_fc_row_005 => (r#"bulk av 005"#, r###"setopt extendedglob; [[ -o extendedglob ]]; print -r $?"###);
        bulk_av_fc_row_006 => (r#"bulk av 006"#, r###"[[ -o no_extendedglob ]]; print -r $?"###);
        bulk_av_fc_row_007 => (r#"bulk av 007"#, r###"print -r $(( 1 , 2 , 3 ))"###);
        bulk_av_fc_row_008 => (r#"bulk av 008"#, r###"print -r $(( 3 < 5 ? 1 : 0 ))"###);
        bulk_av_fc_row_009 => (r#"bulk av 009"#, r###"print -r $(( 0xff & 0x0f ))"###);
        bulk_av_fc_row_010 => (r#"bulk av 010"#, r###"print -r $(( 1 << 4 ))"###);
        bulk_av_fc_row_011 => (r#"bulk av 011"#, r###"print -r $(( 16 >> 2 ))"###);
        bulk_av_fc_row_012 => (r#"bulk av 012"#, r###"print -r $(( -1 >> 1 ))"###);
        bulk_av_fc_row_013 => (r#"bulk av 013"#, r###"print -r $(( 8#17 ))"###);
        bulk_av_fc_row_014 => (r#"bulk av 014"#, r###"print -r $(( 16#ff ))"###);
        bulk_av_fc_row_015 => (r#"bulk av 015"#, r###"print -r $(( 2#1010 ))"###);
        bulk_av_fc_row_016 => (r#"bulk av 016"#, r###"print -r $(( 0b1010 ))"###);
        bulk_av_fc_row_017 => (r#"bulk av 017"#, r###"typeset -F1 c=1.05; print -r $(( c > 1 ))"###);
        bulk_av_fc_row_018 => (r#"bulk av 018"#, r###"print -r $(( 4 % 2 == 0 ))"###);
        bulk_av_fc_row_019 => (r#"bulk av 019"#, r###"print -r $(( 0 - 1 == -1 ))"###);
        bulk_av_fc_row_020 => (r#"bulk av 020"#, r###"print -r $(( 72 / 8 / 3 ))"###);
        bulk_av_fc_row_021 => (r#"bulk av 021"#, r###"print -r $(( 24 % 5 % 3 ))"###);
        bulk_av_fc_row_022 => (r#"bulk av 022"#, r###"print -r $(( 2 | 4 | 8 ))"###);
        bulk_av_fc_row_023 => (r#"bulk av 023"#, r###"print -r $(( 15 ^ 9 ))"###);
        bulk_av_fc_row_024 => (r#"bulk av 024"#, r###"print -r $(( 0 || 0 || 7 ))"###);
        bulk_av_fc_row_025 => (r#"bulk av 025"#, r###"print -r $(( 1 || -1 ))"###);
        bulk_av_fc_row_026 => (r#"bulk av 026"#, r###"print -r $(( (1>0) + (0>0) ))"###);
        bulk_av_fc_row_027 => (r#"bulk av 027"#, r###"print -r $(( 3 > 2 > 1 ))"###);
        bulk_av_fc_row_028 => (r#"bulk av 028"#, r###"print -r $(( (9>8)>>(1<0) ))"###);
        bulk_av_fc_row_029 => (r#"bulk av 029"#, r###"print -r $(( 5 ** 2 % 7 ))"###);
        bulk_av_fc_row_030 => (r#"bulk av 030"#, r###"print -r $(( 11 ** 2 % 50 ))"###);
        bulk_av_fc_row_031 => (r#"bulk av 031"#, r###"print -r $(( 100 / 20 / 5 ))"###);
        bulk_av_fc_row_032 => (r#"bulk av 032"#, r###"print -r $(( 2#101 & 2#010 ))"###);
        bulk_av_fc_row_033 => (r#"bulk av 033"#, r###"print -r $(( 0x80 >> 4 ))"###);
        bulk_av_fc_row_034 => (r#"bulk av 034"#, r###"print -r $(( 5 ** 0 ** 3 ))"###);
        bulk_av_fc_row_035 => (r#"bulk av 035"#, r###"print -r $(( -(-(-5)) ))"###);
        bulk_av_fc_row_036 => (r#"bulk av 036"#, r###"print -r $(( (1+2)*(3+4) ))"###);
        bulk_av_fc_row_037 => (r#"bulk av 037"#, r###"v1=v1; [[ v1 -ef v1 ]]; print -r $?"###);
        bulk_av_fc_row_038 => (r#"bulk av 038"#, r###"[[ "" != x ]]; print -r $?"###);
        bulk_av_fc_row_039 => (r#"bulk av 039"#, r###"[[ -n /dev/null ]]; print -r $?"###);
        bulk_av_fc_row_040 => (r#"bulk av 040"#, r###"setopt extendedglob; [[ mix = [[:digit:]]# ]]; print -r $?"###);
        bulk_av_fc_row_041 => (r#"bulk av 041"#, r####"setopt extendedglob; [[ tag = (#m)[a-z]##_t ]]; print -r $?"####);
        bulk_av_fc_row_042 => (r#"bulk av 042"#, r###"setopt extendedglob; [[ foo = fo(#e) ]]; print -r $?"###);
        bulk_av_fc_row_043 => (r#"bulk av 043"#, r###"setopt extendedglob; [[ foo = (#s)fo ]]; print -r $?"###);
        bulk_av_fc_row_044 => (r#"bulk av 044"#, r###"[[ abc < abd ]]; print -r $?"###);
        bulk_av_fc_row_045 => (r#"bulk av 045"#, r###"[[ abc > abb ]]; print -r $?"###);
        bulk_av_fc_row_046 => (r#"bulk av 046"#, r###"[[ abc != def ]]; print -r $?"###);
        bulk_av_fc_row_047 => (r#"bulk av 047"#, r###"[[ abc == abc ]]; print -r $?"###);
        bulk_av_fc_row_048 => (r#"bulk av 048"#, r###"print -r ${(L)@}; set -- MIXED"###);
    }
}

mod corpus_dash_fc_bulk_aw {
    use super::*;

    parity_gap_tests! {
        bulk_aw_fc_row_001 => (r#"bulk aw 001"#, r###"print -r $(( 4 % 2 == 0 ))"###);
        bulk_aw_fc_row_002 => (r#"bulk aw 002"#, r###"print -r $(( 0 - 1 == -1 ))"###);
        bulk_aw_fc_row_003 => (r#"bulk aw 003"#, r###"print -r $(( 72 / 8 / 3 ))"###);
        bulk_aw_fc_row_004 => (r#"bulk aw 004"#, r###"print -r $(( 24 % 5 % 3 ))"###);
        bulk_aw_fc_row_005 => (r#"bulk aw 005"#, r###"print -r $(( 2 | 4 | 8 ))"###);
        bulk_aw_fc_row_006 => (r#"bulk aw 006"#, r###"print -r $(( 15 ^ 9 ))"###);
        bulk_aw_fc_row_007 => (r#"bulk aw 007"#, r###"print -r $(( 0 || 0 || 7 ))"###);
        bulk_aw_fc_row_008 => (r#"bulk aw 008"#, r###"print -r $(( 1 || -1 ))"###);
        bulk_aw_fc_row_009 => (r#"bulk aw 009"#, r###"print -r $(( (1>0) + (0>0) ))"###);
        bulk_aw_fc_row_010 => (r#"bulk aw 010"#, r###"print -r $(( 3 > 2 > 1 ))"###);
        bulk_aw_fc_row_011 => (r#"bulk aw 011"#, r###"print -r $(( (9>8)>>(1<0) ))"###);
        bulk_aw_fc_row_012 => (r#"bulk aw 012"#, r###"print -r $(( 5 ** 2 % 7 ))"###);
        bulk_aw_fc_row_013 => (r#"bulk aw 013"#, r###"print -r $(( 11 ** 2 % 50 ))"###);
        bulk_aw_fc_row_014 => (r#"bulk aw 014"#, r###"print -r $(( 100 / 20 / 5 ))"###);
        bulk_aw_fc_row_015 => (r#"bulk aw 015"#, r###"print -r $(( 2#101 & 2#010 ))"###);
        bulk_aw_fc_row_016 => (r#"bulk aw 016"#, r###"print -r $(( 0x80 >> 4 ))"###);
        bulk_aw_fc_row_017 => (r#"bulk aw 017"#, r###"print -r $(( 5 ** 0 ** 3 ))"###);
        bulk_aw_fc_row_018 => (r#"bulk aw 018"#, r###"print -r $(( -(-(-5)) ))"###);
        bulk_aw_fc_row_019 => (r#"bulk aw 019"#, r###"print -r $(( (1+2)*(3+4) ))"###);
        bulk_aw_fc_row_020 => (r#"bulk aw 020"#, r###"v1=v1; [[ v1 -ef v1 ]]; print -r $?"###);
        bulk_aw_fc_row_021 => (r#"bulk aw 021"#, r###"[[ "" != x ]]; print -r $?"###);
        bulk_aw_fc_row_022 => (r#"bulk aw 022"#, r###"[[ -n /dev/null ]]; print -r $?"###);
        bulk_aw_fc_row_023 => (r#"bulk aw 023"#, r###"setopt extendedglob; [[ mix = [[:digit:]]# ]]; print -r $?"###);
        bulk_aw_fc_row_024 => (r#"bulk aw 024"#, r####"setopt extendedglob; [[ tag = (#m)[a-z]##_t ]]; print -r $?"####);
        bulk_aw_fc_row_025 => (r#"bulk aw 025"#, r###"setopt extendedglob; [[ foo = fo(#e) ]]; print -r $?"###);
        bulk_aw_fc_row_026 => (r#"bulk aw 026"#, r###"setopt extendedglob; [[ foo = (#s)fo ]]; print -r $?"###);
        bulk_aw_fc_row_027 => (r#"bulk aw 027"#, r###"[[ abc < abd ]]; print -r $?"###);
        bulk_aw_fc_row_028 => (r#"bulk aw 028"#, r###"[[ abc > abb ]]; print -r $?"###);
        bulk_aw_fc_row_029 => (r#"bulk aw 029"#, r###"[[ abc != def ]]; print -r $?"###);
        bulk_aw_fc_row_030 => (r#"bulk aw 030"#, r###"[[ abc == abc ]]; print -r $?"###);
        bulk_aw_fc_row_031 => (r#"bulk aw 031"#, r###"print -r ${(L)@}; set -- MIXED"###);
        bulk_aw_fc_row_032 => (r#"bulk aw 032"#, r###"slice=abcdef; print -r $slice[3,5]"###);
        bulk_aw_fc_row_033 => (r#"bulk aw 033"#, r###"typeset -aS ary=x y; print -r $ary[2]"###);
        bulk_aw_fc_row_034 => (r#"bulk aw 034"#, r###"pushd /tmp >/dev/null 2>&1; popd >/dev/null 2>&1; print -r $?"###);
        bulk_aw_fc_row_035 => (r#"bulk aw 035"#, r###"builtin cd -q / 2>/dev/null; print -r $?"###);
        bulk_aw_fc_row_036 => (r#"bulk aw 036"#, r###"cd /tmp 2>/dev/null; print -r ${PWD:t}"###);
        bulk_aw_fc_row_037 => (r#"bulk aw 037"#, r###"hash -r 2>/dev/null; print -r $?"###);
        bulk_aw_fc_row_038 => (r#"bulk aw 038"#, r###"autoload -Uz is-at-least 2>/dev/null; print -r $?"###);
        bulk_aw_fc_row_039 => (r#"bulk aw 039"#, r###"whence -v print 2>/dev/null; print -r $?"###);
        bulk_aw_fc_row_040 => (r#"bulk aw 040"#, r###"whence -p ls 2>/dev/null | head -1"###);
        bulk_aw_fc_row_041 => (r#"bulk aw 041"#, r###"typeset -f fn 2>/dev/null; fn(){ :; }; print -r $?"###);
        bulk_aw_fc_row_042 => (r#"bulk aw 042"#, r###"functions fn 2>/dev/null; fn(){ :; }; print -r $?"###);
        bulk_aw_fc_row_043 => (r#"bulk aw 043"#, r###"unfunction fn 2>/dev/null; fn(){ :; }; unfunction fn; print -r $?"###);
        bulk_aw_fc_row_044 => (r#"bulk aw 044"#, r###"print -r ${aliases[za]:-none}"###);
        bulk_aw_fc_row_045 => (r#"bulk aw 045"#, r###"print -r ${(t)parameters[PATH]}"###);
        bulk_aw_fc_row_046 => (r#"bulk aw 046"#, r###"print -r ${(k)parameters[(I)PATH]}"###);
        bulk_aw_fc_row_047 => (r#"bulk aw 047"#, r###"print -r ${+parameters[PATH]}"###);
        bulk_aw_fc_row_048 => (r#"bulk aw 048"#, r###"print -r ${+functions[fn]}; fn(){}"###);
    }
}

mod corpus_dash_fc_bulk_ax {
    use super::*;

    parity_gap_tests! {
        bulk_ax_fc_row_001 => (r#"bulk ax 001"#, r###"print -r $(( -(-(-5)) ))"###);
        bulk_ax_fc_row_002 => (r#"bulk ax 002"#, r###"print -r $(( (1+2)*(3+4) ))"###);
        bulk_ax_fc_row_003 => (r#"bulk ax 003"#, r###"v1=v1; [[ v1 -ef v1 ]]; print -r $?"###);
        bulk_ax_fc_row_004 => (r#"bulk ax 004"#, r###"[[ "" != x ]]; print -r $?"###);
        bulk_ax_fc_row_005 => (r#"bulk ax 005"#, r###"[[ -n /dev/null ]]; print -r $?"###);
        bulk_ax_fc_row_006 => (r#"bulk ax 006"#, r###"setopt extendedglob; [[ mix = [[:digit:]]# ]]; print -r $?"###);
        bulk_ax_fc_row_007 => (r#"bulk ax 007"#, r####"setopt extendedglob; [[ tag = (#m)[a-z]##_t ]]; print -r $?"####);
        bulk_ax_fc_row_008 => (r#"bulk ax 008"#, r###"setopt extendedglob; [[ foo = fo(#e) ]]; print -r $?"###);
        bulk_ax_fc_row_009 => (r#"bulk ax 009"#, r###"setopt extendedglob; [[ foo = (#s)fo ]]; print -r $?"###);
        bulk_ax_fc_row_010 => (r#"bulk ax 010"#, r###"[[ abc < abd ]]; print -r $?"###);
        bulk_ax_fc_row_011 => (r#"bulk ax 011"#, r###"[[ abc > abb ]]; print -r $?"###);
        bulk_ax_fc_row_012 => (r#"bulk ax 012"#, r###"[[ abc != def ]]; print -r $?"###);
        bulk_ax_fc_row_013 => (r#"bulk ax 013"#, r###"[[ abc == abc ]]; print -r $?"###);
        bulk_ax_fc_row_014 => (r#"bulk ax 014"#, r###"print -r ${(L)@}; set -- MIXED"###);
        bulk_ax_fc_row_015 => (r#"bulk ax 015"#, r###"slice=abcdef; print -r $slice[3,5]"###);
        bulk_ax_fc_row_016 => (r#"bulk ax 016"#, r###"typeset -aS ary=x y; print -r $ary[2]"###);
        bulk_ax_fc_row_017 => (r#"bulk ax 017"#, r###"pushd /tmp >/dev/null 2>&1; popd >/dev/null 2>&1; print -r $?"###);
        bulk_ax_fc_row_018 => (r#"bulk ax 018"#, r###"builtin cd -q / 2>/dev/null; print -r $?"###);
        bulk_ax_fc_row_019 => (r#"bulk ax 019"#, r###"cd /tmp 2>/dev/null; print -r ${PWD:t}"###);
        bulk_ax_fc_row_020 => (r#"bulk ax 020"#, r###"hash -r 2>/dev/null; print -r $?"###);
        bulk_ax_fc_row_021 => (r#"bulk ax 021"#, r###"autoload -Uz is-at-least 2>/dev/null; print -r $?"###);
        bulk_ax_fc_row_022 => (r#"bulk ax 022"#, r###"whence -v print 2>/dev/null; print -r $?"###);
        bulk_ax_fc_row_023 => (r#"bulk ax 023"#, r###"whence -p ls 2>/dev/null | head -1"###);
        bulk_ax_fc_row_024 => (r#"bulk ax 024"#, r###"typeset -f fn 2>/dev/null; fn(){ :; }; print -r $?"###);
        bulk_ax_fc_row_025 => (r#"bulk ax 025"#, r###"functions fn 2>/dev/null; fn(){ :; }; print -r $?"###);
        bulk_ax_fc_row_026 => (r#"bulk ax 026"#, r###"unfunction fn 2>/dev/null; fn(){ :; }; unfunction fn; print -r $?"###);
        bulk_ax_fc_row_027 => (r#"bulk ax 027"#, r###"print -r ${aliases[za]:-none}"###);
        bulk_ax_fc_row_028 => (r#"bulk ax 028"#, r###"print -r ${(t)parameters[PATH]}"###);
        bulk_ax_fc_row_029 => (r#"bulk ax 029"#, r###"print -r ${(k)parameters[(I)PATH]}"###);
        bulk_ax_fc_row_030 => (r#"bulk ax 030"#, r###"print -r ${+parameters[PATH]}"###);
        bulk_ax_fc_row_031 => (r#"bulk ax 031"#, r###"print -r ${+functions[fn]}; fn(){}"###);
        bulk_ax_fc_row_032 => (r#"bulk ax 032"#, r###"print -r ${+commands[print]}"###);
        bulk_ax_fc_row_033 => (r#"bulk ax 033"#, r###"print -r ${+zsh_eval_context}"###);
        bulk_ax_fc_row_034 => (r#"bulk ax 034"#, r###"print -r ${+functrace}"###);
        bulk_ax_fc_row_035 => (r#"bulk ax 035"#, r###"print -r ${+funcstack}"###);
        bulk_ax_fc_row_036 => (r#"bulk ax 036"#, r###"print -r ${+funcfiletrace}"###);
        bulk_ax_fc_row_037 => (r#"bulk ax 037"#, r###"print -r ${+jobstates}"###);
        bulk_ax_fc_row_038 => (r#"bulk ax 038"#, r###"print -r ${+jobtexts}"###);
        bulk_ax_fc_row_039 => (r#"bulk ax 039"#, r###"print -r ${+jobdirs}"###);
        bulk_ax_fc_row_040 => (r#"bulk ax 040"#, r###"print -r ${+historywords}"###);
        bulk_ax_fc_row_041 => (r#"bulk ax 041"#, r###"print -r ${+usergroups}"###);
        bulk_ax_fc_row_042 => (r#"bulk ax 042"#, r###"print -r ${+dis_builtins}"###);
        bulk_ax_fc_row_043 => (r#"bulk ax 043"#, r###"print -r ${+dis_widgets}"###);
        bulk_ax_fc_row_044 => (r#"bulk ax 044"#, r###"print -r ${+dis_reswords}"###);
        bulk_ax_fc_row_045 => (r#"bulk ax 045"#, r###"print -r ${+dis_patchars}"###);
        bulk_ax_fc_row_046 => (r#"bulk ax 046"#, r###"print -r ${+dis_commands}"###);
        bulk_ax_fc_row_047 => (r#"bulk ax 047"#, r###"print -r ${+module_path}"###);
        bulk_ax_fc_row_048 => (r#"bulk ax 048"#, r###"print -r ${+functrace}"###);
    }
}

mod corpus_dash_fc_bulk_ay {
    use super::*;

    parity_gap_tests! {
        bulk_ay_fc_row_001 => (r#"bulk ay 001"#, r###"builtin cd -q / 2>/dev/null; print -r $?"###);
        bulk_ay_fc_row_002 => (r#"bulk ay 002"#, r###"cd /tmp 2>/dev/null; print -r ${PWD:t}"###);
        bulk_ay_fc_row_003 => (r#"bulk ay 003"#, r###"hash -r 2>/dev/null; print -r $?"###);
        bulk_ay_fc_row_004 => (r#"bulk ay 004"#, r###"autoload -Uz is-at-least 2>/dev/null; print -r $?"###);
        bulk_ay_fc_row_005 => (r#"bulk ay 005"#, r###"whence -v print 2>/dev/null; print -r $?"###);
        bulk_ay_fc_row_006 => (r#"bulk ay 006"#, r###"whence -p ls 2>/dev/null | head -1"###);
        bulk_ay_fc_row_007 => (r#"bulk ay 007"#, r###"typeset -f fn 2>/dev/null; fn(){ :; }; print -r $?"###);
        bulk_ay_fc_row_008 => (r#"bulk ay 008"#, r###"functions fn 2>/dev/null; fn(){ :; }; print -r $?"###);
        bulk_ay_fc_row_009 => (r#"bulk ay 009"#, r###"unfunction fn 2>/dev/null; fn(){ :; }; unfunction fn; print -r $?"###);
        bulk_ay_fc_row_010 => (r#"bulk ay 010"#, r###"print -r ${aliases[za]:-none}"###);
        bulk_ay_fc_row_011 => (r#"bulk ay 011"#, r###"print -r ${(t)parameters[PATH]}"###);
        bulk_ay_fc_row_012 => (r#"bulk ay 012"#, r###"print -r ${(k)parameters[(I)PATH]}"###);
        bulk_ay_fc_row_013 => (r#"bulk ay 013"#, r###"print -r ${+parameters[PATH]}"###);
        bulk_ay_fc_row_014 => (r#"bulk ay 014"#, r###"print -r ${+functions[fn]}; fn(){}"###);
        bulk_ay_fc_row_015 => (r#"bulk ay 015"#, r###"print -r ${+commands[print]}"###);
        bulk_ay_fc_row_016 => (r#"bulk ay 016"#, r###"print -r ${+zsh_eval_context}"###);
        bulk_ay_fc_row_017 => (r#"bulk ay 017"#, r###"print -r ${+functrace}"###);
        bulk_ay_fc_row_018 => (r#"bulk ay 018"#, r###"print -r ${+funcstack}"###);
        bulk_ay_fc_row_019 => (r#"bulk ay 019"#, r###"print -r ${+funcfiletrace}"###);
        bulk_ay_fc_row_020 => (r#"bulk ay 020"#, r###"print -r ${+jobstates}"###);
        bulk_ay_fc_row_021 => (r#"bulk ay 021"#, r###"print -r ${+jobtexts}"###);
        bulk_ay_fc_row_022 => (r#"bulk ay 022"#, r###"print -r ${+jobdirs}"###);
        bulk_ay_fc_row_023 => (r#"bulk ay 023"#, r###"print -r ${+historywords}"###);
        bulk_ay_fc_row_024 => (r#"bulk ay 024"#, r###"print -r ${+usergroups}"###);
        bulk_ay_fc_row_025 => (r#"bulk ay 025"#, r###"print -r ${+dis_builtins}"###);
        bulk_ay_fc_row_026 => (r#"bulk ay 026"#, r###"print -r ${+dis_widgets}"###);
        bulk_ay_fc_row_027 => (r#"bulk ay 027"#, r###"print -r ${+dis_reswords}"###);
        bulk_ay_fc_row_028 => (r#"bulk ay 028"#, r###"print -r ${+dis_patchars}"###);
        bulk_ay_fc_row_029 => (r#"bulk ay 029"#, r###"print -r ${+dis_commands}"###);
        bulk_ay_fc_row_030 => (r#"bulk ay 030"#, r###"print -r ${+module_path}"###);
        bulk_ay_fc_row_031 => (r#"bulk ay 031"#, r###"print -r ${+functrace}"###);
        bulk_ay_fc_row_032 => (r#"bulk ay 032"#, r###"true | true | false; print -r ${pipestatus[3]}"###);
        bulk_ay_fc_row_033 => (r#"bulk ay 033"#, r###"{ true; false; }; print -r $?"###);
        bulk_ay_fc_row_034 => (r#"bulk ay 034"#, r###"fn(){ typeset -a la=(x y); print -r ${#la}; }; fn"###);
        bulk_ay_fc_row_035 => (r#"bulk ay 035"#, r###"print -r ${arr[@]:1:2}; arr=(a b c d)"###);
        bulk_ay_fc_row_036 => (r#"bulk ay 036"#, r###"print -r ${(pj:,:)a}; a=(x y)"###);
        bulk_ay_fc_row_037 => (r#"bulk ay 037"#, r###"print -r ${(Mk)h}; typeset -A h; h=(x 1 y 2)"###);
        bulk_ay_fc_row_038 => (r#"bulk ay 038"#, r###"print -r ${(oa)n}; n=(10 2 1)"###);
        bulk_ay_fc_row_039 => (r#"bulk ay 039"#, r###"print -r ${(On)n}; n=(10 2 1)"###);
        bulk_ay_fc_row_040 => (r#"bulk ay 040"#, r###"print -r ${(n)a}; a=(1 2 3)"###);
        bulk_ay_fc_row_041 => (r#"bulk ay 041"#, r###"print -r ${(N)a}; a=(1 2 3)"###);
        bulk_ay_fc_row_042 => (r#"bulk ay 042"#, r###"print -r ${(w)#w}; w=a b c"###);
        bulk_ay_fc_row_043 => (r#"bulk ay 043"#, r###"print -r ${(t)x}; x=hello"###);
        bulk_ay_fc_row_044 => (r#"bulk ay 044"#, r###"unset y; print -r ${+y}"###);
        bulk_ay_fc_row_045 => (r#"bulk ay 045"#, r###"x=hello; print -r ${+x}"###);
        bulk_ay_fc_row_046 => (r#"bulk ay 046"#, r###"print -r ${(q+)x}; x=hi"###);
        bulk_ay_fc_row_047 => (r#"bulk ay 047"#, r###"x=foo; print -r ${x:s/foo/bar/}"###);
        bulk_ay_fc_row_048 => (r#"bulk ay 048"#, r###"x=foofoo; print -r ${x//foo/bar}"###);
    }
}

mod corpus_dash_fc_bulk_az {
    use super::*;

    parity_gap_tests! {
        bulk_az_fc_row_001 => (r#"bulk az 001"#, r###"print -r ${+funcstack}"###);
        bulk_az_fc_row_002 => (r#"bulk az 002"#, r###"print -r ${+funcfiletrace}"###);
        bulk_az_fc_row_003 => (r#"bulk az 003"#, r###"print -r ${+jobstates}"###);
        bulk_az_fc_row_004 => (r#"bulk az 004"#, r###"print -r ${+jobtexts}"###);
        bulk_az_fc_row_005 => (r#"bulk az 005"#, r###"print -r ${+jobdirs}"###);
        bulk_az_fc_row_006 => (r#"bulk az 006"#, r###"print -r ${+historywords}"###);
        bulk_az_fc_row_007 => (r#"bulk az 007"#, r###"print -r ${+usergroups}"###);
        bulk_az_fc_row_008 => (r#"bulk az 008"#, r###"print -r ${+dis_builtins}"###);
        bulk_az_fc_row_009 => (r#"bulk az 009"#, r###"print -r ${+dis_widgets}"###);
        bulk_az_fc_row_010 => (r#"bulk az 010"#, r###"print -r ${+dis_reswords}"###);
        bulk_az_fc_row_011 => (r#"bulk az 011"#, r###"print -r ${+dis_patchars}"###);
        bulk_az_fc_row_012 => (r#"bulk az 012"#, r###"print -r ${+dis_commands}"###);
        bulk_az_fc_row_013 => (r#"bulk az 013"#, r###"print -r ${+module_path}"###);
        bulk_az_fc_row_014 => (r#"bulk az 014"#, r###"print -r ${+functrace}"###);
        bulk_az_fc_row_015 => (r#"bulk az 015"#, r###"true | true | false; print -r ${pipestatus[3]}"###);
        bulk_az_fc_row_016 => (r#"bulk az 016"#, r###"{ true; false; }; print -r $?"###);
        bulk_az_fc_row_017 => (r#"bulk az 017"#, r###"fn(){ typeset -a la=(x y); print -r ${#la}; }; fn"###);
        bulk_az_fc_row_018 => (r#"bulk az 018"#, r###"print -r ${arr[@]:1:2}; arr=(a b c d)"###);
        bulk_az_fc_row_019 => (r#"bulk az 019"#, r###"print -r ${(pj:,:)a}; a=(x y)"###);
        bulk_az_fc_row_020 => (r#"bulk az 020"#, r###"print -r ${(Mk)h}; typeset -A h; h=(x 1 y 2)"###);
        bulk_az_fc_row_021 => (r#"bulk az 021"#, r###"print -r ${(oa)n}; n=(10 2 1)"###);
        bulk_az_fc_row_022 => (r#"bulk az 022"#, r###"print -r ${(On)n}; n=(10 2 1)"###);
        bulk_az_fc_row_023 => (r#"bulk az 023"#, r###"print -r ${(n)a}; a=(1 2 3)"###);
        bulk_az_fc_row_024 => (r#"bulk az 024"#, r###"print -r ${(N)a}; a=(1 2 3)"###);
        bulk_az_fc_row_025 => (r#"bulk az 025"#, r###"print -r ${(w)#w}; w=a b c"###);
        bulk_az_fc_row_026 => (r#"bulk az 026"#, r###"print -r ${(t)x}; x=hello"###);
        bulk_az_fc_row_027 => (r#"bulk az 027"#, r###"unset y; print -r ${+y}"###);
        bulk_az_fc_row_028 => (r#"bulk az 028"#, r###"x=hello; print -r ${+x}"###);
        bulk_az_fc_row_029 => (r#"bulk az 029"#, r###"print -r ${(q+)x}; x=hi"###);
        bulk_az_fc_row_030 => (r#"bulk az 030"#, r###"x=foo; print -r ${x:s/foo/bar/}"###);
        bulk_az_fc_row_031 => (r#"bulk az 031"#, r###"x=foofoo; print -r ${x//foo/bar}"###);
        bulk_az_fc_row_032 => (r#"bulk az 032"#, r###"x=abc; print -r ${x/#a/z}"###);
        bulk_az_fc_row_033 => (r#"bulk az 033"#, r###"x=abc; print -r ${x/%c/z}"###);
        bulk_az_fc_row_034 => (r#"bulk az 034"#, r###"print -r ${(j::)a}; a=(x y)"###);
        bulk_az_fc_row_035 => (r#"bulk az 035"#, r###"print -r ${(pj::)a}; a=(x y)"###);
        bulk_az_fc_row_036 => (r#"bulk az 036"#, r###"print -r ${(ps:\n:)x}; x=$'a\nb'"###);
        bulk_az_fc_row_037 => (r#"bulk az 037"#, r###"print -r ${(e)x}; x=$'2+2'"###);
        bulk_az_fc_row_038 => (r#"bulk az 038"#, r###"integer co=0; : $(( co=6 )); print -r $co"###);
        bulk_az_fc_row_039 => (r#"bulk az 039"#, r###"print -r $(( 1<<0 ))"###);
        bulk_az_fc_row_040 => (r#"bulk az 040"#, r###"print -r $(( 1<<10 ))"###);
        bulk_az_fc_row_041 => (r#"bulk az 041"#, r###"print -r $(( 0x7fffffff & 0 ))"###);
        bulk_az_fc_row_042 => (r#"bulk az 042"#, r###"print -r $(( 1000003 % 97 ))"###);
        bulk_az_fc_row_043 => (r#"bulk az 043"#, r###"print -r $(( 63 & 31 | 15 ))"###);
        bulk_az_fc_row_044 => (r#"bulk az 044"#, r###"print -r $(( 0x10001 % 256 ))"###);
        bulk_az_fc_row_045 => (r#"bulk az 045"#, r###"print -r $(( 2*2*2*2 ))"###);
        bulk_az_fc_row_046 => (r#"bulk az 046"#, r###"print -r $(( (1==1)+(0==1) ))"###);
        bulk_az_fc_row_047 => (r#"bulk az 047"#, r###"print -r $(( 1 && (0 || 1) ))"###);
        bulk_az_fc_row_048 => (r#"bulk az 048"#, r###"[[ zero = <-> ]]; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_ba {
    use super::*;

    parity_gap_tests! {
        bulk_ba_fc_row_001 => (r#"bulk ba 001"#, r###"print -r ${arr[@]:1:2}; arr=(a b c d)"###);
        bulk_ba_fc_row_002 => (r#"bulk ba 002"#, r###"print -r ${(pj:,:)a}; a=(x y)"###);
        bulk_ba_fc_row_003 => (r#"bulk ba 003"#, r###"print -r ${(Mk)h}; typeset -A h; h=(x 1 y 2)"###);
        bulk_ba_fc_row_004 => (r#"bulk ba 004"#, r###"print -r ${(oa)n}; n=(10 2 1)"###);
        bulk_ba_fc_row_005 => (r#"bulk ba 005"#, r###"print -r ${(On)n}; n=(10 2 1)"###);
        bulk_ba_fc_row_006 => (r#"bulk ba 006"#, r###"print -r ${(n)a}; a=(1 2 3)"###);
        bulk_ba_fc_row_007 => (r#"bulk ba 007"#, r###"print -r ${(N)a}; a=(1 2 3)"###);
        bulk_ba_fc_row_008 => (r#"bulk ba 008"#, r###"print -r ${(w)#w}; w=a b c"###);
        bulk_ba_fc_row_009 => (r#"bulk ba 009"#, r###"print -r ${(t)x}; x=hello"###);
        bulk_ba_fc_row_010 => (r#"bulk ba 010"#, r###"unset y; print -r ${+y}"###);
        bulk_ba_fc_row_011 => (r#"bulk ba 011"#, r###"x=hello; print -r ${+x}"###);
        bulk_ba_fc_row_012 => (r#"bulk ba 012"#, r###"print -r ${(q+)x}; x=hi"###);
        bulk_ba_fc_row_013 => (r#"bulk ba 013"#, r###"x=foo; print -r ${x:s/foo/bar/}"###);
        bulk_ba_fc_row_014 => (r#"bulk ba 014"#, r###"x=foofoo; print -r ${x//foo/bar}"###);
        bulk_ba_fc_row_015 => (r#"bulk ba 015"#, r###"x=abc; print -r ${x/#a/z}"###);
        bulk_ba_fc_row_016 => (r#"bulk ba 016"#, r###"x=abc; print -r ${x/%c/z}"###);
        bulk_ba_fc_row_017 => (r#"bulk ba 017"#, r###"print -r ${(j::)a}; a=(x y)"###);
        bulk_ba_fc_row_018 => (r#"bulk ba 018"#, r###"print -r ${(pj::)a}; a=(x y)"###);
        bulk_ba_fc_row_019 => (r#"bulk ba 019"#, r###"print -r ${(ps:\n:)x}; x=$'a\nb'"###);
        bulk_ba_fc_row_020 => (r#"bulk ba 020"#, r###"print -r ${(e)x}; x=$'2+2'"###);
        bulk_ba_fc_row_021 => (r#"bulk ba 021"#, r###"integer co=0; : $(( co=6 )); print -r $co"###);
        bulk_ba_fc_row_022 => (r#"bulk ba 022"#, r###"print -r $(( 1<<0 ))"###);
        bulk_ba_fc_row_023 => (r#"bulk ba 023"#, r###"print -r $(( 1<<10 ))"###);
        bulk_ba_fc_row_024 => (r#"bulk ba 024"#, r###"print -r $(( 0x7fffffff & 0 ))"###);
        bulk_ba_fc_row_025 => (r#"bulk ba 025"#, r###"print -r $(( 1000003 % 97 ))"###);
        bulk_ba_fc_row_026 => (r#"bulk ba 026"#, r###"print -r $(( 63 & 31 | 15 ))"###);
        bulk_ba_fc_row_027 => (r#"bulk ba 027"#, r###"print -r $(( 0x10001 % 256 ))"###);
        bulk_ba_fc_row_028 => (r#"bulk ba 028"#, r###"print -r $(( 2*2*2*2 ))"###);
        bulk_ba_fc_row_029 => (r#"bulk ba 029"#, r###"print -r $(( (1==1)+(0==1) ))"###);
        bulk_ba_fc_row_030 => (r#"bulk ba 030"#, r###"print -r $(( 1 && (0 || 1) ))"###);
        bulk_ba_fc_row_031 => (r#"bulk ba 031"#, r###"[[ zero = <-> ]]; print -r $?"###);
        bulk_ba_fc_row_032 => (r#"bulk ba 032"#, r###"[[ . = . ]]; print -r $?"###);
        bulk_ba_fc_row_033 => (r#"bulk ba 033"#, r###"[[ a -lt b ]]; print -r $?"###);
        bulk_ba_fc_row_034 => (r#"bulk ba 034"#, r####"[[ ABC = [A-Z]## ]]; print -r $?"####);
        bulk_ba_fc_row_035 => (r#"bulk ba 035"#, r###"[[ AAA =~ ^A+ ]]; print -r $?"###);
        bulk_ba_fc_row_036 => (r#"bulk ba 036"#, r###"[[ bot = *ot* ]]; print -r $?"###);
        bulk_ba_fc_row_037 => (r#"bulk ba 037"#, r###"[[ -e /dev/null ]]; print -r $?"###);
        bulk_ba_fc_row_038 => (r#"bulk ba 038"#, r###"[[ -s /dev/null ]]; print -r $?"###);
        bulk_ba_fc_row_039 => (r#"bulk ba 039"#, r###"[[ -u /etc/hosts ]]; print -r $?"###);
        bulk_ba_fc_row_040 => (r#"bulk ba 040"#, r###"[[ -g / ]]; print -r $?"###);
        bulk_ba_fc_row_041 => (r#"bulk ba 041"#, r###"[[ -k /tmp ]]; print -r $?"###);
        bulk_ba_fc_row_042 => (r#"bulk ba 042"#, r###"[[ -b /dev/null ]]; print -r $?"###);
        bulk_ba_fc_row_043 => (r#"bulk ba 043"#, r###"[[ -c /dev/null ]]; print -r $?"###);
        bulk_ba_fc_row_044 => (r#"bulk ba 044"#, r###"print -r ${(l:5::0:)n}; n=42"###);
        bulk_ba_fc_row_045 => (r#"bulk ba 045"#, r###"print -r ${(r:5::0:)n}; n=42"###);
        bulk_ba_fc_row_046 => (r#"bulk ba 046"#, r###"print -r ${(c)str}; str=hello"###);
        bulk_ba_fc_row_047 => (r#"bulk ba 047"#, r###"print -r ${(u)arr}; arr=(a A b)"###);
        bulk_ba_fc_row_048 => (r#"bulk ba 048"#, r###"print -r ${(L)str}; str=HELLO"###);
    }
}

mod corpus_dash_fc_bulk_bb {
    use super::*;

    parity_gap_tests! {
        bulk_bb_fc_row_001 => (r#"bulk bb 001"#, r###"print -r ${(j::)a}; a=(x y)"###);
        bulk_bb_fc_row_002 => (r#"bulk bb 002"#, r###"print -r ${(pj::)a}; a=(x y)"###);
        bulk_bb_fc_row_003 => (r#"bulk bb 003"#, r###"print -r ${(ps:\n:)x}; x=$'a\nb'"###);
        bulk_bb_fc_row_004 => (r#"bulk bb 004"#, r###"print -r ${(e)x}; x=$'2+2'"###);
        bulk_bb_fc_row_005 => (r#"bulk bb 005"#, r###"integer co=0; : $(( co=6 )); print -r $co"###);
        bulk_bb_fc_row_006 => (r#"bulk bb 006"#, r###"print -r $(( 1<<0 ))"###);
        bulk_bb_fc_row_007 => (r#"bulk bb 007"#, r###"print -r $(( 1<<10 ))"###);
        bulk_bb_fc_row_008 => (r#"bulk bb 008"#, r###"print -r $(( 0x7fffffff & 0 ))"###);
        bulk_bb_fc_row_009 => (r#"bulk bb 009"#, r###"print -r $(( 1000003 % 97 ))"###);
        bulk_bb_fc_row_010 => (r#"bulk bb 010"#, r###"print -r $(( 63 & 31 | 15 ))"###);
        bulk_bb_fc_row_011 => (r#"bulk bb 011"#, r###"print -r $(( 0x10001 % 256 ))"###);
        bulk_bb_fc_row_012 => (r#"bulk bb 012"#, r###"print -r $(( 2*2*2*2 ))"###);
        bulk_bb_fc_row_013 => (r#"bulk bb 013"#, r###"print -r $(( (1==1)+(0==1) ))"###);
        bulk_bb_fc_row_014 => (r#"bulk bb 014"#, r###"print -r $(( 1 && (0 || 1) ))"###);
        bulk_bb_fc_row_015 => (r#"bulk bb 015"#, r###"[[ zero = <-> ]]; print -r $?"###);
        bulk_bb_fc_row_016 => (r#"bulk bb 016"#, r###"[[ . = . ]]; print -r $?"###);
        bulk_bb_fc_row_017 => (r#"bulk bb 017"#, r###"[[ a -lt b ]]; print -r $?"###);
        bulk_bb_fc_row_018 => (r#"bulk bb 018"#, r####"[[ ABC = [A-Z]## ]]; print -r $?"####);
        bulk_bb_fc_row_019 => (r#"bulk bb 019"#, r###"[[ AAA =~ ^A+ ]]; print -r $?"###);
        bulk_bb_fc_row_020 => (r#"bulk bb 020"#, r###"[[ bot = *ot* ]]; print -r $?"###);
        bulk_bb_fc_row_021 => (r#"bulk bb 021"#, r###"[[ -e /dev/null ]]; print -r $?"###);
        bulk_bb_fc_row_022 => (r#"bulk bb 022"#, r###"[[ -s /dev/null ]]; print -r $?"###);
        bulk_bb_fc_row_023 => (r#"bulk bb 023"#, r###"[[ -u /etc/hosts ]]; print -r $?"###);
        bulk_bb_fc_row_024 => (r#"bulk bb 024"#, r###"[[ -g / ]]; print -r $?"###);
        bulk_bb_fc_row_025 => (r#"bulk bb 025"#, r###"[[ -k /tmp ]]; print -r $?"###);
        bulk_bb_fc_row_026 => (r#"bulk bb 026"#, r###"[[ -b /dev/null ]]; print -r $?"###);
        bulk_bb_fc_row_027 => (r#"bulk bb 027"#, r###"[[ -c /dev/null ]]; print -r $?"###);
        bulk_bb_fc_row_028 => (r#"bulk bb 028"#, r###"print -r ${(l:5::0:)n}; n=42"###);
        bulk_bb_fc_row_029 => (r#"bulk bb 029"#, r###"print -r ${(r:5::0:)n}; n=42"###);
        bulk_bb_fc_row_030 => (r#"bulk bb 030"#, r###"print -r ${(c)str}; str=hello"###);
        bulk_bb_fc_row_031 => (r#"bulk bb 031"#, r###"print -r ${(u)arr}; arr=(a A b)"###);
        bulk_bb_fc_row_032 => (r#"bulk bb 032"#, r###"print -r ${(L)str}; str=HELLO"###);
        bulk_bb_fc_row_033 => (r#"bulk bb 033"#, r###"print -r ${(U)str}; str=hello"###);
        bulk_bb_fc_row_034 => (r#"bulk bb 034"#, r###"print -r ${(C)str}; str=hello world"###);
        bulk_bb_fc_row_035 => (r#"bulk bb 035"#, r###"print -r ${(Q)str}; str=$'a\nb'"###);
        bulk_bb_fc_row_036 => (r#"bulk bb 036"#, r###"print -r ${(qq)str}; str=hi"###);
        bulk_bb_fc_row_037 => (r#"bulk bb 037"#, r###"print -r ${(V)str}; str=hi"###);
        bulk_bb_fc_row_038 => (r#"bulk bb 038"#, r###"print -r ${(z)str}; str=$'a\0b'"###);
        bulk_bb_fc_row_039 => (r#"bulk bb 039"#, r###"print -r ${(j:-:)a}; a=(x y)"###);
        bulk_bb_fc_row_040 => (r#"bulk bb 040"#, r###"print -r ${(pj:-:)a}; a=(x y)"###);
        bulk_bb_fc_row_041 => (r#"bulk bb 041"#, r###"a=(1 2 3); print -r ${a[1,-1]}"###);
        bulk_bb_fc_row_042 => (r#"bulk bb 042"#, r###"a=(1 2 3); print -r ${a[1,2]}"###);
        bulk_bb_fc_row_043 => (r#"bulk bb 043"#, r###"a=(1 2 3); print -r ${a[-1]}"###);
        bulk_bb_fc_row_044 => (r#"bulk bb 044"#, r###"a=(1 2 3); print -r ${a[(i)2]}"###);
        bulk_bb_fc_row_045 => (r#"bulk bb 045"#, r###"a=(1 2 3); print -r ${a[(I)2]}"###);
        bulk_bb_fc_row_046 => (r#"bulk bb 046"#, r###"a=(1 2 3); print -r ${a[(R)9]}"###);
        bulk_bb_fc_row_047 => (r#"bulk bb 047"#, r###"a=(1 2 3); print -r ${a[(r)2]}"###);
        bulk_bb_fc_row_048 => (r#"bulk bb 048"#, r###"typeset -A m; m=(k v); print -r ${m[k]}"###);
    }
}

mod corpus_dash_fc_bulk_bc {
    use super::*;

    parity_gap_tests! {
        bulk_bc_fc_row_001 => (r#"bulk bc 001"#, r####"[[ ABC = [A-Z]## ]]; print -r $?"####);
        bulk_bc_fc_row_002 => (r#"bulk bc 002"#, r###"[[ AAA =~ ^A+ ]]; print -r $?"###);
        bulk_bc_fc_row_003 => (r#"bulk bc 003"#, r###"[[ bot = *ot* ]]; print -r $?"###);
        bulk_bc_fc_row_004 => (r#"bulk bc 004"#, r###"[[ -e /dev/null ]]; print -r $?"###);
        bulk_bc_fc_row_005 => (r#"bulk bc 005"#, r###"[[ -s /dev/null ]]; print -r $?"###);
        bulk_bc_fc_row_006 => (r#"bulk bc 006"#, r###"[[ -u /etc/hosts ]]; print -r $?"###);
        bulk_bc_fc_row_007 => (r#"bulk bc 007"#, r###"[[ -g / ]]; print -r $?"###);
        bulk_bc_fc_row_008 => (r#"bulk bc 008"#, r###"[[ -k /tmp ]]; print -r $?"###);
        bulk_bc_fc_row_009 => (r#"bulk bc 009"#, r###"[[ -b /dev/null ]]; print -r $?"###);
        bulk_bc_fc_row_010 => (r#"bulk bc 010"#, r###"[[ -c /dev/null ]]; print -r $?"###);
        bulk_bc_fc_row_011 => (r#"bulk bc 011"#, r###"print -r ${(l:5::0:)n}; n=42"###);
        bulk_bc_fc_row_012 => (r#"bulk bc 012"#, r###"print -r ${(r:5::0:)n}; n=42"###);
        bulk_bc_fc_row_013 => (r#"bulk bc 013"#, r###"print -r ${(c)str}; str=hello"###);
        bulk_bc_fc_row_014 => (r#"bulk bc 014"#, r###"print -r ${(u)arr}; arr=(a A b)"###);
        bulk_bc_fc_row_015 => (r#"bulk bc 015"#, r###"print -r ${(L)str}; str=HELLO"###);
        bulk_bc_fc_row_016 => (r#"bulk bc 016"#, r###"print -r ${(U)str}; str=hello"###);
        bulk_bc_fc_row_017 => (r#"bulk bc 017"#, r###"print -r ${(C)str}; str=hello world"###);
        bulk_bc_fc_row_018 => (r#"bulk bc 018"#, r###"print -r ${(Q)str}; str=$'a\nb'"###);
        bulk_bc_fc_row_019 => (r#"bulk bc 019"#, r###"print -r ${(qq)str}; str=hi"###);
        bulk_bc_fc_row_020 => (r#"bulk bc 020"#, r###"print -r ${(V)str}; str=hi"###);
        bulk_bc_fc_row_021 => (r#"bulk bc 021"#, r###"print -r ${(z)str}; str=$'a\0b'"###);
        bulk_bc_fc_row_022 => (r#"bulk bc 022"#, r###"print -r ${(j:-:)a}; a=(x y)"###);
        bulk_bc_fc_row_023 => (r#"bulk bc 023"#, r###"print -r ${(pj:-:)a}; a=(x y)"###);
        bulk_bc_fc_row_024 => (r#"bulk bc 024"#, r###"a=(1 2 3); print -r ${a[1,-1]}"###);
        bulk_bc_fc_row_025 => (r#"bulk bc 025"#, r###"a=(1 2 3); print -r ${a[1,2]}"###);
        bulk_bc_fc_row_026 => (r#"bulk bc 026"#, r###"a=(1 2 3); print -r ${a[-1]}"###);
        bulk_bc_fc_row_027 => (r#"bulk bc 027"#, r###"a=(1 2 3); print -r ${a[(i)2]}"###);
        bulk_bc_fc_row_028 => (r#"bulk bc 028"#, r###"a=(1 2 3); print -r ${a[(I)2]}"###);
        bulk_bc_fc_row_029 => (r#"bulk bc 029"#, r###"a=(1 2 3); print -r ${a[(R)9]}"###);
        bulk_bc_fc_row_030 => (r#"bulk bc 030"#, r###"a=(1 2 3); print -r ${a[(r)2]}"###);
        bulk_bc_fc_row_031 => (r#"bulk bc 031"#, r###"typeset -A m; m=(k v); print -r ${m[k]}"###);
        bulk_bc_fc_row_032 => (r#"bulk bc 032"#, r###"typeset -A m; m=(k v); print -r ${(k)m}"###);
        bulk_bc_fc_row_033 => (r#"bulk bc 033"#, r###"typeset -A m; m=(k v); print -r ${(v)m}"###);
        bulk_bc_fc_row_034 => (r#"bulk bc 034"#, r###"typeset -A m; m=(a 1 b 2); print -r ${(kv)m}"###);
        bulk_bc_fc_row_035 => (r#"bulk bc 035"#, r###"print -r ${(o)lst}; lst=(z a m)"###);
        bulk_bc_fc_row_036 => (r#"bulk bc 036"#, r###"print -r ${(O)lst}; lst=(z a m)"###);
        bulk_bc_fc_row_037 => (r#"bulk bc 037"#, r###"print -r ${(i)lst}; lst=(z a m)"###);
        bulk_bc_fc_row_038 => (r#"bulk bc 038"#, r###"a=(x y); print -r ${^a}"###);
        bulk_bc_fc_row_039 => (r#"bulk bc 039"#, r###"a=(1 2); b=(a b); print -r ${^a}${^b}"###);
        bulk_bc_fc_row_040 => (r#"bulk bc 040"#, r###"setopt braceccl; print -r {a,b}"###);
        bulk_bc_fc_row_041 => (r#"bulk bc 041"#, r###"print -r {1..3}"###);
        bulk_bc_fc_row_042 => (r#"bulk bc 042"#, r###"print -r {01..03}"###);
        bulk_bc_fc_row_043 => (r#"bulk bc 043"#, r###"print -r {a..c}"###);
        bulk_bc_fc_row_044 => (r#"bulk bc 044"#, r###"print -r {1..4..2}"###);
        bulk_bc_fc_row_045 => (r#"bulk bc 045"#, r###"print -r ${~pattern}; pattern='*'; :"###);
        bulk_bc_fc_row_046 => (r#"bulk bc 046"#, r###"integer x=3; (( x++ )); print -r $x"###);
        bulk_bc_fc_row_047 => (r#"bulk bc 047"#, r###"integer x=3; (( ++x )); print -r $x"###);
        bulk_bc_fc_row_048 => (r#"bulk bc 048"#, r###"integer x=3; (( x-- )); print -r $x"###);
    }
}

mod corpus_dash_fc_bulk_bd {
    use super::*;

    parity_gap_tests! {
        bulk_bd_fc_row_001 => (r#"bulk bd 001"#, r###"print -r ${(U)str}; str=hello"###);
        bulk_bd_fc_row_002 => (r#"bulk bd 002"#, r###"print -r ${(C)str}; str=hello world"###);
        bulk_bd_fc_row_003 => (r#"bulk bd 003"#, r###"print -r ${(Q)str}; str=$'a\nb'"###);
        bulk_bd_fc_row_004 => (r#"bulk bd 004"#, r###"print -r ${(qq)str}; str=hi"###);
        bulk_bd_fc_row_005 => (r#"bulk bd 005"#, r###"print -r ${(V)str}; str=hi"###);
        bulk_bd_fc_row_006 => (r#"bulk bd 006"#, r###"print -r ${(z)str}; str=$'a\0b'"###);
        bulk_bd_fc_row_007 => (r#"bulk bd 007"#, r###"print -r ${(j:-:)a}; a=(x y)"###);
        bulk_bd_fc_row_008 => (r#"bulk bd 008"#, r###"print -r ${(pj:-:)a}; a=(x y)"###);
        bulk_bd_fc_row_009 => (r#"bulk bd 009"#, r###"a=(1 2 3); print -r ${a[1,-1]}"###);
        bulk_bd_fc_row_010 => (r#"bulk bd 010"#, r###"a=(1 2 3); print -r ${a[1,2]}"###);
        bulk_bd_fc_row_011 => (r#"bulk bd 011"#, r###"a=(1 2 3); print -r ${a[-1]}"###);
        bulk_bd_fc_row_012 => (r#"bulk bd 012"#, r###"a=(1 2 3); print -r ${a[(i)2]}"###);
        bulk_bd_fc_row_013 => (r#"bulk bd 013"#, r###"a=(1 2 3); print -r ${a[(I)2]}"###);
        bulk_bd_fc_row_014 => (r#"bulk bd 014"#, r###"a=(1 2 3); print -r ${a[(R)9]}"###);
        bulk_bd_fc_row_015 => (r#"bulk bd 015"#, r###"a=(1 2 3); print -r ${a[(r)2]}"###);
        bulk_bd_fc_row_016 => (r#"bulk bd 016"#, r###"typeset -A m; m=(k v); print -r ${m[k]}"###);
        bulk_bd_fc_row_017 => (r#"bulk bd 017"#, r###"typeset -A m; m=(k v); print -r ${(k)m}"###);
        bulk_bd_fc_row_018 => (r#"bulk bd 018"#, r###"typeset -A m; m=(k v); print -r ${(v)m}"###);
        bulk_bd_fc_row_019 => (r#"bulk bd 019"#, r###"typeset -A m; m=(a 1 b 2); print -r ${(kv)m}"###);
        bulk_bd_fc_row_020 => (r#"bulk bd 020"#, r###"print -r ${(o)lst}; lst=(z a m)"###);
        bulk_bd_fc_row_021 => (r#"bulk bd 021"#, r###"print -r ${(O)lst}; lst=(z a m)"###);
        bulk_bd_fc_row_022 => (r#"bulk bd 022"#, r###"print -r ${(i)lst}; lst=(z a m)"###);
        bulk_bd_fc_row_023 => (r#"bulk bd 023"#, r###"a=(x y); print -r ${^a}"###);
        bulk_bd_fc_row_024 => (r#"bulk bd 024"#, r###"a=(1 2); b=(a b); print -r ${^a}${^b}"###);
        bulk_bd_fc_row_025 => (r#"bulk bd 025"#, r###"setopt braceccl; print -r {a,b}"###);
        bulk_bd_fc_row_026 => (r#"bulk bd 026"#, r###"print -r {1..3}"###);
        bulk_bd_fc_row_027 => (r#"bulk bd 027"#, r###"print -r {01..03}"###);
        bulk_bd_fc_row_028 => (r#"bulk bd 028"#, r###"print -r {a..c}"###);
        bulk_bd_fc_row_029 => (r#"bulk bd 029"#, r###"print -r {1..4..2}"###);
        bulk_bd_fc_row_030 => (r#"bulk bd 030"#, r###"print -r ${~pattern}; pattern='*'; :"###);
        bulk_bd_fc_row_031 => (r#"bulk bd 031"#, r###"integer x=3; (( x++ )); print -r $x"###);
        bulk_bd_fc_row_032 => (r#"bulk bd 032"#, r###"integer x=3; (( ++x )); print -r $x"###);
        bulk_bd_fc_row_033 => (r#"bulk bd 033"#, r###"integer x=3; (( x-- )); print -r $x"###);
        bulk_bd_fc_row_034 => (r#"bulk bd 034"#, r###"integer x=3; print -r $(( x ** 2 ))"###);
        bulk_bd_fc_row_035 => (r#"bulk bd 035"#, r###"float f=1.5; print -r $(( f + 1 ))"###);
        bulk_bd_fc_row_036 => (r#"bulk bd 036"#, r###"print -r $(( 7 / 2 ))"###);
        bulk_bd_fc_row_037 => (r#"bulk bd 037"#, r###"print -r $(( 7.0 / 2 ))"###);
        bulk_bd_fc_row_038 => (r#"bulk bd 038"#, r###"(( 1 )); print -r $?"###);
        bulk_bd_fc_row_039 => (r#"bulk bd 039"#, r###"(( 0 )); print -r $?"###);
        bulk_bd_fc_row_040 => (r#"bulk bd 040"#, r###": $(( 0 )) || print -r z"###);
        bulk_bd_fc_row_041 => (r#"bulk bd 041"#, r###": $(( 1 )) && print -r y"###);
        bulk_bd_fc_row_042 => (r#"bulk bd 042"#, r###"let x=2+2; print -r $x"###);
        bulk_bd_fc_row_043 => (r#"bulk bd 043"#, r###"(( x = 5 )); print -r $x"###);
        bulk_bd_fc_row_044 => (r#"bulk bd 044"#, r###"typeset -F f=2.5; print -r $f"###);
        bulk_bd_fc_row_045 => (r#"bulk bd 045"#, r###"typeset -E e=2.5; print -r $e"###);
        bulk_bd_fc_row_046 => (r#"bulk bd 046"#, r###"typeset -i n=07; print -r $n"###);
        bulk_bd_fc_row_047 => (r#"bulk bd 047"#, r###"typeset -l s=ABC; print -r $s"###);
        bulk_bd_fc_row_048 => (r#"bulk bd 048"#, r###"typeset -u s=abc; print -r $s"###);
    }
}

mod corpus_dash_fc_bulk_be {
    use super::*;

    parity_gap_tests! {
        bulk_be_fc_row_001 => (r#"bulk be 001"#, r###"a=(1 2 3); print -r ${a[(R)9]}"###);
        bulk_be_fc_row_002 => (r#"bulk be 002"#, r###"a=(1 2 3); print -r ${a[(r)2]}"###);
        bulk_be_fc_row_003 => (r#"bulk be 003"#, r###"typeset -A m; m=(k v); print -r ${m[k]}"###);
        bulk_be_fc_row_004 => (r#"bulk be 004"#, r###"typeset -A m; m=(k v); print -r ${(k)m}"###);
        bulk_be_fc_row_005 => (r#"bulk be 005"#, r###"typeset -A m; m=(k v); print -r ${(v)m}"###);
        bulk_be_fc_row_006 => (r#"bulk be 006"#, r###"typeset -A m; m=(a 1 b 2); print -r ${(kv)m}"###);
        bulk_be_fc_row_007 => (r#"bulk be 007"#, r###"print -r ${(o)lst}; lst=(z a m)"###);
        bulk_be_fc_row_008 => (r#"bulk be 008"#, r###"print -r ${(O)lst}; lst=(z a m)"###);
        bulk_be_fc_row_009 => (r#"bulk be 009"#, r###"print -r ${(i)lst}; lst=(z a m)"###);
        bulk_be_fc_row_010 => (r#"bulk be 010"#, r###"a=(x y); print -r ${^a}"###);
        bulk_be_fc_row_011 => (r#"bulk be 011"#, r###"a=(1 2); b=(a b); print -r ${^a}${^b}"###);
        bulk_be_fc_row_012 => (r#"bulk be 012"#, r###"setopt braceccl; print -r {a,b}"###);
        bulk_be_fc_row_013 => (r#"bulk be 013"#, r###"print -r {1..3}"###);
        bulk_be_fc_row_014 => (r#"bulk be 014"#, r###"print -r {01..03}"###);
        bulk_be_fc_row_015 => (r#"bulk be 015"#, r###"print -r {a..c}"###);
        bulk_be_fc_row_016 => (r#"bulk be 016"#, r###"print -r {1..4..2}"###);
        bulk_be_fc_row_017 => (r#"bulk be 017"#, r###"print -r ${~pattern}; pattern='*'; :"###);
        bulk_be_fc_row_018 => (r#"bulk be 018"#, r###"integer x=3; (( x++ )); print -r $x"###);
        bulk_be_fc_row_019 => (r#"bulk be 019"#, r###"integer x=3; (( ++x )); print -r $x"###);
        bulk_be_fc_row_020 => (r#"bulk be 020"#, r###"integer x=3; (( x-- )); print -r $x"###);
        bulk_be_fc_row_021 => (r#"bulk be 021"#, r###"integer x=3; print -r $(( x ** 2 ))"###);
        bulk_be_fc_row_022 => (r#"bulk be 022"#, r###"float f=1.5; print -r $(( f + 1 ))"###);
        bulk_be_fc_row_023 => (r#"bulk be 023"#, r###"print -r $(( 7 / 2 ))"###);
        bulk_be_fc_row_024 => (r#"bulk be 024"#, r###"print -r $(( 7.0 / 2 ))"###);
        bulk_be_fc_row_025 => (r#"bulk be 025"#, r###"(( 1 )); print -r $?"###);
        bulk_be_fc_row_026 => (r#"bulk be 026"#, r###"(( 0 )); print -r $?"###);
        bulk_be_fc_row_027 => (r#"bulk be 027"#, r###": $(( 0 )) || print -r z"###);
        bulk_be_fc_row_028 => (r#"bulk be 028"#, r###": $(( 1 )) && print -r y"###);
        bulk_be_fc_row_029 => (r#"bulk be 029"#, r###"let x=2+2; print -r $x"###);
        bulk_be_fc_row_030 => (r#"bulk be 030"#, r###"(( x = 5 )); print -r $x"###);
        bulk_be_fc_row_031 => (r#"bulk be 031"#, r###"typeset -F f=2.5; print -r $f"###);
        bulk_be_fc_row_032 => (r#"bulk be 032"#, r###"typeset -E e=2.5; print -r $e"###);
        bulk_be_fc_row_033 => (r#"bulk be 033"#, r###"typeset -i n=07; print -r $n"###);
        bulk_be_fc_row_034 => (r#"bulk be 034"#, r###"typeset -l s=ABC; print -r $s"###);
        bulk_be_fc_row_035 => (r#"bulk be 035"#, r###"typeset -u s=abc; print -r $s"###);
        bulk_be_fc_row_036 => (r#"bulk be 036"#, r###"typeset -r x=1; x=2; print -r $x"###);
        bulk_be_fc_row_037 => (r#"bulk be 037"#, r###"typeset -h s; s=abc; print -r $s"###);
        bulk_be_fc_row_038 => (r#"bulk be 038"#, r###"typeset -H s; s=abc; print -r $s"###);
        bulk_be_fc_row_039 => (r#"bulk be 039"#, r###"typeset -b n=255; print -r $n"###);
        bulk_be_fc_row_040 => (r#"bulk be 040"#, r###"typeset -o n=7; print -r $n"###);
        bulk_be_fc_row_041 => (r#"bulk be 041"#, r###"typeset -aU u; u=(a a b); print -r ${(j:,:)u}"###);
        bulk_be_fc_row_042 => (r#"bulk be 042"#, r###"local a; a=1; print -r $a"###);
        bulk_be_fc_row_043 => (r#"bulk be 043"#, r###"local -i n=5; print -r $(( n * 2 ))"###);
        bulk_be_fc_row_044 => (r#"bulk be 044"#, r###"local -a arr; arr=(x); print -r $arr[1]"###);
        bulk_be_fc_row_045 => (r#"bulk be 045"#, r###"fn(){ local x=1; print -r $x; }; fn"###);
        bulk_be_fc_row_046 => (r#"bulk be 046"#, r###"fn(){ typeset -a a; a=(1); print -r ${#a}; }; fn"###);
        bulk_be_fc_row_047 => (r#"bulk be 047"#, r###"autoload -Uz add-zsh-hook 2>/dev/null; print -r $?"###);
        bulk_be_fc_row_048 => (r#"bulk be 048"#, r###"emulate -L zsh; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_bf {
    use super::*;

    parity_gap_tests! {
        bulk_bf_fc_row_001 => (r#"bulk bf 001"#, r###"print -r ${~pattern}; pattern='*'; :"###);
        bulk_bf_fc_row_002 => (r#"bulk bf 002"#, r###"integer x=3; (( x++ )); print -r $x"###);
        bulk_bf_fc_row_003 => (r#"bulk bf 003"#, r###"integer x=3; (( ++x )); print -r $x"###);
        bulk_bf_fc_row_004 => (r#"bulk bf 004"#, r###"integer x=3; (( x-- )); print -r $x"###);
        bulk_bf_fc_row_005 => (r#"bulk bf 005"#, r###"integer x=3; print -r $(( x ** 2 ))"###);
        bulk_bf_fc_row_006 => (r#"bulk bf 006"#, r###"float f=1.5; print -r $(( f + 1 ))"###);
        bulk_bf_fc_row_007 => (r#"bulk bf 007"#, r###"print -r $(( 7 / 2 ))"###);
        bulk_bf_fc_row_008 => (r#"bulk bf 008"#, r###"print -r $(( 7.0 / 2 ))"###);
        bulk_bf_fc_row_009 => (r#"bulk bf 009"#, r###"(( 1 )); print -r $?"###);
        bulk_bf_fc_row_010 => (r#"bulk bf 010"#, r###"(( 0 )); print -r $?"###);
        bulk_bf_fc_row_011 => (r#"bulk bf 011"#, r###": $(( 0 )) || print -r z"###);
        bulk_bf_fc_row_012 => (r#"bulk bf 012"#, r###": $(( 1 )) && print -r y"###);
        bulk_bf_fc_row_013 => (r#"bulk bf 013"#, r###"let x=2+2; print -r $x"###);
        bulk_bf_fc_row_014 => (r#"bulk bf 014"#, r###"(( x = 5 )); print -r $x"###);
        bulk_bf_fc_row_015 => (r#"bulk bf 015"#, r###"typeset -F f=2.5; print -r $f"###);
        bulk_bf_fc_row_016 => (r#"bulk bf 016"#, r###"typeset -E e=2.5; print -r $e"###);
        bulk_bf_fc_row_017 => (r#"bulk bf 017"#, r###"typeset -i n=07; print -r $n"###);
        bulk_bf_fc_row_018 => (r#"bulk bf 018"#, r###"typeset -l s=ABC; print -r $s"###);
        bulk_bf_fc_row_019 => (r#"bulk bf 019"#, r###"typeset -u s=abc; print -r $s"###);
        bulk_bf_fc_row_020 => (r#"bulk bf 020"#, r###"typeset -r x=1; x=2; print -r $x"###);
        bulk_bf_fc_row_021 => (r#"bulk bf 021"#, r###"typeset -h s; s=abc; print -r $s"###);
        bulk_bf_fc_row_022 => (r#"bulk bf 022"#, r###"typeset -H s; s=abc; print -r $s"###);
        bulk_bf_fc_row_023 => (r#"bulk bf 023"#, r###"typeset -b n=255; print -r $n"###);
        bulk_bf_fc_row_024 => (r#"bulk bf 024"#, r###"typeset -o n=7; print -r $n"###);
        bulk_bf_fc_row_025 => (r#"bulk bf 025"#, r###"typeset -aU u; u=(a a b); print -r ${(j:,:)u}"###);
        bulk_bf_fc_row_026 => (r#"bulk bf 026"#, r###"local a; a=1; print -r $a"###);
        bulk_bf_fc_row_027 => (r#"bulk bf 027"#, r###"local -i n=5; print -r $(( n * 2 ))"###);
        bulk_bf_fc_row_028 => (r#"bulk bf 028"#, r###"local -a arr; arr=(x); print -r $arr[1]"###);
        bulk_bf_fc_row_029 => (r#"bulk bf 029"#, r###"fn(){ local x=1; print -r $x; }; fn"###);
        bulk_bf_fc_row_030 => (r#"bulk bf 030"#, r###"fn(){ typeset -a a; a=(1); print -r ${#a}; }; fn"###);
        bulk_bf_fc_row_031 => (r#"bulk bf 031"#, r###"autoload -Uz add-zsh-hook 2>/dev/null; print -r $?"###);
        bulk_bf_fc_row_032 => (r#"bulk bf 032"#, r###"emulate -L zsh; print -r $?"###);
        bulk_bf_fc_row_033 => (r#"bulk bf 033"#, r###"setopt localoptions; print -r $?"###);
        bulk_bf_fc_row_034 => (r#"bulk bf 034"#, r###"unsetopt localoptions 2>/dev/null; print -r $?"###);
        bulk_bf_fc_row_035 => (r#"bulk bf 035"#, r###"setopt pipefail; false | true; print -r $?"###);
        bulk_bf_fc_row_036 => (r#"bulk bf 036"#, r###"setopt no_pipefail; false | true; print -r $?"###);
        bulk_bf_fc_row_037 => (r#"bulk bf 037"#, r###"setopt nullglob; print -r ${#files}; files=(/no/such/*)"###);
        bulk_bf_fc_row_038 => (r#"bulk bf 038"#, r###"setopt nonomatch; print -r ${#files}; files=(/no/such/*)"###);
        bulk_bf_fc_row_039 => (r#"bulk bf 039"#, r###"setopt extendedglob; print -r $?"###);
        bulk_bf_fc_row_040 => (r#"bulk bf 040"#, r###"setopt shwordsplit; print -r $?"###);
        bulk_bf_fc_row_041 => (r#"bulk bf 041"#, r###"setopt no_shwordsplit; print -r $?"###);
        bulk_bf_fc_row_042 => (r#"bulk bf 042"#, r###"setopt interactivecomments; print -r $?"###);
        bulk_bf_fc_row_043 => (r#"bulk bf 043"#, r###"setopt no_interactivecomments; print -r $?"###);
        bulk_bf_fc_row_044 => (r#"bulk bf 044"#, r###"setopt multios; print -r $?"###);
        bulk_bf_fc_row_045 => (r#"bulk bf 045"#, r###"setopt noclobber; print -r $?"###);
        bulk_bf_fc_row_046 => (r#"bulk bf 046"#, r###"setopt clobber; print -r $?"###);
        bulk_bf_fc_row_047 => (r#"bulk bf 047"#, r###"setopt histexpand; print -r $?"###);
        bulk_bf_fc_row_048 => (r#"bulk bf 048"#, r###"setopt no_histexpand; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_bg {
    use super::*;

    parity_gap_tests! {
        bulk_bg_fc_row_001 => (r#"bulk bg 001"#, r###"typeset -l s=ABC; print -r $s"###);
        bulk_bg_fc_row_002 => (r#"bulk bg 002"#, r###"typeset -u s=abc; print -r $s"###);
        bulk_bg_fc_row_003 => (r#"bulk bg 003"#, r###"typeset -r x=1; x=2; print -r $x"###);
        bulk_bg_fc_row_004 => (r#"bulk bg 004"#, r###"typeset -h s; s=abc; print -r $s"###);
        bulk_bg_fc_row_005 => (r#"bulk bg 005"#, r###"typeset -H s; s=abc; print -r $s"###);
        bulk_bg_fc_row_006 => (r#"bulk bg 006"#, r###"typeset -b n=255; print -r $n"###);
        bulk_bg_fc_row_007 => (r#"bulk bg 007"#, r###"typeset -o n=7; print -r $n"###);
        bulk_bg_fc_row_008 => (r#"bulk bg 008"#, r###"typeset -aU u; u=(a a b); print -r ${(j:,:)u}"###);
        bulk_bg_fc_row_009 => (r#"bulk bg 009"#, r###"local a; a=1; print -r $a"###);
        bulk_bg_fc_row_010 => (r#"bulk bg 010"#, r###"local -i n=5; print -r $(( n * 2 ))"###);
        bulk_bg_fc_row_011 => (r#"bulk bg 011"#, r###"local -a arr; arr=(x); print -r $arr[1]"###);
        bulk_bg_fc_row_012 => (r#"bulk bg 012"#, r###"fn(){ local x=1; print -r $x; }; fn"###);
        bulk_bg_fc_row_013 => (r#"bulk bg 013"#, r###"fn(){ typeset -a a; a=(1); print -r ${#a}; }; fn"###);
        bulk_bg_fc_row_014 => (r#"bulk bg 014"#, r###"autoload -Uz add-zsh-hook 2>/dev/null; print -r $?"###);
        bulk_bg_fc_row_015 => (r#"bulk bg 015"#, r###"emulate -L zsh; print -r $?"###);
        bulk_bg_fc_row_016 => (r#"bulk bg 016"#, r###"setopt localoptions; print -r $?"###);
        bulk_bg_fc_row_017 => (r#"bulk bg 017"#, r###"unsetopt localoptions 2>/dev/null; print -r $?"###);
        bulk_bg_fc_row_018 => (r#"bulk bg 018"#, r###"setopt pipefail; false | true; print -r $?"###);
        bulk_bg_fc_row_019 => (r#"bulk bg 019"#, r###"setopt no_pipefail; false | true; print -r $?"###);
        bulk_bg_fc_row_020 => (r#"bulk bg 020"#, r###"setopt nullglob; print -r ${#files}; files=(/no/such/*)"###);
        bulk_bg_fc_row_021 => (r#"bulk bg 021"#, r###"setopt nonomatch; print -r ${#files}; files=(/no/such/*)"###);
        bulk_bg_fc_row_022 => (r#"bulk bg 022"#, r###"setopt extendedglob; print -r $?"###);
        bulk_bg_fc_row_023 => (r#"bulk bg 023"#, r###"setopt shwordsplit; print -r $?"###);
        bulk_bg_fc_row_024 => (r#"bulk bg 024"#, r###"setopt no_shwordsplit; print -r $?"###);
        bulk_bg_fc_row_025 => (r#"bulk bg 025"#, r###"setopt interactivecomments; print -r $?"###);
        bulk_bg_fc_row_026 => (r#"bulk bg 026"#, r###"setopt no_interactivecomments; print -r $?"###);
        bulk_bg_fc_row_027 => (r#"bulk bg 027"#, r###"setopt multios; print -r $?"###);
        bulk_bg_fc_row_028 => (r#"bulk bg 028"#, r###"setopt noclobber; print -r $?"###);
        bulk_bg_fc_row_029 => (r#"bulk bg 029"#, r###"setopt clobber; print -r $?"###);
        bulk_bg_fc_row_030 => (r#"bulk bg 030"#, r###"setopt histexpand; print -r $?"###);
        bulk_bg_fc_row_031 => (r#"bulk bg 031"#, r###"setopt no_histexpand; print -r $?"###);
        bulk_bg_fc_row_032 => (r#"bulk bg 032"#, r###"setopt banghist; print -r $?"###);
        bulk_bg_fc_row_033 => (r#"bulk bg 033"#, r###"setopt sharehistory; print -r $?"###);
        bulk_bg_fc_row_034 => (r#"bulk bg 034"#, r###"setopt incappendhistory; print -r $?"###);
        bulk_bg_fc_row_035 => (r#"bulk bg 035"#, r###"setopt extendedhistory; print -r $?"###);
        bulk_bg_fc_row_036 => (r#"bulk bg 036"#, r###"setopt histignoredups; print -r $?"###);
        bulk_bg_fc_row_037 => (r#"bulk bg 037"#, r###"setopt histignorespace; print -r $?"###);
        bulk_bg_fc_row_038 => (r#"bulk bg 038"#, r###"setopt histreduceblanks; print -r $?"###);
        bulk_bg_fc_row_039 => (r#"bulk bg 039"#, r###"setopt histverify; print -r $?"###);
        bulk_bg_fc_row_040 => (r#"bulk bg 040"#, r###"setopt appendhistory; print -r $?"###);
        bulk_bg_fc_row_041 => (r#"bulk bg 041"#, r###"setopt no_beep; print -r $?"###);
        bulk_bg_fc_row_042 => (r#"bulk bg 042"#, r###"setopt no_listbeep; print -r $?"###);
        bulk_bg_fc_row_043 => (r#"bulk bg 043"#, r###"setopt auto_cd; print -r $?"###);
        bulk_bg_fc_row_044 => (r#"bulk bg 044"#, r###"setopt no_auto_cd; print -r $?"###);
        bulk_bg_fc_row_045 => (r#"bulk bg 045"#, r###"setopt correct; print -r $?"###);
        bulk_bg_fc_row_046 => (r#"bulk bg 046"#, r###"setopt nocorrect; print -r $?"###);
        bulk_bg_fc_row_047 => (r#"bulk bg 047"#, r###"setopt completealiases; print -r $?"###);
        bulk_bg_fc_row_048 => (r#"bulk bg 048"#, r###"setopt globdots; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_bh {
    use super::*;

    parity_gap_tests! {
        bulk_bh_fc_row_001 => (r#"bulk bh 001"#, r###"unsetopt localoptions 2>/dev/null; print -r $?"###);
        bulk_bh_fc_row_002 => (r#"bulk bh 002"#, r###"setopt pipefail; false | true; print -r $?"###);
        bulk_bh_fc_row_003 => (r#"bulk bh 003"#, r###"setopt no_pipefail; false | true; print -r $?"###);
        bulk_bh_fc_row_004 => (r#"bulk bh 004"#, r###"setopt nullglob; print -r ${#files}; files=(/no/such/*)"###);
        bulk_bh_fc_row_005 => (r#"bulk bh 005"#, r###"setopt nonomatch; print -r ${#files}; files=(/no/such/*)"###);
        bulk_bh_fc_row_006 => (r#"bulk bh 006"#, r###"setopt extendedglob; print -r $?"###);
        bulk_bh_fc_row_007 => (r#"bulk bh 007"#, r###"setopt shwordsplit; print -r $?"###);
        bulk_bh_fc_row_008 => (r#"bulk bh 008"#, r###"setopt no_shwordsplit; print -r $?"###);
        bulk_bh_fc_row_009 => (r#"bulk bh 009"#, r###"setopt interactivecomments; print -r $?"###);
        bulk_bh_fc_row_010 => (r#"bulk bh 010"#, r###"setopt no_interactivecomments; print -r $?"###);
        bulk_bh_fc_row_011 => (r#"bulk bh 011"#, r###"setopt multios; print -r $?"###);
        bulk_bh_fc_row_012 => (r#"bulk bh 012"#, r###"setopt noclobber; print -r $?"###);
        bulk_bh_fc_row_013 => (r#"bulk bh 013"#, r###"setopt clobber; print -r $?"###);
        bulk_bh_fc_row_014 => (r#"bulk bh 014"#, r###"setopt histexpand; print -r $?"###);
        bulk_bh_fc_row_015 => (r#"bulk bh 015"#, r###"setopt no_histexpand; print -r $?"###);
        bulk_bh_fc_row_016 => (r#"bulk bh 016"#, r###"setopt banghist; print -r $?"###);
        bulk_bh_fc_row_017 => (r#"bulk bh 017"#, r###"setopt sharehistory; print -r $?"###);
        bulk_bh_fc_row_018 => (r#"bulk bh 018"#, r###"setopt incappendhistory; print -r $?"###);
        bulk_bh_fc_row_019 => (r#"bulk bh 019"#, r###"setopt extendedhistory; print -r $?"###);
        bulk_bh_fc_row_020 => (r#"bulk bh 020"#, r###"setopt histignoredups; print -r $?"###);
        bulk_bh_fc_row_021 => (r#"bulk bh 021"#, r###"setopt histignorespace; print -r $?"###);
        bulk_bh_fc_row_022 => (r#"bulk bh 022"#, r###"setopt histreduceblanks; print -r $?"###);
        bulk_bh_fc_row_023 => (r#"bulk bh 023"#, r###"setopt histverify; print -r $?"###);
        bulk_bh_fc_row_024 => (r#"bulk bh 024"#, r###"setopt appendhistory; print -r $?"###);
        bulk_bh_fc_row_025 => (r#"bulk bh 025"#, r###"setopt no_beep; print -r $?"###);
        bulk_bh_fc_row_026 => (r#"bulk bh 026"#, r###"setopt no_listbeep; print -r $?"###);
        bulk_bh_fc_row_027 => (r#"bulk bh 027"#, r###"setopt auto_cd; print -r $?"###);
        bulk_bh_fc_row_028 => (r#"bulk bh 028"#, r###"setopt no_auto_cd; print -r $?"###);
        bulk_bh_fc_row_029 => (r#"bulk bh 029"#, r###"setopt correct; print -r $?"###);
        bulk_bh_fc_row_030 => (r#"bulk bh 030"#, r###"setopt nocorrect; print -r $?"###);
        bulk_bh_fc_row_031 => (r#"bulk bh 031"#, r###"setopt completealiases; print -r $?"###);
        bulk_bh_fc_row_032 => (r#"bulk bh 032"#, r###"setopt globdots; print -r $?"###);
        bulk_bh_fc_row_033 => (r#"bulk bh 033"#, r###"setopt noglobdots; print -r $?"###);
        bulk_bh_fc_row_034 => (r#"bulk bh 034"#, r###"setopt numericglobsort; print -r $?"###);
        bulk_bh_fc_row_035 => (r#"bulk bh 035"#, r###"setopt markdirs; print -r $?"###);
        bulk_bh_fc_row_036 => (r#"bulk bh 036"#, r###"setopt nomarkdirs; print -r $?"###);
        bulk_bh_fc_row_037 => (r#"bulk bh 037"#, r###"setopt chase_links; print -r $?"###);
        bulk_bh_fc_row_038 => (r#"bulk bh 038"#, r###"setopt no_chase_links; print -r $?"###);
        bulk_bh_fc_row_039 => (r#"bulk bh 039"#, r###"setopt pushdignoredups; print -r $?"###);
        bulk_bh_fc_row_040 => (r#"bulk bh 040"#, r###"setopt pushdsilent; print -r $?"###);
        bulk_bh_fc_row_041 => (r#"bulk bh 041"#, r###"setopt pushdtohome; print -r $?"###);
        bulk_bh_fc_row_042 => (r#"bulk bh 042"#, r###"setopt autopushd; print -r $?"###);
        bulk_bh_fc_row_043 => (r#"bulk bh 043"#, r###"setopt pushdminus; print -r $?"###);
        bulk_bh_fc_row_044 => (r#"bulk bh 044"#, r###"setopt pushdignoredups; print -r $?"###);
        bulk_bh_fc_row_045 => (r#"bulk bh 045"#, r###"dirs -p 2>/dev/null | head -1; print -r $?"###);
        bulk_bh_fc_row_046 => (r#"bulk bh 046"#, r###"pushd /tmp 2>/dev/null; popd 2>/dev/null; print -r $?"###);
        bulk_bh_fc_row_047 => (r#"bulk bh 047"#, r###"cd -q / 2>/dev/null; print -r $?"###);
        bulk_bh_fc_row_048 => (r#"bulk bh 048"#, r###"print -r $PWD"###);
    }
}

mod corpus_dash_fc_bulk_bi {
    use super::*;

    parity_gap_tests! {
        bulk_bi_fc_row_001 => (r#"bulk bi 001"#, r###"setopt incappendhistory; print -r $?"###);
        bulk_bi_fc_row_002 => (r#"bulk bi 002"#, r###"setopt extendedhistory; print -r $?"###);
        bulk_bi_fc_row_003 => (r#"bulk bi 003"#, r###"setopt histignoredups; print -r $?"###);
        bulk_bi_fc_row_004 => (r#"bulk bi 004"#, r###"setopt histignorespace; print -r $?"###);
        bulk_bi_fc_row_005 => (r#"bulk bi 005"#, r###"setopt histreduceblanks; print -r $?"###);
        bulk_bi_fc_row_006 => (r#"bulk bi 006"#, r###"setopt histverify; print -r $?"###);
        bulk_bi_fc_row_007 => (r#"bulk bi 007"#, r###"setopt appendhistory; print -r $?"###);
        bulk_bi_fc_row_008 => (r#"bulk bi 008"#, r###"setopt no_beep; print -r $?"###);
        bulk_bi_fc_row_009 => (r#"bulk bi 009"#, r###"setopt no_listbeep; print -r $?"###);
        bulk_bi_fc_row_010 => (r#"bulk bi 010"#, r###"setopt auto_cd; print -r $?"###);
        bulk_bi_fc_row_011 => (r#"bulk bi 011"#, r###"setopt no_auto_cd; print -r $?"###);
        bulk_bi_fc_row_012 => (r#"bulk bi 012"#, r###"setopt correct; print -r $?"###);
        bulk_bi_fc_row_013 => (r#"bulk bi 013"#, r###"setopt nocorrect; print -r $?"###);
        bulk_bi_fc_row_014 => (r#"bulk bi 014"#, r###"setopt completealiases; print -r $?"###);
        bulk_bi_fc_row_015 => (r#"bulk bi 015"#, r###"setopt globdots; print -r $?"###);
        bulk_bi_fc_row_016 => (r#"bulk bi 016"#, r###"setopt noglobdots; print -r $?"###);
        bulk_bi_fc_row_017 => (r#"bulk bi 017"#, r###"setopt numericglobsort; print -r $?"###);
        bulk_bi_fc_row_018 => (r#"bulk bi 018"#, r###"setopt markdirs; print -r $?"###);
        bulk_bi_fc_row_019 => (r#"bulk bi 019"#, r###"setopt nomarkdirs; print -r $?"###);
        bulk_bi_fc_row_020 => (r#"bulk bi 020"#, r###"setopt chase_links; print -r $?"###);
        bulk_bi_fc_row_021 => (r#"bulk bi 021"#, r###"setopt no_chase_links; print -r $?"###);
        bulk_bi_fc_row_022 => (r#"bulk bi 022"#, r###"setopt pushdignoredups; print -r $?"###);
        bulk_bi_fc_row_023 => (r#"bulk bi 023"#, r###"setopt pushdsilent; print -r $?"###);
        bulk_bi_fc_row_024 => (r#"bulk bi 024"#, r###"setopt pushdtohome; print -r $?"###);
        bulk_bi_fc_row_025 => (r#"bulk bi 025"#, r###"setopt autopushd; print -r $?"###);
        bulk_bi_fc_row_026 => (r#"bulk bi 026"#, r###"setopt pushdminus; print -r $?"###);
        bulk_bi_fc_row_027 => (r#"bulk bi 027"#, r###"setopt pushdignoredups; print -r $?"###);
        bulk_bi_fc_row_028 => (r#"bulk bi 028"#, r###"dirs -p 2>/dev/null | head -1; print -r $?"###);
        bulk_bi_fc_row_029 => (r#"bulk bi 029"#, r###"pushd /tmp 2>/dev/null; popd 2>/dev/null; print -r $?"###);
        bulk_bi_fc_row_030 => (r#"bulk bi 030"#, r###"cd -q / 2>/dev/null; print -r $?"###);
        bulk_bi_fc_row_031 => (r#"bulk bi 031"#, r###"print -r $PWD"###);
        bulk_bi_fc_row_032 => (r#"bulk bi 032"#, r###"print -r ${PWD:h}"###);
        bulk_bi_fc_row_033 => (r#"bulk bi 033"#, r###"print -r ${PWD:t}"###);
        bulk_bi_fc_row_034 => (r#"bulk bi 034"#, r###"print -r ${PWD:r}"###);
        bulk_bi_fc_row_035 => (r#"bulk bi 035"#, r###"print -r ${PWD:e}"###);
        bulk_bi_fc_row_036 => (r#"bulk bi 036"#, r###"print -r ${PWD:a}"###);
        bulk_bi_fc_row_037 => (r#"bulk bi 037"#, r###"print -r ${PWD:A}"###);
        bulk_bi_fc_row_038 => (r#"bulk bi 038"#, r###"read -r line <<< 'one'; print -r $line"###);
        bulk_bi_fc_row_039 => (r#"bulk bi 039"#, r###"read -r a b <<< 'x y'; print -r $a-$b"###);
        bulk_bi_fc_row_040 => (r#"bulk bi 040"#, r###"print -r $'tab\there'"###);
        bulk_bi_fc_row_041 => (r#"bulk bi 041"#, r###"print -r $'line1\nline2'"###);
        bulk_bi_fc_row_042 => (r#"bulk bi 042"#, r###"printf '%q\n' 'a b'"###);
        bulk_bi_fc_row_043 => (r#"bulk bi 043"#, r###"printf '%s\n' ok"###);
        bulk_bi_fc_row_044 => (r#"bulk bi 044"#, r###"print -rn -- end"###);
        bulk_bi_fc_row_045 => (r#"bulk bi 045"#, r###"print -rl -- a b"###);
        bulk_bi_fc_row_046 => (r#"bulk bi 046"#, r###"print -fc '%s\n' hi"###);
        bulk_bi_fc_row_047 => (r#"bulk bi 047"#, r###"whence -w print 2>/dev/null; print -r $?"###);
        bulk_bi_fc_row_048 => (r#"bulk bi 048"#, r###"whence -c print 2>/dev/null; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_bj {
    use super::*;

    parity_gap_tests! {
        bulk_bj_fc_row_001 => (r#"bulk bj 001"#, r###"setopt markdirs; print -r $?"###);
        bulk_bj_fc_row_002 => (r#"bulk bj 002"#, r###"setopt nomarkdirs; print -r $?"###);
        bulk_bj_fc_row_003 => (r#"bulk bj 003"#, r###"setopt chase_links; print -r $?"###);
        bulk_bj_fc_row_004 => (r#"bulk bj 004"#, r###"setopt no_chase_links; print -r $?"###);
        bulk_bj_fc_row_005 => (r#"bulk bj 005"#, r###"setopt pushdignoredups; print -r $?"###);
        bulk_bj_fc_row_006 => (r#"bulk bj 006"#, r###"setopt pushdsilent; print -r $?"###);
        bulk_bj_fc_row_007 => (r#"bulk bj 007"#, r###"setopt pushdtohome; print -r $?"###);
        bulk_bj_fc_row_008 => (r#"bulk bj 008"#, r###"setopt autopushd; print -r $?"###);
        bulk_bj_fc_row_009 => (r#"bulk bj 009"#, r###"setopt pushdminus; print -r $?"###);
        bulk_bj_fc_row_010 => (r#"bulk bj 010"#, r###"setopt pushdignoredups; print -r $?"###);
        bulk_bj_fc_row_011 => (r#"bulk bj 011"#, r###"dirs -p 2>/dev/null | head -1; print -r $?"###);
        bulk_bj_fc_row_012 => (r#"bulk bj 012"#, r###"pushd /tmp 2>/dev/null; popd 2>/dev/null; print -r $?"###);
        bulk_bj_fc_row_013 => (r#"bulk bj 013"#, r###"cd -q / 2>/dev/null; print -r $?"###);
        bulk_bj_fc_row_014 => (r#"bulk bj 014"#, r###"print -r $PWD"###);
        bulk_bj_fc_row_015 => (r#"bulk bj 015"#, r###"print -r ${PWD:h}"###);
        bulk_bj_fc_row_016 => (r#"bulk bj 016"#, r###"print -r ${PWD:t}"###);
        bulk_bj_fc_row_017 => (r#"bulk bj 017"#, r###"print -r ${PWD:r}"###);
        bulk_bj_fc_row_018 => (r#"bulk bj 018"#, r###"print -r ${PWD:e}"###);
        bulk_bj_fc_row_019 => (r#"bulk bj 019"#, r###"print -r ${PWD:a}"###);
        bulk_bj_fc_row_020 => (r#"bulk bj 020"#, r###"print -r ${PWD:A}"###);
        bulk_bj_fc_row_021 => (r#"bulk bj 021"#, r###"read -r line <<< 'one'; print -r $line"###);
        bulk_bj_fc_row_022 => (r#"bulk bj 022"#, r###"read -r a b <<< 'x y'; print -r $a-$b"###);
        bulk_bj_fc_row_023 => (r#"bulk bj 023"#, r###"print -r $'tab\there'"###);
        bulk_bj_fc_row_024 => (r#"bulk bj 024"#, r###"print -r $'line1\nline2'"###);
        bulk_bj_fc_row_025 => (r#"bulk bj 025"#, r###"printf '%q\n' 'a b'"###);
        bulk_bj_fc_row_026 => (r#"bulk bj 026"#, r###"printf '%s\n' ok"###);
        bulk_bj_fc_row_027 => (r#"bulk bj 027"#, r###"print -rn -- end"###);
        bulk_bj_fc_row_028 => (r#"bulk bj 028"#, r###"print -rl -- a b"###);
        bulk_bj_fc_row_029 => (r#"bulk bj 029"#, r###"print -fc '%s\n' hi"###);
        bulk_bj_fc_row_030 => (r#"bulk bj 030"#, r###"whence -w print 2>/dev/null; print -r $?"###);
        bulk_bj_fc_row_031 => (r#"bulk bj 031"#, r###"whence -c print 2>/dev/null; print -r $?"###);
        bulk_bj_fc_row_032 => (r#"bulk bj 032"#, r###"which print 2>/dev/null; print -r $?"###);
        bulk_bj_fc_row_033 => (r#"bulk bj 033"#, r###"command -v print 2>/dev/null; print -r $?"###);
        bulk_bj_fc_row_034 => (r#"bulk bj 034"#, r###"hash -r 2>/dev/null; print -r $?"###);
        bulk_bj_fc_row_035 => (r#"bulk bj 035"#, r###"rehash 2>/dev/null; print -r $?"###);
        bulk_bj_fc_row_036 => (r#"bulk bj 036"#, r###"unalias za 2>/dev/null; alias za=1; unalias za; print -r $?"###);
        bulk_bj_fc_row_037 => (r#"bulk bj 037"#, r###"alias -L za 2>/dev/null; alias za=z; print -r $?"###);
        bulk_bj_fc_row_038 => (r#"bulk bj 038"#, r###"export ZA=1; print -r $ZA"###);
        bulk_bj_fc_row_039 => (r#"bulk bj 039"#, r###"typeset +Z ZA; ZA=1; print -r $ZA"###);
        bulk_bj_fc_row_040 => (r#"bulk bj 040"#, r###"typeset -x ZB=2; print -r $ZB"###);
        bulk_bj_fc_row_041 => (r#"bulk bj 041"#, r###"unset ZC; ZC=1; unset ZC; print -r ${+ZC}"###);
        bulk_bj_fc_row_042 => (r#"bulk bj 042"#, r###"typeset -tH hx; hx=ff; print -r $hx"###);
        bulk_bj_fc_row_043 => (r#"bulk bj 043"#, r###"shift; print -r $1; set -- a b c"###);
        bulk_bj_fc_row_044 => (r#"bulk bj 044"#, r###"(( $# )); print -r $#"###);
        bulk_bj_fc_row_045 => (r#"bulk bj 045"#, r###"print -r ${argv[1]}"###);
        bulk_bj_fc_row_046 => (r#"bulk bj 046"#, r###"print -r ${*[1]}"###);
        bulk_bj_fc_row_047 => (r#"bulk bj 047"#, r###"print -r $@[1]"###);
        bulk_bj_fc_row_048 => (r#"bulk bj 048"#, r###"print -r ${@:2}"###);
    }
}

mod corpus_dash_fc_bulk_bk {
    use super::*;

    parity_gap_tests! {
        bulk_bk_fc_row_001 => (r#"bulk bk 001"#, r###"print -r ${PWD:t}"###);
        bulk_bk_fc_row_002 => (r#"bulk bk 002"#, r###"print -r ${PWD:r}"###);
        bulk_bk_fc_row_003 => (r#"bulk bk 003"#, r###"print -r ${PWD:e}"###);
        bulk_bk_fc_row_004 => (r#"bulk bk 004"#, r###"print -r ${PWD:a}"###);
        bulk_bk_fc_row_005 => (r#"bulk bk 005"#, r###"print -r ${PWD:A}"###);
        bulk_bk_fc_row_006 => (r#"bulk bk 006"#, r###"read -r line <<< 'one'; print -r $line"###);
        bulk_bk_fc_row_007 => (r#"bulk bk 007"#, r###"read -r a b <<< 'x y'; print -r $a-$b"###);
        bulk_bk_fc_row_008 => (r#"bulk bk 008"#, r###"print -r $'tab\there'"###);
        bulk_bk_fc_row_009 => (r#"bulk bk 009"#, r###"print -r $'line1\nline2'"###);
        bulk_bk_fc_row_010 => (r#"bulk bk 010"#, r###"printf '%q\n' 'a b'"###);
        bulk_bk_fc_row_011 => (r#"bulk bk 011"#, r###"printf '%s\n' ok"###);
        bulk_bk_fc_row_012 => (r#"bulk bk 012"#, r###"print -rn -- end"###);
        bulk_bk_fc_row_013 => (r#"bulk bk 013"#, r###"print -rl -- a b"###);
        bulk_bk_fc_row_014 => (r#"bulk bk 014"#, r###"print -fc '%s\n' hi"###);
        bulk_bk_fc_row_015 => (r#"bulk bk 015"#, r###"whence -w print 2>/dev/null; print -r $?"###);
        bulk_bk_fc_row_016 => (r#"bulk bk 016"#, r###"whence -c print 2>/dev/null; print -r $?"###);
        bulk_bk_fc_row_017 => (r#"bulk bk 017"#, r###"which print 2>/dev/null; print -r $?"###);
        bulk_bk_fc_row_018 => (r#"bulk bk 018"#, r###"command -v print 2>/dev/null; print -r $?"###);
        bulk_bk_fc_row_019 => (r#"bulk bk 019"#, r###"hash -r 2>/dev/null; print -r $?"###);
        bulk_bk_fc_row_020 => (r#"bulk bk 020"#, r###"rehash 2>/dev/null; print -r $?"###);
        bulk_bk_fc_row_021 => (r#"bulk bk 021"#, r###"unalias za 2>/dev/null; alias za=1; unalias za; print -r $?"###);
        bulk_bk_fc_row_022 => (r#"bulk bk 022"#, r###"alias -L za 2>/dev/null; alias za=z; print -r $?"###);
        bulk_bk_fc_row_023 => (r#"bulk bk 023"#, r###"export ZA=1; print -r $ZA"###);
        bulk_bk_fc_row_024 => (r#"bulk bk 024"#, r###"typeset +Z ZA; ZA=1; print -r $ZA"###);
        bulk_bk_fc_row_025 => (r#"bulk bk 025"#, r###"typeset -x ZB=2; print -r $ZB"###);
        bulk_bk_fc_row_026 => (r#"bulk bk 026"#, r###"unset ZC; ZC=1; unset ZC; print -r ${+ZC}"###);
        bulk_bk_fc_row_027 => (r#"bulk bk 027"#, r###"typeset -tH hx; hx=ff; print -r $hx"###);
        bulk_bk_fc_row_028 => (r#"bulk bk 028"#, r###"shift; print -r $1; set -- a b c"###);
        bulk_bk_fc_row_029 => (r#"bulk bk 029"#, r###"(( $# )); print -r $#"###);
        bulk_bk_fc_row_030 => (r#"bulk bk 030"#, r###"print -r ${argv[1]}"###);
        bulk_bk_fc_row_031 => (r#"bulk bk 031"#, r###"print -r ${*[1]}"###);
        bulk_bk_fc_row_032 => (r#"bulk bk 032"#, r###"print -r $@[1]"###);
        bulk_bk_fc_row_033 => (r#"bulk bk 033"#, r###"print -r ${@:2}"###);
        bulk_bk_fc_row_034 => (r#"bulk bk 034"#, r###"select x in a b; do print -r $x; break; done <<< ''"###);
        bulk_bk_fc_row_035 => (r#"bulk bk 035"#, r###"zmodload zsh/zutil 2>/dev/null; print -r $?"###);
        bulk_bk_fc_row_036 => (r#"bulk bk 036"#, r###"zmodload -l 2>/dev/null | head -1; print -r $?"###);
        bulk_bk_fc_row_037 => (r#"bulk bk 037"#, r###"getconf PATH 2>/dev/null | head -c 1; print -r $?"###);
        bulk_bk_fc_row_038 => (r#"bulk bk 038"#, r###"getconf ARG_MAX 2>/dev/null; print -r $?"###);
        bulk_bk_fc_row_039 => (r#"bulk bk 039"#, r###"str=%n; print -r ${(%)str}"###);
        bulk_bk_fc_row_040 => (r#"bulk bk 040"#, r###"str=%N; print -r ${(%)str}"###);
        bulk_bk_fc_row_041 => (r#"bulk bk 041"#, r###"str=%~; print -r ${(%)str}"###);
        bulk_bk_fc_row_042 => (r#"bulk bk 042"#, r###"str=%d; print -r ${(%)str}"###);
        bulk_bk_fc_row_043 => (r#"bulk bk 043"#, r###"str=%m; print -r ${(%)str}"###);
        bulk_bk_fc_row_044 => (r#"bulk bk 044"#, r###"str=%#; print -r ${(%)str}"###);
        bulk_bk_fc_row_045 => (r#"bulk bk 045"#, r###"str=%?; print -r ${(%)str}"###);
        bulk_bk_fc_row_046 => (r#"bulk bk 046"#, r###"str=%_; print -r ${(%)str}"###);
        bulk_bk_fc_row_047 => (r#"bulk bk 047"#, r###"str=%h; print -r ${(%)str}"###);
        bulk_bk_fc_row_048 => (r#"bulk bk 048"#, r###"str=%!; print -r ${(%)str}"###);
    }
}

mod corpus_dash_fc_bulk_bl {
    use super::*;

    parity_gap_tests! {
        bulk_bl_fc_row_001 => (r#"bulk bl 001"#, r###"command -v print 2>/dev/null; print -r $?"###);
        bulk_bl_fc_row_002 => (r#"bulk bl 002"#, r###"hash -r 2>/dev/null; print -r $?"###);
        bulk_bl_fc_row_003 => (r#"bulk bl 003"#, r###"rehash 2>/dev/null; print -r $?"###);
        bulk_bl_fc_row_004 => (r#"bulk bl 004"#, r###"unalias za 2>/dev/null; alias za=1; unalias za; print -r $?"###);
        bulk_bl_fc_row_005 => (r#"bulk bl 005"#, r###"alias -L za 2>/dev/null; alias za=z; print -r $?"###);
        bulk_bl_fc_row_006 => (r#"bulk bl 006"#, r###"export ZA=1; print -r $ZA"###);
        bulk_bl_fc_row_007 => (r#"bulk bl 007"#, r###"typeset +Z ZA; ZA=1; print -r $ZA"###);
        bulk_bl_fc_row_008 => (r#"bulk bl 008"#, r###"typeset -x ZB=2; print -r $ZB"###);
        bulk_bl_fc_row_009 => (r#"bulk bl 009"#, r###"unset ZC; ZC=1; unset ZC; print -r ${+ZC}"###);
        bulk_bl_fc_row_010 => (r#"bulk bl 010"#, r###"typeset -tH hx; hx=ff; print -r $hx"###);
        bulk_bl_fc_row_011 => (r#"bulk bl 011"#, r###"shift; print -r $1; set -- a b c"###);
        bulk_bl_fc_row_012 => (r#"bulk bl 012"#, r###"(( $# )); print -r $#"###);
        bulk_bl_fc_row_013 => (r#"bulk bl 013"#, r###"print -r ${argv[1]}"###);
        bulk_bl_fc_row_014 => (r#"bulk bl 014"#, r###"print -r ${*[1]}"###);
        bulk_bl_fc_row_015 => (r#"bulk bl 015"#, r###"print -r $@[1]"###);
        bulk_bl_fc_row_016 => (r#"bulk bl 016"#, r###"print -r ${@:2}"###);
        bulk_bl_fc_row_017 => (r#"bulk bl 017"#, r###"select x in a b; do print -r $x; break; done <<< ''"###);
        bulk_bl_fc_row_018 => (r#"bulk bl 018"#, r###"zmodload zsh/zutil 2>/dev/null; print -r $?"###);
        bulk_bl_fc_row_019 => (r#"bulk bl 019"#, r###"zmodload -l 2>/dev/null | head -1; print -r $?"###);
        bulk_bl_fc_row_020 => (r#"bulk bl 020"#, r###"getconf PATH 2>/dev/null | head -c 1; print -r $?"###);
        bulk_bl_fc_row_021 => (r#"bulk bl 021"#, r###"getconf ARG_MAX 2>/dev/null; print -r $?"###);
        bulk_bl_fc_row_022 => (r#"bulk bl 022"#, r###"str=%n; print -r ${(%)str}"###);
        bulk_bl_fc_row_023 => (r#"bulk bl 023"#, r###"str=%N; print -r ${(%)str}"###);
        bulk_bl_fc_row_024 => (r#"bulk bl 024"#, r###"str=%~; print -r ${(%)str}"###);
        bulk_bl_fc_row_025 => (r#"bulk bl 025"#, r###"str=%d; print -r ${(%)str}"###);
        bulk_bl_fc_row_026 => (r#"bulk bl 026"#, r###"str=%m; print -r ${(%)str}"###);
        bulk_bl_fc_row_027 => (r#"bulk bl 027"#, r###"str=%#; print -r ${(%)str}"###);
        bulk_bl_fc_row_028 => (r#"bulk bl 028"#, r###"str=%?; print -r ${(%)str}"###);
        bulk_bl_fc_row_029 => (r#"bulk bl 029"#, r###"str=%_; print -r ${(%)str}"###);
        bulk_bl_fc_row_030 => (r#"bulk bl 030"#, r###"str=%h; print -r ${(%)str}"###);
        bulk_bl_fc_row_031 => (r#"bulk bl 031"#, r###"str=%!; print -r ${(%)str}"###);
        bulk_bl_fc_row_032 => (r#"bulk bl 032"#, r###"str=%i; print -r ${(%)str}"###);
        bulk_bl_fc_row_033 => (r#"bulk bl 033"#, r###"str=%I; print -r ${(%)str}"###);
        bulk_bl_fc_row_034 => (r#"bulk bl 034"#, r###"str=%j; print -r ${(%)str}"###);
        bulk_bl_fc_row_035 => (r#"bulk bl 035"#, r###"str=%C; print -r ${(%)str}"###);
        bulk_bl_fc_row_036 => (r#"bulk bl 036"#, r###"str=%c; print -r ${(%)str}"###);
        bulk_bl_fc_row_037 => (r#"bulk bl 037"#, r###"str=%D; print -r ${(%)str}"###);
        bulk_bl_fc_row_038 => (r#"bulk bl 038"#, r###"str=%W; print -r ${(%)str}"###);
        bulk_bl_fc_row_039 => (r#"bulk bl 039"#, r###"str=%*; print -r ${(%)str}"###);
        bulk_bl_fc_row_040 => (r#"bulk bl 040"#, r###"str=%v; print -r ${(%)str}"###);
        bulk_bl_fc_row_041 => (r#"bulk bl 041"#, r###"str=%L; print -r ${(%)str}"###);
        bulk_bl_fc_row_042 => (r#"bulk bl 042"#, r###"str=%l; print -r ${(%)str}"###);
        bulk_bl_fc_row_043 => (r#"bulk bl 043"#, r###"str=%y; print -r ${(%)str}"###);
        bulk_bl_fc_row_044 => (r#"bulk bl 044"#, r###"str=%/; print -r ${(%)str}"###);
        bulk_bl_fc_row_045 => (r#"bulk bl 045"#, r###"str=%<; print -r ${(%)str}"###);
        bulk_bl_fc_row_046 => (r#"bulk bl 046"#, r###"str=%>; print -r ${(%)str}"###);
        bulk_bl_fc_row_047 => (r#"bulk bl 047"#, r###"true; print -r $?"###);
        bulk_bl_fc_row_048 => (r#"bulk bl 048"#, r###"false; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_bm {
    use super::*;

    parity_gap_tests! {
        bulk_bm_fc_row_001 => (r#"bulk bm 001"#, r###"select x in a b; do print -r $x; break; done <<< ''"###);
        bulk_bm_fc_row_002 => (r#"bulk bm 002"#, r###"zmodload zsh/zutil 2>/dev/null; print -r $?"###);
        bulk_bm_fc_row_003 => (r#"bulk bm 003"#, r###"zmodload -l 2>/dev/null | head -1; print -r $?"###);
        bulk_bm_fc_row_004 => (r#"bulk bm 004"#, r###"getconf PATH 2>/dev/null | head -c 1; print -r $?"###);
        bulk_bm_fc_row_005 => (r#"bulk bm 005"#, r###"getconf ARG_MAX 2>/dev/null; print -r $?"###);
        bulk_bm_fc_row_006 => (r#"bulk bm 006"#, r###"str=%n; print -r ${(%)str}"###);
        bulk_bm_fc_row_007 => (r#"bulk bm 007"#, r###"str=%N; print -r ${(%)str}"###);
        bulk_bm_fc_row_008 => (r#"bulk bm 008"#, r###"str=%~; print -r ${(%)str}"###);
        bulk_bm_fc_row_009 => (r#"bulk bm 009"#, r###"str=%d; print -r ${(%)str}"###);
        bulk_bm_fc_row_010 => (r#"bulk bm 010"#, r###"str=%m; print -r ${(%)str}"###);
        bulk_bm_fc_row_011 => (r#"bulk bm 011"#, r###"str=%#; print -r ${(%)str}"###);
        bulk_bm_fc_row_012 => (r#"bulk bm 012"#, r###"str=%?; print -r ${(%)str}"###);
        bulk_bm_fc_row_013 => (r#"bulk bm 013"#, r###"str=%_; print -r ${(%)str}"###);
        bulk_bm_fc_row_014 => (r#"bulk bm 014"#, r###"str=%h; print -r ${(%)str}"###);
        bulk_bm_fc_row_015 => (r#"bulk bm 015"#, r###"str=%!; print -r ${(%)str}"###);
        bulk_bm_fc_row_016 => (r#"bulk bm 016"#, r###"str=%i; print -r ${(%)str}"###);
        bulk_bm_fc_row_017 => (r#"bulk bm 017"#, r###"str=%I; print -r ${(%)str}"###);
        bulk_bm_fc_row_018 => (r#"bulk bm 018"#, r###"str=%j; print -r ${(%)str}"###);
        bulk_bm_fc_row_019 => (r#"bulk bm 019"#, r###"str=%C; print -r ${(%)str}"###);
        bulk_bm_fc_row_020 => (r#"bulk bm 020"#, r###"str=%c; print -r ${(%)str}"###);
        bulk_bm_fc_row_021 => (r#"bulk bm 021"#, r###"str=%D; print -r ${(%)str}"###);
        bulk_bm_fc_row_022 => (r#"bulk bm 022"#, r###"str=%W; print -r ${(%)str}"###);
        bulk_bm_fc_row_023 => (r#"bulk bm 023"#, r###"str=%*; print -r ${(%)str}"###);
        bulk_bm_fc_row_024 => (r#"bulk bm 024"#, r###"str=%v; print -r ${(%)str}"###);
        bulk_bm_fc_row_025 => (r#"bulk bm 025"#, r###"str=%L; print -r ${(%)str}"###);
        bulk_bm_fc_row_026 => (r#"bulk bm 026"#, r###"str=%l; print -r ${(%)str}"###);
        bulk_bm_fc_row_027 => (r#"bulk bm 027"#, r###"str=%y; print -r ${(%)str}"###);
        bulk_bm_fc_row_028 => (r#"bulk bm 028"#, r###"str=%/; print -r ${(%)str}"###);
        bulk_bm_fc_row_029 => (r#"bulk bm 029"#, r###"str=%<; print -r ${(%)str}"###);
        bulk_bm_fc_row_030 => (r#"bulk bm 030"#, r###"str=%>; print -r ${(%)str}"###);
        bulk_bm_fc_row_031 => (r#"bulk bm 031"#, r###"true; print -r $?"###);
        bulk_bm_fc_row_032 => (r#"bulk bm 032"#, r###"false; print -r $?"###);
        bulk_bm_fc_row_033 => (r#"bulk bm 033"#, r###"print -r hello"###);
        bulk_bm_fc_row_034 => (r#"bulk bm 034"#, r###"echo one two"###);
        bulk_bm_fc_row_035 => (r#"bulk bm 035"#, r###"test 1 -eq 1; print -r $?"###);
        bulk_bm_fc_row_036 => (r#"bulk bm 036"#, r###"[ 1 -eq 1 ]; print -r $?"###);
        bulk_bm_fc_row_037 => (r#"bulk bm 037"#, r###"command true; print -r $?"###);
        bulk_bm_fc_row_038 => (r#"bulk bm 038"#, r###"builtin true; print -r $?"###);
        bulk_bm_fc_row_039 => (r#"bulk bm 039"#, r###"if true; then echo t; fi"###);
        bulk_bm_fc_row_040 => (r#"bulk bm 040"#, r###"if false; then echo e; else echo f; fi"###);
        bulk_bm_fc_row_041 => (r#"bulk bm 041"#, r###"for i in a b; do print -r $i; done"###);
        bulk_bm_fc_row_042 => (r#"bulk bm 042"#, r###"i=0; while (( i < 2 )); do print -r $i; (( i++ )); done"###);
        bulk_bm_fc_row_043 => (r#"bulk bm 043"#, r###"repeat 2; do print -r r; done"###);
        bulk_bm_fc_row_044 => (r#"bulk bm 044"#, r###"case x in (x) echo ok ;; esac"###);
        bulk_bm_fc_row_045 => (r#"bulk bm 045"#, r###"[[ 1 -eq 1 ]] && echo and || echo or"###);
        bulk_bm_fc_row_046 => (r#"bulk bm 046"#, r###"[[ 1 -eq 2 ]] || echo orbranch"###);
        bulk_bm_fc_row_047 => (r#"bulk bm 047"#, r###"{ echo a; echo b; }"###);
        bulk_bm_fc_row_048 => (r#"bulk bm 048"#, r###"(echo sub)"###);
    }
}

mod corpus_dash_fc_bulk_bn {
    use super::*;

    parity_gap_tests! {
        bulk_bn_fc_row_001 => (r#"bulk bn 001"#, r###"str=%i; print -r ${(%)str}"###);
        bulk_bn_fc_row_002 => (r#"bulk bn 002"#, r###"str=%I; print -r ${(%)str}"###);
        bulk_bn_fc_row_003 => (r#"bulk bn 003"#, r###"str=%j; print -r ${(%)str}"###);
        bulk_bn_fc_row_004 => (r#"bulk bn 004"#, r###"str=%C; print -r ${(%)str}"###);
        bulk_bn_fc_row_005 => (r#"bulk bn 005"#, r###"str=%c; print -r ${(%)str}"###);
        bulk_bn_fc_row_006 => (r#"bulk bn 006"#, r###"str=%D; print -r ${(%)str}"###);
        bulk_bn_fc_row_007 => (r#"bulk bn 007"#, r###"str=%W; print -r ${(%)str}"###);
        bulk_bn_fc_row_008 => (r#"bulk bn 008"#, r###"str=%*; print -r ${(%)str}"###);
        bulk_bn_fc_row_009 => (r#"bulk bn 009"#, r###"str=%v; print -r ${(%)str}"###);
        bulk_bn_fc_row_010 => (r#"bulk bn 010"#, r###"str=%L; print -r ${(%)str}"###);
        bulk_bn_fc_row_011 => (r#"bulk bn 011"#, r###"str=%l; print -r ${(%)str}"###);
        bulk_bn_fc_row_012 => (r#"bulk bn 012"#, r###"str=%y; print -r ${(%)str}"###);
        bulk_bn_fc_row_013 => (r#"bulk bn 013"#, r###"str=%/; print -r ${(%)str}"###);
        bulk_bn_fc_row_014 => (r#"bulk bn 014"#, r###"str=%<; print -r ${(%)str}"###);
        bulk_bn_fc_row_015 => (r#"bulk bn 015"#, r###"str=%>; print -r ${(%)str}"###);
        bulk_bn_fc_row_016 => (r#"bulk bn 016"#, r###"true; print -r $?"###);
        bulk_bn_fc_row_017 => (r#"bulk bn 017"#, r###"false; print -r $?"###);
        bulk_bn_fc_row_018 => (r#"bulk bn 018"#, r###"print -r hello"###);
        bulk_bn_fc_row_019 => (r#"bulk bn 019"#, r###"echo one two"###);
        bulk_bn_fc_row_020 => (r#"bulk bn 020"#, r###"test 1 -eq 1; print -r $?"###);
        bulk_bn_fc_row_021 => (r#"bulk bn 021"#, r###"[ 1 -eq 1 ]; print -r $?"###);
        bulk_bn_fc_row_022 => (r#"bulk bn 022"#, r###"command true; print -r $?"###);
        bulk_bn_fc_row_023 => (r#"bulk bn 023"#, r###"builtin true; print -r $?"###);
        bulk_bn_fc_row_024 => (r#"bulk bn 024"#, r###"if true; then echo t; fi"###);
        bulk_bn_fc_row_025 => (r#"bulk bn 025"#, r###"if false; then echo e; else echo f; fi"###);
        bulk_bn_fc_row_026 => (r#"bulk bn 026"#, r###"for i in a b; do print -r $i; done"###);
        bulk_bn_fc_row_027 => (r#"bulk bn 027"#, r###"i=0; while (( i < 2 )); do print -r $i; (( i++ )); done"###);
        bulk_bn_fc_row_028 => (r#"bulk bn 028"#, r###"repeat 2; do print -r r; done"###);
        bulk_bn_fc_row_029 => (r#"bulk bn 029"#, r###"case x in (x) echo ok ;; esac"###);
        bulk_bn_fc_row_030 => (r#"bulk bn 030"#, r###"[[ 1 -eq 1 ]] && echo and || echo or"###);
        bulk_bn_fc_row_031 => (r#"bulk bn 031"#, r###"[[ 1 -eq 2 ]] || echo orbranch"###);
        bulk_bn_fc_row_032 => (r#"bulk bn 032"#, r###"{ echo a; echo b; }"###);
        bulk_bn_fc_row_033 => (r#"bulk bn 033"#, r###"(echo sub)"###);
        bulk_bn_fc_row_034 => (r#"bulk bn 034"#, r###"(( 1 )) || echo no"###);
        bulk_bn_fc_row_035 => (r#"bulk bn 035"#, r###"(( 0 )) && echo no"###);
        bulk_bn_fc_row_036 => (r#"bulk bn 036"#, r###"print -r $(( 1 + 2 ))"###);
        bulk_bn_fc_row_037 => (r#"bulk bn 037"#, r###"print -r $(( 17 % 5 ))"###);
        bulk_bn_fc_row_038 => (r#"bulk bn 038"#, r###"print -r $(( 2 ** 8 ))"###);
        bulk_bn_fc_row_039 => (r#"bulk bn 039"#, r###"print -r $(( 1 && 0 || 2 ))"###);
        bulk_bn_fc_row_040 => (r#"bulk bn 040"#, r###"print -r $(( !0 ))"###);
        bulk_bn_fc_row_041 => (r#"bulk bn 041"#, r###"integer n=5; (( n += 2 )); print -r $n"###);
        bulk_bn_fc_row_042 => (r#"bulk bn 042"#, r###"integer n=5; (( n -= 1 )); print -r $n"###);
        bulk_bn_fc_row_043 => (r#"bulk bn 043"#, r###"integer n=5; (( n *= 2 )); print -r $n"###);
        bulk_bn_fc_row_044 => (r#"bulk bn 044"#, r###"integer n=5; (( n |= 3 )); print -r $n"###);
        bulk_bn_fc_row_045 => (r#"bulk bn 045"#, r###"integer n=5; (( n &= 3 )); print -r $n"###);
        bulk_bn_fc_row_046 => (r#"bulk bn 046"#, r###"print -r $(( true ))"###);
        bulk_bn_fc_row_047 => (r#"bulk bn 047"#, r###"print -r $(( false ))"###);
        bulk_bn_fc_row_048 => (r#"bulk bn 048"#, r###"[[ -e / ]]; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_bo {
    use super::*;

    parity_gap_tests! {
        bulk_bo_fc_row_001 => (r#"bulk bo 001"#, r###"str=%l; print -r ${(%)str}"###);
        bulk_bo_fc_row_002 => (r#"bulk bo 002"#, r###"str=%y; print -r ${(%)str}"###);
        bulk_bo_fc_row_003 => (r#"bulk bo 003"#, r###"str=%/; print -r ${(%)str}"###);
        bulk_bo_fc_row_004 => (r#"bulk bo 004"#, r###"str=%<; print -r ${(%)str}"###);
        bulk_bo_fc_row_005 => (r#"bulk bo 005"#, r###"str=%>; print -r ${(%)str}"###);
        bulk_bo_fc_row_006 => (r#"bulk bo 006"#, r###"true; print -r $?"###);
        bulk_bo_fc_row_007 => (r#"bulk bo 007"#, r###"false; print -r $?"###);
        bulk_bo_fc_row_008 => (r#"bulk bo 008"#, r###"print -r hello"###);
        bulk_bo_fc_row_009 => (r#"bulk bo 009"#, r###"echo one two"###);
        bulk_bo_fc_row_010 => (r#"bulk bo 010"#, r###"test 1 -eq 1; print -r $?"###);
        bulk_bo_fc_row_011 => (r#"bulk bo 011"#, r###"[ 1 -eq 1 ]; print -r $?"###);
        bulk_bo_fc_row_012 => (r#"bulk bo 012"#, r###"command true; print -r $?"###);
        bulk_bo_fc_row_013 => (r#"bulk bo 013"#, r###"builtin true; print -r $?"###);
        bulk_bo_fc_row_014 => (r#"bulk bo 014"#, r###"if true; then echo t; fi"###);
        bulk_bo_fc_row_015 => (r#"bulk bo 015"#, r###"if false; then echo e; else echo f; fi"###);
        bulk_bo_fc_row_016 => (r#"bulk bo 016"#, r###"for i in a b; do print -r $i; done"###);
        bulk_bo_fc_row_017 => (r#"bulk bo 017"#, r###"i=0; while (( i < 2 )); do print -r $i; (( i++ )); done"###);
        bulk_bo_fc_row_018 => (r#"bulk bo 018"#, r###"repeat 2; do print -r r; done"###);
        bulk_bo_fc_row_019 => (r#"bulk bo 019"#, r###"case x in (x) echo ok ;; esac"###);
        bulk_bo_fc_row_020 => (r#"bulk bo 020"#, r###"[[ 1 -eq 1 ]] && echo and || echo or"###);
        bulk_bo_fc_row_021 => (r#"bulk bo 021"#, r###"[[ 1 -eq 2 ]] || echo orbranch"###);
        bulk_bo_fc_row_022 => (r#"bulk bo 022"#, r###"{ echo a; echo b; }"###);
        bulk_bo_fc_row_023 => (r#"bulk bo 023"#, r###"(echo sub)"###);
        bulk_bo_fc_row_024 => (r#"bulk bo 024"#, r###"(( 1 )) || echo no"###);
        bulk_bo_fc_row_025 => (r#"bulk bo 025"#, r###"(( 0 )) && echo no"###);
        bulk_bo_fc_row_026 => (r#"bulk bo 026"#, r###"print -r $(( 1 + 2 ))"###);
        bulk_bo_fc_row_027 => (r#"bulk bo 027"#, r###"print -r $(( 17 % 5 ))"###);
        bulk_bo_fc_row_028 => (r#"bulk bo 028"#, r###"print -r $(( 2 ** 8 ))"###);
        bulk_bo_fc_row_029 => (r#"bulk bo 029"#, r###"print -r $(( 1 && 0 || 2 ))"###);
        bulk_bo_fc_row_030 => (r#"bulk bo 030"#, r###"print -r $(( !0 ))"###);
        bulk_bo_fc_row_031 => (r#"bulk bo 031"#, r###"integer n=5; (( n += 2 )); print -r $n"###);
        bulk_bo_fc_row_032 => (r#"bulk bo 032"#, r###"integer n=5; (( n -= 1 )); print -r $n"###);
        bulk_bo_fc_row_033 => (r#"bulk bo 033"#, r###"integer n=5; (( n *= 2 )); print -r $n"###);
        bulk_bo_fc_row_034 => (r#"bulk bo 034"#, r###"integer n=5; (( n |= 3 )); print -r $n"###);
        bulk_bo_fc_row_035 => (r#"bulk bo 035"#, r###"integer n=5; (( n &= 3 )); print -r $n"###);
        bulk_bo_fc_row_036 => (r#"bulk bo 036"#, r###"print -r $(( true ))"###);
        bulk_bo_fc_row_037 => (r#"bulk bo 037"#, r###"print -r $(( false ))"###);
        bulk_bo_fc_row_038 => (r#"bulk bo 038"#, r###"[[ -e / ]]; print -r $?"###);
        bulk_bo_fc_row_039 => (r#"bulk bo 039"#, r###"[[ -d /tmp ]]; print -r $?"###);
        bulk_bo_fc_row_040 => (r#"bulk bo 040"#, r###"[[ -f /etc/hosts ]]; print -r $?"###);
        bulk_bo_fc_row_041 => (r#"bulk bo 041"#, r###"[[ -r /etc/hosts ]]; print -r $?"###);
        bulk_bo_fc_row_042 => (r#"bulk bo 042"#, r###"[[ -w /tmp ]]; print -r $?"###);
        bulk_bo_fc_row_043 => (r#"bulk bo 043"#, r###"[[ -x /bin/sh ]]; print -r $?"###);
        bulk_bo_fc_row_044 => (r#"bulk bo 044"#, r###"[[ 42 = <-> ]]; print -r $?"###);
        bulk_bo_fc_row_045 => (r#"bulk bo 045"#, r###"[[ abc = <-> ]]; print -r $?"###);
        bulk_bo_fc_row_046 => (r#"bulk bo 046"#, r####"[[ host = ##host ]]; print -r $?"####);
        bulk_bo_fc_row_047 => (r#"bulk bo 047"#, r###"[[ -v x ]]; print -r $?; x=1"###);
        bulk_bo_fc_row_048 => (r#"bulk bo 048"#, r###"unset y; [[ -v y ]]; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_bp {
    use super::*;

    parity_gap_tests! {
        bulk_bp_fc_row_001 => (r#"bulk bp 001"#, r###"i=0; while (( i < 2 )); do print -r $i; (( i++ )); done"###);
        bulk_bp_fc_row_002 => (r#"bulk bp 002"#, r###"repeat 2; do print -r r; done"###);
        bulk_bp_fc_row_003 => (r#"bulk bp 003"#, r###"case x in (x) echo ok ;; esac"###);
        bulk_bp_fc_row_004 => (r#"bulk bp 004"#, r###"[[ 1 -eq 1 ]] && echo and || echo or"###);
        bulk_bp_fc_row_005 => (r#"bulk bp 005"#, r###"[[ 1 -eq 2 ]] || echo orbranch"###);
        bulk_bp_fc_row_006 => (r#"bulk bp 006"#, r###"{ echo a; echo b; }"###);
        bulk_bp_fc_row_007 => (r#"bulk bp 007"#, r###"(echo sub)"###);
        bulk_bp_fc_row_008 => (r#"bulk bp 008"#, r###"(( 1 )) || echo no"###);
        bulk_bp_fc_row_009 => (r#"bulk bp 009"#, r###"(( 0 )) && echo no"###);
        bulk_bp_fc_row_010 => (r#"bulk bp 010"#, r###"print -r $(( 1 + 2 ))"###);
        bulk_bp_fc_row_011 => (r#"bulk bp 011"#, r###"print -r $(( 17 % 5 ))"###);
        bulk_bp_fc_row_012 => (r#"bulk bp 012"#, r###"print -r $(( 2 ** 8 ))"###);
        bulk_bp_fc_row_013 => (r#"bulk bp 013"#, r###"print -r $(( 1 && 0 || 2 ))"###);
        bulk_bp_fc_row_014 => (r#"bulk bp 014"#, r###"print -r $(( !0 ))"###);
        bulk_bp_fc_row_015 => (r#"bulk bp 015"#, r###"integer n=5; (( n += 2 )); print -r $n"###);
        bulk_bp_fc_row_016 => (r#"bulk bp 016"#, r###"integer n=5; (( n -= 1 )); print -r $n"###);
        bulk_bp_fc_row_017 => (r#"bulk bp 017"#, r###"integer n=5; (( n *= 2 )); print -r $n"###);
        bulk_bp_fc_row_018 => (r#"bulk bp 018"#, r###"integer n=5; (( n |= 3 )); print -r $n"###);
        bulk_bp_fc_row_019 => (r#"bulk bp 019"#, r###"integer n=5; (( n &= 3 )); print -r $n"###);
        bulk_bp_fc_row_020 => (r#"bulk bp 020"#, r###"print -r $(( true ))"###);
        bulk_bp_fc_row_021 => (r#"bulk bp 021"#, r###"print -r $(( false ))"###);
        bulk_bp_fc_row_022 => (r#"bulk bp 022"#, r###"[[ -e / ]]; print -r $?"###);
        bulk_bp_fc_row_023 => (r#"bulk bp 023"#, r###"[[ -d /tmp ]]; print -r $?"###);
        bulk_bp_fc_row_024 => (r#"bulk bp 024"#, r###"[[ -f /etc/hosts ]]; print -r $?"###);
        bulk_bp_fc_row_025 => (r#"bulk bp 025"#, r###"[[ -r /etc/hosts ]]; print -r $?"###);
        bulk_bp_fc_row_026 => (r#"bulk bp 026"#, r###"[[ -w /tmp ]]; print -r $?"###);
        bulk_bp_fc_row_027 => (r#"bulk bp 027"#, r###"[[ -x /bin/sh ]]; print -r $?"###);
        bulk_bp_fc_row_028 => (r#"bulk bp 028"#, r###"[[ 42 = <-> ]]; print -r $?"###);
        bulk_bp_fc_row_029 => (r#"bulk bp 029"#, r###"[[ abc = <-> ]]; print -r $?"###);
        bulk_bp_fc_row_030 => (r#"bulk bp 030"#, r####"[[ host = ##host ]]; print -r $?"####);
        bulk_bp_fc_row_031 => (r#"bulk bp 031"#, r###"[[ -v x ]]; print -r $?; x=1"###);
        bulk_bp_fc_row_032 => (r#"bulk bp 032"#, r###"unset y; [[ -v y ]]; print -r $?"###);
        bulk_bp_fc_row_033 => (r#"bulk bp 033"#, r###"setopt extendedglob; [[ abc = (#i)ABC ]]; print -r $?"###);
        bulk_bp_fc_row_034 => (r#"bulk bp 034"#, r###"setopt extendedglob; [[ foo = (#b)oo ]]; print -r $?"###);
        bulk_bp_fc_row_035 => (r#"bulk bp 035"#, r###"[[ abc = a* ]]; print -r $?"###);
        bulk_bp_fc_row_036 => (r#"bulk bp 036"#, r###"[[ abc =~ ^a ]]; print -r $?"###);
        bulk_bp_fc_row_037 => (r#"bulk bp 037"#, r###"[[ -z '' ]]; print -r $?"###);
        bulk_bp_fc_row_038 => (r#"bulk bp 038"#, r###"[[ -n abc ]]; print -r $?"###);
        bulk_bp_fc_row_039 => (r#"bulk bp 039"#, r###"typeset -i n=10; print -r $n"###);
        bulk_bp_fc_row_040 => (r#"bulk bp 040"#, r###"typeset -l n=AbC; print -r $n"###);
        bulk_bp_fc_row_041 => (r#"bulk bp 041"#, r###"typeset -u n=xy; print -r $n"###);
        bulk_bp_fc_row_042 => (r#"bulk bp 042"#, r###"typeset -Z5 n=7; print -r $n"###);
        bulk_bp_fc_row_043 => (r#"bulk bp 043"#, r###"typeset -a a=(x y); print -r ${a[2]}"###);
        bulk_bp_fc_row_044 => (r#"bulk bp 044"#, r###"arr=(1 2); arr+=3; print -r ${arr[@]}"###);
        bulk_bp_fc_row_045 => (r#"bulk bp 045"#, r###"arr=(1); arr[1]+=2; print -r ${arr[1]}"###);
        bulk_bp_fc_row_046 => (r#"bulk bp 046"#, r###"unset v; print -r ${v:-def}"###);
        bulk_bp_fc_row_047 => (r#"bulk bp 047"#, r###"v=set; print -r ${v:+yes}"###);
        bulk_bp_fc_row_048 => (r#"bulk bp 048"#, r###"unset v; : ${v::=def}; print -r $v"###);
    }
}

mod corpus_dash_fc_bulk_bq {
    use super::*;

    parity_gap_tests! {
        bulk_bq_fc_row_001 => (r#"bulk bq 001"#, r###"integer n=5; (( n |= 3 )); print -r $n"###);
        bulk_bq_fc_row_002 => (r#"bulk bq 002"#, r###"integer n=5; (( n &= 3 )); print -r $n"###);
        bulk_bq_fc_row_003 => (r#"bulk bq 003"#, r###"print -r $(( true ))"###);
        bulk_bq_fc_row_004 => (r#"bulk bq 004"#, r###"print -r $(( false ))"###);
        bulk_bq_fc_row_005 => (r#"bulk bq 005"#, r###"[[ -e / ]]; print -r $?"###);
        bulk_bq_fc_row_006 => (r#"bulk bq 006"#, r###"[[ -d /tmp ]]; print -r $?"###);
        bulk_bq_fc_row_007 => (r#"bulk bq 007"#, r###"[[ -f /etc/hosts ]]; print -r $?"###);
        bulk_bq_fc_row_008 => (r#"bulk bq 008"#, r###"[[ -r /etc/hosts ]]; print -r $?"###);
        bulk_bq_fc_row_009 => (r#"bulk bq 009"#, r###"[[ -w /tmp ]]; print -r $?"###);
        bulk_bq_fc_row_010 => (r#"bulk bq 010"#, r###"[[ -x /bin/sh ]]; print -r $?"###);
        bulk_bq_fc_row_011 => (r#"bulk bq 011"#, r###"[[ 42 = <-> ]]; print -r $?"###);
        bulk_bq_fc_row_012 => (r#"bulk bq 012"#, r###"[[ abc = <-> ]]; print -r $?"###);
        bulk_bq_fc_row_013 => (r#"bulk bq 013"#, r####"[[ host = ##host ]]; print -r $?"####);
        bulk_bq_fc_row_014 => (r#"bulk bq 014"#, r###"[[ -v x ]]; print -r $?; x=1"###);
        bulk_bq_fc_row_015 => (r#"bulk bq 015"#, r###"unset y; [[ -v y ]]; print -r $?"###);
        bulk_bq_fc_row_016 => (r#"bulk bq 016"#, r###"setopt extendedglob; [[ abc = (#i)ABC ]]; print -r $?"###);
        bulk_bq_fc_row_017 => (r#"bulk bq 017"#, r###"setopt extendedglob; [[ foo = (#b)oo ]]; print -r $?"###);
        bulk_bq_fc_row_018 => (r#"bulk bq 018"#, r###"[[ abc = a* ]]; print -r $?"###);
        bulk_bq_fc_row_019 => (r#"bulk bq 019"#, r###"[[ abc =~ ^a ]]; print -r $?"###);
        bulk_bq_fc_row_020 => (r#"bulk bq 020"#, r###"[[ -z '' ]]; print -r $?"###);
        bulk_bq_fc_row_021 => (r#"bulk bq 021"#, r###"[[ -n abc ]]; print -r $?"###);
        bulk_bq_fc_row_022 => (r#"bulk bq 022"#, r###"typeset -i n=10; print -r $n"###);
        bulk_bq_fc_row_023 => (r#"bulk bq 023"#, r###"typeset -l n=AbC; print -r $n"###);
        bulk_bq_fc_row_024 => (r#"bulk bq 024"#, r###"typeset -u n=xy; print -r $n"###);
        bulk_bq_fc_row_025 => (r#"bulk bq 025"#, r###"typeset -Z5 n=7; print -r $n"###);
        bulk_bq_fc_row_026 => (r#"bulk bq 026"#, r###"typeset -a a=(x y); print -r ${a[2]}"###);
        bulk_bq_fc_row_027 => (r#"bulk bq 027"#, r###"arr=(1 2); arr+=3; print -r ${arr[@]}"###);
        bulk_bq_fc_row_028 => (r#"bulk bq 028"#, r###"arr=(1); arr[1]+=2; print -r ${arr[1]}"###);
        bulk_bq_fc_row_029 => (r#"bulk bq 029"#, r###"unset v; print -r ${v:-def}"###);
        bulk_bq_fc_row_030 => (r#"bulk bq 030"#, r###"v=set; print -r ${v:+yes}"###);
        bulk_bq_fc_row_031 => (r#"bulk bq 031"#, r###"unset v; : ${v::=def}; print -r $v"###);
        bulk_bq_fc_row_032 => (r#"bulk bq 032"#, r###"print -r ${PWD:h}"###);
        bulk_bq_fc_row_033 => (r#"bulk bq 033"#, r###"print -r ${PWD:t}"###);
        bulk_bq_fc_row_034 => (r#"bulk bq 034"#, r###"true | true; print -r $?"###);
        bulk_bq_fc_row_035 => (r#"bulk bq 035"#, r###"true | false; print -r $?"###);
        bulk_bq_fc_row_036 => (r#"bulk bq 036"#, r###"print -r ${pipestatus[1]}; true | false"###);
        bulk_bq_fc_row_037 => (r#"bulk bq 037"#, r###"print -r ${#pipestatus}; true | true | true"###);
        bulk_bq_fc_row_038 => (r#"bulk bq 038"#, r###"set -- a b c; shift; print -r $1"###);
        bulk_bq_fc_row_039 => (r#"bulk bq 039"#, r###"set -- a b c; shift 2; print -r $#"###);
        bulk_bq_fc_row_040 => (r#"bulk bq 040"#, r###"fn(){ print -r $1; }; fn x"###);
        bulk_bq_fc_row_041 => (r#"bulk bq 041"#, r###"fn(){ local x=2; print -r $x; }; fn"###);
        bulk_bq_fc_row_042 => (r#"bulk bq 042"#, r###"fn(){ return 2; }; fn; print -r $?"###);
        bulk_bq_fc_row_043 => (r#"bulk bq 043"#, r###"print -r ${(q)x}; x=hi"###);
        bulk_bq_fc_row_044 => (r#"bulk bq 044"#, r###"print -r ${(qq)x}; x=hi"###);
        bulk_bq_fc_row_045 => (r#"bulk bq 045"#, r###"x=hi; print -r ${(q-)x}"###);
        bulk_bq_fc_row_046 => (r#"bulk bq 046"#, r###"x=hi; print -r ${(q+)x}"###);
        bulk_bq_fc_row_047 => (r#"bulk bq 047"#, r###"print -r ${(w)w}; w=a b c"###);
        bulk_bq_fc_row_048 => (r#"bulk bq 048"#, r###"print -r ${(u)a}; a=(a a b)"###);
    }
}

mod corpus_dash_fc_bulk_br {
    use super::*;

    parity_gap_tests! {
        bulk_br_fc_row_001 => (r#"bulk br 001"#, r###"[[ abc = a* ]]; print -r $?"###);
        bulk_br_fc_row_002 => (r#"bulk br 002"#, r###"[[ abc =~ ^a ]]; print -r $?"###);
        bulk_br_fc_row_003 => (r#"bulk br 003"#, r###"[[ -z '' ]]; print -r $?"###);
        bulk_br_fc_row_004 => (r#"bulk br 004"#, r###"[[ -n abc ]]; print -r $?"###);
        bulk_br_fc_row_005 => (r#"bulk br 005"#, r###"typeset -i n=10; print -r $n"###);
        bulk_br_fc_row_006 => (r#"bulk br 006"#, r###"typeset -l n=AbC; print -r $n"###);
        bulk_br_fc_row_007 => (r#"bulk br 007"#, r###"typeset -u n=xy; print -r $n"###);
        bulk_br_fc_row_008 => (r#"bulk br 008"#, r###"typeset -Z5 n=7; print -r $n"###);
        bulk_br_fc_row_009 => (r#"bulk br 009"#, r###"typeset -a a=(x y); print -r ${a[2]}"###);
        bulk_br_fc_row_010 => (r#"bulk br 010"#, r###"arr=(1 2); arr+=3; print -r ${arr[@]}"###);
        bulk_br_fc_row_011 => (r#"bulk br 011"#, r###"arr=(1); arr[1]+=2; print -r ${arr[1]}"###);
        bulk_br_fc_row_012 => (r#"bulk br 012"#, r###"unset v; print -r ${v:-def}"###);
        bulk_br_fc_row_013 => (r#"bulk br 013"#, r###"v=set; print -r ${v:+yes}"###);
        bulk_br_fc_row_014 => (r#"bulk br 014"#, r###"unset v; : ${v::=def}; print -r $v"###);
        bulk_br_fc_row_015 => (r#"bulk br 015"#, r###"print -r ${PWD:h}"###);
        bulk_br_fc_row_016 => (r#"bulk br 016"#, r###"print -r ${PWD:t}"###);
        bulk_br_fc_row_017 => (r#"bulk br 017"#, r###"true | true; print -r $?"###);
        bulk_br_fc_row_018 => (r#"bulk br 018"#, r###"true | false; print -r $?"###);
        bulk_br_fc_row_019 => (r#"bulk br 019"#, r###"print -r ${pipestatus[1]}; true | false"###);
        bulk_br_fc_row_020 => (r#"bulk br 020"#, r###"print -r ${#pipestatus}; true | true | true"###);
        bulk_br_fc_row_021 => (r#"bulk br 021"#, r###"set -- a b c; shift; print -r $1"###);
        bulk_br_fc_row_022 => (r#"bulk br 022"#, r###"set -- a b c; shift 2; print -r $#"###);
        bulk_br_fc_row_023 => (r#"bulk br 023"#, r###"fn(){ print -r $1; }; fn x"###);
        bulk_br_fc_row_024 => (r#"bulk br 024"#, r###"fn(){ local x=2; print -r $x; }; fn"###);
        bulk_br_fc_row_025 => (r#"bulk br 025"#, r###"fn(){ return 2; }; fn; print -r $?"###);
        bulk_br_fc_row_026 => (r#"bulk br 026"#, r###"print -r ${(q)x}; x=hi"###);
        bulk_br_fc_row_027 => (r#"bulk br 027"#, r###"print -r ${(qq)x}; x=hi"###);
        bulk_br_fc_row_028 => (r#"bulk br 028"#, r###"x=hi; print -r ${(q-)x}"###);
        bulk_br_fc_row_029 => (r#"bulk br 029"#, r###"x=hi; print -r ${(q+)x}"###);
        bulk_br_fc_row_030 => (r#"bulk br 030"#, r###"print -r ${(w)w}; w=a b c"###);
        bulk_br_fc_row_031 => (r#"bulk br 031"#, r###"print -r ${(u)a}; a=(a a b)"###);
        bulk_br_fc_row_032 => (r#"bulk br 032"#, r###"print -r ${(o)a}; a=(c b a)"###);
        bulk_br_fc_row_033 => (r#"bulk br 033"#, r###"print -r ${(j:,:)a}; a=(x y)"###);
        bulk_br_fc_row_034 => (r#"bulk br 034"#, r###"arr=(a b c); print -r ${arr[(I)b]}"###);
        bulk_br_fc_row_035 => (r#"bulk br 035"#, r###"arr=(a b c); print -r ${arr[(R)b]}"###);
        bulk_br_fc_row_036 => (r#"bulk br 036"#, r###"arr=(9 8 7); print -r ${arr[-2,-1]}"###);
        bulk_br_fc_row_037 => (r#"bulk br 037"#, r###"typeset -A h; h=(k v); print -r ${(k)h}"###);
        bulk_br_fc_row_038 => (r#"bulk br 038"#, r###"typeset -A h; h=(a 1 b 2); print -r ${(kv)h}"###);
        bulk_br_fc_row_039 => (r#"bulk br 039"#, r###"print -r ${+options}"###);
        bulk_br_fc_row_040 => (r#"bulk br 040"#, r###"print -r ${+parameters}"###);
        bulk_br_fc_row_041 => (r#"bulk br 041"#, r###"print -r ${+aliases}"###);
        bulk_br_fc_row_042 => (r#"bulk br 042"#, r###"print -r ${+functions}"###);
        bulk_br_fc_row_043 => (r#"bulk br 043"#, r###"print -r $ZSH_NAME"###);
        bulk_br_fc_row_044 => (r#"bulk br 044"#, r###"print -r ${ZSH_VERSION%%.*}"###);
        bulk_br_fc_row_045 => (r#"bulk br 045"#, r###"whence -w print"###);
        bulk_br_fc_row_046 => (r#"bulk br 046"#, r###"command -v true"###);
        bulk_br_fc_row_047 => (r#"bulk br 047"#, r###"emulate -L zsh; print -r $?"###);
        bulk_br_fc_row_048 => (r#"bulk br 048"#, r###"alias za='echo z'; za; unalias za 2>/dev/null"###);
    }
}

mod corpus_dash_fc_bulk_bs {
    use super::*;

    parity_gap_tests! {
        bulk_bs_fc_row_001 => (r#"bulk bs 001"#, r###"true | false; print -r $?"###);
        bulk_bs_fc_row_002 => (r#"bulk bs 002"#, r###"print -r ${pipestatus[1]}; true | false"###);
        bulk_bs_fc_row_003 => (r#"bulk bs 003"#, r###"print -r ${#pipestatus}; true | true | true"###);
        bulk_bs_fc_row_004 => (r#"bulk bs 004"#, r###"set -- a b c; shift; print -r $1"###);
        bulk_bs_fc_row_005 => (r#"bulk bs 005"#, r###"set -- a b c; shift 2; print -r $#"###);
        bulk_bs_fc_row_006 => (r#"bulk bs 006"#, r###"fn(){ print -r $1; }; fn x"###);
        bulk_bs_fc_row_007 => (r#"bulk bs 007"#, r###"fn(){ local x=2; print -r $x; }; fn"###);
        bulk_bs_fc_row_008 => (r#"bulk bs 008"#, r###"fn(){ return 2; }; fn; print -r $?"###);
        bulk_bs_fc_row_009 => (r#"bulk bs 009"#, r###"print -r ${(q)x}; x=hi"###);
        bulk_bs_fc_row_010 => (r#"bulk bs 010"#, r###"print -r ${(qq)x}; x=hi"###);
        bulk_bs_fc_row_011 => (r#"bulk bs 011"#, r###"x=hi; print -r ${(q-)x}"###);
        bulk_bs_fc_row_012 => (r#"bulk bs 012"#, r###"x=hi; print -r ${(q+)x}"###);
        bulk_bs_fc_row_013 => (r#"bulk bs 013"#, r###"print -r ${(w)w}; w=a b c"###);
        bulk_bs_fc_row_014 => (r#"bulk bs 014"#, r###"print -r ${(u)a}; a=(a a b)"###);
        bulk_bs_fc_row_015 => (r#"bulk bs 015"#, r###"print -r ${(o)a}; a=(c b a)"###);
        bulk_bs_fc_row_016 => (r#"bulk bs 016"#, r###"print -r ${(j:,:)a}; a=(x y)"###);
        bulk_bs_fc_row_017 => (r#"bulk bs 017"#, r###"arr=(a b c); print -r ${arr[(I)b]}"###);
        bulk_bs_fc_row_018 => (r#"bulk bs 018"#, r###"arr=(a b c); print -r ${arr[(R)b]}"###);
        bulk_bs_fc_row_019 => (r#"bulk bs 019"#, r###"arr=(9 8 7); print -r ${arr[-2,-1]}"###);
        bulk_bs_fc_row_020 => (r#"bulk bs 020"#, r###"typeset -A h; h=(k v); print -r ${(k)h}"###);
        bulk_bs_fc_row_021 => (r#"bulk bs 021"#, r###"typeset -A h; h=(a 1 b 2); print -r ${(kv)h}"###);
        bulk_bs_fc_row_022 => (r#"bulk bs 022"#, r###"print -r ${+options}"###);
        bulk_bs_fc_row_023 => (r#"bulk bs 023"#, r###"print -r ${+parameters}"###);
        bulk_bs_fc_row_024 => (r#"bulk bs 024"#, r###"print -r ${+aliases}"###);
        bulk_bs_fc_row_025 => (r#"bulk bs 025"#, r###"print -r ${+functions}"###);
        bulk_bs_fc_row_026 => (r#"bulk bs 026"#, r###"print -r $ZSH_NAME"###);
        bulk_bs_fc_row_027 => (r#"bulk bs 027"#, r###"print -r ${ZSH_VERSION%%.*}"###);
        bulk_bs_fc_row_028 => (r#"bulk bs 028"#, r###"whence -w print"###);
        bulk_bs_fc_row_029 => (r#"bulk bs 029"#, r###"command -v true"###);
        bulk_bs_fc_row_030 => (r#"bulk bs 030"#, r###"emulate -L zsh; print -r $?"###);
        bulk_bs_fc_row_031 => (r#"bulk bs 031"#, r###"alias za='echo z'; za; unalias za 2>/dev/null"###);
        bulk_bs_fc_row_032 => (r#"bulk bs 032"#, r###"read -r line <<< 'one two'; print -r $line"###);
        bulk_bs_fc_row_033 => (r#"bulk bs 033"#, r###"cat <<< 'herestring'"###);
        bulk_bs_fc_row_034 => (r#"bulk bs 034"#, r###"echo hello 2>/dev/null"###);
        bulk_bs_fc_row_035 => (r#"bulk bs 035"#, r###"printf '%s\n' a b c | head -1"###);
        bulk_bs_fc_row_036 => (r#"bulk bs 036"#, r###"true && echo yes"###);
        bulk_bs_fc_row_037 => (r#"bulk bs 037"#, r###"false || echo yes"###);
        bulk_bs_fc_row_038 => (r#"bulk bs 038"#, r###"(exit 3); print -r $?"###);
        bulk_bs_fc_row_039 => (r#"bulk bs 039"#, r###"print -r ${status}; (exit 4)"###);
        bulk_bs_fc_row_040 => (r#"bulk bs 040"#, r###"print -r $(( 1_000 + 1 ))"###);
        bulk_bs_fc_row_041 => (r#"bulk bs 041"#, r###"print -r $(( 5#101 ))"###);
        bulk_bs_fc_row_042 => (r#"bulk bs 042"#, r###"print -r $(( 0b1111 ))"###);
        bulk_bs_fc_row_043 => (r#"bulk bs 043"#, r###"print -r $(( 2 ** 3 ** 2 ))"###);
        bulk_bs_fc_row_044 => (r#"bulk bs 044"#, r###"float f=1.5; print -r $(( f * 2 ))"###);
        bulk_bs_fc_row_045 => (r#"bulk bs 045"#, r###"typeset -F2 f=3.14; print -r $f"###);
        bulk_bs_fc_row_046 => (r#"bulk bs 046"#, r###"[[ /etc/hosts -nt /tmp ]]; print -r $?"###);
        bulk_bs_fc_row_047 => (r#"bulk bs 047"#, r###"[[ /tmp -ot /etc/hosts ]]; print -r $?"###);
        bulk_bs_fc_row_048 => (r#"bulk bs 048"#, r###"[[ /etc/hosts -ef /etc/hosts ]]; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_bt {
    use super::*;

    parity_gap_tests! {
        bulk_bt_fc_row_001 => (r#"bulk bt 001"#, r###"arr=(a b c); print -r ${arr[(I)b]}"###);
        bulk_bt_fc_row_002 => (r#"bulk bt 002"#, r###"arr=(a b c); print -r ${arr[(R)b]}"###);
        bulk_bt_fc_row_003 => (r#"bulk bt 003"#, r###"arr=(9 8 7); print -r ${arr[-2,-1]}"###);
        bulk_bt_fc_row_004 => (r#"bulk bt 004"#, r###"typeset -A h; h=(k v); print -r ${(k)h}"###);
        bulk_bt_fc_row_005 => (r#"bulk bt 005"#, r###"typeset -A h; h=(a 1 b 2); print -r ${(kv)h}"###);
        bulk_bt_fc_row_006 => (r#"bulk bt 006"#, r###"print -r ${+options}"###);
        bulk_bt_fc_row_007 => (r#"bulk bt 007"#, r###"print -r ${+parameters}"###);
        bulk_bt_fc_row_008 => (r#"bulk bt 008"#, r###"print -r ${+aliases}"###);
        bulk_bt_fc_row_009 => (r#"bulk bt 009"#, r###"print -r ${+functions}"###);
        bulk_bt_fc_row_010 => (r#"bulk bt 010"#, r###"print -r $ZSH_NAME"###);
        bulk_bt_fc_row_011 => (r#"bulk bt 011"#, r###"print -r ${ZSH_VERSION%%.*}"###);
        bulk_bt_fc_row_012 => (r#"bulk bt 012"#, r###"whence -w print"###);
        bulk_bt_fc_row_013 => (r#"bulk bt 013"#, r###"command -v true"###);
        bulk_bt_fc_row_014 => (r#"bulk bt 014"#, r###"emulate -L zsh; print -r $?"###);
        bulk_bt_fc_row_015 => (r#"bulk bt 015"#, r###"alias za='echo z'; za; unalias za 2>/dev/null"###);
        bulk_bt_fc_row_016 => (r#"bulk bt 016"#, r###"read -r line <<< 'one two'; print -r $line"###);
        bulk_bt_fc_row_017 => (r#"bulk bt 017"#, r###"cat <<< 'herestring'"###);
        bulk_bt_fc_row_018 => (r#"bulk bt 018"#, r###"echo hello 2>/dev/null"###);
        bulk_bt_fc_row_019 => (r#"bulk bt 019"#, r###"printf '%s\n' a b c | head -1"###);
        bulk_bt_fc_row_020 => (r#"bulk bt 020"#, r###"true && echo yes"###);
        bulk_bt_fc_row_021 => (r#"bulk bt 021"#, r###"false || echo yes"###);
        bulk_bt_fc_row_022 => (r#"bulk bt 022"#, r###"(exit 3); print -r $?"###);
        bulk_bt_fc_row_023 => (r#"bulk bt 023"#, r###"print -r ${status}; (exit 4)"###);
        bulk_bt_fc_row_024 => (r#"bulk bt 024"#, r###"print -r $(( 1_000 + 1 ))"###);
        bulk_bt_fc_row_025 => (r#"bulk bt 025"#, r###"print -r $(( 5#101 ))"###);
        bulk_bt_fc_row_026 => (r#"bulk bt 026"#, r###"print -r $(( 0b1111 ))"###);
        bulk_bt_fc_row_027 => (r#"bulk bt 027"#, r###"print -r $(( 2 ** 3 ** 2 ))"###);
        bulk_bt_fc_row_028 => (r#"bulk bt 028"#, r###"float f=1.5; print -r $(( f * 2 ))"###);
        bulk_bt_fc_row_029 => (r#"bulk bt 029"#, r###"typeset -F2 f=3.14; print -r $f"###);
        bulk_bt_fc_row_030 => (r#"bulk bt 030"#, r###"[[ /etc/hosts -nt /tmp ]]; print -r $?"###);
        bulk_bt_fc_row_031 => (r#"bulk bt 031"#, r###"[[ /tmp -ot /etc/hosts ]]; print -r $?"###);
        bulk_bt_fc_row_032 => (r#"bulk bt 032"#, r###"[[ /etc/hosts -ef /etc/hosts ]]; print -r $?"###);
        bulk_bt_fc_row_033 => (r#"bulk bt 033"#, r####"setopt extendedglob; [[ abc = [a-z]## ]]; print -r $?"####);
        bulk_bt_fc_row_034 => (r#"bulk bt 034"#, r###"print -r ${(L)${(U)m}}; m=aBc"###);
        bulk_bt_fc_row_035 => (r#"bulk bt 035"#, r###"print -r ${#x}; x=hello"###);
        bulk_bt_fc_row_036 => (r#"bulk bt 036"#, r###"print -r ${#a}; a=(a b c)"###);
        bulk_bt_fc_row_037 => (r#"bulk bt 037"#, r###"print -r ${(c)#a}; a=(ab cd)"###);
        bulk_bt_fc_row_038 => (r#"bulk bt 038"#, r###"print -r ${(b)x}; x=hi"###);
        bulk_bt_fc_row_039 => (r#"bulk bt 039"#, r###"print -r ${(e):-2+2}"###);
        bulk_bt_fc_row_040 => (r#"bulk bt 040"#, r###"print -r ${(P)r}; r=HOME"###);
        bulk_bt_fc_row_041 => (r#"bulk bt 041"#, r###"print -r ${(on)n}; n=(10 2 1)"###);
        bulk_bt_fc_row_042 => (r#"bulk bt 042"#, r###"print -r ${(eu)n}; n=(a A b)"###);
        bulk_bt_fc_row_043 => (r#"bulk bt 043"#, r###"typeset -aU u=(a a b); print -r ${#u}"###);
        bulk_bt_fc_row_044 => (r#"bulk bt 044"#, r###"typeset -h hv=1; print -r ${+hv}"###);
        bulk_bt_fc_row_045 => (r#"bulk bt 045"#, r###"x=a1a2; p=a; print -r ${x//p/r}"###);
        bulk_bt_fc_row_046 => (r#"bulk bt 046"#, r###"for i in 1 2 3; do (( i == 2 )) && continue; print -r $i; done"###);
        bulk_bt_fc_row_047 => (r#"bulk bt 047"#, r###"while :; do break; print -r n; done; print -r after"###);
        bulk_bt_fc_row_048 => (r#"bulk bt 048"#, r###"case w in (a|b) echo ab ;; *) echo star ;; esac"###);
    }
}

mod corpus_dash_fc_bulk_bu {
    use super::*;

    parity_gap_tests! {
        bulk_bu_fc_row_001 => (r#"bulk bu 001"#, r###"cat <<< 'herestring'"###);
        bulk_bu_fc_row_002 => (r#"bulk bu 002"#, r###"echo hello 2>/dev/null"###);
        bulk_bu_fc_row_003 => (r#"bulk bu 003"#, r###"printf '%s\n' a b c | head -1"###);
        bulk_bu_fc_row_004 => (r#"bulk bu 004"#, r###"true && echo yes"###);
        bulk_bu_fc_row_005 => (r#"bulk bu 005"#, r###"false || echo yes"###);
        bulk_bu_fc_row_006 => (r#"bulk bu 006"#, r###"(exit 3); print -r $?"###);
        bulk_bu_fc_row_007 => (r#"bulk bu 007"#, r###"print -r ${status}; (exit 4)"###);
        bulk_bu_fc_row_008 => (r#"bulk bu 008"#, r###"print -r $(( 1_000 + 1 ))"###);
        bulk_bu_fc_row_009 => (r#"bulk bu 009"#, r###"print -r $(( 5#101 ))"###);
        bulk_bu_fc_row_010 => (r#"bulk bu 010"#, r###"print -r $(( 0b1111 ))"###);
        bulk_bu_fc_row_011 => (r#"bulk bu 011"#, r###"print -r $(( 2 ** 3 ** 2 ))"###);
        bulk_bu_fc_row_012 => (r#"bulk bu 012"#, r###"float f=1.5; print -r $(( f * 2 ))"###);
        bulk_bu_fc_row_013 => (r#"bulk bu 013"#, r###"typeset -F2 f=3.14; print -r $f"###);
        bulk_bu_fc_row_014 => (r#"bulk bu 014"#, r###"[[ /etc/hosts -nt /tmp ]]; print -r $?"###);
        bulk_bu_fc_row_015 => (r#"bulk bu 015"#, r###"[[ /tmp -ot /etc/hosts ]]; print -r $?"###);
        bulk_bu_fc_row_016 => (r#"bulk bu 016"#, r###"[[ /etc/hosts -ef /etc/hosts ]]; print -r $?"###);
        bulk_bu_fc_row_017 => (r#"bulk bu 017"#, r####"setopt extendedglob; [[ abc = [a-z]## ]]; print -r $?"####);
        bulk_bu_fc_row_018 => (r#"bulk bu 018"#, r###"print -r ${(L)${(U)m}}; m=aBc"###);
        bulk_bu_fc_row_019 => (r#"bulk bu 019"#, r###"print -r ${#x}; x=hello"###);
        bulk_bu_fc_row_020 => (r#"bulk bu 020"#, r###"print -r ${#a}; a=(a b c)"###);
        bulk_bu_fc_row_021 => (r#"bulk bu 021"#, r###"print -r ${(c)#a}; a=(ab cd)"###);
        bulk_bu_fc_row_022 => (r#"bulk bu 022"#, r###"print -r ${(b)x}; x=hi"###);
        bulk_bu_fc_row_023 => (r#"bulk bu 023"#, r###"print -r ${(e):-2+2}"###);
        bulk_bu_fc_row_024 => (r#"bulk bu 024"#, r###"print -r ${(P)r}; r=HOME"###);
        bulk_bu_fc_row_025 => (r#"bulk bu 025"#, r###"print -r ${(on)n}; n=(10 2 1)"###);
        bulk_bu_fc_row_026 => (r#"bulk bu 026"#, r###"print -r ${(eu)n}; n=(a A b)"###);
        bulk_bu_fc_row_027 => (r#"bulk bu 027"#, r###"typeset -aU u=(a a b); print -r ${#u}"###);
        bulk_bu_fc_row_028 => (r#"bulk bu 028"#, r###"typeset -h hv=1; print -r ${+hv}"###);
        bulk_bu_fc_row_029 => (r#"bulk bu 029"#, r###"x=a1a2; p=a; print -r ${x//p/r}"###);
        bulk_bu_fc_row_030 => (r#"bulk bu 030"#, r###"for i in 1 2 3; do (( i == 2 )) && continue; print -r $i; done"###);
        bulk_bu_fc_row_031 => (r#"bulk bu 031"#, r###"while :; do break; print -r n; done; print -r after"###);
        bulk_bu_fc_row_032 => (r#"bulk bu 032"#, r###"case w in (a|b) echo ab ;; *) echo star ;; esac"###);
        bulk_bu_fc_row_033 => (r#"bulk bu 033"#, r###"if [[ -n '' ]]; then echo y; else echo n; fi"###);
        bulk_bu_fc_row_034 => (r#"bulk bu 034"#, r###"print -r ${argv[1]}; set -- p q"###);
        bulk_bu_fc_row_035 => (r#"bulk bu 035"#, r###"print -r $ARGC; set -- a b"###);
        bulk_bu_fc_row_036 => (r#"bulk bu 036"#, r###"print -r ${dirstack[1]:-empty}"###);
        bulk_bu_fc_row_037 => (r#"bulk bu 037"#, r###"print -r ${+pipestatus}"###);
        bulk_bu_fc_row_038 => (r#"bulk bu 038"#, r###"print -r ${+history}"###);
        bulk_bu_fc_row_039 => (r#"bulk bu 039"#, r###"print -r ${+commands}"###);
        bulk_bu_fc_row_040 => (r#"bulk bu 040"#, r###"print -r ${+builtins}"###);
        bulk_bu_fc_row_041 => (r#"bulk bu 041"#, r###"print -r ${+widgets}"###);
        bulk_bu_fc_row_042 => (r#"bulk bu 042"#, r###"print -r ${+terminfo}"###);
        bulk_bu_fc_row_043 => (r#"bulk bu 043"#, r###"print -r ${+modules}"###);
        bulk_bu_fc_row_044 => (r#"bulk bu 044"#, r###"print -r ${+patchars}"###);
        bulk_bu_fc_row_045 => (r#"bulk bu 045"#, r###"print -r ${+reswords}"###);
        bulk_bu_fc_row_046 => (r#"bulk bu 046"#, r###"print -r ${+dis_aliases}"###);
        bulk_bu_fc_row_047 => (r#"bulk bu 047"#, r###"print -r ${+dis_functions}"###);
        bulk_bu_fc_row_048 => (r#"bulk bu 048"#, r###"print -r ${+parameters[(I)PATH]}"###);
    }
}

mod corpus_dash_fc_bulk_bv {
    use super::*;

    parity_gap_tests! {
        bulk_bv_fc_row_001 => (r#"bulk bv 001"#, r###"print -r ${(L)${(U)m}}; m=aBc"###);
        bulk_bv_fc_row_002 => (r#"bulk bv 002"#, r###"print -r ${#x}; x=hello"###);
        bulk_bv_fc_row_003 => (r#"bulk bv 003"#, r###"print -r ${#a}; a=(a b c)"###);
        bulk_bv_fc_row_004 => (r#"bulk bv 004"#, r###"print -r ${(c)#a}; a=(ab cd)"###);
        bulk_bv_fc_row_005 => (r#"bulk bv 005"#, r###"print -r ${(b)x}; x=hi"###);
        bulk_bv_fc_row_006 => (r#"bulk bv 006"#, r###"print -r ${(e):-2+2}"###);
        bulk_bv_fc_row_007 => (r#"bulk bv 007"#, r###"print -r ${(P)r}; r=HOME"###);
        bulk_bv_fc_row_008 => (r#"bulk bv 008"#, r###"print -r ${(on)n}; n=(10 2 1)"###);
        bulk_bv_fc_row_009 => (r#"bulk bv 009"#, r###"print -r ${(eu)n}; n=(a A b)"###);
        bulk_bv_fc_row_010 => (r#"bulk bv 010"#, r###"typeset -aU u=(a a b); print -r ${#u}"###);
        bulk_bv_fc_row_011 => (r#"bulk bv 011"#, r###"typeset -h hv=1; print -r ${+hv}"###);
        bulk_bv_fc_row_012 => (r#"bulk bv 012"#, r###"x=a1a2; p=a; print -r ${x//p/r}"###);
        bulk_bv_fc_row_013 => (r#"bulk bv 013"#, r###"for i in 1 2 3; do (( i == 2 )) && continue; print -r $i; done"###);
        bulk_bv_fc_row_014 => (r#"bulk bv 014"#, r###"while :; do break; print -r n; done; print -r after"###);
        bulk_bv_fc_row_015 => (r#"bulk bv 015"#, r###"case w in (a|b) echo ab ;; *) echo star ;; esac"###);
        bulk_bv_fc_row_016 => (r#"bulk bv 016"#, r###"if [[ -n '' ]]; then echo y; else echo n; fi"###);
        bulk_bv_fc_row_017 => (r#"bulk bv 017"#, r###"print -r ${argv[1]}; set -- p q"###);
        bulk_bv_fc_row_018 => (r#"bulk bv 018"#, r###"print -r $ARGC; set -- a b"###);
        bulk_bv_fc_row_019 => (r#"bulk bv 019"#, r###"print -r ${dirstack[1]:-empty}"###);
        bulk_bv_fc_row_020 => (r#"bulk bv 020"#, r###"print -r ${+pipestatus}"###);
        bulk_bv_fc_row_021 => (r#"bulk bv 021"#, r###"print -r ${+history}"###);
        bulk_bv_fc_row_022 => (r#"bulk bv 022"#, r###"print -r ${+commands}"###);
        bulk_bv_fc_row_023 => (r#"bulk bv 023"#, r###"print -r ${+builtins}"###);
        bulk_bv_fc_row_024 => (r#"bulk bv 024"#, r###"print -r ${+widgets}"###);
        bulk_bv_fc_row_025 => (r#"bulk bv 025"#, r###"print -r ${+terminfo}"###);
        bulk_bv_fc_row_026 => (r#"bulk bv 026"#, r###"print -r ${+modules}"###);
        bulk_bv_fc_row_027 => (r#"bulk bv 027"#, r###"print -r ${+patchars}"###);
        bulk_bv_fc_row_028 => (r#"bulk bv 028"#, r###"print -r ${+reswords}"###);
        bulk_bv_fc_row_029 => (r#"bulk bv 029"#, r###"print -r ${+dis_aliases}"###);
        bulk_bv_fc_row_030 => (r#"bulk bv 030"#, r###"print -r ${+dis_functions}"###);
        bulk_bv_fc_row_031 => (r#"bulk bv 031"#, r###"print -r ${+parameters[(I)PATH]}"###);
        bulk_bv_fc_row_032 => (r#"bulk bv 032"#, r###"arr=(a b c d); print -r ${arr[2,3]}"###);
        bulk_bv_fc_row_033 => (r#"bulk bv 033"#, r###"arr=(1 2 3); print -r ${arr[1,-1]}"###);
        bulk_bv_fc_row_034 => (r#"bulk bv 034"#, r###"s=barfooxyz; print -r ${s[(i)foo]}"###);
        bulk_bv_fc_row_035 => (r#"bulk bv 035"#, r###"typeset -A h; h=(k v); print -r ${h[(R)v]}"###);
        bulk_bv_fc_row_036 => (r#"bulk bv 036"#, r###"typeset -A h; h=(a 1 b 2); print -r ${h[(r)2]}"###);
        bulk_bv_fc_row_037 => (r#"bulk bv 037"#, r###"print -r $(( 9 & 6 ^ 3 ))"###);
        bulk_bv_fc_row_038 => (r#"bulk bv 038"#, r###"print -r $(( 128 >> 2 ))"###);
        bulk_bv_fc_row_039 => (r#"bulk bv 039"#, r###"print -r $(( ~(255) & 0xff ))"###);
        bulk_bv_fc_row_040 => (r#"bulk bv 040"#, r###"print -r $(( 3 <|> 5 ))"###);
        bulk_bv_fc_row_041 => (r#"bulk bv 041"#, r###"print -r $(( 3 <> 5 ))"###);
        bulk_bv_fc_row_042 => (r#"bulk bv 042"#, r###"(( 5#11 )); print -r $?"###);
        bulk_bv_fc_row_043 => (r#"bulk bv 043"#, r###"integer n=5; (( n ^= 3 )); print -r $n"###);
        bulk_bv_fc_row_044 => (r#"bulk bv 044"#, r###"integer n=5; (( n <<= 1 )); print -r $n"###);
        bulk_bv_fc_row_045 => (r#"bulk bv 045"#, r###"integer n=5; (( n >>= 1 )); print -r $n"###);
        bulk_bv_fc_row_046 => (r#"bulk bv 046"#, r###"integer n=5; (( n /= 2 )); print -r $n"###);
        bulk_bv_fc_row_047 => (r#"bulk bv 047"#, r###"integer n=5; (( n %= 3 )); print -r $n"###);
        bulk_bv_fc_row_048 => (r#"bulk bv 048"#, r###"typeset -i8 n=10; print -r $n"###);
    }
}

mod corpus_dash_fc_bulk_bw {
    use super::*;

    parity_gap_tests! {
        bulk_bw_fc_row_001 => (r#"bulk bw 001"#, r###"print -r ${argv[1]}; set -- p q"###);
        bulk_bw_fc_row_002 => (r#"bulk bw 002"#, r###"print -r $ARGC; set -- a b"###);
        bulk_bw_fc_row_003 => (r#"bulk bw 003"#, r###"print -r ${dirstack[1]:-empty}"###);
        bulk_bw_fc_row_004 => (r#"bulk bw 004"#, r###"print -r ${+pipestatus}"###);
        bulk_bw_fc_row_005 => (r#"bulk bw 005"#, r###"print -r ${+history}"###);
        bulk_bw_fc_row_006 => (r#"bulk bw 006"#, r###"print -r ${+commands}"###);
        bulk_bw_fc_row_007 => (r#"bulk bw 007"#, r###"print -r ${+builtins}"###);
        bulk_bw_fc_row_008 => (r#"bulk bw 008"#, r###"print -r ${+widgets}"###);
        bulk_bw_fc_row_009 => (r#"bulk bw 009"#, r###"print -r ${+terminfo}"###);
        bulk_bw_fc_row_010 => (r#"bulk bw 010"#, r###"print -r ${+modules}"###);
        bulk_bw_fc_row_011 => (r#"bulk bw 011"#, r###"print -r ${+patchars}"###);
        bulk_bw_fc_row_012 => (r#"bulk bw 012"#, r###"print -r ${+reswords}"###);
        bulk_bw_fc_row_013 => (r#"bulk bw 013"#, r###"print -r ${+dis_aliases}"###);
        bulk_bw_fc_row_014 => (r#"bulk bw 014"#, r###"print -r ${+dis_functions}"###);
        bulk_bw_fc_row_015 => (r#"bulk bw 015"#, r###"print -r ${+parameters[(I)PATH]}"###);
        bulk_bw_fc_row_016 => (r#"bulk bw 016"#, r###"arr=(a b c d); print -r ${arr[2,3]}"###);
        bulk_bw_fc_row_017 => (r#"bulk bw 017"#, r###"arr=(1 2 3); print -r ${arr[1,-1]}"###);
        bulk_bw_fc_row_018 => (r#"bulk bw 018"#, r###"s=barfooxyz; print -r ${s[(i)foo]}"###);
        bulk_bw_fc_row_019 => (r#"bulk bw 019"#, r###"typeset -A h; h=(k v); print -r ${h[(R)v]}"###);
        bulk_bw_fc_row_020 => (r#"bulk bw 020"#, r###"typeset -A h; h=(a 1 b 2); print -r ${h[(r)2]}"###);
        bulk_bw_fc_row_021 => (r#"bulk bw 021"#, r###"print -r $(( 9 & 6 ^ 3 ))"###);
        bulk_bw_fc_row_022 => (r#"bulk bw 022"#, r###"print -r $(( 128 >> 2 ))"###);
        bulk_bw_fc_row_023 => (r#"bulk bw 023"#, r###"print -r $(( ~(255) & 0xff ))"###);
        bulk_bw_fc_row_024 => (r#"bulk bw 024"#, r###"print -r $(( 3 <|> 5 ))"###);
        bulk_bw_fc_row_025 => (r#"bulk bw 025"#, r###"print -r $(( 3 <> 5 ))"###);
        bulk_bw_fc_row_026 => (r#"bulk bw 026"#, r###"(( 5#11 )); print -r $?"###);
        bulk_bw_fc_row_027 => (r#"bulk bw 027"#, r###"integer n=5; (( n ^= 3 )); print -r $n"###);
        bulk_bw_fc_row_028 => (r#"bulk bw 028"#, r###"integer n=5; (( n <<= 1 )); print -r $n"###);
        bulk_bw_fc_row_029 => (r#"bulk bw 029"#, r###"integer n=5; (( n >>= 1 )); print -r $n"###);
        bulk_bw_fc_row_030 => (r#"bulk bw 030"#, r###"integer n=5; (( n /= 2 )); print -r $n"###);
        bulk_bw_fc_row_031 => (r#"bulk bw 031"#, r###"integer n=5; (( n %= 3 )); print -r $n"###);
        bulk_bw_fc_row_032 => (r#"bulk bw 032"#, r###"typeset -i8 n=10; print -r $n"###);
        bulk_bw_fc_row_033 => (r#"bulk bw 033"#, r###"typeset -i16 n=255; print -r $n"###);
        bulk_bw_fc_row_034 => (r#"bulk bw 034"#, r###"typeset -E2 n=4000; print -r $n"###);
        bulk_bw_fc_row_035 => (r#"bulk bw 035"#, r###"typeset -R4 n=hi; print -r $n"###);
        bulk_bw_fc_row_036 => (r#"bulk bw 036"#, r###"typeset +L n=Ab; print -r $n"###);
        bulk_bw_fc_row_037 => (r#"bulk bw 037"#, r###"typeset +U n=xy; print -r $n"###);
        bulk_bw_fc_row_038 => (r#"bulk bw 038"#, r###"typeset +i n=4; print -r $n"###);
        bulk_bw_fc_row_039 => (r#"bulk bw 039"#, r###"export EX=1; print -r $EX; unset EX"###);
        bulk_bw_fc_row_040 => (r#"bulk bw 040"#, r###"readonly ro=5; print -r $ro"###);
        bulk_bw_fc_row_041 => (r#"bulk bw 041"#, r###"print -r ${${v:-fb}}; unset v"###);
        bulk_bw_fc_row_042 => (r#"bulk bw 042"#, r###"print -r ${${v:+set}:-unset}; unset v"###);
        bulk_bw_fc_row_043 => (r#"bulk bw 043"#, r###"word=$'l1\nl2'; print -r ${(@f)word}"###);
        bulk_bw_fc_row_044 => (r#"bulk bw 044"#, r###"word=  hi  ; print -r ${(W)word}"###);
        bulk_bw_fc_row_045 => (r#"bulk bw 045"#, r###"print -r ${(z)word}; word=a b c"###);
        bulk_bw_fc_row_046 => (r#"bulk bw 046"#, r###"print -r ${(F)x}; x=$'p\nq'"###);
        bulk_bw_fc_row_047 => (r#"bulk bw 047"#, r###"print -r ${(A)x}; x=1 2"###);
        bulk_bw_fc_row_048 => (r#"bulk bw 048"#, r###"print -r ${(aa)x}; x=(1 2)"###);
    }
}

mod corpus_dash_fc_bulk_bx {
    use super::*;

    parity_gap_tests! {
        bulk_bx_fc_row_001 => (r#"bulk bx 001"#, r###"s=barfooxyz; print -r ${s[(i)foo]}"###);
        bulk_bx_fc_row_002 => (r#"bulk bx 002"#, r###"typeset -A h; h=(k v); print -r ${h[(R)v]}"###);
        bulk_bx_fc_row_003 => (r#"bulk bx 003"#, r###"typeset -A h; h=(a 1 b 2); print -r ${h[(r)2]}"###);
        bulk_bx_fc_row_004 => (r#"bulk bx 004"#, r###"print -r $(( 9 & 6 ^ 3 ))"###);
        bulk_bx_fc_row_005 => (r#"bulk bx 005"#, r###"print -r $(( 128 >> 2 ))"###);
        bulk_bx_fc_row_006 => (r#"bulk bx 006"#, r###"print -r $(( ~(255) & 0xff ))"###);
        bulk_bx_fc_row_007 => (r#"bulk bx 007"#, r###"print -r $(( 3 <|> 5 ))"###);
        bulk_bx_fc_row_008 => (r#"bulk bx 008"#, r###"print -r $(( 3 <> 5 ))"###);
        bulk_bx_fc_row_009 => (r#"bulk bx 009"#, r###"(( 5#11 )); print -r $?"###);
        bulk_bx_fc_row_010 => (r#"bulk bx 010"#, r###"integer n=5; (( n ^= 3 )); print -r $n"###);
        bulk_bx_fc_row_011 => (r#"bulk bx 011"#, r###"integer n=5; (( n <<= 1 )); print -r $n"###);
        bulk_bx_fc_row_012 => (r#"bulk bx 012"#, r###"integer n=5; (( n >>= 1 )); print -r $n"###);
        bulk_bx_fc_row_013 => (r#"bulk bx 013"#, r###"integer n=5; (( n /= 2 )); print -r $n"###);
        bulk_bx_fc_row_014 => (r#"bulk bx 014"#, r###"integer n=5; (( n %= 3 )); print -r $n"###);
        bulk_bx_fc_row_015 => (r#"bulk bx 015"#, r###"typeset -i8 n=10; print -r $n"###);
        bulk_bx_fc_row_016 => (r#"bulk bx 016"#, r###"typeset -i16 n=255; print -r $n"###);
        bulk_bx_fc_row_017 => (r#"bulk bx 017"#, r###"typeset -E2 n=4000; print -r $n"###);
        bulk_bx_fc_row_018 => (r#"bulk bx 018"#, r###"typeset -R4 n=hi; print -r $n"###);
        bulk_bx_fc_row_019 => (r#"bulk bx 019"#, r###"typeset +L n=Ab; print -r $n"###);
        bulk_bx_fc_row_020 => (r#"bulk bx 020"#, r###"typeset +U n=xy; print -r $n"###);
        bulk_bx_fc_row_021 => (r#"bulk bx 021"#, r###"typeset +i n=4; print -r $n"###);
        bulk_bx_fc_row_022 => (r#"bulk bx 022"#, r###"export EX=1; print -r $EX; unset EX"###);
        bulk_bx_fc_row_023 => (r#"bulk bx 023"#, r###"readonly ro=5; print -r $ro"###);
        bulk_bx_fc_row_024 => (r#"bulk bx 024"#, r###"print -r ${${v:-fb}}; unset v"###);
        bulk_bx_fc_row_025 => (r#"bulk bx 025"#, r###"print -r ${${v:+set}:-unset}; unset v"###);
        bulk_bx_fc_row_026 => (r#"bulk bx 026"#, r###"word=$'l1\nl2'; print -r ${(@f)word}"###);
        bulk_bx_fc_row_027 => (r#"bulk bx 027"#, r###"word=  hi  ; print -r ${(W)word}"###);
        bulk_bx_fc_row_028 => (r#"bulk bx 028"#, r###"print -r ${(z)word}; word=a b c"###);
        bulk_bx_fc_row_029 => (r#"bulk bx 029"#, r###"print -r ${(F)x}; x=$'p\nq'"###);
        bulk_bx_fc_row_030 => (r#"bulk bx 030"#, r###"print -r ${(A)x}; x=1 2"###);
        bulk_bx_fc_row_031 => (r#"bulk bx 031"#, r###"print -r ${(aa)x}; x=(1 2)"###);
        bulk_bx_fc_row_032 => (r#"bulk bx 032"#, r###"print -r ${(%)2}"###);
        bulk_bx_fc_row_033 => (r#"bulk bx 033"#, r###"o=8; print -r ${(0)o}"###);
        bulk_bx_fc_row_034 => (r#"bulk bx 034"#, r###"str=abc.def; print -r ${str:r}"###);
        bulk_bx_fc_row_035 => (r#"bulk bx 035"#, r###"str=abc.def; print -r ${str:e}"###);
        bulk_bx_fc_row_036 => (r#"bulk bx 036"#, r###"[[ -h /dev/stdin ]]; print -r $?"###);
        bulk_bx_fc_row_037 => (r#"bulk bx 037"#, r###"[[ -p /dev/fd/0 ]]; print -r $?"###);
        bulk_bx_fc_row_038 => (r#"bulk bx 038"#, r###"[[ -O /etc/hosts ]]; print -r $?"###);
        bulk_bx_fc_row_039 => (r#"bulk bx 039"#, r###"[[ -G / ]]; print -r $?"###);
        bulk_bx_fc_row_040 => (r#"bulk bx 040"#, r###"[[ -a /etc/hosts ]]; print -r $?"###);
        bulk_bx_fc_row_041 => (r#"bulk bx 041"#, r###"[[ bee = *ee* ]]; print -r $?"###);
        bulk_bx_fc_row_042 => (r#"bulk bx 042"#, r###"[[ 1 -eq 1 ]]; print -r $?"###);
        bulk_bx_fc_row_043 => (r#"bulk bx 043"#, r###"[[ 1 -ne 2 ]]; print -r $?"###);
        bulk_bx_fc_row_044 => (r#"bulk bx 044"#, r###"[[ 3 -lt 5 ]]; print -r $?"###);
        bulk_bx_fc_row_045 => (r#"bulk bx 045"#, r###"[[ 5 -le 5 ]]; print -r $?"###);
        bulk_bx_fc_row_046 => (r#"bulk bx 046"#, r###"[[ 5 -gt 3 ]]; print -r $?"###);
        bulk_bx_fc_row_047 => (r#"bulk bx 047"#, r###"[[ 5 -ge 5 ]]; print -r $?"###);
        bulk_bx_fc_row_048 => (r#"bulk bx 048"#, r###"[[ -o nullglob ]]; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_by {
    use super::*;

    parity_gap_tests! {
        bulk_by_fc_row_001 => (r#"bulk by 001"#, r###"typeset -i16 n=255; print -r $n"###);
        bulk_by_fc_row_002 => (r#"bulk by 002"#, r###"typeset -E2 n=4000; print -r $n"###);
        bulk_by_fc_row_003 => (r#"bulk by 003"#, r###"typeset -R4 n=hi; print -r $n"###);
        bulk_by_fc_row_004 => (r#"bulk by 004"#, r###"typeset +L n=Ab; print -r $n"###);
        bulk_by_fc_row_005 => (r#"bulk by 005"#, r###"typeset +U n=xy; print -r $n"###);
        bulk_by_fc_row_006 => (r#"bulk by 006"#, r###"typeset +i n=4; print -r $n"###);
        bulk_by_fc_row_007 => (r#"bulk by 007"#, r###"export EX=1; print -r $EX; unset EX"###);
        bulk_by_fc_row_008 => (r#"bulk by 008"#, r###"readonly ro=5; print -r $ro"###);
        bulk_by_fc_row_009 => (r#"bulk by 009"#, r###"print -r ${${v:-fb}}; unset v"###);
        bulk_by_fc_row_010 => (r#"bulk by 010"#, r###"print -r ${${v:+set}:-unset}; unset v"###);
        bulk_by_fc_row_011 => (r#"bulk by 011"#, r###"word=$'l1\nl2'; print -r ${(@f)word}"###);
        bulk_by_fc_row_012 => (r#"bulk by 012"#, r###"word=  hi  ; print -r ${(W)word}"###);
        bulk_by_fc_row_013 => (r#"bulk by 013"#, r###"print -r ${(z)word}; word=a b c"###);
        bulk_by_fc_row_014 => (r#"bulk by 014"#, r###"print -r ${(F)x}; x=$'p\nq'"###);
        bulk_by_fc_row_015 => (r#"bulk by 015"#, r###"print -r ${(A)x}; x=1 2"###);
        bulk_by_fc_row_016 => (r#"bulk by 016"#, r###"print -r ${(aa)x}; x=(1 2)"###);
        bulk_by_fc_row_017 => (r#"bulk by 017"#, r###"print -r ${(%)2}"###);
        bulk_by_fc_row_018 => (r#"bulk by 018"#, r###"o=8; print -r ${(0)o}"###);
        bulk_by_fc_row_019 => (r#"bulk by 019"#, r###"str=abc.def; print -r ${str:r}"###);
        bulk_by_fc_row_020 => (r#"bulk by 020"#, r###"str=abc.def; print -r ${str:e}"###);
        bulk_by_fc_row_021 => (r#"bulk by 021"#, r###"[[ -h /dev/stdin ]]; print -r $?"###);
        bulk_by_fc_row_022 => (r#"bulk by 022"#, r###"[[ -p /dev/fd/0 ]]; print -r $?"###);
        bulk_by_fc_row_023 => (r#"bulk by 023"#, r###"[[ -O /etc/hosts ]]; print -r $?"###);
        bulk_by_fc_row_024 => (r#"bulk by 024"#, r###"[[ -G / ]]; print -r $?"###);
        bulk_by_fc_row_025 => (r#"bulk by 025"#, r###"[[ -a /etc/hosts ]]; print -r $?"###);
        bulk_by_fc_row_026 => (r#"bulk by 026"#, r###"[[ bee = *ee* ]]; print -r $?"###);
        bulk_by_fc_row_027 => (r#"bulk by 027"#, r###"[[ 1 -eq 1 ]]; print -r $?"###);
        bulk_by_fc_row_028 => (r#"bulk by 028"#, r###"[[ 1 -ne 2 ]]; print -r $?"###);
        bulk_by_fc_row_029 => (r#"bulk by 029"#, r###"[[ 3 -lt 5 ]]; print -r $?"###);
        bulk_by_fc_row_030 => (r#"bulk by 030"#, r###"[[ 5 -le 5 ]]; print -r $?"###);
        bulk_by_fc_row_031 => (r#"bulk by 031"#, r###"[[ 5 -gt 3 ]]; print -r $?"###);
        bulk_by_fc_row_032 => (r#"bulk by 032"#, r###"[[ 5 -ge 5 ]]; print -r $?"###);
        bulk_by_fc_row_033 => (r#"bulk by 033"#, r###"[[ -o nullglob ]]; print -r $?"###);
        bulk_by_fc_row_034 => (r#"bulk by 034"#, r###"unsetopt extendedglob 2>/dev/null; [[ -o extendedglob ]]; print -r $?"###);
        bulk_by_fc_row_035 => (r#"bulk by 035"#, r###"setopt extendedglob; [[ -o extendedglob ]]; print -r $?"###);
        bulk_by_fc_row_036 => (r#"bulk by 036"#, r###"[[ -o no_extendedglob ]]; print -r $?"###);
        bulk_by_fc_row_037 => (r#"bulk by 037"#, r###"print -r $(( 1 , 2 , 3 ))"###);
        bulk_by_fc_row_038 => (r#"bulk by 038"#, r###"print -r $(( 3 < 5 ? 1 : 0 ))"###);
        bulk_by_fc_row_039 => (r#"bulk by 039"#, r###"print -r $(( 0xff & 0x0f ))"###);
        bulk_by_fc_row_040 => (r#"bulk by 040"#, r###"print -r $(( 1 << 4 ))"###);
        bulk_by_fc_row_041 => (r#"bulk by 041"#, r###"print -r $(( 16 >> 2 ))"###);
        bulk_by_fc_row_042 => (r#"bulk by 042"#, r###"print -r $(( -1 >> 1 ))"###);
        bulk_by_fc_row_043 => (r#"bulk by 043"#, r###"print -r $(( 8#17 ))"###);
        bulk_by_fc_row_044 => (r#"bulk by 044"#, r###"print -r $(( 16#ff ))"###);
        bulk_by_fc_row_045 => (r#"bulk by 045"#, r###"print -r $(( 2#1010 ))"###);
        bulk_by_fc_row_046 => (r#"bulk by 046"#, r###"print -r $(( 0b1010 ))"###);
        bulk_by_fc_row_047 => (r#"bulk by 047"#, r###"typeset -F1 c=1.05; print -r $(( c > 1 ))"###);
        bulk_by_fc_row_048 => (r#"bulk by 048"#, r###"print -r $(( 4 % 2 == 0 ))"###);
    }
}

mod corpus_dash_fc_bulk_bz {
    use super::*;

    parity_gap_tests! {
        bulk_bz_fc_row_001 => (r#"bulk bz 001"#, r###"o=8; print -r ${(0)o}"###);
        bulk_bz_fc_row_002 => (r#"bulk bz 002"#, r###"str=abc.def; print -r ${str:r}"###);
        bulk_bz_fc_row_003 => (r#"bulk bz 003"#, r###"str=abc.def; print -r ${str:e}"###);
        bulk_bz_fc_row_004 => (r#"bulk bz 004"#, r###"[[ -h /dev/stdin ]]; print -r $?"###);
        bulk_bz_fc_row_005 => (r#"bulk bz 005"#, r###"[[ -p /dev/fd/0 ]]; print -r $?"###);
        bulk_bz_fc_row_006 => (r#"bulk bz 006"#, r###"[[ -O /etc/hosts ]]; print -r $?"###);
        bulk_bz_fc_row_007 => (r#"bulk bz 007"#, r###"[[ -G / ]]; print -r $?"###);
        bulk_bz_fc_row_008 => (r#"bulk bz 008"#, r###"[[ -a /etc/hosts ]]; print -r $?"###);
        bulk_bz_fc_row_009 => (r#"bulk bz 009"#, r###"[[ bee = *ee* ]]; print -r $?"###);
        bulk_bz_fc_row_010 => (r#"bulk bz 010"#, r###"[[ 1 -eq 1 ]]; print -r $?"###);
        bulk_bz_fc_row_011 => (r#"bulk bz 011"#, r###"[[ 1 -ne 2 ]]; print -r $?"###);
        bulk_bz_fc_row_012 => (r#"bulk bz 012"#, r###"[[ 3 -lt 5 ]]; print -r $?"###);
        bulk_bz_fc_row_013 => (r#"bulk bz 013"#, r###"[[ 5 -le 5 ]]; print -r $?"###);
        bulk_bz_fc_row_014 => (r#"bulk bz 014"#, r###"[[ 5 -gt 3 ]]; print -r $?"###);
        bulk_bz_fc_row_015 => (r#"bulk bz 015"#, r###"[[ 5 -ge 5 ]]; print -r $?"###);
        bulk_bz_fc_row_016 => (r#"bulk bz 016"#, r###"[[ -o nullglob ]]; print -r $?"###);
        bulk_bz_fc_row_017 => (r#"bulk bz 017"#, r###"unsetopt extendedglob 2>/dev/null; [[ -o extendedglob ]]; print -r $?"###);
        bulk_bz_fc_row_018 => (r#"bulk bz 018"#, r###"setopt extendedglob; [[ -o extendedglob ]]; print -r $?"###);
        bulk_bz_fc_row_019 => (r#"bulk bz 019"#, r###"[[ -o no_extendedglob ]]; print -r $?"###);
        bulk_bz_fc_row_020 => (r#"bulk bz 020"#, r###"print -r $(( 1 , 2 , 3 ))"###);
        bulk_bz_fc_row_021 => (r#"bulk bz 021"#, r###"print -r $(( 3 < 5 ? 1 : 0 ))"###);
        bulk_bz_fc_row_022 => (r#"bulk bz 022"#, r###"print -r $(( 0xff & 0x0f ))"###);
        bulk_bz_fc_row_023 => (r#"bulk bz 023"#, r###"print -r $(( 1 << 4 ))"###);
        bulk_bz_fc_row_024 => (r#"bulk bz 024"#, r###"print -r $(( 16 >> 2 ))"###);
        bulk_bz_fc_row_025 => (r#"bulk bz 025"#, r###"print -r $(( -1 >> 1 ))"###);
        bulk_bz_fc_row_026 => (r#"bulk bz 026"#, r###"print -r $(( 8#17 ))"###);
        bulk_bz_fc_row_027 => (r#"bulk bz 027"#, r###"print -r $(( 16#ff ))"###);
        bulk_bz_fc_row_028 => (r#"bulk bz 028"#, r###"print -r $(( 2#1010 ))"###);
        bulk_bz_fc_row_029 => (r#"bulk bz 029"#, r###"print -r $(( 0b1010 ))"###);
        bulk_bz_fc_row_030 => (r#"bulk bz 030"#, r###"typeset -F1 c=1.05; print -r $(( c > 1 ))"###);
        bulk_bz_fc_row_031 => (r#"bulk bz 031"#, r###"print -r $(( 4 % 2 == 0 ))"###);
        bulk_bz_fc_row_032 => (r#"bulk bz 032"#, r###"print -r $(( 0 - 1 == -1 ))"###);
        bulk_bz_fc_row_033 => (r#"bulk bz 033"#, r###"print -r $(( 72 / 8 / 3 ))"###);
        bulk_bz_fc_row_034 => (r#"bulk bz 034"#, r###"print -r $(( 24 % 5 % 3 ))"###);
        bulk_bz_fc_row_035 => (r#"bulk bz 035"#, r###"print -r $(( 2 | 4 | 8 ))"###);
        bulk_bz_fc_row_036 => (r#"bulk bz 036"#, r###"print -r $(( 15 ^ 9 ))"###);
        bulk_bz_fc_row_037 => (r#"bulk bz 037"#, r###"print -r $(( 0 || 0 || 7 ))"###);
        bulk_bz_fc_row_038 => (r#"bulk bz 038"#, r###"print -r $(( 1 || -1 ))"###);
        bulk_bz_fc_row_039 => (r#"bulk bz 039"#, r###"print -r $(( (1>0) + (0>0) ))"###);
        bulk_bz_fc_row_040 => (r#"bulk bz 040"#, r###"print -r $(( 3 > 2 > 1 ))"###);
        bulk_bz_fc_row_041 => (r#"bulk bz 041"#, r###"print -r $(( (9>8)>>(1<0) ))"###);
        bulk_bz_fc_row_042 => (r#"bulk bz 042"#, r###"print -r $(( 5 ** 2 % 7 ))"###);
        bulk_bz_fc_row_043 => (r#"bulk bz 043"#, r###"print -r $(( 11 ** 2 % 50 ))"###);
        bulk_bz_fc_row_044 => (r#"bulk bz 044"#, r###"print -r $(( 100 / 20 / 5 ))"###);
        bulk_bz_fc_row_045 => (r#"bulk bz 045"#, r###"print -r $(( 2#101 & 2#010 ))"###);
        bulk_bz_fc_row_046 => (r#"bulk bz 046"#, r###"print -r $(( 0x80 >> 4 ))"###);
        bulk_bz_fc_row_047 => (r#"bulk bz 047"#, r###"print -r $(( 5 ** 0 ** 3 ))"###);
        bulk_bz_fc_row_048 => (r#"bulk bz 048"#, r###"print -r $(( -(-(-5)) ))"###);
    }
}

mod corpus_dash_fc_bulk_ca {
    use super::*;

    parity_gap_tests! {
        bulk_ca_fc_row_001 => (r#"bulk ca 001"#, r###"setopt extendedglob; [[ -o extendedglob ]]; print -r $?"###);
        bulk_ca_fc_row_002 => (r#"bulk ca 002"#, r###"[[ -o no_extendedglob ]]; print -r $?"###);
        bulk_ca_fc_row_003 => (r#"bulk ca 003"#, r###"print -r $(( 1 , 2 , 3 ))"###);
        bulk_ca_fc_row_004 => (r#"bulk ca 004"#, r###"print -r $(( 3 < 5 ? 1 : 0 ))"###);
        bulk_ca_fc_row_005 => (r#"bulk ca 005"#, r###"print -r $(( 0xff & 0x0f ))"###);
        bulk_ca_fc_row_006 => (r#"bulk ca 006"#, r###"print -r $(( 1 << 4 ))"###);
        bulk_ca_fc_row_007 => (r#"bulk ca 007"#, r###"print -r $(( 16 >> 2 ))"###);
        bulk_ca_fc_row_008 => (r#"bulk ca 008"#, r###"print -r $(( -1 >> 1 ))"###);
        bulk_ca_fc_row_009 => (r#"bulk ca 009"#, r###"print -r $(( 8#17 ))"###);
        bulk_ca_fc_row_010 => (r#"bulk ca 010"#, r###"print -r $(( 16#ff ))"###);
        bulk_ca_fc_row_011 => (r#"bulk ca 011"#, r###"print -r $(( 2#1010 ))"###);
        bulk_ca_fc_row_012 => (r#"bulk ca 012"#, r###"print -r $(( 0b1010 ))"###);
        bulk_ca_fc_row_013 => (r#"bulk ca 013"#, r###"typeset -F1 c=1.05; print -r $(( c > 1 ))"###);
        bulk_ca_fc_row_014 => (r#"bulk ca 014"#, r###"print -r $(( 4 % 2 == 0 ))"###);
        bulk_ca_fc_row_015 => (r#"bulk ca 015"#, r###"print -r $(( 0 - 1 == -1 ))"###);
        bulk_ca_fc_row_016 => (r#"bulk ca 016"#, r###"print -r $(( 72 / 8 / 3 ))"###);
        bulk_ca_fc_row_017 => (r#"bulk ca 017"#, r###"print -r $(( 24 % 5 % 3 ))"###);
        bulk_ca_fc_row_018 => (r#"bulk ca 018"#, r###"print -r $(( 2 | 4 | 8 ))"###);
        bulk_ca_fc_row_019 => (r#"bulk ca 019"#, r###"print -r $(( 15 ^ 9 ))"###);
        bulk_ca_fc_row_020 => (r#"bulk ca 020"#, r###"print -r $(( 0 || 0 || 7 ))"###);
        bulk_ca_fc_row_021 => (r#"bulk ca 021"#, r###"print -r $(( 1 || -1 ))"###);
        bulk_ca_fc_row_022 => (r#"bulk ca 022"#, r###"print -r $(( (1>0) + (0>0) ))"###);
        bulk_ca_fc_row_023 => (r#"bulk ca 023"#, r###"print -r $(( 3 > 2 > 1 ))"###);
        bulk_ca_fc_row_024 => (r#"bulk ca 024"#, r###"print -r $(( (9>8)>>(1<0) ))"###);
        bulk_ca_fc_row_025 => (r#"bulk ca 025"#, r###"print -r $(( 5 ** 2 % 7 ))"###);
        bulk_ca_fc_row_026 => (r#"bulk ca 026"#, r###"print -r $(( 11 ** 2 % 50 ))"###);
        bulk_ca_fc_row_027 => (r#"bulk ca 027"#, r###"print -r $(( 100 / 20 / 5 ))"###);
        bulk_ca_fc_row_028 => (r#"bulk ca 028"#, r###"print -r $(( 2#101 & 2#010 ))"###);
        bulk_ca_fc_row_029 => (r#"bulk ca 029"#, r###"print -r $(( 0x80 >> 4 ))"###);
        bulk_ca_fc_row_030 => (r#"bulk ca 030"#, r###"print -r $(( 5 ** 0 ** 3 ))"###);
        bulk_ca_fc_row_031 => (r#"bulk ca 031"#, r###"print -r $(( -(-(-5)) ))"###);
        bulk_ca_fc_row_032 => (r#"bulk ca 032"#, r###"print -r $(( (1+2)*(3+4) ))"###);
        bulk_ca_fc_row_033 => (r#"bulk ca 033"#, r###"v1=v1; [[ v1 -ef v1 ]]; print -r $?"###);
        bulk_ca_fc_row_034 => (r#"bulk ca 034"#, r###"[[ "" != x ]]; print -r $?"###);
        bulk_ca_fc_row_035 => (r#"bulk ca 035"#, r###"[[ -n /dev/null ]]; print -r $?"###);
        bulk_ca_fc_row_036 => (r#"bulk ca 036"#, r###"setopt extendedglob; [[ mix = [[:digit:]]# ]]; print -r $?"###);
        bulk_ca_fc_row_037 => (r#"bulk ca 037"#, r####"setopt extendedglob; [[ tag = (#m)[a-z]##_t ]]; print -r $?"####);
        bulk_ca_fc_row_038 => (r#"bulk ca 038"#, r###"setopt extendedglob; [[ foo = fo(#e) ]]; print -r $?"###);
        bulk_ca_fc_row_039 => (r#"bulk ca 039"#, r###"setopt extendedglob; [[ foo = (#s)fo ]]; print -r $?"###);
        bulk_ca_fc_row_040 => (r#"bulk ca 040"#, r###"[[ abc < abd ]]; print -r $?"###);
        bulk_ca_fc_row_041 => (r#"bulk ca 041"#, r###"[[ abc > abb ]]; print -r $?"###);
        bulk_ca_fc_row_042 => (r#"bulk ca 042"#, r###"[[ abc != def ]]; print -r $?"###);
        bulk_ca_fc_row_043 => (r#"bulk ca 043"#, r###"[[ abc == abc ]]; print -r $?"###);
        bulk_ca_fc_row_044 => (r#"bulk ca 044"#, r###"print -r ${(L)@}; set -- MIXED"###);
        bulk_ca_fc_row_045 => (r#"bulk ca 045"#, r###"slice=abcdef; print -r $slice[3,5]"###);
        bulk_ca_fc_row_046 => (r#"bulk ca 046"#, r###"typeset -aS ary=x y; print -r $ary[2]"###);
        bulk_ca_fc_row_047 => (r#"bulk ca 047"#, r###"pushd /tmp >/dev/null 2>&1; popd >/dev/null 2>&1; print -r $?"###);
        bulk_ca_fc_row_048 => (r#"bulk ca 048"#, r###"builtin cd -q / 2>/dev/null; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_cb {
    use super::*;

    parity_gap_tests! {
        bulk_cb_fc_row_001 => (r#"bulk cb 001"#, r###"print -r $(( 2 | 4 | 8 ))"###);
        bulk_cb_fc_row_002 => (r#"bulk cb 002"#, r###"print -r $(( 15 ^ 9 ))"###);
        bulk_cb_fc_row_003 => (r#"bulk cb 003"#, r###"print -r $(( 0 || 0 || 7 ))"###);
        bulk_cb_fc_row_004 => (r#"bulk cb 004"#, r###"print -r $(( 1 || -1 ))"###);
        bulk_cb_fc_row_005 => (r#"bulk cb 005"#, r###"print -r $(( (1>0) + (0>0) ))"###);
        bulk_cb_fc_row_006 => (r#"bulk cb 006"#, r###"print -r $(( 3 > 2 > 1 ))"###);
        bulk_cb_fc_row_007 => (r#"bulk cb 007"#, r###"print -r $(( (9>8)>>(1<0) ))"###);
        bulk_cb_fc_row_008 => (r#"bulk cb 008"#, r###"print -r $(( 5 ** 2 % 7 ))"###);
        bulk_cb_fc_row_009 => (r#"bulk cb 009"#, r###"print -r $(( 11 ** 2 % 50 ))"###);
        bulk_cb_fc_row_010 => (r#"bulk cb 010"#, r###"print -r $(( 100 / 20 / 5 ))"###);
        bulk_cb_fc_row_011 => (r#"bulk cb 011"#, r###"print -r $(( 2#101 & 2#010 ))"###);
        bulk_cb_fc_row_012 => (r#"bulk cb 012"#, r###"print -r $(( 0x80 >> 4 ))"###);
        bulk_cb_fc_row_013 => (r#"bulk cb 013"#, r###"print -r $(( 5 ** 0 ** 3 ))"###);
        bulk_cb_fc_row_014 => (r#"bulk cb 014"#, r###"print -r $(( -(-(-5)) ))"###);
        bulk_cb_fc_row_015 => (r#"bulk cb 015"#, r###"print -r $(( (1+2)*(3+4) ))"###);
        bulk_cb_fc_row_016 => (r#"bulk cb 016"#, r###"v1=v1; [[ v1 -ef v1 ]]; print -r $?"###);
        bulk_cb_fc_row_017 => (r#"bulk cb 017"#, r###"[[ "" != x ]]; print -r $?"###);
        bulk_cb_fc_row_018 => (r#"bulk cb 018"#, r###"[[ -n /dev/null ]]; print -r $?"###);
        bulk_cb_fc_row_019 => (r#"bulk cb 019"#, r###"setopt extendedglob; [[ mix = [[:digit:]]# ]]; print -r $?"###);
        bulk_cb_fc_row_020 => (r#"bulk cb 020"#, r####"setopt extendedglob; [[ tag = (#m)[a-z]##_t ]]; print -r $?"####);
        bulk_cb_fc_row_021 => (r#"bulk cb 021"#, r###"setopt extendedglob; [[ foo = fo(#e) ]]; print -r $?"###);
        bulk_cb_fc_row_022 => (r#"bulk cb 022"#, r###"setopt extendedglob; [[ foo = (#s)fo ]]; print -r $?"###);
        bulk_cb_fc_row_023 => (r#"bulk cb 023"#, r###"[[ abc < abd ]]; print -r $?"###);
        bulk_cb_fc_row_024 => (r#"bulk cb 024"#, r###"[[ abc > abb ]]; print -r $?"###);
        bulk_cb_fc_row_025 => (r#"bulk cb 025"#, r###"[[ abc != def ]]; print -r $?"###);
        bulk_cb_fc_row_026 => (r#"bulk cb 026"#, r###"[[ abc == abc ]]; print -r $?"###);
        bulk_cb_fc_row_027 => (r#"bulk cb 027"#, r###"print -r ${(L)@}; set -- MIXED"###);
        bulk_cb_fc_row_028 => (r#"bulk cb 028"#, r###"slice=abcdef; print -r $slice[3,5]"###);
        bulk_cb_fc_row_029 => (r#"bulk cb 029"#, r###"typeset -aS ary=x y; print -r $ary[2]"###);
        bulk_cb_fc_row_030 => (r#"bulk cb 030"#, r###"pushd /tmp >/dev/null 2>&1; popd >/dev/null 2>&1; print -r $?"###);
        bulk_cb_fc_row_031 => (r#"bulk cb 031"#, r###"builtin cd -q / 2>/dev/null; print -r $?"###);
        bulk_cb_fc_row_032 => (r#"bulk cb 032"#, r###"cd /tmp 2>/dev/null; print -r ${PWD:t}"###);
        bulk_cb_fc_row_033 => (r#"bulk cb 033"#, r###"hash -r 2>/dev/null; print -r $?"###);
        bulk_cb_fc_row_034 => (r#"bulk cb 034"#, r###"autoload -Uz is-at-least 2>/dev/null; print -r $?"###);
        bulk_cb_fc_row_035 => (r#"bulk cb 035"#, r###"whence -v print 2>/dev/null; print -r $?"###);
        bulk_cb_fc_row_036 => (r#"bulk cb 036"#, r###"whence -p ls 2>/dev/null | head -1"###);
        bulk_cb_fc_row_037 => (r#"bulk cb 037"#, r###"typeset -f fn 2>/dev/null; fn(){ :; }; print -r $?"###);
        bulk_cb_fc_row_038 => (r#"bulk cb 038"#, r###"functions fn 2>/dev/null; fn(){ :; }; print -r $?"###);
        bulk_cb_fc_row_039 => (r#"bulk cb 039"#, r###"unfunction fn 2>/dev/null; fn(){ :; }; unfunction fn; print -r $?"###);
        bulk_cb_fc_row_040 => (r#"bulk cb 040"#, r###"print -r ${aliases[za]:-none}"###);
        bulk_cb_fc_row_041 => (r#"bulk cb 041"#, r###"print -r ${(t)parameters[PATH]}"###);
        bulk_cb_fc_row_042 => (r#"bulk cb 042"#, r###"print -r ${(k)parameters[(I)PATH]}"###);
        bulk_cb_fc_row_043 => (r#"bulk cb 043"#, r###"print -r ${+parameters[PATH]}"###);
        bulk_cb_fc_row_044 => (r#"bulk cb 044"#, r###"print -r ${+functions[fn]}; fn(){}"###);
        bulk_cb_fc_row_045 => (r#"bulk cb 045"#, r###"print -r ${+commands[print]}"###);
        bulk_cb_fc_row_046 => (r#"bulk cb 046"#, r###"print -r ${+zsh_eval_context}"###);
        bulk_cb_fc_row_047 => (r#"bulk cb 047"#, r###"print -r ${+functrace}"###);
        bulk_cb_fc_row_048 => (r#"bulk cb 048"#, r###"print -r ${+funcstack}"###);
    }
}

mod corpus_dash_fc_bulk_cc {
    use super::*;

    parity_gap_tests! {
        bulk_cc_fc_row_001 => (r#"bulk cc 001"#, r###"[[ -n /dev/null ]]; print -r $?"###);
        bulk_cc_fc_row_002 => (r#"bulk cc 002"#, r###"setopt extendedglob; [[ mix = [[:digit:]]# ]]; print -r $?"###);
        bulk_cc_fc_row_003 => (r#"bulk cc 003"#, r####"setopt extendedglob; [[ tag = (#m)[a-z]##_t ]]; print -r $?"####);
        bulk_cc_fc_row_004 => (r#"bulk cc 004"#, r###"setopt extendedglob; [[ foo = fo(#e) ]]; print -r $?"###);
        bulk_cc_fc_row_005 => (r#"bulk cc 005"#, r###"setopt extendedglob; [[ foo = (#s)fo ]]; print -r $?"###);
        bulk_cc_fc_row_006 => (r#"bulk cc 006"#, r###"[[ abc < abd ]]; print -r $?"###);
        bulk_cc_fc_row_007 => (r#"bulk cc 007"#, r###"[[ abc > abb ]]; print -r $?"###);
        bulk_cc_fc_row_008 => (r#"bulk cc 008"#, r###"[[ abc != def ]]; print -r $?"###);
        bulk_cc_fc_row_009 => (r#"bulk cc 009"#, r###"[[ abc == abc ]]; print -r $?"###);
        bulk_cc_fc_row_010 => (r#"bulk cc 010"#, r###"print -r ${(L)@}; set -- MIXED"###);
        bulk_cc_fc_row_011 => (r#"bulk cc 011"#, r###"slice=abcdef; print -r $slice[3,5]"###);
        bulk_cc_fc_row_012 => (r#"bulk cc 012"#, r###"typeset -aS ary=x y; print -r $ary[2]"###);
        bulk_cc_fc_row_013 => (r#"bulk cc 013"#, r###"pushd /tmp >/dev/null 2>&1; popd >/dev/null 2>&1; print -r $?"###);
        bulk_cc_fc_row_014 => (r#"bulk cc 014"#, r###"builtin cd -q / 2>/dev/null; print -r $?"###);
        bulk_cc_fc_row_015 => (r#"bulk cc 015"#, r###"cd /tmp 2>/dev/null; print -r ${PWD:t}"###);
        bulk_cc_fc_row_016 => (r#"bulk cc 016"#, r###"hash -r 2>/dev/null; print -r $?"###);
        bulk_cc_fc_row_017 => (r#"bulk cc 017"#, r###"autoload -Uz is-at-least 2>/dev/null; print -r $?"###);
        bulk_cc_fc_row_018 => (r#"bulk cc 018"#, r###"whence -v print 2>/dev/null; print -r $?"###);
        bulk_cc_fc_row_019 => (r#"bulk cc 019"#, r###"whence -p ls 2>/dev/null | head -1"###);
        bulk_cc_fc_row_020 => (r#"bulk cc 020"#, r###"typeset -f fn 2>/dev/null; fn(){ :; }; print -r $?"###);
        bulk_cc_fc_row_021 => (r#"bulk cc 021"#, r###"functions fn 2>/dev/null; fn(){ :; }; print -r $?"###);
        bulk_cc_fc_row_022 => (r#"bulk cc 022"#, r###"unfunction fn 2>/dev/null; fn(){ :; }; unfunction fn; print -r $?"###);
        bulk_cc_fc_row_023 => (r#"bulk cc 023"#, r###"print -r ${aliases[za]:-none}"###);
        bulk_cc_fc_row_024 => (r#"bulk cc 024"#, r###"print -r ${(t)parameters[PATH]}"###);
        bulk_cc_fc_row_025 => (r#"bulk cc 025"#, r###"print -r ${(k)parameters[(I)PATH]}"###);
        bulk_cc_fc_row_026 => (r#"bulk cc 026"#, r###"print -r ${+parameters[PATH]}"###);
        bulk_cc_fc_row_027 => (r#"bulk cc 027"#, r###"print -r ${+functions[fn]}; fn(){}"###);
        bulk_cc_fc_row_028 => (r#"bulk cc 028"#, r###"print -r ${+commands[print]}"###);
        bulk_cc_fc_row_029 => (r#"bulk cc 029"#, r###"print -r ${+zsh_eval_context}"###);
        bulk_cc_fc_row_030 => (r#"bulk cc 030"#, r###"print -r ${+functrace}"###);
        bulk_cc_fc_row_031 => (r#"bulk cc 031"#, r###"print -r ${+funcstack}"###);
        bulk_cc_fc_row_032 => (r#"bulk cc 032"#, r###"print -r ${+funcfiletrace}"###);
        bulk_cc_fc_row_033 => (r#"bulk cc 033"#, r###"print -r ${+jobstates}"###);
        bulk_cc_fc_row_034 => (r#"bulk cc 034"#, r###"print -r ${+jobtexts}"###);
        bulk_cc_fc_row_035 => (r#"bulk cc 035"#, r###"print -r ${+jobdirs}"###);
        bulk_cc_fc_row_036 => (r#"bulk cc 036"#, r###"print -r ${+historywords}"###);
        bulk_cc_fc_row_037 => (r#"bulk cc 037"#, r###"print -r ${+usergroups}"###);
        bulk_cc_fc_row_038 => (r#"bulk cc 038"#, r###"print -r ${+dis_builtins}"###);
        bulk_cc_fc_row_039 => (r#"bulk cc 039"#, r###"print -r ${+dis_widgets}"###);
        bulk_cc_fc_row_040 => (r#"bulk cc 040"#, r###"print -r ${+dis_reswords}"###);
        bulk_cc_fc_row_041 => (r#"bulk cc 041"#, r###"print -r ${+dis_patchars}"###);
        bulk_cc_fc_row_042 => (r#"bulk cc 042"#, r###"print -r ${+dis_commands}"###);
        bulk_cc_fc_row_043 => (r#"bulk cc 043"#, r###"print -r ${+module_path}"###);
        bulk_cc_fc_row_044 => (r#"bulk cc 044"#, r###"print -r ${+functrace}"###);
        bulk_cc_fc_row_045 => (r#"bulk cc 045"#, r###"true | true | false; print -r ${pipestatus[3]}"###);
        bulk_cc_fc_row_046 => (r#"bulk cc 046"#, r###"{ true; false; }; print -r $?"###);
        bulk_cc_fc_row_047 => (r#"bulk cc 047"#, r###"fn(){ typeset -a la=(x y); print -r ${#la}; }; fn"###);
        bulk_cc_fc_row_048 => (r#"bulk cc 048"#, r###"print -r ${arr[@]:1:2}; arr=(a b c d)"###);
    }
}

mod corpus_dash_fc_bulk_cd {
    use super::*;

    parity_gap_tests! {
        bulk_cd_fc_row_001 => (r#"bulk cd 001"#, r###"whence -v print 2>/dev/null; print -r $?"###);
        bulk_cd_fc_row_002 => (r#"bulk cd 002"#, r###"whence -p ls 2>/dev/null | head -1"###);
        bulk_cd_fc_row_003 => (r#"bulk cd 003"#, r###"typeset -f fn 2>/dev/null; fn(){ :; }; print -r $?"###);
        bulk_cd_fc_row_004 => (r#"bulk cd 004"#, r###"functions fn 2>/dev/null; fn(){ :; }; print -r $?"###);
        bulk_cd_fc_row_005 => (r#"bulk cd 005"#, r###"unfunction fn 2>/dev/null; fn(){ :; }; unfunction fn; print -r $?"###);
        bulk_cd_fc_row_006 => (r#"bulk cd 006"#, r###"print -r ${aliases[za]:-none}"###);
        bulk_cd_fc_row_007 => (r#"bulk cd 007"#, r###"print -r ${(t)parameters[PATH]}"###);
        bulk_cd_fc_row_008 => (r#"bulk cd 008"#, r###"print -r ${(k)parameters[(I)PATH]}"###);
        bulk_cd_fc_row_009 => (r#"bulk cd 009"#, r###"print -r ${+parameters[PATH]}"###);
        bulk_cd_fc_row_010 => (r#"bulk cd 010"#, r###"print -r ${+functions[fn]}; fn(){}"###);
        bulk_cd_fc_row_011 => (r#"bulk cd 011"#, r###"print -r ${+commands[print]}"###);
        bulk_cd_fc_row_012 => (r#"bulk cd 012"#, r###"print -r ${+zsh_eval_context}"###);
        bulk_cd_fc_row_013 => (r#"bulk cd 013"#, r###"print -r ${+functrace}"###);
        bulk_cd_fc_row_014 => (r#"bulk cd 014"#, r###"print -r ${+funcstack}"###);
        bulk_cd_fc_row_015 => (r#"bulk cd 015"#, r###"print -r ${+funcfiletrace}"###);
        bulk_cd_fc_row_016 => (r#"bulk cd 016"#, r###"print -r ${+jobstates}"###);
        bulk_cd_fc_row_017 => (r#"bulk cd 017"#, r###"print -r ${+jobtexts}"###);
        bulk_cd_fc_row_018 => (r#"bulk cd 018"#, r###"print -r ${+jobdirs}"###);
        bulk_cd_fc_row_019 => (r#"bulk cd 019"#, r###"print -r ${+historywords}"###);
        bulk_cd_fc_row_020 => (r#"bulk cd 020"#, r###"print -r ${+usergroups}"###);
        bulk_cd_fc_row_021 => (r#"bulk cd 021"#, r###"print -r ${+dis_builtins}"###);
        bulk_cd_fc_row_022 => (r#"bulk cd 022"#, r###"print -r ${+dis_widgets}"###);
        bulk_cd_fc_row_023 => (r#"bulk cd 023"#, r###"print -r ${+dis_reswords}"###);
        bulk_cd_fc_row_024 => (r#"bulk cd 024"#, r###"print -r ${+dis_patchars}"###);
        bulk_cd_fc_row_025 => (r#"bulk cd 025"#, r###"print -r ${+dis_commands}"###);
        bulk_cd_fc_row_026 => (r#"bulk cd 026"#, r###"print -r ${+module_path}"###);
        bulk_cd_fc_row_027 => (r#"bulk cd 027"#, r###"print -r ${+functrace}"###);
        bulk_cd_fc_row_028 => (r#"bulk cd 028"#, r###"true | true | false; print -r ${pipestatus[3]}"###);
        bulk_cd_fc_row_029 => (r#"bulk cd 029"#, r###"{ true; false; }; print -r $?"###);
        bulk_cd_fc_row_030 => (r#"bulk cd 030"#, r###"fn(){ typeset -a la=(x y); print -r ${#la}; }; fn"###);
        bulk_cd_fc_row_031 => (r#"bulk cd 031"#, r###"print -r ${arr[@]:1:2}; arr=(a b c d)"###);
        bulk_cd_fc_row_032 => (r#"bulk cd 032"#, r###"print -r ${(pj:,:)a}; a=(x y)"###);
        bulk_cd_fc_row_033 => (r#"bulk cd 033"#, r###"print -r ${(Mk)h}; typeset -A h; h=(x 1 y 2)"###);
        bulk_cd_fc_row_034 => (r#"bulk cd 034"#, r###"print -r ${(oa)n}; n=(10 2 1)"###);
        bulk_cd_fc_row_035 => (r#"bulk cd 035"#, r###"print -r ${(On)n}; n=(10 2 1)"###);
        bulk_cd_fc_row_036 => (r#"bulk cd 036"#, r###"print -r ${(n)a}; a=(1 2 3)"###);
        bulk_cd_fc_row_037 => (r#"bulk cd 037"#, r###"print -r ${(N)a}; a=(1 2 3)"###);
        bulk_cd_fc_row_038 => (r#"bulk cd 038"#, r###"print -r ${(w)#w}; w=a b c"###);
        bulk_cd_fc_row_039 => (r#"bulk cd 039"#, r###"print -r ${(t)x}; x=hello"###);
        bulk_cd_fc_row_040 => (r#"bulk cd 040"#, r###"unset y; print -r ${+y}"###);
        bulk_cd_fc_row_041 => (r#"bulk cd 041"#, r###"x=hello; print -r ${+x}"###);
        bulk_cd_fc_row_042 => (r#"bulk cd 042"#, r###"print -r ${(q+)x}; x=hi"###);
        bulk_cd_fc_row_043 => (r#"bulk cd 043"#, r###"x=foo; print -r ${x:s/foo/bar/}"###);
        bulk_cd_fc_row_044 => (r#"bulk cd 044"#, r###"x=foofoo; print -r ${x//foo/bar}"###);
        bulk_cd_fc_row_045 => (r#"bulk cd 045"#, r###"x=abc; print -r ${x/#a/z}"###);
        bulk_cd_fc_row_046 => (r#"bulk cd 046"#, r###"x=abc; print -r ${x/%c/z}"###);
        bulk_cd_fc_row_047 => (r#"bulk cd 047"#, r###"print -r ${(j::)a}; a=(x y)"###);
        bulk_cd_fc_row_048 => (r#"bulk cd 048"#, r###"print -r ${(pj::)a}; a=(x y)"###);
    }
}

mod corpus_dash_fc_bulk_ce {
    use super::*;

    parity_gap_tests! {
        bulk_ce_fc_row_001 => (r#"bulk ce 001"#, r###"print -r ${+jobdirs}"###);
        bulk_ce_fc_row_002 => (r#"bulk ce 002"#, r###"print -r ${+historywords}"###);
        bulk_ce_fc_row_003 => (r#"bulk ce 003"#, r###"print -r ${+usergroups}"###);
        bulk_ce_fc_row_004 => (r#"bulk ce 004"#, r###"print -r ${+dis_builtins}"###);
        bulk_ce_fc_row_005 => (r#"bulk ce 005"#, r###"print -r ${+dis_widgets}"###);
        bulk_ce_fc_row_006 => (r#"bulk ce 006"#, r###"print -r ${+dis_reswords}"###);
        bulk_ce_fc_row_007 => (r#"bulk ce 007"#, r###"print -r ${+dis_patchars}"###);
        bulk_ce_fc_row_008 => (r#"bulk ce 008"#, r###"print -r ${+dis_commands}"###);
        bulk_ce_fc_row_009 => (r#"bulk ce 009"#, r###"print -r ${+module_path}"###);
        bulk_ce_fc_row_010 => (r#"bulk ce 010"#, r###"print -r ${+functrace}"###);
        bulk_ce_fc_row_011 => (r#"bulk ce 011"#, r###"true | true | false; print -r ${pipestatus[3]}"###);
        bulk_ce_fc_row_012 => (r#"bulk ce 012"#, r###"{ true; false; }; print -r $?"###);
        bulk_ce_fc_row_013 => (r#"bulk ce 013"#, r###"fn(){ typeset -a la=(x y); print -r ${#la}; }; fn"###);
        bulk_ce_fc_row_014 => (r#"bulk ce 014"#, r###"print -r ${arr[@]:1:2}; arr=(a b c d)"###);
        bulk_ce_fc_row_015 => (r#"bulk ce 015"#, r###"print -r ${(pj:,:)a}; a=(x y)"###);
        bulk_ce_fc_row_016 => (r#"bulk ce 016"#, r###"print -r ${(Mk)h}; typeset -A h; h=(x 1 y 2)"###);
        bulk_ce_fc_row_017 => (r#"bulk ce 017"#, r###"print -r ${(oa)n}; n=(10 2 1)"###);
        bulk_ce_fc_row_018 => (r#"bulk ce 018"#, r###"print -r ${(On)n}; n=(10 2 1)"###);
        bulk_ce_fc_row_019 => (r#"bulk ce 019"#, r###"print -r ${(n)a}; a=(1 2 3)"###);
        bulk_ce_fc_row_020 => (r#"bulk ce 020"#, r###"print -r ${(N)a}; a=(1 2 3)"###);
        bulk_ce_fc_row_021 => (r#"bulk ce 021"#, r###"print -r ${(w)#w}; w=a b c"###);
        bulk_ce_fc_row_022 => (r#"bulk ce 022"#, r###"print -r ${(t)x}; x=hello"###);
        bulk_ce_fc_row_023 => (r#"bulk ce 023"#, r###"unset y; print -r ${+y}"###);
        bulk_ce_fc_row_024 => (r#"bulk ce 024"#, r###"x=hello; print -r ${+x}"###);
        bulk_ce_fc_row_025 => (r#"bulk ce 025"#, r###"print -r ${(q+)x}; x=hi"###);
        bulk_ce_fc_row_026 => (r#"bulk ce 026"#, r###"x=foo; print -r ${x:s/foo/bar/}"###);
        bulk_ce_fc_row_027 => (r#"bulk ce 027"#, r###"x=foofoo; print -r ${x//foo/bar}"###);
        bulk_ce_fc_row_028 => (r#"bulk ce 028"#, r###"x=abc; print -r ${x/#a/z}"###);
        bulk_ce_fc_row_029 => (r#"bulk ce 029"#, r###"x=abc; print -r ${x/%c/z}"###);
        bulk_ce_fc_row_030 => (r#"bulk ce 030"#, r###"print -r ${(j::)a}; a=(x y)"###);
        bulk_ce_fc_row_031 => (r#"bulk ce 031"#, r###"print -r ${(pj::)a}; a=(x y)"###);
        bulk_ce_fc_row_032 => (r#"bulk ce 032"#, r###"print -r ${(ps:\n:)x}; x=$'a\nb'"###);
        bulk_ce_fc_row_033 => (r#"bulk ce 033"#, r###"print -r ${(e)x}; x=$'2+2'"###);
        bulk_ce_fc_row_034 => (r#"bulk ce 034"#, r###"integer co=0; : $(( co=6 )); print -r $co"###);
        bulk_ce_fc_row_035 => (r#"bulk ce 035"#, r###"print -r $(( 1<<0 ))"###);
        bulk_ce_fc_row_036 => (r#"bulk ce 036"#, r###"print -r $(( 1<<10 ))"###);
        bulk_ce_fc_row_037 => (r#"bulk ce 037"#, r###"print -r $(( 0x7fffffff & 0 ))"###);
        bulk_ce_fc_row_038 => (r#"bulk ce 038"#, r###"print -r $(( 1000003 % 97 ))"###);
        bulk_ce_fc_row_039 => (r#"bulk ce 039"#, r###"print -r $(( 63 & 31 | 15 ))"###);
        bulk_ce_fc_row_040 => (r#"bulk ce 040"#, r###"print -r $(( 0x10001 % 256 ))"###);
        bulk_ce_fc_row_041 => (r#"bulk ce 041"#, r###"print -r $(( 2*2*2*2 ))"###);
        bulk_ce_fc_row_042 => (r#"bulk ce 042"#, r###"print -r $(( (1==1)+(0==1) ))"###);
        bulk_ce_fc_row_043 => (r#"bulk ce 043"#, r###"print -r $(( 1 && (0 || 1) ))"###);
        bulk_ce_fc_row_044 => (r#"bulk ce 044"#, r###"[[ zero = <-> ]]; print -r $?"###);
        bulk_ce_fc_row_045 => (r#"bulk ce 045"#, r###"[[ . = . ]]; print -r $?"###);
        bulk_ce_fc_row_046 => (r#"bulk ce 046"#, r###"[[ a -lt b ]]; print -r $?"###);
        bulk_ce_fc_row_047 => (r#"bulk ce 047"#, r####"[[ ABC = [A-Z]## ]]; print -r $?"####);
        bulk_ce_fc_row_048 => (r#"bulk ce 048"#, r###"[[ AAA =~ ^A+ ]]; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_cf {
    use super::*;

    parity_gap_tests! {
        bulk_cf_fc_row_001 => (r#"bulk cf 001"#, r###"print -r ${(oa)n}; n=(10 2 1)"###);
        bulk_cf_fc_row_002 => (r#"bulk cf 002"#, r###"print -r ${(On)n}; n=(10 2 1)"###);
        bulk_cf_fc_row_003 => (r#"bulk cf 003"#, r###"print -r ${(n)a}; a=(1 2 3)"###);
        bulk_cf_fc_row_004 => (r#"bulk cf 004"#, r###"print -r ${(N)a}; a=(1 2 3)"###);
        bulk_cf_fc_row_005 => (r#"bulk cf 005"#, r###"print -r ${(w)#w}; w=a b c"###);
        bulk_cf_fc_row_006 => (r#"bulk cf 006"#, r###"print -r ${(t)x}; x=hello"###);
        bulk_cf_fc_row_007 => (r#"bulk cf 007"#, r###"unset y; print -r ${+y}"###);
        bulk_cf_fc_row_008 => (r#"bulk cf 008"#, r###"x=hello; print -r ${+x}"###);
        bulk_cf_fc_row_009 => (r#"bulk cf 009"#, r###"print -r ${(q+)x}; x=hi"###);
        bulk_cf_fc_row_010 => (r#"bulk cf 010"#, r###"x=foo; print -r ${x:s/foo/bar/}"###);
        bulk_cf_fc_row_011 => (r#"bulk cf 011"#, r###"x=foofoo; print -r ${x//foo/bar}"###);
        bulk_cf_fc_row_012 => (r#"bulk cf 012"#, r###"x=abc; print -r ${x/#a/z}"###);
        bulk_cf_fc_row_013 => (r#"bulk cf 013"#, r###"x=abc; print -r ${x/%c/z}"###);
        bulk_cf_fc_row_014 => (r#"bulk cf 014"#, r###"print -r ${(j::)a}; a=(x y)"###);
        bulk_cf_fc_row_015 => (r#"bulk cf 015"#, r###"print -r ${(pj::)a}; a=(x y)"###);
        bulk_cf_fc_row_016 => (r#"bulk cf 016"#, r###"print -r ${(ps:\n:)x}; x=$'a\nb'"###);
        bulk_cf_fc_row_017 => (r#"bulk cf 017"#, r###"print -r ${(e)x}; x=$'2+2'"###);
        bulk_cf_fc_row_018 => (r#"bulk cf 018"#, r###"integer co=0; : $(( co=6 )); print -r $co"###);
        bulk_cf_fc_row_019 => (r#"bulk cf 019"#, r###"print -r $(( 1<<0 ))"###);
        bulk_cf_fc_row_020 => (r#"bulk cf 020"#, r###"print -r $(( 1<<10 ))"###);
        bulk_cf_fc_row_021 => (r#"bulk cf 021"#, r###"print -r $(( 0x7fffffff & 0 ))"###);
        bulk_cf_fc_row_022 => (r#"bulk cf 022"#, r###"print -r $(( 1000003 % 97 ))"###);
        bulk_cf_fc_row_023 => (r#"bulk cf 023"#, r###"print -r $(( 63 & 31 | 15 ))"###);
        bulk_cf_fc_row_024 => (r#"bulk cf 024"#, r###"print -r $(( 0x10001 % 256 ))"###);
        bulk_cf_fc_row_025 => (r#"bulk cf 025"#, r###"print -r $(( 2*2*2*2 ))"###);
        bulk_cf_fc_row_026 => (r#"bulk cf 026"#, r###"print -r $(( (1==1)+(0==1) ))"###);
        bulk_cf_fc_row_027 => (r#"bulk cf 027"#, r###"print -r $(( 1 && (0 || 1) ))"###);
        bulk_cf_fc_row_028 => (r#"bulk cf 028"#, r###"[[ zero = <-> ]]; print -r $?"###);
        bulk_cf_fc_row_029 => (r#"bulk cf 029"#, r###"[[ . = . ]]; print -r $?"###);
        bulk_cf_fc_row_030 => (r#"bulk cf 030"#, r###"[[ a -lt b ]]; print -r $?"###);
        bulk_cf_fc_row_031 => (r#"bulk cf 031"#, r####"[[ ABC = [A-Z]## ]]; print -r $?"####);
        bulk_cf_fc_row_032 => (r#"bulk cf 032"#, r###"[[ AAA =~ ^A+ ]]; print -r $?"###);
        bulk_cf_fc_row_033 => (r#"bulk cf 033"#, r###"[[ bot = *ot* ]]; print -r $?"###);
        bulk_cf_fc_row_034 => (r#"bulk cf 034"#, r###"[[ -e /dev/null ]]; print -r $?"###);
        bulk_cf_fc_row_035 => (r#"bulk cf 035"#, r###"[[ -s /dev/null ]]; print -r $?"###);
        bulk_cf_fc_row_036 => (r#"bulk cf 036"#, r###"[[ -u /etc/hosts ]]; print -r $?"###);
        bulk_cf_fc_row_037 => (r#"bulk cf 037"#, r###"[[ -g / ]]; print -r $?"###);
        bulk_cf_fc_row_038 => (r#"bulk cf 038"#, r###"[[ -k /tmp ]]; print -r $?"###);
        bulk_cf_fc_row_039 => (r#"bulk cf 039"#, r###"[[ -b /dev/null ]]; print -r $?"###);
        bulk_cf_fc_row_040 => (r#"bulk cf 040"#, r###"[[ -c /dev/null ]]; print -r $?"###);
        bulk_cf_fc_row_041 => (r#"bulk cf 041"#, r###"print -r ${(l:5::0:)n}; n=42"###);
        bulk_cf_fc_row_042 => (r#"bulk cf 042"#, r###"print -r ${(r:5::0:)n}; n=42"###);
        bulk_cf_fc_row_043 => (r#"bulk cf 043"#, r###"print -r ${(c)str}; str=hello"###);
        bulk_cf_fc_row_044 => (r#"bulk cf 044"#, r###"print -r ${(u)arr}; arr=(a A b)"###);
        bulk_cf_fc_row_045 => (r#"bulk cf 045"#, r###"print -r ${(L)str}; str=HELLO"###);
        bulk_cf_fc_row_046 => (r#"bulk cf 046"#, r###"print -r ${(U)str}; str=hello"###);
        bulk_cf_fc_row_047 => (r#"bulk cf 047"#, r###"print -r ${(C)str}; str=hello world"###);
        bulk_cf_fc_row_048 => (r#"bulk cf 048"#, r###"print -r ${(Q)str}; str=$'a\nb'"###);
    }
}

mod corpus_dash_fc_bulk_cg {
    use super::*;

    parity_gap_tests! {
        bulk_cg_fc_row_001 => (r#"bulk cg 001"#, r###"integer co=0; : $(( co=6 )); print -r $co"###);
        bulk_cg_fc_row_002 => (r#"bulk cg 002"#, r###"print -r $(( 1<<0 ))"###);
        bulk_cg_fc_row_003 => (r#"bulk cg 003"#, r###"print -r $(( 1<<10 ))"###);
        bulk_cg_fc_row_004 => (r#"bulk cg 004"#, r###"print -r $(( 0x7fffffff & 0 ))"###);
        bulk_cg_fc_row_005 => (r#"bulk cg 005"#, r###"print -r $(( 1000003 % 97 ))"###);
        bulk_cg_fc_row_006 => (r#"bulk cg 006"#, r###"print -r $(( 63 & 31 | 15 ))"###);
        bulk_cg_fc_row_007 => (r#"bulk cg 007"#, r###"print -r $(( 0x10001 % 256 ))"###);
        bulk_cg_fc_row_008 => (r#"bulk cg 008"#, r###"print -r $(( 2*2*2*2 ))"###);
        bulk_cg_fc_row_009 => (r#"bulk cg 009"#, r###"print -r $(( (1==1)+(0==1) ))"###);
        bulk_cg_fc_row_010 => (r#"bulk cg 010"#, r###"print -r $(( 1 && (0 || 1) ))"###);
        bulk_cg_fc_row_011 => (r#"bulk cg 011"#, r###"[[ zero = <-> ]]; print -r $?"###);
        bulk_cg_fc_row_012 => (r#"bulk cg 012"#, r###"[[ . = . ]]; print -r $?"###);
        bulk_cg_fc_row_013 => (r#"bulk cg 013"#, r###"[[ a -lt b ]]; print -r $?"###);
        bulk_cg_fc_row_014 => (r#"bulk cg 014"#, r####"[[ ABC = [A-Z]## ]]; print -r $?"####);
        bulk_cg_fc_row_015 => (r#"bulk cg 015"#, r###"[[ AAA =~ ^A+ ]]; print -r $?"###);
        bulk_cg_fc_row_016 => (r#"bulk cg 016"#, r###"[[ bot = *ot* ]]; print -r $?"###);
        bulk_cg_fc_row_017 => (r#"bulk cg 017"#, r###"[[ -e /dev/null ]]; print -r $?"###);
        bulk_cg_fc_row_018 => (r#"bulk cg 018"#, r###"[[ -s /dev/null ]]; print -r $?"###);
        bulk_cg_fc_row_019 => (r#"bulk cg 019"#, r###"[[ -u /etc/hosts ]]; print -r $?"###);
        bulk_cg_fc_row_020 => (r#"bulk cg 020"#, r###"[[ -g / ]]; print -r $?"###);
        bulk_cg_fc_row_021 => (r#"bulk cg 021"#, r###"[[ -k /tmp ]]; print -r $?"###);
        bulk_cg_fc_row_022 => (r#"bulk cg 022"#, r###"[[ -b /dev/null ]]; print -r $?"###);
        bulk_cg_fc_row_023 => (r#"bulk cg 023"#, r###"[[ -c /dev/null ]]; print -r $?"###);
        bulk_cg_fc_row_024 => (r#"bulk cg 024"#, r###"print -r ${(l:5::0:)n}; n=42"###);
        bulk_cg_fc_row_025 => (r#"bulk cg 025"#, r###"print -r ${(r:5::0:)n}; n=42"###);
        bulk_cg_fc_row_026 => (r#"bulk cg 026"#, r###"print -r ${(c)str}; str=hello"###);
        bulk_cg_fc_row_027 => (r#"bulk cg 027"#, r###"print -r ${(u)arr}; arr=(a A b)"###);
        bulk_cg_fc_row_028 => (r#"bulk cg 028"#, r###"print -r ${(L)str}; str=HELLO"###);
        bulk_cg_fc_row_029 => (r#"bulk cg 029"#, r###"print -r ${(U)str}; str=hello"###);
        bulk_cg_fc_row_030 => (r#"bulk cg 030"#, r###"print -r ${(C)str}; str=hello world"###);
        bulk_cg_fc_row_031 => (r#"bulk cg 031"#, r###"print -r ${(Q)str}; str=$'a\nb'"###);
        bulk_cg_fc_row_032 => (r#"bulk cg 032"#, r###"print -r ${(qq)str}; str=hi"###);
        bulk_cg_fc_row_033 => (r#"bulk cg 033"#, r###"print -r ${(V)str}; str=hi"###);
        bulk_cg_fc_row_034 => (r#"bulk cg 034"#, r###"print -r ${(z)str}; str=$'a\0b'"###);
        bulk_cg_fc_row_035 => (r#"bulk cg 035"#, r###"print -r ${(j:-:)a}; a=(x y)"###);
        bulk_cg_fc_row_036 => (r#"bulk cg 036"#, r###"print -r ${(pj:-:)a}; a=(x y)"###);
        bulk_cg_fc_row_037 => (r#"bulk cg 037"#, r###"a=(1 2 3); print -r ${a[1,-1]}"###);
        bulk_cg_fc_row_038 => (r#"bulk cg 038"#, r###"a=(1 2 3); print -r ${a[1,2]}"###);
        bulk_cg_fc_row_039 => (r#"bulk cg 039"#, r###"a=(1 2 3); print -r ${a[-1]}"###);
        bulk_cg_fc_row_040 => (r#"bulk cg 040"#, r###"a=(1 2 3); print -r ${a[(i)2]}"###);
        bulk_cg_fc_row_041 => (r#"bulk cg 041"#, r###"a=(1 2 3); print -r ${a[(I)2]}"###);
        bulk_cg_fc_row_042 => (r#"bulk cg 042"#, r###"a=(1 2 3); print -r ${a[(R)9]}"###);
        bulk_cg_fc_row_043 => (r#"bulk cg 043"#, r###"a=(1 2 3); print -r ${a[(r)2]}"###);
        bulk_cg_fc_row_044 => (r#"bulk cg 044"#, r###"typeset -A m; m=(k v); print -r ${m[k]}"###);
        bulk_cg_fc_row_045 => (r#"bulk cg 045"#, r###"typeset -A m; m=(k v); print -r ${(k)m}"###);
        bulk_cg_fc_row_046 => (r#"bulk cg 046"#, r###"typeset -A m; m=(k v); print -r ${(v)m}"###);
        bulk_cg_fc_row_047 => (r#"bulk cg 047"#, r###"typeset -A m; m=(a 1 b 2); print -r ${(kv)m}"###);
        bulk_cg_fc_row_048 => (r#"bulk cg 048"#, r###"print -r ${(o)lst}; lst=(z a m)"###);
    }
}

mod corpus_dash_fc_bulk_ch {
    use super::*;

    parity_gap_tests! {
        bulk_ch_fc_row_001 => (r#"bulk ch 001"#, r###"[[ -s /dev/null ]]; print -r $?"###);
        bulk_ch_fc_row_002 => (r#"bulk ch 002"#, r###"[[ -u /etc/hosts ]]; print -r $?"###);
        bulk_ch_fc_row_003 => (r#"bulk ch 003"#, r###"[[ -g / ]]; print -r $?"###);
        bulk_ch_fc_row_004 => (r#"bulk ch 004"#, r###"[[ -k /tmp ]]; print -r $?"###);
        bulk_ch_fc_row_005 => (r#"bulk ch 005"#, r###"[[ -b /dev/null ]]; print -r $?"###);
        bulk_ch_fc_row_006 => (r#"bulk ch 006"#, r###"[[ -c /dev/null ]]; print -r $?"###);
        bulk_ch_fc_row_007 => (r#"bulk ch 007"#, r###"print -r ${(l:5::0:)n}; n=42"###);
        bulk_ch_fc_row_008 => (r#"bulk ch 008"#, r###"print -r ${(r:5::0:)n}; n=42"###);
        bulk_ch_fc_row_009 => (r#"bulk ch 009"#, r###"print -r ${(c)str}; str=hello"###);
        bulk_ch_fc_row_010 => (r#"bulk ch 010"#, r###"print -r ${(u)arr}; arr=(a A b)"###);
        bulk_ch_fc_row_011 => (r#"bulk ch 011"#, r###"print -r ${(L)str}; str=HELLO"###);
        bulk_ch_fc_row_012 => (r#"bulk ch 012"#, r###"print -r ${(U)str}; str=hello"###);
        bulk_ch_fc_row_013 => (r#"bulk ch 013"#, r###"print -r ${(C)str}; str=hello world"###);
        bulk_ch_fc_row_014 => (r#"bulk ch 014"#, r###"print -r ${(Q)str}; str=$'a\nb'"###);
        bulk_ch_fc_row_015 => (r#"bulk ch 015"#, r###"print -r ${(qq)str}; str=hi"###);
        bulk_ch_fc_row_016 => (r#"bulk ch 016"#, r###"print -r ${(V)str}; str=hi"###);
        bulk_ch_fc_row_017 => (r#"bulk ch 017"#, r###"print -r ${(z)str}; str=$'a\0b'"###);
        bulk_ch_fc_row_018 => (r#"bulk ch 018"#, r###"print -r ${(j:-:)a}; a=(x y)"###);
        bulk_ch_fc_row_019 => (r#"bulk ch 019"#, r###"print -r ${(pj:-:)a}; a=(x y)"###);
        bulk_ch_fc_row_020 => (r#"bulk ch 020"#, r###"a=(1 2 3); print -r ${a[1,-1]}"###);
        bulk_ch_fc_row_021 => (r#"bulk ch 021"#, r###"a=(1 2 3); print -r ${a[1,2]}"###);
        bulk_ch_fc_row_022 => (r#"bulk ch 022"#, r###"a=(1 2 3); print -r ${a[-1]}"###);
        bulk_ch_fc_row_023 => (r#"bulk ch 023"#, r###"a=(1 2 3); print -r ${a[(i)2]}"###);
        bulk_ch_fc_row_024 => (r#"bulk ch 024"#, r###"a=(1 2 3); print -r ${a[(I)2]}"###);
        bulk_ch_fc_row_025 => (r#"bulk ch 025"#, r###"a=(1 2 3); print -r ${a[(R)9]}"###);
        bulk_ch_fc_row_026 => (r#"bulk ch 026"#, r###"a=(1 2 3); print -r ${a[(r)2]}"###);
        bulk_ch_fc_row_027 => (r#"bulk ch 027"#, r###"typeset -A m; m=(k v); print -r ${m[k]}"###);
        bulk_ch_fc_row_028 => (r#"bulk ch 028"#, r###"typeset -A m; m=(k v); print -r ${(k)m}"###);
        bulk_ch_fc_row_029 => (r#"bulk ch 029"#, r###"typeset -A m; m=(k v); print -r ${(v)m}"###);
        bulk_ch_fc_row_030 => (r#"bulk ch 030"#, r###"typeset -A m; m=(a 1 b 2); print -r ${(kv)m}"###);
        bulk_ch_fc_row_031 => (r#"bulk ch 031"#, r###"print -r ${(o)lst}; lst=(z a m)"###);
        bulk_ch_fc_row_032 => (r#"bulk ch 032"#, r###"print -r ${(O)lst}; lst=(z a m)"###);
        bulk_ch_fc_row_033 => (r#"bulk ch 033"#, r###"print -r ${(i)lst}; lst=(z a m)"###);
        bulk_ch_fc_row_034 => (r#"bulk ch 034"#, r###"a=(x y); print -r ${^a}"###);
        bulk_ch_fc_row_035 => (r#"bulk ch 035"#, r###"a=(1 2); b=(a b); print -r ${^a}${^b}"###);
        bulk_ch_fc_row_036 => (r#"bulk ch 036"#, r###"setopt braceccl; print -r {a,b}"###);
        bulk_ch_fc_row_037 => (r#"bulk ch 037"#, r###"print -r {1..3}"###);
        bulk_ch_fc_row_038 => (r#"bulk ch 038"#, r###"print -r {01..03}"###);
        bulk_ch_fc_row_039 => (r#"bulk ch 039"#, r###"print -r {a..c}"###);
        bulk_ch_fc_row_040 => (r#"bulk ch 040"#, r###"print -r {1..4..2}"###);
        bulk_ch_fc_row_041 => (r#"bulk ch 041"#, r###"print -r ${~pattern}; pattern='*'; :"###);
        bulk_ch_fc_row_042 => (r#"bulk ch 042"#, r###"integer x=3; (( x++ )); print -r $x"###);
        bulk_ch_fc_row_043 => (r#"bulk ch 043"#, r###"integer x=3; (( ++x )); print -r $x"###);
        bulk_ch_fc_row_044 => (r#"bulk ch 044"#, r###"integer x=3; (( x-- )); print -r $x"###);
        bulk_ch_fc_row_045 => (r#"bulk ch 045"#, r###"integer x=3; print -r $(( x ** 2 ))"###);
        bulk_ch_fc_row_046 => (r#"bulk ch 046"#, r###"float f=1.5; print -r $(( f + 1 ))"###);
        bulk_ch_fc_row_047 => (r#"bulk ch 047"#, r###"print -r $(( 7 / 2 ))"###);
        bulk_ch_fc_row_048 => (r#"bulk ch 048"#, r###"print -r $(( 7.0 / 2 ))"###);
    }
}

mod corpus_dash_fc_bulk_ci {
    use super::*;

    parity_gap_tests! {
        bulk_ci_fc_row_001 => (r#"bulk ci 001"#, r###"print -r ${(V)str}; str=hi"###);
        bulk_ci_fc_row_002 => (r#"bulk ci 002"#, r###"print -r ${(z)str}; str=$'a\0b'"###);
        bulk_ci_fc_row_003 => (r#"bulk ci 003"#, r###"print -r ${(j:-:)a}; a=(x y)"###);
        bulk_ci_fc_row_004 => (r#"bulk ci 004"#, r###"print -r ${(pj:-:)a}; a=(x y)"###);
        bulk_ci_fc_row_005 => (r#"bulk ci 005"#, r###"a=(1 2 3); print -r ${a[1,-1]}"###);
        bulk_ci_fc_row_006 => (r#"bulk ci 006"#, r###"a=(1 2 3); print -r ${a[1,2]}"###);
        bulk_ci_fc_row_007 => (r#"bulk ci 007"#, r###"a=(1 2 3); print -r ${a[-1]}"###);
        bulk_ci_fc_row_008 => (r#"bulk ci 008"#, r###"a=(1 2 3); print -r ${a[(i)2]}"###);
        bulk_ci_fc_row_009 => (r#"bulk ci 009"#, r###"a=(1 2 3); print -r ${a[(I)2]}"###);
        bulk_ci_fc_row_010 => (r#"bulk ci 010"#, r###"a=(1 2 3); print -r ${a[(R)9]}"###);
        bulk_ci_fc_row_011 => (r#"bulk ci 011"#, r###"a=(1 2 3); print -r ${a[(r)2]}"###);
        bulk_ci_fc_row_012 => (r#"bulk ci 012"#, r###"typeset -A m; m=(k v); print -r ${m[k]}"###);
        bulk_ci_fc_row_013 => (r#"bulk ci 013"#, r###"typeset -A m; m=(k v); print -r ${(k)m}"###);
        bulk_ci_fc_row_014 => (r#"bulk ci 014"#, r###"typeset -A m; m=(k v); print -r ${(v)m}"###);
        bulk_ci_fc_row_015 => (r#"bulk ci 015"#, r###"typeset -A m; m=(a 1 b 2); print -r ${(kv)m}"###);
        bulk_ci_fc_row_016 => (r#"bulk ci 016"#, r###"print -r ${(o)lst}; lst=(z a m)"###);
        bulk_ci_fc_row_017 => (r#"bulk ci 017"#, r###"print -r ${(O)lst}; lst=(z a m)"###);
        bulk_ci_fc_row_018 => (r#"bulk ci 018"#, r###"print -r ${(i)lst}; lst=(z a m)"###);
        bulk_ci_fc_row_019 => (r#"bulk ci 019"#, r###"a=(x y); print -r ${^a}"###);
        bulk_ci_fc_row_020 => (r#"bulk ci 020"#, r###"a=(1 2); b=(a b); print -r ${^a}${^b}"###);
        bulk_ci_fc_row_021 => (r#"bulk ci 021"#, r###"setopt braceccl; print -r {a,b}"###);
        bulk_ci_fc_row_022 => (r#"bulk ci 022"#, r###"print -r {1..3}"###);
        bulk_ci_fc_row_023 => (r#"bulk ci 023"#, r###"print -r {01..03}"###);
        bulk_ci_fc_row_024 => (r#"bulk ci 024"#, r###"print -r {a..c}"###);
        bulk_ci_fc_row_025 => (r#"bulk ci 025"#, r###"print -r {1..4..2}"###);
        bulk_ci_fc_row_026 => (r#"bulk ci 026"#, r###"print -r ${~pattern}; pattern='*'; :"###);
        bulk_ci_fc_row_027 => (r#"bulk ci 027"#, r###"integer x=3; (( x++ )); print -r $x"###);
        bulk_ci_fc_row_028 => (r#"bulk ci 028"#, r###"integer x=3; (( ++x )); print -r $x"###);
        bulk_ci_fc_row_029 => (r#"bulk ci 029"#, r###"integer x=3; (( x-- )); print -r $x"###);
        bulk_ci_fc_row_030 => (r#"bulk ci 030"#, r###"integer x=3; print -r $(( x ** 2 ))"###);
        bulk_ci_fc_row_031 => (r#"bulk ci 031"#, r###"float f=1.5; print -r $(( f + 1 ))"###);
        bulk_ci_fc_row_032 => (r#"bulk ci 032"#, r###"print -r $(( 7 / 2 ))"###);
        bulk_ci_fc_row_033 => (r#"bulk ci 033"#, r###"print -r $(( 7.0 / 2 ))"###);
        bulk_ci_fc_row_034 => (r#"bulk ci 034"#, r###"(( 1 )); print -r $?"###);
        bulk_ci_fc_row_035 => (r#"bulk ci 035"#, r###"(( 0 )); print -r $?"###);
        bulk_ci_fc_row_036 => (r#"bulk ci 036"#, r###": $(( 0 )) || print -r z"###);
        bulk_ci_fc_row_037 => (r#"bulk ci 037"#, r###": $(( 1 )) && print -r y"###);
        bulk_ci_fc_row_038 => (r#"bulk ci 038"#, r###"let x=2+2; print -r $x"###);
        bulk_ci_fc_row_039 => (r#"bulk ci 039"#, r###"(( x = 5 )); print -r $x"###);
        bulk_ci_fc_row_040 => (r#"bulk ci 040"#, r###"typeset -F f=2.5; print -r $f"###);
        bulk_ci_fc_row_041 => (r#"bulk ci 041"#, r###"typeset -E e=2.5; print -r $e"###);
        bulk_ci_fc_row_042 => (r#"bulk ci 042"#, r###"typeset -i n=07; print -r $n"###);
        bulk_ci_fc_row_043 => (r#"bulk ci 043"#, r###"typeset -l s=ABC; print -r $s"###);
        bulk_ci_fc_row_044 => (r#"bulk ci 044"#, r###"typeset -u s=abc; print -r $s"###);
        bulk_ci_fc_row_045 => (r#"bulk ci 045"#, r###"typeset -r x=1; x=2; print -r $x"###);
        bulk_ci_fc_row_046 => (r#"bulk ci 046"#, r###"typeset -h s; s=abc; print -r $s"###);
        bulk_ci_fc_row_047 => (r#"bulk ci 047"#, r###"typeset -H s; s=abc; print -r $s"###);
        bulk_ci_fc_row_048 => (r#"bulk ci 048"#, r###"typeset -b n=255; print -r $n"###);
    }
}

mod corpus_dash_fc_bulk_cj {
    use super::*;

    parity_gap_tests! {
        bulk_cj_fc_row_001 => (r#"bulk cj 001"#, r###"typeset -A m; m=(k v); print -r ${(v)m}"###);
        bulk_cj_fc_row_002 => (r#"bulk cj 002"#, r###"typeset -A m; m=(a 1 b 2); print -r ${(kv)m}"###);
        bulk_cj_fc_row_003 => (r#"bulk cj 003"#, r###"print -r ${(o)lst}; lst=(z a m)"###);
        bulk_cj_fc_row_004 => (r#"bulk cj 004"#, r###"print -r ${(O)lst}; lst=(z a m)"###);
        bulk_cj_fc_row_005 => (r#"bulk cj 005"#, r###"print -r ${(i)lst}; lst=(z a m)"###);
        bulk_cj_fc_row_006 => (r#"bulk cj 006"#, r###"a=(x y); print -r ${^a}"###);
        bulk_cj_fc_row_007 => (r#"bulk cj 007"#, r###"a=(1 2); b=(a b); print -r ${^a}${^b}"###);
        bulk_cj_fc_row_008 => (r#"bulk cj 008"#, r###"setopt braceccl; print -r {a,b}"###);
        bulk_cj_fc_row_009 => (r#"bulk cj 009"#, r###"print -r {1..3}"###);
        bulk_cj_fc_row_010 => (r#"bulk cj 010"#, r###"print -r {01..03}"###);
        bulk_cj_fc_row_011 => (r#"bulk cj 011"#, r###"print -r {a..c}"###);
        bulk_cj_fc_row_012 => (r#"bulk cj 012"#, r###"print -r {1..4..2}"###);
        bulk_cj_fc_row_013 => (r#"bulk cj 013"#, r###"print -r ${~pattern}; pattern='*'; :"###);
        bulk_cj_fc_row_014 => (r#"bulk cj 014"#, r###"integer x=3; (( x++ )); print -r $x"###);
        bulk_cj_fc_row_015 => (r#"bulk cj 015"#, r###"integer x=3; (( ++x )); print -r $x"###);
        bulk_cj_fc_row_016 => (r#"bulk cj 016"#, r###"integer x=3; (( x-- )); print -r $x"###);
        bulk_cj_fc_row_017 => (r#"bulk cj 017"#, r###"integer x=3; print -r $(( x ** 2 ))"###);
        bulk_cj_fc_row_018 => (r#"bulk cj 018"#, r###"float f=1.5; print -r $(( f + 1 ))"###);
        bulk_cj_fc_row_019 => (r#"bulk cj 019"#, r###"print -r $(( 7 / 2 ))"###);
        bulk_cj_fc_row_020 => (r#"bulk cj 020"#, r###"print -r $(( 7.0 / 2 ))"###);
        bulk_cj_fc_row_021 => (r#"bulk cj 021"#, r###"(( 1 )); print -r $?"###);
        bulk_cj_fc_row_022 => (r#"bulk cj 022"#, r###"(( 0 )); print -r $?"###);
        bulk_cj_fc_row_023 => (r#"bulk cj 023"#, r###": $(( 0 )) || print -r z"###);
        bulk_cj_fc_row_024 => (r#"bulk cj 024"#, r###": $(( 1 )) && print -r y"###);
        bulk_cj_fc_row_025 => (r#"bulk cj 025"#, r###"let x=2+2; print -r $x"###);
        bulk_cj_fc_row_026 => (r#"bulk cj 026"#, r###"(( x = 5 )); print -r $x"###);
        bulk_cj_fc_row_027 => (r#"bulk cj 027"#, r###"typeset -F f=2.5; print -r $f"###);
        bulk_cj_fc_row_028 => (r#"bulk cj 028"#, r###"typeset -E e=2.5; print -r $e"###);
        bulk_cj_fc_row_029 => (r#"bulk cj 029"#, r###"typeset -i n=07; print -r $n"###);
        bulk_cj_fc_row_030 => (r#"bulk cj 030"#, r###"typeset -l s=ABC; print -r $s"###);
        bulk_cj_fc_row_031 => (r#"bulk cj 031"#, r###"typeset -u s=abc; print -r $s"###);
        bulk_cj_fc_row_032 => (r#"bulk cj 032"#, r###"typeset -r x=1; x=2; print -r $x"###);
        bulk_cj_fc_row_033 => (r#"bulk cj 033"#, r###"typeset -h s; s=abc; print -r $s"###);
        bulk_cj_fc_row_034 => (r#"bulk cj 034"#, r###"typeset -H s; s=abc; print -r $s"###);
        bulk_cj_fc_row_035 => (r#"bulk cj 035"#, r###"typeset -b n=255; print -r $n"###);
        bulk_cj_fc_row_036 => (r#"bulk cj 036"#, r###"typeset -o n=7; print -r $n"###);
        bulk_cj_fc_row_037 => (r#"bulk cj 037"#, r###"typeset -aU u; u=(a a b); print -r ${(j:,:)u}"###);
        bulk_cj_fc_row_038 => (r#"bulk cj 038"#, r###"local a; a=1; print -r $a"###);
        bulk_cj_fc_row_039 => (r#"bulk cj 039"#, r###"local -i n=5; print -r $(( n * 2 ))"###);
        bulk_cj_fc_row_040 => (r#"bulk cj 040"#, r###"local -a arr; arr=(x); print -r $arr[1]"###);
        bulk_cj_fc_row_041 => (r#"bulk cj 041"#, r###"fn(){ local x=1; print -r $x; }; fn"###);
        bulk_cj_fc_row_042 => (r#"bulk cj 042"#, r###"fn(){ typeset -a a; a=(1); print -r ${#a}; }; fn"###);
        bulk_cj_fc_row_043 => (r#"bulk cj 043"#, r###"autoload -Uz add-zsh-hook 2>/dev/null; print -r $?"###);
        bulk_cj_fc_row_044 => (r#"bulk cj 044"#, r###"emulate -L zsh; print -r $?"###);
        bulk_cj_fc_row_045 => (r#"bulk cj 045"#, r###"setopt localoptions; print -r $?"###);
        bulk_cj_fc_row_046 => (r#"bulk cj 046"#, r###"unsetopt localoptions 2>/dev/null; print -r $?"###);
        bulk_cj_fc_row_047 => (r#"bulk cj 047"#, r###"setopt pipefail; false | true; print -r $?"###);
        bulk_cj_fc_row_048 => (r#"bulk cj 048"#, r###"setopt no_pipefail; false | true; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_ck {
    use super::*;

    parity_gap_tests! {
        bulk_ck_fc_row_001 => (r#"bulk ck 001"#, r###"integer x=3; print -r $(( x ** 2 ))"###);
        bulk_ck_fc_row_002 => (r#"bulk ck 002"#, r###"float f=1.5; print -r $(( f + 1 ))"###);
        bulk_ck_fc_row_003 => (r#"bulk ck 003"#, r###"print -r $(( 7 / 2 ))"###);
        bulk_ck_fc_row_004 => (r#"bulk ck 004"#, r###"print -r $(( 7.0 / 2 ))"###);
        bulk_ck_fc_row_005 => (r#"bulk ck 005"#, r###"(( 1 )); print -r $?"###);
        bulk_ck_fc_row_006 => (r#"bulk ck 006"#, r###"(( 0 )); print -r $?"###);
        bulk_ck_fc_row_007 => (r#"bulk ck 007"#, r###": $(( 0 )) || print -r z"###);
        bulk_ck_fc_row_008 => (r#"bulk ck 008"#, r###": $(( 1 )) && print -r y"###);
        bulk_ck_fc_row_009 => (r#"bulk ck 009"#, r###"let x=2+2; print -r $x"###);
        bulk_ck_fc_row_010 => (r#"bulk ck 010"#, r###"(( x = 5 )); print -r $x"###);
        bulk_ck_fc_row_011 => (r#"bulk ck 011"#, r###"typeset -F f=2.5; print -r $f"###);
        bulk_ck_fc_row_012 => (r#"bulk ck 012"#, r###"typeset -E e=2.5; print -r $e"###);
        bulk_ck_fc_row_013 => (r#"bulk ck 013"#, r###"typeset -i n=07; print -r $n"###);
        bulk_ck_fc_row_014 => (r#"bulk ck 014"#, r###"typeset -l s=ABC; print -r $s"###);
        bulk_ck_fc_row_015 => (r#"bulk ck 015"#, r###"typeset -u s=abc; print -r $s"###);
        bulk_ck_fc_row_016 => (r#"bulk ck 016"#, r###"typeset -r x=1; x=2; print -r $x"###);
        bulk_ck_fc_row_017 => (r#"bulk ck 017"#, r###"typeset -h s; s=abc; print -r $s"###);
        bulk_ck_fc_row_018 => (r#"bulk ck 018"#, r###"typeset -H s; s=abc; print -r $s"###);
        bulk_ck_fc_row_019 => (r#"bulk ck 019"#, r###"typeset -b n=255; print -r $n"###);
        bulk_ck_fc_row_020 => (r#"bulk ck 020"#, r###"typeset -o n=7; print -r $n"###);
        bulk_ck_fc_row_021 => (r#"bulk ck 021"#, r###"typeset -aU u; u=(a a b); print -r ${(j:,:)u}"###);
        bulk_ck_fc_row_022 => (r#"bulk ck 022"#, r###"local a; a=1; print -r $a"###);
        bulk_ck_fc_row_023 => (r#"bulk ck 023"#, r###"local -i n=5; print -r $(( n * 2 ))"###);
        bulk_ck_fc_row_024 => (r#"bulk ck 024"#, r###"local -a arr; arr=(x); print -r $arr[1]"###);
        bulk_ck_fc_row_025 => (r#"bulk ck 025"#, r###"fn(){ local x=1; print -r $x; }; fn"###);
        bulk_ck_fc_row_026 => (r#"bulk ck 026"#, r###"fn(){ typeset -a a; a=(1); print -r ${#a}; }; fn"###);
        bulk_ck_fc_row_027 => (r#"bulk ck 027"#, r###"autoload -Uz add-zsh-hook 2>/dev/null; print -r $?"###);
        bulk_ck_fc_row_028 => (r#"bulk ck 028"#, r###"emulate -L zsh; print -r $?"###);
        bulk_ck_fc_row_029 => (r#"bulk ck 029"#, r###"setopt localoptions; print -r $?"###);
        bulk_ck_fc_row_030 => (r#"bulk ck 030"#, r###"unsetopt localoptions 2>/dev/null; print -r $?"###);
        bulk_ck_fc_row_031 => (r#"bulk ck 031"#, r###"setopt pipefail; false | true; print -r $?"###);
        bulk_ck_fc_row_032 => (r#"bulk ck 032"#, r###"setopt no_pipefail; false | true; print -r $?"###);
        bulk_ck_fc_row_033 => (r#"bulk ck 033"#, r###"setopt nullglob; print -r ${#files}; files=(/no/such/*)"###);
        bulk_ck_fc_row_034 => (r#"bulk ck 034"#, r###"setopt nonomatch; print -r ${#files}; files=(/no/such/*)"###);
        bulk_ck_fc_row_035 => (r#"bulk ck 035"#, r###"setopt extendedglob; print -r $?"###);
        bulk_ck_fc_row_036 => (r#"bulk ck 036"#, r###"setopt shwordsplit; print -r $?"###);
        bulk_ck_fc_row_037 => (r#"bulk ck 037"#, r###"setopt no_shwordsplit; print -r $?"###);
        bulk_ck_fc_row_038 => (r#"bulk ck 038"#, r###"setopt interactivecomments; print -r $?"###);
        bulk_ck_fc_row_039 => (r#"bulk ck 039"#, r###"setopt no_interactivecomments; print -r $?"###);
        bulk_ck_fc_row_040 => (r#"bulk ck 040"#, r###"setopt multios; print -r $?"###);
        bulk_ck_fc_row_041 => (r#"bulk ck 041"#, r###"setopt noclobber; print -r $?"###);
        bulk_ck_fc_row_042 => (r#"bulk ck 042"#, r###"setopt clobber; print -r $?"###);
        bulk_ck_fc_row_043 => (r#"bulk ck 043"#, r###"setopt histexpand; print -r $?"###);
        bulk_ck_fc_row_044 => (r#"bulk ck 044"#, r###"setopt no_histexpand; print -r $?"###);
        bulk_ck_fc_row_045 => (r#"bulk ck 045"#, r###"setopt banghist; print -r $?"###);
        bulk_ck_fc_row_046 => (r#"bulk ck 046"#, r###"setopt sharehistory; print -r $?"###);
        bulk_ck_fc_row_047 => (r#"bulk ck 047"#, r###"setopt incappendhistory; print -r $?"###);
        bulk_ck_fc_row_048 => (r#"bulk ck 048"#, r###"setopt extendedhistory; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_cl {
    use super::*;

    parity_gap_tests! {
        bulk_cl_fc_row_001 => (r#"bulk cl 001"#, r###"typeset -H s; s=abc; print -r $s"###);
        bulk_cl_fc_row_002 => (r#"bulk cl 002"#, r###"typeset -b n=255; print -r $n"###);
        bulk_cl_fc_row_003 => (r#"bulk cl 003"#, r###"typeset -o n=7; print -r $n"###);
        bulk_cl_fc_row_004 => (r#"bulk cl 004"#, r###"typeset -aU u; u=(a a b); print -r ${(j:,:)u}"###);
        bulk_cl_fc_row_005 => (r#"bulk cl 005"#, r###"local a; a=1; print -r $a"###);
        bulk_cl_fc_row_006 => (r#"bulk cl 006"#, r###"local -i n=5; print -r $(( n * 2 ))"###);
        bulk_cl_fc_row_007 => (r#"bulk cl 007"#, r###"local -a arr; arr=(x); print -r $arr[1]"###);
        bulk_cl_fc_row_008 => (r#"bulk cl 008"#, r###"fn(){ local x=1; print -r $x; }; fn"###);
        bulk_cl_fc_row_009 => (r#"bulk cl 009"#, r###"fn(){ typeset -a a; a=(1); print -r ${#a}; }; fn"###);
        bulk_cl_fc_row_010 => (r#"bulk cl 010"#, r###"autoload -Uz add-zsh-hook 2>/dev/null; print -r $?"###);
        bulk_cl_fc_row_011 => (r#"bulk cl 011"#, r###"emulate -L zsh; print -r $?"###);
        bulk_cl_fc_row_012 => (r#"bulk cl 012"#, r###"setopt localoptions; print -r $?"###);
        bulk_cl_fc_row_013 => (r#"bulk cl 013"#, r###"unsetopt localoptions 2>/dev/null; print -r $?"###);
        bulk_cl_fc_row_014 => (r#"bulk cl 014"#, r###"setopt pipefail; false | true; print -r $?"###);
        bulk_cl_fc_row_015 => (r#"bulk cl 015"#, r###"setopt no_pipefail; false | true; print -r $?"###);
        bulk_cl_fc_row_016 => (r#"bulk cl 016"#, r###"setopt nullglob; print -r ${#files}; files=(/no/such/*)"###);
        bulk_cl_fc_row_017 => (r#"bulk cl 017"#, r###"setopt nonomatch; print -r ${#files}; files=(/no/such/*)"###);
        bulk_cl_fc_row_018 => (r#"bulk cl 018"#, r###"setopt extendedglob; print -r $?"###);
        bulk_cl_fc_row_019 => (r#"bulk cl 019"#, r###"setopt shwordsplit; print -r $?"###);
        bulk_cl_fc_row_020 => (r#"bulk cl 020"#, r###"setopt no_shwordsplit; print -r $?"###);
        bulk_cl_fc_row_021 => (r#"bulk cl 021"#, r###"setopt interactivecomments; print -r $?"###);
        bulk_cl_fc_row_022 => (r#"bulk cl 022"#, r###"setopt no_interactivecomments; print -r $?"###);
        bulk_cl_fc_row_023 => (r#"bulk cl 023"#, r###"setopt multios; print -r $?"###);
        bulk_cl_fc_row_024 => (r#"bulk cl 024"#, r###"setopt noclobber; print -r $?"###);
        bulk_cl_fc_row_025 => (r#"bulk cl 025"#, r###"setopt clobber; print -r $?"###);
        bulk_cl_fc_row_026 => (r#"bulk cl 026"#, r###"setopt histexpand; print -r $?"###);
        bulk_cl_fc_row_027 => (r#"bulk cl 027"#, r###"setopt no_histexpand; print -r $?"###);
        bulk_cl_fc_row_028 => (r#"bulk cl 028"#, r###"setopt banghist; print -r $?"###);
        bulk_cl_fc_row_029 => (r#"bulk cl 029"#, r###"setopt sharehistory; print -r $?"###);
        bulk_cl_fc_row_030 => (r#"bulk cl 030"#, r###"setopt incappendhistory; print -r $?"###);
        bulk_cl_fc_row_031 => (r#"bulk cl 031"#, r###"setopt extendedhistory; print -r $?"###);
        bulk_cl_fc_row_032 => (r#"bulk cl 032"#, r###"setopt histignoredups; print -r $?"###);
        bulk_cl_fc_row_033 => (r#"bulk cl 033"#, r###"setopt histignorespace; print -r $?"###);
        bulk_cl_fc_row_034 => (r#"bulk cl 034"#, r###"setopt histreduceblanks; print -r $?"###);
        bulk_cl_fc_row_035 => (r#"bulk cl 035"#, r###"setopt histverify; print -r $?"###);
        bulk_cl_fc_row_036 => (r#"bulk cl 036"#, r###"setopt appendhistory; print -r $?"###);
        bulk_cl_fc_row_037 => (r#"bulk cl 037"#, r###"setopt no_beep; print -r $?"###);
        bulk_cl_fc_row_038 => (r#"bulk cl 038"#, r###"setopt no_listbeep; print -r $?"###);
        bulk_cl_fc_row_039 => (r#"bulk cl 039"#, r###"setopt auto_cd; print -r $?"###);
        bulk_cl_fc_row_040 => (r#"bulk cl 040"#, r###"setopt no_auto_cd; print -r $?"###);
        bulk_cl_fc_row_041 => (r#"bulk cl 041"#, r###"setopt correct; print -r $?"###);
        bulk_cl_fc_row_042 => (r#"bulk cl 042"#, r###"setopt nocorrect; print -r $?"###);
        bulk_cl_fc_row_043 => (r#"bulk cl 043"#, r###"setopt completealiases; print -r $?"###);
        bulk_cl_fc_row_044 => (r#"bulk cl 044"#, r###"setopt globdots; print -r $?"###);
        bulk_cl_fc_row_045 => (r#"bulk cl 045"#, r###"setopt noglobdots; print -r $?"###);
        bulk_cl_fc_row_046 => (r#"bulk cl 046"#, r###"setopt numericglobsort; print -r $?"###);
        bulk_cl_fc_row_047 => (r#"bulk cl 047"#, r###"setopt markdirs; print -r $?"###);
        bulk_cl_fc_row_048 => (r#"bulk cl 048"#, r###"setopt nomarkdirs; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_cm {
    use super::*;

    parity_gap_tests! {
        bulk_cm_fc_row_001 => (r#"bulk cm 001"#, r###"setopt nonomatch; print -r ${#files}; files=(/no/such/*)"###);
        bulk_cm_fc_row_002 => (r#"bulk cm 002"#, r###"setopt extendedglob; print -r $?"###);
        bulk_cm_fc_row_003 => (r#"bulk cm 003"#, r###"setopt shwordsplit; print -r $?"###);
        bulk_cm_fc_row_004 => (r#"bulk cm 004"#, r###"setopt no_shwordsplit; print -r $?"###);
        bulk_cm_fc_row_005 => (r#"bulk cm 005"#, r###"setopt interactivecomments; print -r $?"###);
        bulk_cm_fc_row_006 => (r#"bulk cm 006"#, r###"setopt no_interactivecomments; print -r $?"###);
        bulk_cm_fc_row_007 => (r#"bulk cm 007"#, r###"setopt multios; print -r $?"###);
        bulk_cm_fc_row_008 => (r#"bulk cm 008"#, r###"setopt noclobber; print -r $?"###);
        bulk_cm_fc_row_009 => (r#"bulk cm 009"#, r###"setopt clobber; print -r $?"###);
        bulk_cm_fc_row_010 => (r#"bulk cm 010"#, r###"setopt histexpand; print -r $?"###);
        bulk_cm_fc_row_011 => (r#"bulk cm 011"#, r###"setopt no_histexpand; print -r $?"###);
        bulk_cm_fc_row_012 => (r#"bulk cm 012"#, r###"setopt banghist; print -r $?"###);
        bulk_cm_fc_row_013 => (r#"bulk cm 013"#, r###"setopt sharehistory; print -r $?"###);
        bulk_cm_fc_row_014 => (r#"bulk cm 014"#, r###"setopt incappendhistory; print -r $?"###);
        bulk_cm_fc_row_015 => (r#"bulk cm 015"#, r###"setopt extendedhistory; print -r $?"###);
        bulk_cm_fc_row_016 => (r#"bulk cm 016"#, r###"setopt histignoredups; print -r $?"###);
        bulk_cm_fc_row_017 => (r#"bulk cm 017"#, r###"setopt histignorespace; print -r $?"###);
        bulk_cm_fc_row_018 => (r#"bulk cm 018"#, r###"setopt histreduceblanks; print -r $?"###);
        bulk_cm_fc_row_019 => (r#"bulk cm 019"#, r###"setopt histverify; print -r $?"###);
        bulk_cm_fc_row_020 => (r#"bulk cm 020"#, r###"setopt appendhistory; print -r $?"###);
        bulk_cm_fc_row_021 => (r#"bulk cm 021"#, r###"setopt no_beep; print -r $?"###);
        bulk_cm_fc_row_022 => (r#"bulk cm 022"#, r###"setopt no_listbeep; print -r $?"###);
        bulk_cm_fc_row_023 => (r#"bulk cm 023"#, r###"setopt auto_cd; print -r $?"###);
        bulk_cm_fc_row_024 => (r#"bulk cm 024"#, r###"setopt no_auto_cd; print -r $?"###);
        bulk_cm_fc_row_025 => (r#"bulk cm 025"#, r###"setopt correct; print -r $?"###);
        bulk_cm_fc_row_026 => (r#"bulk cm 026"#, r###"setopt nocorrect; print -r $?"###);
        bulk_cm_fc_row_027 => (r#"bulk cm 027"#, r###"setopt completealiases; print -r $?"###);
        bulk_cm_fc_row_028 => (r#"bulk cm 028"#, r###"setopt globdots; print -r $?"###);
        bulk_cm_fc_row_029 => (r#"bulk cm 029"#, r###"setopt noglobdots; print -r $?"###);
        bulk_cm_fc_row_030 => (r#"bulk cm 030"#, r###"setopt numericglobsort; print -r $?"###);
        bulk_cm_fc_row_031 => (r#"bulk cm 031"#, r###"setopt markdirs; print -r $?"###);
        bulk_cm_fc_row_032 => (r#"bulk cm 032"#, r###"setopt nomarkdirs; print -r $?"###);
        bulk_cm_fc_row_033 => (r#"bulk cm 033"#, r###"setopt chase_links; print -r $?"###);
        bulk_cm_fc_row_034 => (r#"bulk cm 034"#, r###"setopt no_chase_links; print -r $?"###);
        bulk_cm_fc_row_035 => (r#"bulk cm 035"#, r###"setopt pushdignoredups; print -r $?"###);
        bulk_cm_fc_row_036 => (r#"bulk cm 036"#, r###"setopt pushdsilent; print -r $?"###);
        bulk_cm_fc_row_037 => (r#"bulk cm 037"#, r###"setopt pushdtohome; print -r $?"###);
        bulk_cm_fc_row_038 => (r#"bulk cm 038"#, r###"setopt autopushd; print -r $?"###);
        bulk_cm_fc_row_039 => (r#"bulk cm 039"#, r###"setopt pushdminus; print -r $?"###);
        bulk_cm_fc_row_040 => (r#"bulk cm 040"#, r###"setopt pushdignoredups; print -r $?"###);
        bulk_cm_fc_row_041 => (r#"bulk cm 041"#, r###"dirs -p 2>/dev/null | head -1; print -r $?"###);
        bulk_cm_fc_row_042 => (r#"bulk cm 042"#, r###"pushd /tmp 2>/dev/null; popd 2>/dev/null; print -r $?"###);
        bulk_cm_fc_row_043 => (r#"bulk cm 043"#, r###"cd -q / 2>/dev/null; print -r $?"###);
        bulk_cm_fc_row_044 => (r#"bulk cm 044"#, r###"print -r $PWD"###);
        bulk_cm_fc_row_045 => (r#"bulk cm 045"#, r###"print -r ${PWD:h}"###);
        bulk_cm_fc_row_046 => (r#"bulk cm 046"#, r###"print -r ${PWD:t}"###);
        bulk_cm_fc_row_047 => (r#"bulk cm 047"#, r###"print -r ${PWD:r}"###);
        bulk_cm_fc_row_048 => (r#"bulk cm 048"#, r###"print -r ${PWD:e}"###);
    }
}

mod corpus_dash_fc_bulk_cn {
    use super::*;

    parity_gap_tests! {
        bulk_cn_fc_row_001 => (r#"bulk cn 001"#, r###"setopt histreduceblanks; print -r $?"###);
        bulk_cn_fc_row_002 => (r#"bulk cn 002"#, r###"setopt histverify; print -r $?"###);
        bulk_cn_fc_row_003 => (r#"bulk cn 003"#, r###"setopt appendhistory; print -r $?"###);
        bulk_cn_fc_row_004 => (r#"bulk cn 004"#, r###"setopt no_beep; print -r $?"###);
        bulk_cn_fc_row_005 => (r#"bulk cn 005"#, r###"setopt no_listbeep; print -r $?"###);
        bulk_cn_fc_row_006 => (r#"bulk cn 006"#, r###"setopt auto_cd; print -r $?"###);
        bulk_cn_fc_row_007 => (r#"bulk cn 007"#, r###"setopt no_auto_cd; print -r $?"###);
        bulk_cn_fc_row_008 => (r#"bulk cn 008"#, r###"setopt correct; print -r $?"###);
        bulk_cn_fc_row_009 => (r#"bulk cn 009"#, r###"setopt nocorrect; print -r $?"###);
        bulk_cn_fc_row_010 => (r#"bulk cn 010"#, r###"setopt completealiases; print -r $?"###);
        bulk_cn_fc_row_011 => (r#"bulk cn 011"#, r###"setopt globdots; print -r $?"###);
        bulk_cn_fc_row_012 => (r#"bulk cn 012"#, r###"setopt noglobdots; print -r $?"###);
        bulk_cn_fc_row_013 => (r#"bulk cn 013"#, r###"setopt numericglobsort; print -r $?"###);
        bulk_cn_fc_row_014 => (r#"bulk cn 014"#, r###"setopt markdirs; print -r $?"###);
        bulk_cn_fc_row_015 => (r#"bulk cn 015"#, r###"setopt nomarkdirs; print -r $?"###);
        bulk_cn_fc_row_016 => (r#"bulk cn 016"#, r###"setopt chase_links; print -r $?"###);
        bulk_cn_fc_row_017 => (r#"bulk cn 017"#, r###"setopt no_chase_links; print -r $?"###);
        bulk_cn_fc_row_018 => (r#"bulk cn 018"#, r###"setopt pushdignoredups; print -r $?"###);
        bulk_cn_fc_row_019 => (r#"bulk cn 019"#, r###"setopt pushdsilent; print -r $?"###);
        bulk_cn_fc_row_020 => (r#"bulk cn 020"#, r###"setopt pushdtohome; print -r $?"###);
        bulk_cn_fc_row_021 => (r#"bulk cn 021"#, r###"setopt autopushd; print -r $?"###);
        bulk_cn_fc_row_022 => (r#"bulk cn 022"#, r###"setopt pushdminus; print -r $?"###);
        bulk_cn_fc_row_023 => (r#"bulk cn 023"#, r###"setopt pushdignoredups; print -r $?"###);
        bulk_cn_fc_row_024 => (r#"bulk cn 024"#, r###"dirs -p 2>/dev/null | head -1; print -r $?"###);
        bulk_cn_fc_row_025 => (r#"bulk cn 025"#, r###"pushd /tmp 2>/dev/null; popd 2>/dev/null; print -r $?"###);
        bulk_cn_fc_row_026 => (r#"bulk cn 026"#, r###"cd -q / 2>/dev/null; print -r $?"###);
        bulk_cn_fc_row_027 => (r#"bulk cn 027"#, r###"print -r $PWD"###);
        bulk_cn_fc_row_028 => (r#"bulk cn 028"#, r###"print -r ${PWD:h}"###);
        bulk_cn_fc_row_029 => (r#"bulk cn 029"#, r###"print -r ${PWD:t}"###);
        bulk_cn_fc_row_030 => (r#"bulk cn 030"#, r###"print -r ${PWD:r}"###);
        bulk_cn_fc_row_031 => (r#"bulk cn 031"#, r###"print -r ${PWD:e}"###);
        bulk_cn_fc_row_032 => (r#"bulk cn 032"#, r###"print -r ${PWD:a}"###);
        bulk_cn_fc_row_033 => (r#"bulk cn 033"#, r###"print -r ${PWD:A}"###);
        bulk_cn_fc_row_034 => (r#"bulk cn 034"#, r###"read -r line <<< 'one'; print -r $line"###);
        bulk_cn_fc_row_035 => (r#"bulk cn 035"#, r###"read -r a b <<< 'x y'; print -r $a-$b"###);
        bulk_cn_fc_row_036 => (r#"bulk cn 036"#, r###"print -r $'tab\there'"###);
        bulk_cn_fc_row_037 => (r#"bulk cn 037"#, r###"print -r $'line1\nline2'"###);
        bulk_cn_fc_row_038 => (r#"bulk cn 038"#, r###"printf '%q\n' 'a b'"###);
        bulk_cn_fc_row_039 => (r#"bulk cn 039"#, r###"printf '%s\n' ok"###);
        bulk_cn_fc_row_040 => (r#"bulk cn 040"#, r###"print -rn -- end"###);
        bulk_cn_fc_row_041 => (r#"bulk cn 041"#, r###"print -rl -- a b"###);
        bulk_cn_fc_row_042 => (r#"bulk cn 042"#, r###"print -fc '%s\n' hi"###);
        bulk_cn_fc_row_043 => (r#"bulk cn 043"#, r###"whence -w print 2>/dev/null; print -r $?"###);
        bulk_cn_fc_row_044 => (r#"bulk cn 044"#, r###"whence -c print 2>/dev/null; print -r $?"###);
        bulk_cn_fc_row_045 => (r#"bulk cn 045"#, r###"which print 2>/dev/null; print -r $?"###);
        bulk_cn_fc_row_046 => (r#"bulk cn 046"#, r###"command -v print 2>/dev/null; print -r $?"###);
        bulk_cn_fc_row_047 => (r#"bulk cn 047"#, r###"hash -r 2>/dev/null; print -r $?"###);
        bulk_cn_fc_row_048 => (r#"bulk cn 048"#, r###"rehash 2>/dev/null; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_co {
    use super::*;

    parity_gap_tests! {
        bulk_co_fc_row_001 => (r#"bulk co 001"#, r###"setopt pushdignoredups; print -r $?"###);
        bulk_co_fc_row_002 => (r#"bulk co 002"#, r###"setopt pushdsilent; print -r $?"###);
        bulk_co_fc_row_003 => (r#"bulk co 003"#, r###"setopt pushdtohome; print -r $?"###);
        bulk_co_fc_row_004 => (r#"bulk co 004"#, r###"setopt autopushd; print -r $?"###);
        bulk_co_fc_row_005 => (r#"bulk co 005"#, r###"setopt pushdminus; print -r $?"###);
        bulk_co_fc_row_006 => (r#"bulk co 006"#, r###"setopt pushdignoredups; print -r $?"###);
        bulk_co_fc_row_007 => (r#"bulk co 007"#, r###"dirs -p 2>/dev/null | head -1; print -r $?"###);
        bulk_co_fc_row_008 => (r#"bulk co 008"#, r###"pushd /tmp 2>/dev/null; popd 2>/dev/null; print -r $?"###);
        bulk_co_fc_row_009 => (r#"bulk co 009"#, r###"cd -q / 2>/dev/null; print -r $?"###);
        bulk_co_fc_row_010 => (r#"bulk co 010"#, r###"print -r $PWD"###);
        bulk_co_fc_row_011 => (r#"bulk co 011"#, r###"print -r ${PWD:h}"###);
        bulk_co_fc_row_012 => (r#"bulk co 012"#, r###"print -r ${PWD:t}"###);
        bulk_co_fc_row_013 => (r#"bulk co 013"#, r###"print -r ${PWD:r}"###);
        bulk_co_fc_row_014 => (r#"bulk co 014"#, r###"print -r ${PWD:e}"###);
        bulk_co_fc_row_015 => (r#"bulk co 015"#, r###"print -r ${PWD:a}"###);
        bulk_co_fc_row_016 => (r#"bulk co 016"#, r###"print -r ${PWD:A}"###);
        bulk_co_fc_row_017 => (r#"bulk co 017"#, r###"read -r line <<< 'one'; print -r $line"###);
        bulk_co_fc_row_018 => (r#"bulk co 018"#, r###"read -r a b <<< 'x y'; print -r $a-$b"###);
        bulk_co_fc_row_019 => (r#"bulk co 019"#, r###"print -r $'tab\there'"###);
        bulk_co_fc_row_020 => (r#"bulk co 020"#, r###"print -r $'line1\nline2'"###);
        bulk_co_fc_row_021 => (r#"bulk co 021"#, r###"printf '%q\n' 'a b'"###);
        bulk_co_fc_row_022 => (r#"bulk co 022"#, r###"printf '%s\n' ok"###);
        bulk_co_fc_row_023 => (r#"bulk co 023"#, r###"print -rn -- end"###);
        bulk_co_fc_row_024 => (r#"bulk co 024"#, r###"print -rl -- a b"###);
        bulk_co_fc_row_025 => (r#"bulk co 025"#, r###"print -fc '%s\n' hi"###);
        bulk_co_fc_row_026 => (r#"bulk co 026"#, r###"whence -w print 2>/dev/null; print -r $?"###);
        bulk_co_fc_row_027 => (r#"bulk co 027"#, r###"whence -c print 2>/dev/null; print -r $?"###);
        bulk_co_fc_row_028 => (r#"bulk co 028"#, r###"which print 2>/dev/null; print -r $?"###);
        bulk_co_fc_row_029 => (r#"bulk co 029"#, r###"command -v print 2>/dev/null; print -r $?"###);
        bulk_co_fc_row_030 => (r#"bulk co 030"#, r###"hash -r 2>/dev/null; print -r $?"###);
        bulk_co_fc_row_031 => (r#"bulk co 031"#, r###"rehash 2>/dev/null; print -r $?"###);
        bulk_co_fc_row_032 => (r#"bulk co 032"#, r###"unalias za 2>/dev/null; alias za=1; unalias za; print -r $?"###);
        bulk_co_fc_row_033 => (r#"bulk co 033"#, r###"alias -L za 2>/dev/null; alias za=z; print -r $?"###);
        bulk_co_fc_row_034 => (r#"bulk co 034"#, r###"export ZA=1; print -r $ZA"###);
        bulk_co_fc_row_035 => (r#"bulk co 035"#, r###"typeset +Z ZA; ZA=1; print -r $ZA"###);
        bulk_co_fc_row_036 => (r#"bulk co 036"#, r###"typeset -x ZB=2; print -r $ZB"###);
        bulk_co_fc_row_037 => (r#"bulk co 037"#, r###"unset ZC; ZC=1; unset ZC; print -r ${+ZC}"###);
        bulk_co_fc_row_038 => (r#"bulk co 038"#, r###"typeset -tH hx; hx=ff; print -r $hx"###);
        bulk_co_fc_row_039 => (r#"bulk co 039"#, r###"shift; print -r $1; set -- a b c"###);
        bulk_co_fc_row_040 => (r#"bulk co 040"#, r###"(( $# )); print -r $#"###);
        bulk_co_fc_row_041 => (r#"bulk co 041"#, r###"print -r ${argv[1]}"###);
        bulk_co_fc_row_042 => (r#"bulk co 042"#, r###"print -r ${*[1]}"###);
        bulk_co_fc_row_043 => (r#"bulk co 043"#, r###"print -r $@[1]"###);
        bulk_co_fc_row_044 => (r#"bulk co 044"#, r###"print -r ${@:2}"###);
        bulk_co_fc_row_045 => (r#"bulk co 045"#, r###"select x in a b; do print -r $x; break; done <<< ''"###);
        bulk_co_fc_row_046 => (r#"bulk co 046"#, r###"zmodload zsh/zutil 2>/dev/null; print -r $?"###);
        bulk_co_fc_row_047 => (r#"bulk co 047"#, r###"zmodload -l 2>/dev/null | head -1; print -r $?"###);
        bulk_co_fc_row_048 => (r#"bulk co 048"#, r###"getconf PATH 2>/dev/null | head -c 1; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_cp {
    use super::*;

    parity_gap_tests! {
        bulk_cp_fc_row_001 => (r#"bulk cp 001"#, r###"print -r ${PWD:A}"###);
        bulk_cp_fc_row_002 => (r#"bulk cp 002"#, r###"read -r line <<< 'one'; print -r $line"###);
        bulk_cp_fc_row_003 => (r#"bulk cp 003"#, r###"read -r a b <<< 'x y'; print -r $a-$b"###);
        bulk_cp_fc_row_004 => (r#"bulk cp 004"#, r###"print -r $'tab\there'"###);
        bulk_cp_fc_row_005 => (r#"bulk cp 005"#, r###"print -r $'line1\nline2'"###);
        bulk_cp_fc_row_006 => (r#"bulk cp 006"#, r###"printf '%q\n' 'a b'"###);
        bulk_cp_fc_row_007 => (r#"bulk cp 007"#, r###"printf '%s\n' ok"###);
        bulk_cp_fc_row_008 => (r#"bulk cp 008"#, r###"print -rn -- end"###);
        bulk_cp_fc_row_009 => (r#"bulk cp 009"#, r###"print -rl -- a b"###);
        bulk_cp_fc_row_010 => (r#"bulk cp 010"#, r###"print -fc '%s\n' hi"###);
        bulk_cp_fc_row_011 => (r#"bulk cp 011"#, r###"whence -w print 2>/dev/null; print -r $?"###);
        bulk_cp_fc_row_012 => (r#"bulk cp 012"#, r###"whence -c print 2>/dev/null; print -r $?"###);
        bulk_cp_fc_row_013 => (r#"bulk cp 013"#, r###"which print 2>/dev/null; print -r $?"###);
        bulk_cp_fc_row_014 => (r#"bulk cp 014"#, r###"command -v print 2>/dev/null; print -r $?"###);
        bulk_cp_fc_row_015 => (r#"bulk cp 015"#, r###"hash -r 2>/dev/null; print -r $?"###);
        bulk_cp_fc_row_016 => (r#"bulk cp 016"#, r###"rehash 2>/dev/null; print -r $?"###);
        bulk_cp_fc_row_017 => (r#"bulk cp 017"#, r###"unalias za 2>/dev/null; alias za=1; unalias za; print -r $?"###);
        bulk_cp_fc_row_018 => (r#"bulk cp 018"#, r###"alias -L za 2>/dev/null; alias za=z; print -r $?"###);
        bulk_cp_fc_row_019 => (r#"bulk cp 019"#, r###"export ZA=1; print -r $ZA"###);
        bulk_cp_fc_row_020 => (r#"bulk cp 020"#, r###"typeset +Z ZA; ZA=1; print -r $ZA"###);
        bulk_cp_fc_row_021 => (r#"bulk cp 021"#, r###"typeset -x ZB=2; print -r $ZB"###);
        bulk_cp_fc_row_022 => (r#"bulk cp 022"#, r###"unset ZC; ZC=1; unset ZC; print -r ${+ZC}"###);
        bulk_cp_fc_row_023 => (r#"bulk cp 023"#, r###"typeset -tH hx; hx=ff; print -r $hx"###);
        bulk_cp_fc_row_024 => (r#"bulk cp 024"#, r###"shift; print -r $1; set -- a b c"###);
        bulk_cp_fc_row_025 => (r#"bulk cp 025"#, r###"(( $# )); print -r $#"###);
        bulk_cp_fc_row_026 => (r#"bulk cp 026"#, r###"print -r ${argv[1]}"###);
        bulk_cp_fc_row_027 => (r#"bulk cp 027"#, r###"print -r ${*[1]}"###);
        bulk_cp_fc_row_028 => (r#"bulk cp 028"#, r###"print -r $@[1]"###);
        bulk_cp_fc_row_029 => (r#"bulk cp 029"#, r###"print -r ${@:2}"###);
        bulk_cp_fc_row_030 => (r#"bulk cp 030"#, r###"select x in a b; do print -r $x; break; done <<< ''"###);
        bulk_cp_fc_row_031 => (r#"bulk cp 031"#, r###"zmodload zsh/zutil 2>/dev/null; print -r $?"###);
        bulk_cp_fc_row_032 => (r#"bulk cp 032"#, r###"zmodload -l 2>/dev/null | head -1; print -r $?"###);
        bulk_cp_fc_row_033 => (r#"bulk cp 033"#, r###"getconf PATH 2>/dev/null | head -c 1; print -r $?"###);
        bulk_cp_fc_row_034 => (r#"bulk cp 034"#, r###"getconf ARG_MAX 2>/dev/null; print -r $?"###);
        bulk_cp_fc_row_035 => (r#"bulk cp 035"#, r###"str=%n; print -r ${(%)str}"###);
        bulk_cp_fc_row_036 => (r#"bulk cp 036"#, r###"str=%N; print -r ${(%)str}"###);
        bulk_cp_fc_row_037 => (r#"bulk cp 037"#, r###"str=%~; print -r ${(%)str}"###);
        bulk_cp_fc_row_038 => (r#"bulk cp 038"#, r###"str=%d; print -r ${(%)str}"###);
        bulk_cp_fc_row_039 => (r#"bulk cp 039"#, r###"str=%m; print -r ${(%)str}"###);
        bulk_cp_fc_row_040 => (r#"bulk cp 040"#, r###"str=%#; print -r ${(%)str}"###);
        bulk_cp_fc_row_041 => (r#"bulk cp 041"#, r###"str=%?; print -r ${(%)str}"###);
        bulk_cp_fc_row_042 => (r#"bulk cp 042"#, r###"str=%_; print -r ${(%)str}"###);
        bulk_cp_fc_row_043 => (r#"bulk cp 043"#, r###"str=%h; print -r ${(%)str}"###);
        bulk_cp_fc_row_044 => (r#"bulk cp 044"#, r###"str=%!; print -r ${(%)str}"###);
        bulk_cp_fc_row_045 => (r#"bulk cp 045"#, r###"str=%i; print -r ${(%)str}"###);
        bulk_cp_fc_row_046 => (r#"bulk cp 046"#, r###"str=%I; print -r ${(%)str}"###);
        bulk_cp_fc_row_047 => (r#"bulk cp 047"#, r###"str=%j; print -r ${(%)str}"###);
        bulk_cp_fc_row_048 => (r#"bulk cp 048"#, r###"str=%C; print -r ${(%)str}"###);
    }
}

mod corpus_dash_fc_bulk_cq {
    use super::*;

    parity_gap_tests! {
        bulk_cq_fc_row_001 => (r#"bulk cq 001"#, r###"alias -L za 2>/dev/null; alias za=z; print -r $?"###);
        bulk_cq_fc_row_002 => (r#"bulk cq 002"#, r###"export ZA=1; print -r $ZA"###);
        bulk_cq_fc_row_003 => (r#"bulk cq 003"#, r###"typeset +Z ZA; ZA=1; print -r $ZA"###);
        bulk_cq_fc_row_004 => (r#"bulk cq 004"#, r###"typeset -x ZB=2; print -r $ZB"###);
        bulk_cq_fc_row_005 => (r#"bulk cq 005"#, r###"unset ZC; ZC=1; unset ZC; print -r ${+ZC}"###);
        bulk_cq_fc_row_006 => (r#"bulk cq 006"#, r###"typeset -tH hx; hx=ff; print -r $hx"###);
        bulk_cq_fc_row_007 => (r#"bulk cq 007"#, r###"shift; print -r $1; set -- a b c"###);
        bulk_cq_fc_row_008 => (r#"bulk cq 008"#, r###"(( $# )); print -r $#"###);
        bulk_cq_fc_row_009 => (r#"bulk cq 009"#, r###"print -r ${argv[1]}"###);
        bulk_cq_fc_row_010 => (r#"bulk cq 010"#, r###"print -r ${*[1]}"###);
        bulk_cq_fc_row_011 => (r#"bulk cq 011"#, r###"print -r $@[1]"###);
        bulk_cq_fc_row_012 => (r#"bulk cq 012"#, r###"print -r ${@:2}"###);
        bulk_cq_fc_row_013 => (r#"bulk cq 013"#, r###"select x in a b; do print -r $x; break; done <<< ''"###);
        bulk_cq_fc_row_014 => (r#"bulk cq 014"#, r###"zmodload zsh/zutil 2>/dev/null; print -r $?"###);
        bulk_cq_fc_row_015 => (r#"bulk cq 015"#, r###"zmodload -l 2>/dev/null | head -1; print -r $?"###);
        bulk_cq_fc_row_016 => (r#"bulk cq 016"#, r###"getconf PATH 2>/dev/null | head -c 1; print -r $?"###);
        bulk_cq_fc_row_017 => (r#"bulk cq 017"#, r###"getconf ARG_MAX 2>/dev/null; print -r $?"###);
        bulk_cq_fc_row_018 => (r#"bulk cq 018"#, r###"str=%n; print -r ${(%)str}"###);
        bulk_cq_fc_row_019 => (r#"bulk cq 019"#, r###"str=%N; print -r ${(%)str}"###);
        bulk_cq_fc_row_020 => (r#"bulk cq 020"#, r###"str=%~; print -r ${(%)str}"###);
        bulk_cq_fc_row_021 => (r#"bulk cq 021"#, r###"str=%d; print -r ${(%)str}"###);
        bulk_cq_fc_row_022 => (r#"bulk cq 022"#, r###"str=%m; print -r ${(%)str}"###);
        bulk_cq_fc_row_023 => (r#"bulk cq 023"#, r###"str=%#; print -r ${(%)str}"###);
        bulk_cq_fc_row_024 => (r#"bulk cq 024"#, r###"str=%?; print -r ${(%)str}"###);
        bulk_cq_fc_row_025 => (r#"bulk cq 025"#, r###"str=%_; print -r ${(%)str}"###);
        bulk_cq_fc_row_026 => (r#"bulk cq 026"#, r###"str=%h; print -r ${(%)str}"###);
        bulk_cq_fc_row_027 => (r#"bulk cq 027"#, r###"str=%!; print -r ${(%)str}"###);
        bulk_cq_fc_row_028 => (r#"bulk cq 028"#, r###"str=%i; print -r ${(%)str}"###);
        bulk_cq_fc_row_029 => (r#"bulk cq 029"#, r###"str=%I; print -r ${(%)str}"###);
        bulk_cq_fc_row_030 => (r#"bulk cq 030"#, r###"str=%j; print -r ${(%)str}"###);
        bulk_cq_fc_row_031 => (r#"bulk cq 031"#, r###"str=%C; print -r ${(%)str}"###);
        bulk_cq_fc_row_032 => (r#"bulk cq 032"#, r###"str=%c; print -r ${(%)str}"###);
        bulk_cq_fc_row_033 => (r#"bulk cq 033"#, r###"str=%D; print -r ${(%)str}"###);
        bulk_cq_fc_row_034 => (r#"bulk cq 034"#, r###"str=%W; print -r ${(%)str}"###);
        bulk_cq_fc_row_035 => (r#"bulk cq 035"#, r###"str=%*; print -r ${(%)str}"###);
        bulk_cq_fc_row_036 => (r#"bulk cq 036"#, r###"str=%v; print -r ${(%)str}"###);
        bulk_cq_fc_row_037 => (r#"bulk cq 037"#, r###"str=%L; print -r ${(%)str}"###);
        bulk_cq_fc_row_038 => (r#"bulk cq 038"#, r###"str=%l; print -r ${(%)str}"###);
        bulk_cq_fc_row_039 => (r#"bulk cq 039"#, r###"str=%y; print -r ${(%)str}"###);
        bulk_cq_fc_row_040 => (r#"bulk cq 040"#, r###"str=%/; print -r ${(%)str}"###);
        bulk_cq_fc_row_041 => (r#"bulk cq 041"#, r###"str=%<; print -r ${(%)str}"###);
        bulk_cq_fc_row_042 => (r#"bulk cq 042"#, r###"str=%>; print -r ${(%)str}"###);
        bulk_cq_fc_row_043 => (r#"bulk cq 043"#, r###"true; print -r $?"###);
        bulk_cq_fc_row_044 => (r#"bulk cq 044"#, r###"false; print -r $?"###);
        bulk_cq_fc_row_045 => (r#"bulk cq 045"#, r###"print -r hello"###);
        bulk_cq_fc_row_046 => (r#"bulk cq 046"#, r###"echo one two"###);
        bulk_cq_fc_row_047 => (r#"bulk cq 047"#, r###"test 1 -eq 1; print -r $?"###);
        bulk_cq_fc_row_048 => (r#"bulk cq 048"#, r###"[ 1 -eq 1 ]; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_cr {
    use super::*;

    parity_gap_tests! {
        bulk_cr_fc_row_001 => (r#"bulk cr 001"#, r###"getconf PATH 2>/dev/null | head -c 1; print -r $?"###);
        bulk_cr_fc_row_002 => (r#"bulk cr 002"#, r###"getconf ARG_MAX 2>/dev/null; print -r $?"###);
        bulk_cr_fc_row_003 => (r#"bulk cr 003"#, r###"str=%n; print -r ${(%)str}"###);
        bulk_cr_fc_row_004 => (r#"bulk cr 004"#, r###"str=%N; print -r ${(%)str}"###);
        bulk_cr_fc_row_005 => (r#"bulk cr 005"#, r###"str=%~; print -r ${(%)str}"###);
        bulk_cr_fc_row_006 => (r#"bulk cr 006"#, r###"str=%d; print -r ${(%)str}"###);
        bulk_cr_fc_row_007 => (r#"bulk cr 007"#, r###"str=%m; print -r ${(%)str}"###);
        bulk_cr_fc_row_008 => (r#"bulk cr 008"#, r###"str=%#; print -r ${(%)str}"###);
        bulk_cr_fc_row_009 => (r#"bulk cr 009"#, r###"str=%?; print -r ${(%)str}"###);
        bulk_cr_fc_row_010 => (r#"bulk cr 010"#, r###"str=%_; print -r ${(%)str}"###);
        bulk_cr_fc_row_011 => (r#"bulk cr 011"#, r###"str=%h; print -r ${(%)str}"###);
        bulk_cr_fc_row_012 => (r#"bulk cr 012"#, r###"str=%!; print -r ${(%)str}"###);
        bulk_cr_fc_row_013 => (r#"bulk cr 013"#, r###"str=%i; print -r ${(%)str}"###);
        bulk_cr_fc_row_014 => (r#"bulk cr 014"#, r###"str=%I; print -r ${(%)str}"###);
        bulk_cr_fc_row_015 => (r#"bulk cr 015"#, r###"str=%j; print -r ${(%)str}"###);
        bulk_cr_fc_row_016 => (r#"bulk cr 016"#, r###"str=%C; print -r ${(%)str}"###);
        bulk_cr_fc_row_017 => (r#"bulk cr 017"#, r###"str=%c; print -r ${(%)str}"###);
        bulk_cr_fc_row_018 => (r#"bulk cr 018"#, r###"str=%D; print -r ${(%)str}"###);
        bulk_cr_fc_row_019 => (r#"bulk cr 019"#, r###"str=%W; print -r ${(%)str}"###);
        bulk_cr_fc_row_020 => (r#"bulk cr 020"#, r###"str=%*; print -r ${(%)str}"###);
        bulk_cr_fc_row_021 => (r#"bulk cr 021"#, r###"str=%v; print -r ${(%)str}"###);
        bulk_cr_fc_row_022 => (r#"bulk cr 022"#, r###"str=%L; print -r ${(%)str}"###);
        bulk_cr_fc_row_023 => (r#"bulk cr 023"#, r###"str=%l; print -r ${(%)str}"###);
        bulk_cr_fc_row_024 => (r#"bulk cr 024"#, r###"str=%y; print -r ${(%)str}"###);
        bulk_cr_fc_row_025 => (r#"bulk cr 025"#, r###"str=%/; print -r ${(%)str}"###);
        bulk_cr_fc_row_026 => (r#"bulk cr 026"#, r###"str=%<; print -r ${(%)str}"###);
        bulk_cr_fc_row_027 => (r#"bulk cr 027"#, r###"str=%>; print -r ${(%)str}"###);
        bulk_cr_fc_row_028 => (r#"bulk cr 028"#, r###"true; print -r $?"###);
        bulk_cr_fc_row_029 => (r#"bulk cr 029"#, r###"false; print -r $?"###);
        bulk_cr_fc_row_030 => (r#"bulk cr 030"#, r###"print -r hello"###);
        bulk_cr_fc_row_031 => (r#"bulk cr 031"#, r###"echo one two"###);
        bulk_cr_fc_row_032 => (r#"bulk cr 032"#, r###"test 1 -eq 1; print -r $?"###);
        bulk_cr_fc_row_033 => (r#"bulk cr 033"#, r###"[ 1 -eq 1 ]; print -r $?"###);
        bulk_cr_fc_row_034 => (r#"bulk cr 034"#, r###"command true; print -r $?"###);
        bulk_cr_fc_row_035 => (r#"bulk cr 035"#, r###"builtin true; print -r $?"###);
        bulk_cr_fc_row_036 => (r#"bulk cr 036"#, r###"if true; then echo t; fi"###);
        bulk_cr_fc_row_037 => (r#"bulk cr 037"#, r###"if false; then echo e; else echo f; fi"###);
        bulk_cr_fc_row_038 => (r#"bulk cr 038"#, r###"for i in a b; do print -r $i; done"###);
        bulk_cr_fc_row_039 => (r#"bulk cr 039"#, r###"i=0; while (( i < 2 )); do print -r $i; (( i++ )); done"###);
        bulk_cr_fc_row_040 => (r#"bulk cr 040"#, r###"repeat 2; do print -r r; done"###);
        bulk_cr_fc_row_041 => (r#"bulk cr 041"#, r###"case x in (x) echo ok ;; esac"###);
        bulk_cr_fc_row_042 => (r#"bulk cr 042"#, r###"[[ 1 -eq 1 ]] && echo and || echo or"###);
        bulk_cr_fc_row_043 => (r#"bulk cr 043"#, r###"[[ 1 -eq 2 ]] || echo orbranch"###);
        bulk_cr_fc_row_044 => (r#"bulk cr 044"#, r###"{ echo a; echo b; }"###);
        bulk_cr_fc_row_045 => (r#"bulk cr 045"#, r###"(echo sub)"###);
        bulk_cr_fc_row_046 => (r#"bulk cr 046"#, r###"(( 1 )) || echo no"###);
        bulk_cr_fc_row_047 => (r#"bulk cr 047"#, r###"(( 0 )) && echo no"###);
        bulk_cr_fc_row_048 => (r#"bulk cr 048"#, r###"print -r $(( 1 + 2 ))"###);
    }
}

mod corpus_dash_fc_bulk_cs {
    use super::*;

    parity_gap_tests! {
        bulk_cs_fc_row_001 => (r#"bulk cs 001"#, r###"str=%i; print -r ${(%)str}"###);
        bulk_cs_fc_row_002 => (r#"bulk cs 002"#, r###"str=%I; print -r ${(%)str}"###);
        bulk_cs_fc_row_003 => (r#"bulk cs 003"#, r###"str=%j; print -r ${(%)str}"###);
        bulk_cs_fc_row_004 => (r#"bulk cs 004"#, r###"str=%C; print -r ${(%)str}"###);
        bulk_cs_fc_row_005 => (r#"bulk cs 005"#, r###"str=%c; print -r ${(%)str}"###);
        bulk_cs_fc_row_006 => (r#"bulk cs 006"#, r###"str=%D; print -r ${(%)str}"###);
        bulk_cs_fc_row_007 => (r#"bulk cs 007"#, r###"str=%W; print -r ${(%)str}"###);
        bulk_cs_fc_row_008 => (r#"bulk cs 008"#, r###"str=%*; print -r ${(%)str}"###);
        bulk_cs_fc_row_009 => (r#"bulk cs 009"#, r###"str=%v; print -r ${(%)str}"###);
        bulk_cs_fc_row_010 => (r#"bulk cs 010"#, r###"str=%L; print -r ${(%)str}"###);
        bulk_cs_fc_row_011 => (r#"bulk cs 011"#, r###"str=%l; print -r ${(%)str}"###);
        bulk_cs_fc_row_012 => (r#"bulk cs 012"#, r###"str=%y; print -r ${(%)str}"###);
        bulk_cs_fc_row_013 => (r#"bulk cs 013"#, r###"str=%/; print -r ${(%)str}"###);
        bulk_cs_fc_row_014 => (r#"bulk cs 014"#, r###"str=%<; print -r ${(%)str}"###);
        bulk_cs_fc_row_015 => (r#"bulk cs 015"#, r###"str=%>; print -r ${(%)str}"###);
        bulk_cs_fc_row_016 => (r#"bulk cs 016"#, r###"true; print -r $?"###);
        bulk_cs_fc_row_017 => (r#"bulk cs 017"#, r###"false; print -r $?"###);
        bulk_cs_fc_row_018 => (r#"bulk cs 018"#, r###"print -r hello"###);
        bulk_cs_fc_row_019 => (r#"bulk cs 019"#, r###"echo one two"###);
        bulk_cs_fc_row_020 => (r#"bulk cs 020"#, r###"test 1 -eq 1; print -r $?"###);
        bulk_cs_fc_row_021 => (r#"bulk cs 021"#, r###"[ 1 -eq 1 ]; print -r $?"###);
        bulk_cs_fc_row_022 => (r#"bulk cs 022"#, r###"command true; print -r $?"###);
        bulk_cs_fc_row_023 => (r#"bulk cs 023"#, r###"builtin true; print -r $?"###);
        bulk_cs_fc_row_024 => (r#"bulk cs 024"#, r###"if true; then echo t; fi"###);
        bulk_cs_fc_row_025 => (r#"bulk cs 025"#, r###"if false; then echo e; else echo f; fi"###);
        bulk_cs_fc_row_026 => (r#"bulk cs 026"#, r###"for i in a b; do print -r $i; done"###);
        bulk_cs_fc_row_027 => (r#"bulk cs 027"#, r###"i=0; while (( i < 2 )); do print -r $i; (( i++ )); done"###);
        bulk_cs_fc_row_028 => (r#"bulk cs 028"#, r###"repeat 2; do print -r r; done"###);
        bulk_cs_fc_row_029 => (r#"bulk cs 029"#, r###"case x in (x) echo ok ;; esac"###);
        bulk_cs_fc_row_030 => (r#"bulk cs 030"#, r###"[[ 1 -eq 1 ]] && echo and || echo or"###);
        bulk_cs_fc_row_031 => (r#"bulk cs 031"#, r###"[[ 1 -eq 2 ]] || echo orbranch"###);
        bulk_cs_fc_row_032 => (r#"bulk cs 032"#, r###"{ echo a; echo b; }"###);
        bulk_cs_fc_row_033 => (r#"bulk cs 033"#, r###"(echo sub)"###);
        bulk_cs_fc_row_034 => (r#"bulk cs 034"#, r###"(( 1 )) || echo no"###);
        bulk_cs_fc_row_035 => (r#"bulk cs 035"#, r###"(( 0 )) && echo no"###);
        bulk_cs_fc_row_036 => (r#"bulk cs 036"#, r###"print -r $(( 1 + 2 ))"###);
        bulk_cs_fc_row_037 => (r#"bulk cs 037"#, r###"print -r $(( 17 % 5 ))"###);
        bulk_cs_fc_row_038 => (r#"bulk cs 038"#, r###"print -r $(( 2 ** 8 ))"###);
        bulk_cs_fc_row_039 => (r#"bulk cs 039"#, r###"print -r $(( 1 && 0 || 2 ))"###);
        bulk_cs_fc_row_040 => (r#"bulk cs 040"#, r###"print -r $(( !0 ))"###);
        bulk_cs_fc_row_041 => (r#"bulk cs 041"#, r###"integer n=5; (( n += 2 )); print -r $n"###);
        bulk_cs_fc_row_042 => (r#"bulk cs 042"#, r###"integer n=5; (( n -= 1 )); print -r $n"###);
        bulk_cs_fc_row_043 => (r#"bulk cs 043"#, r###"integer n=5; (( n *= 2 )); print -r $n"###);
        bulk_cs_fc_row_044 => (r#"bulk cs 044"#, r###"integer n=5; (( n |= 3 )); print -r $n"###);
        bulk_cs_fc_row_045 => (r#"bulk cs 045"#, r###"integer n=5; (( n &= 3 )); print -r $n"###);
        bulk_cs_fc_row_046 => (r#"bulk cs 046"#, r###"print -r $(( true ))"###);
        bulk_cs_fc_row_047 => (r#"bulk cs 047"#, r###"print -r $(( false ))"###);
        bulk_cs_fc_row_048 => (r#"bulk cs 048"#, r###"[[ -e / ]]; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_ct {
    use super::*;

    parity_gap_tests! {
        bulk_ct_fc_row_001 => (r#"bulk ct 001"#, r###"str=%>; print -r ${(%)str}"###);
        bulk_ct_fc_row_002 => (r#"bulk ct 002"#, r###"true; print -r $?"###);
        bulk_ct_fc_row_003 => (r#"bulk ct 003"#, r###"false; print -r $?"###);
        bulk_ct_fc_row_004 => (r#"bulk ct 004"#, r###"print -r hello"###);
        bulk_ct_fc_row_005 => (r#"bulk ct 005"#, r###"echo one two"###);
        bulk_ct_fc_row_006 => (r#"bulk ct 006"#, r###"test 1 -eq 1; print -r $?"###);
        bulk_ct_fc_row_007 => (r#"bulk ct 007"#, r###"[ 1 -eq 1 ]; print -r $?"###);
        bulk_ct_fc_row_008 => (r#"bulk ct 008"#, r###"command true; print -r $?"###);
        bulk_ct_fc_row_009 => (r#"bulk ct 009"#, r###"builtin true; print -r $?"###);
        bulk_ct_fc_row_010 => (r#"bulk ct 010"#, r###"if true; then echo t; fi"###);
        bulk_ct_fc_row_011 => (r#"bulk ct 011"#, r###"if false; then echo e; else echo f; fi"###);
        bulk_ct_fc_row_012 => (r#"bulk ct 012"#, r###"for i in a b; do print -r $i; done"###);
        bulk_ct_fc_row_013 => (r#"bulk ct 013"#, r###"i=0; while (( i < 2 )); do print -r $i; (( i++ )); done"###);
        bulk_ct_fc_row_014 => (r#"bulk ct 014"#, r###"repeat 2; do print -r r; done"###);
        bulk_ct_fc_row_015 => (r#"bulk ct 015"#, r###"case x in (x) echo ok ;; esac"###);
        bulk_ct_fc_row_016 => (r#"bulk ct 016"#, r###"[[ 1 -eq 1 ]] && echo and || echo or"###);
        bulk_ct_fc_row_017 => (r#"bulk ct 017"#, r###"[[ 1 -eq 2 ]] || echo orbranch"###);
        bulk_ct_fc_row_018 => (r#"bulk ct 018"#, r###"{ echo a; echo b; }"###);
        bulk_ct_fc_row_019 => (r#"bulk ct 019"#, r###"(echo sub)"###);
        bulk_ct_fc_row_020 => (r#"bulk ct 020"#, r###"(( 1 )) || echo no"###);
        bulk_ct_fc_row_021 => (r#"bulk ct 021"#, r###"(( 0 )) && echo no"###);
        bulk_ct_fc_row_022 => (r#"bulk ct 022"#, r###"print -r $(( 1 + 2 ))"###);
        bulk_ct_fc_row_023 => (r#"bulk ct 023"#, r###"print -r $(( 17 % 5 ))"###);
        bulk_ct_fc_row_024 => (r#"bulk ct 024"#, r###"print -r $(( 2 ** 8 ))"###);
        bulk_ct_fc_row_025 => (r#"bulk ct 025"#, r###"print -r $(( 1 && 0 || 2 ))"###);
        bulk_ct_fc_row_026 => (r#"bulk ct 026"#, r###"print -r $(( !0 ))"###);
        bulk_ct_fc_row_027 => (r#"bulk ct 027"#, r###"integer n=5; (( n += 2 )); print -r $n"###);
        bulk_ct_fc_row_028 => (r#"bulk ct 028"#, r###"integer n=5; (( n -= 1 )); print -r $n"###);
        bulk_ct_fc_row_029 => (r#"bulk ct 029"#, r###"integer n=5; (( n *= 2 )); print -r $n"###);
        bulk_ct_fc_row_030 => (r#"bulk ct 030"#, r###"integer n=5; (( n |= 3 )); print -r $n"###);
        bulk_ct_fc_row_031 => (r#"bulk ct 031"#, r###"integer n=5; (( n &= 3 )); print -r $n"###);
        bulk_ct_fc_row_032 => (r#"bulk ct 032"#, r###"print -r $(( true ))"###);
        bulk_ct_fc_row_033 => (r#"bulk ct 033"#, r###"print -r $(( false ))"###);
        bulk_ct_fc_row_034 => (r#"bulk ct 034"#, r###"[[ -e / ]]; print -r $?"###);
        bulk_ct_fc_row_035 => (r#"bulk ct 035"#, r###"[[ -d /tmp ]]; print -r $?"###);
        bulk_ct_fc_row_036 => (r#"bulk ct 036"#, r###"[[ -f /etc/hosts ]]; print -r $?"###);
        bulk_ct_fc_row_037 => (r#"bulk ct 037"#, r###"[[ -r /etc/hosts ]]; print -r $?"###);
        bulk_ct_fc_row_038 => (r#"bulk ct 038"#, r###"[[ -w /tmp ]]; print -r $?"###);
        bulk_ct_fc_row_039 => (r#"bulk ct 039"#, r###"[[ -x /bin/sh ]]; print -r $?"###);
        bulk_ct_fc_row_040 => (r#"bulk ct 040"#, r###"[[ 42 = <-> ]]; print -r $?"###);
        bulk_ct_fc_row_041 => (r#"bulk ct 041"#, r###"[[ abc = <-> ]]; print -r $?"###);
        bulk_ct_fc_row_042 => (r#"bulk ct 042"#, r####"[[ host = ##host ]]; print -r $?"####);
        bulk_ct_fc_row_043 => (r#"bulk ct 043"#, r###"[[ -v x ]]; print -r $?; x=1"###);
        bulk_ct_fc_row_044 => (r#"bulk ct 044"#, r###"unset y; [[ -v y ]]; print -r $?"###);
        bulk_ct_fc_row_045 => (r#"bulk ct 045"#, r###"setopt extendedglob; [[ abc = (#i)ABC ]]; print -r $?"###);
        bulk_ct_fc_row_046 => (r#"bulk ct 046"#, r###"setopt extendedglob; [[ foo = (#b)oo ]]; print -r $?"###);
        bulk_ct_fc_row_047 => (r#"bulk ct 047"#, r###"[[ abc = a* ]]; print -r $?"###);
        bulk_ct_fc_row_048 => (r#"bulk ct 048"#, r###"[[ abc =~ ^a ]]; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_cu {
    use super::*;

    parity_gap_tests! {
        bulk_cu_fc_row_001 => (r#"bulk cu 001"#, r###"[[ 1 -eq 2 ]] || echo orbranch"###);
        bulk_cu_fc_row_002 => (r#"bulk cu 002"#, r###"{ echo a; echo b; }"###);
        bulk_cu_fc_row_003 => (r#"bulk cu 003"#, r###"(echo sub)"###);
        bulk_cu_fc_row_004 => (r#"bulk cu 004"#, r###"(( 1 )) || echo no"###);
        bulk_cu_fc_row_005 => (r#"bulk cu 005"#, r###"(( 0 )) && echo no"###);
        bulk_cu_fc_row_006 => (r#"bulk cu 006"#, r###"print -r $(( 1 + 2 ))"###);
        bulk_cu_fc_row_007 => (r#"bulk cu 007"#, r###"print -r $(( 17 % 5 ))"###);
        bulk_cu_fc_row_008 => (r#"bulk cu 008"#, r###"print -r $(( 2 ** 8 ))"###);
        bulk_cu_fc_row_009 => (r#"bulk cu 009"#, r###"print -r $(( 1 && 0 || 2 ))"###);
        bulk_cu_fc_row_010 => (r#"bulk cu 010"#, r###"print -r $(( !0 ))"###);
        bulk_cu_fc_row_011 => (r#"bulk cu 011"#, r###"integer n=5; (( n += 2 )); print -r $n"###);
        bulk_cu_fc_row_012 => (r#"bulk cu 012"#, r###"integer n=5; (( n -= 1 )); print -r $n"###);
        bulk_cu_fc_row_013 => (r#"bulk cu 013"#, r###"integer n=5; (( n *= 2 )); print -r $n"###);
        bulk_cu_fc_row_014 => (r#"bulk cu 014"#, r###"integer n=5; (( n |= 3 )); print -r $n"###);
        bulk_cu_fc_row_015 => (r#"bulk cu 015"#, r###"integer n=5; (( n &= 3 )); print -r $n"###);
        bulk_cu_fc_row_016 => (r#"bulk cu 016"#, r###"print -r $(( true ))"###);
        bulk_cu_fc_row_017 => (r#"bulk cu 017"#, r###"print -r $(( false ))"###);
        bulk_cu_fc_row_018 => (r#"bulk cu 018"#, r###"[[ -e / ]]; print -r $?"###);
        bulk_cu_fc_row_019 => (r#"bulk cu 019"#, r###"[[ -d /tmp ]]; print -r $?"###);
        bulk_cu_fc_row_020 => (r#"bulk cu 020"#, r###"[[ -f /etc/hosts ]]; print -r $?"###);
        bulk_cu_fc_row_021 => (r#"bulk cu 021"#, r###"[[ -r /etc/hosts ]]; print -r $?"###);
        bulk_cu_fc_row_022 => (r#"bulk cu 022"#, r###"[[ -w /tmp ]]; print -r $?"###);
        bulk_cu_fc_row_023 => (r#"bulk cu 023"#, r###"[[ -x /bin/sh ]]; print -r $?"###);
        bulk_cu_fc_row_024 => (r#"bulk cu 024"#, r###"[[ 42 = <-> ]]; print -r $?"###);
        bulk_cu_fc_row_025 => (r#"bulk cu 025"#, r###"[[ abc = <-> ]]; print -r $?"###);
        bulk_cu_fc_row_026 => (r#"bulk cu 026"#, r####"[[ host = ##host ]]; print -r $?"####);
        bulk_cu_fc_row_027 => (r#"bulk cu 027"#, r###"[[ -v x ]]; print -r $?; x=1"###);
        bulk_cu_fc_row_028 => (r#"bulk cu 028"#, r###"unset y; [[ -v y ]]; print -r $?"###);
        bulk_cu_fc_row_029 => (r#"bulk cu 029"#, r###"setopt extendedglob; [[ abc = (#i)ABC ]]; print -r $?"###);
        bulk_cu_fc_row_030 => (r#"bulk cu 030"#, r###"setopt extendedglob; [[ foo = (#b)oo ]]; print -r $?"###);
        bulk_cu_fc_row_031 => (r#"bulk cu 031"#, r###"[[ abc = a* ]]; print -r $?"###);
        bulk_cu_fc_row_032 => (r#"bulk cu 032"#, r###"[[ abc =~ ^a ]]; print -r $?"###);
        bulk_cu_fc_row_033 => (r#"bulk cu 033"#, r###"[[ -z '' ]]; print -r $?"###);
        bulk_cu_fc_row_034 => (r#"bulk cu 034"#, r###"[[ -n abc ]]; print -r $?"###);
        bulk_cu_fc_row_035 => (r#"bulk cu 035"#, r###"typeset -i n=10; print -r $n"###);
        bulk_cu_fc_row_036 => (r#"bulk cu 036"#, r###"typeset -l n=AbC; print -r $n"###);
        bulk_cu_fc_row_037 => (r#"bulk cu 037"#, r###"typeset -u n=xy; print -r $n"###);
        bulk_cu_fc_row_038 => (r#"bulk cu 038"#, r###"typeset -Z5 n=7; print -r $n"###);
        bulk_cu_fc_row_039 => (r#"bulk cu 039"#, r###"typeset -a a=(x y); print -r ${a[2]}"###);
        bulk_cu_fc_row_040 => (r#"bulk cu 040"#, r###"arr=(1 2); arr+=3; print -r ${arr[@]}"###);
        bulk_cu_fc_row_041 => (r#"bulk cu 041"#, r###"arr=(1); arr[1]+=2; print -r ${arr[1]}"###);
        bulk_cu_fc_row_042 => (r#"bulk cu 042"#, r###"unset v; print -r ${v:-def}"###);
        bulk_cu_fc_row_043 => (r#"bulk cu 043"#, r###"v=set; print -r ${v:+yes}"###);
        bulk_cu_fc_row_044 => (r#"bulk cu 044"#, r###"unset v; : ${v::=def}; print -r $v"###);
        bulk_cu_fc_row_045 => (r#"bulk cu 045"#, r###"print -r ${PWD:h}"###);
        bulk_cu_fc_row_046 => (r#"bulk cu 046"#, r###"print -r ${PWD:t}"###);
        bulk_cu_fc_row_047 => (r#"bulk cu 047"#, r###"true | true; print -r $?"###);
        bulk_cu_fc_row_048 => (r#"bulk cu 048"#, r###"true | false; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_cv {
    use super::*;

    parity_gap_tests! {
        bulk_cv_fc_row_001 => (r#"bulk cv 001"#, r###"[[ -e / ]]; print -r $?"###);
        bulk_cv_fc_row_002 => (r#"bulk cv 002"#, r###"[[ -d /tmp ]]; print -r $?"###);
        bulk_cv_fc_row_003 => (r#"bulk cv 003"#, r###"[[ -f /etc/hosts ]]; print -r $?"###);
        bulk_cv_fc_row_004 => (r#"bulk cv 004"#, r###"[[ -r /etc/hosts ]]; print -r $?"###);
        bulk_cv_fc_row_005 => (r#"bulk cv 005"#, r###"[[ -w /tmp ]]; print -r $?"###);
        bulk_cv_fc_row_006 => (r#"bulk cv 006"#, r###"[[ -x /bin/sh ]]; print -r $?"###);
        bulk_cv_fc_row_007 => (r#"bulk cv 007"#, r###"[[ 42 = <-> ]]; print -r $?"###);
        bulk_cv_fc_row_008 => (r#"bulk cv 008"#, r###"[[ abc = <-> ]]; print -r $?"###);
        bulk_cv_fc_row_009 => (r#"bulk cv 009"#, r####"[[ host = ##host ]]; print -r $?"####);
        bulk_cv_fc_row_010 => (r#"bulk cv 010"#, r###"[[ -v x ]]; print -r $?; x=1"###);
        bulk_cv_fc_row_011 => (r#"bulk cv 011"#, r###"unset y; [[ -v y ]]; print -r $?"###);
        bulk_cv_fc_row_012 => (r#"bulk cv 012"#, r###"setopt extendedglob; [[ abc = (#i)ABC ]]; print -r $?"###);
        bulk_cv_fc_row_013 => (r#"bulk cv 013"#, r###"setopt extendedglob; [[ foo = (#b)oo ]]; print -r $?"###);
        bulk_cv_fc_row_014 => (r#"bulk cv 014"#, r###"[[ abc = a* ]]; print -r $?"###);
        bulk_cv_fc_row_015 => (r#"bulk cv 015"#, r###"[[ abc =~ ^a ]]; print -r $?"###);
        bulk_cv_fc_row_016 => (r#"bulk cv 016"#, r###"[[ -z '' ]]; print -r $?"###);
        bulk_cv_fc_row_017 => (r#"bulk cv 017"#, r###"[[ -n abc ]]; print -r $?"###);
        bulk_cv_fc_row_018 => (r#"bulk cv 018"#, r###"typeset -i n=10; print -r $n"###);
        bulk_cv_fc_row_019 => (r#"bulk cv 019"#, r###"typeset -l n=AbC; print -r $n"###);
        bulk_cv_fc_row_020 => (r#"bulk cv 020"#, r###"typeset -u n=xy; print -r $n"###);
        bulk_cv_fc_row_021 => (r#"bulk cv 021"#, r###"typeset -Z5 n=7; print -r $n"###);
        bulk_cv_fc_row_022 => (r#"bulk cv 022"#, r###"typeset -a a=(x y); print -r ${a[2]}"###);
        bulk_cv_fc_row_023 => (r#"bulk cv 023"#, r###"arr=(1 2); arr+=3; print -r ${arr[@]}"###);
        bulk_cv_fc_row_024 => (r#"bulk cv 024"#, r###"arr=(1); arr[1]+=2; print -r ${arr[1]}"###);
        bulk_cv_fc_row_025 => (r#"bulk cv 025"#, r###"unset v; print -r ${v:-def}"###);
        bulk_cv_fc_row_026 => (r#"bulk cv 026"#, r###"v=set; print -r ${v:+yes}"###);
        bulk_cv_fc_row_027 => (r#"bulk cv 027"#, r###"unset v; : ${v::=def}; print -r $v"###);
        bulk_cv_fc_row_028 => (r#"bulk cv 028"#, r###"print -r ${PWD:h}"###);
        bulk_cv_fc_row_029 => (r#"bulk cv 029"#, r###"print -r ${PWD:t}"###);
        bulk_cv_fc_row_030 => (r#"bulk cv 030"#, r###"true | true; print -r $?"###);
        bulk_cv_fc_row_031 => (r#"bulk cv 031"#, r###"true | false; print -r $?"###);
        bulk_cv_fc_row_032 => (r#"bulk cv 032"#, r###"print -r ${pipestatus[1]}; true | false"###);
        bulk_cv_fc_row_033 => (r#"bulk cv 033"#, r###"print -r ${#pipestatus}; true | true | true"###);
        bulk_cv_fc_row_034 => (r#"bulk cv 034"#, r###"set -- a b c; shift; print -r $1"###);
        bulk_cv_fc_row_035 => (r#"bulk cv 035"#, r###"set -- a b c; shift 2; print -r $#"###);
        bulk_cv_fc_row_036 => (r#"bulk cv 036"#, r###"fn(){ print -r $1; }; fn x"###);
        bulk_cv_fc_row_037 => (r#"bulk cv 037"#, r###"fn(){ local x=2; print -r $x; }; fn"###);
        bulk_cv_fc_row_038 => (r#"bulk cv 038"#, r###"fn(){ return 2; }; fn; print -r $?"###);
        bulk_cv_fc_row_039 => (r#"bulk cv 039"#, r###"print -r ${(q)x}; x=hi"###);
        bulk_cv_fc_row_040 => (r#"bulk cv 040"#, r###"print -r ${(qq)x}; x=hi"###);
        bulk_cv_fc_row_041 => (r#"bulk cv 041"#, r###"x=hi; print -r ${(q-)x}"###);
        bulk_cv_fc_row_042 => (r#"bulk cv 042"#, r###"x=hi; print -r ${(q+)x}"###);
        bulk_cv_fc_row_043 => (r#"bulk cv 043"#, r###"print -r ${(w)w}; w=a b c"###);
        bulk_cv_fc_row_044 => (r#"bulk cv 044"#, r###"print -r ${(u)a}; a=(a a b)"###);
        bulk_cv_fc_row_045 => (r#"bulk cv 045"#, r###"print -r ${(o)a}; a=(c b a)"###);
        bulk_cv_fc_row_046 => (r#"bulk cv 046"#, r###"print -r ${(j:,:)a}; a=(x y)"###);
        bulk_cv_fc_row_047 => (r#"bulk cv 047"#, r###"arr=(a b c); print -r ${arr[(I)b]}"###);
        bulk_cv_fc_row_048 => (r#"bulk cv 048"#, r###"arr=(a b c); print -r ${arr[(R)b]}"###);
    }
}

mod corpus_dash_fc_bulk_cw {
    use super::*;

    parity_gap_tests! {
        bulk_cw_fc_row_001 => (r#"bulk cw 001"#, r###"typeset -i n=10; print -r $n"###);
        bulk_cw_fc_row_002 => (r#"bulk cw 002"#, r###"typeset -l n=AbC; print -r $n"###);
        bulk_cw_fc_row_003 => (r#"bulk cw 003"#, r###"typeset -u n=xy; print -r $n"###);
        bulk_cw_fc_row_004 => (r#"bulk cw 004"#, r###"typeset -Z5 n=7; print -r $n"###);
        bulk_cw_fc_row_005 => (r#"bulk cw 005"#, r###"typeset -a a=(x y); print -r ${a[2]}"###);
        bulk_cw_fc_row_006 => (r#"bulk cw 006"#, r###"arr=(1 2); arr+=3; print -r ${arr[@]}"###);
        bulk_cw_fc_row_007 => (r#"bulk cw 007"#, r###"arr=(1); arr[1]+=2; print -r ${arr[1]}"###);
        bulk_cw_fc_row_008 => (r#"bulk cw 008"#, r###"unset v; print -r ${v:-def}"###);
        bulk_cw_fc_row_009 => (r#"bulk cw 009"#, r###"v=set; print -r ${v:+yes}"###);
        bulk_cw_fc_row_010 => (r#"bulk cw 010"#, r###"unset v; : ${v::=def}; print -r $v"###);
        bulk_cw_fc_row_011 => (r#"bulk cw 011"#, r###"print -r ${PWD:h}"###);
        bulk_cw_fc_row_012 => (r#"bulk cw 012"#, r###"print -r ${PWD:t}"###);
        bulk_cw_fc_row_013 => (r#"bulk cw 013"#, r###"true | true; print -r $?"###);
        bulk_cw_fc_row_014 => (r#"bulk cw 014"#, r###"true | false; print -r $?"###);
        bulk_cw_fc_row_015 => (r#"bulk cw 015"#, r###"print -r ${pipestatus[1]}; true | false"###);
        bulk_cw_fc_row_016 => (r#"bulk cw 016"#, r###"print -r ${#pipestatus}; true | true | true"###);
        bulk_cw_fc_row_017 => (r#"bulk cw 017"#, r###"set -- a b c; shift; print -r $1"###);
        bulk_cw_fc_row_018 => (r#"bulk cw 018"#, r###"set -- a b c; shift 2; print -r $#"###);
        bulk_cw_fc_row_019 => (r#"bulk cw 019"#, r###"fn(){ print -r $1; }; fn x"###);
        bulk_cw_fc_row_020 => (r#"bulk cw 020"#, r###"fn(){ local x=2; print -r $x; }; fn"###);
        bulk_cw_fc_row_021 => (r#"bulk cw 021"#, r###"fn(){ return 2; }; fn; print -r $?"###);
        bulk_cw_fc_row_022 => (r#"bulk cw 022"#, r###"print -r ${(q)x}; x=hi"###);
        bulk_cw_fc_row_023 => (r#"bulk cw 023"#, r###"print -r ${(qq)x}; x=hi"###);
        bulk_cw_fc_row_024 => (r#"bulk cw 024"#, r###"x=hi; print -r ${(q-)x}"###);
        bulk_cw_fc_row_025 => (r#"bulk cw 025"#, r###"x=hi; print -r ${(q+)x}"###);
        bulk_cw_fc_row_026 => (r#"bulk cw 026"#, r###"print -r ${(w)w}; w=a b c"###);
        bulk_cw_fc_row_027 => (r#"bulk cw 027"#, r###"print -r ${(u)a}; a=(a a b)"###);
        bulk_cw_fc_row_028 => (r#"bulk cw 028"#, r###"print -r ${(o)a}; a=(c b a)"###);
        bulk_cw_fc_row_029 => (r#"bulk cw 029"#, r###"print -r ${(j:,:)a}; a=(x y)"###);
        bulk_cw_fc_row_030 => (r#"bulk cw 030"#, r###"arr=(a b c); print -r ${arr[(I)b]}"###);
        bulk_cw_fc_row_031 => (r#"bulk cw 031"#, r###"arr=(a b c); print -r ${arr[(R)b]}"###);
        bulk_cw_fc_row_032 => (r#"bulk cw 032"#, r###"arr=(9 8 7); print -r ${arr[-2,-1]}"###);
        bulk_cw_fc_row_033 => (r#"bulk cw 033"#, r###"typeset -A h; h=(k v); print -r ${(k)h}"###);
        bulk_cw_fc_row_034 => (r#"bulk cw 034"#, r###"typeset -A h; h=(a 1 b 2); print -r ${(kv)h}"###);
        bulk_cw_fc_row_035 => (r#"bulk cw 035"#, r###"print -r ${+options}"###);
        bulk_cw_fc_row_036 => (r#"bulk cw 036"#, r###"print -r ${+parameters}"###);
        bulk_cw_fc_row_037 => (r#"bulk cw 037"#, r###"print -r ${+aliases}"###);
        bulk_cw_fc_row_038 => (r#"bulk cw 038"#, r###"print -r ${+functions}"###);
        bulk_cw_fc_row_039 => (r#"bulk cw 039"#, r###"print -r $ZSH_NAME"###);
        bulk_cw_fc_row_040 => (r#"bulk cw 040"#, r###"print -r ${ZSH_VERSION%%.*}"###);
        bulk_cw_fc_row_041 => (r#"bulk cw 041"#, r###"whence -w print"###);
        bulk_cw_fc_row_042 => (r#"bulk cw 042"#, r###"command -v true"###);
        bulk_cw_fc_row_043 => (r#"bulk cw 043"#, r###"emulate -L zsh; print -r $?"###);
        bulk_cw_fc_row_044 => (r#"bulk cw 044"#, r###"alias za='echo z'; za; unalias za 2>/dev/null"###);
        bulk_cw_fc_row_045 => (r#"bulk cw 045"#, r###"read -r line <<< 'one two'; print -r $line"###);
        bulk_cw_fc_row_046 => (r#"bulk cw 046"#, r###"cat <<< 'herestring'"###);
        bulk_cw_fc_row_047 => (r#"bulk cw 047"#, r###"echo hello 2>/dev/null"###);
        bulk_cw_fc_row_048 => (r#"bulk cw 048"#, r###"printf '%s\n' a b c | head -1"###);
    }
}

mod corpus_dash_fc_bulk_cx {
    use super::*;

    parity_gap_tests! {
        bulk_cx_fc_row_001 => (r#"bulk cx 001"#, r###"set -- a b c; shift 2; print -r $#"###);
        bulk_cx_fc_row_002 => (r#"bulk cx 002"#, r###"fn(){ print -r $1; }; fn x"###);
        bulk_cx_fc_row_003 => (r#"bulk cx 003"#, r###"fn(){ local x=2; print -r $x; }; fn"###);
        bulk_cx_fc_row_004 => (r#"bulk cx 004"#, r###"fn(){ return 2; }; fn; print -r $?"###);
        bulk_cx_fc_row_005 => (r#"bulk cx 005"#, r###"print -r ${(q)x}; x=hi"###);
        bulk_cx_fc_row_006 => (r#"bulk cx 006"#, r###"print -r ${(qq)x}; x=hi"###);
        bulk_cx_fc_row_007 => (r#"bulk cx 007"#, r###"x=hi; print -r ${(q-)x}"###);
        bulk_cx_fc_row_008 => (r#"bulk cx 008"#, r###"x=hi; print -r ${(q+)x}"###);
        bulk_cx_fc_row_009 => (r#"bulk cx 009"#, r###"print -r ${(w)w}; w=a b c"###);
        bulk_cx_fc_row_010 => (r#"bulk cx 010"#, r###"print -r ${(u)a}; a=(a a b)"###);
        bulk_cx_fc_row_011 => (r#"bulk cx 011"#, r###"print -r ${(o)a}; a=(c b a)"###);
        bulk_cx_fc_row_012 => (r#"bulk cx 012"#, r###"print -r ${(j:,:)a}; a=(x y)"###);
        bulk_cx_fc_row_013 => (r#"bulk cx 013"#, r###"arr=(a b c); print -r ${arr[(I)b]}"###);
        bulk_cx_fc_row_014 => (r#"bulk cx 014"#, r###"arr=(a b c); print -r ${arr[(R)b]}"###);
        bulk_cx_fc_row_015 => (r#"bulk cx 015"#, r###"arr=(9 8 7); print -r ${arr[-2,-1]}"###);
        bulk_cx_fc_row_016 => (r#"bulk cx 016"#, r###"typeset -A h; h=(k v); print -r ${(k)h}"###);
        bulk_cx_fc_row_017 => (r#"bulk cx 017"#, r###"typeset -A h; h=(a 1 b 2); print -r ${(kv)h}"###);
        bulk_cx_fc_row_018 => (r#"bulk cx 018"#, r###"print -r ${+options}"###);
        bulk_cx_fc_row_019 => (r#"bulk cx 019"#, r###"print -r ${+parameters}"###);
        bulk_cx_fc_row_020 => (r#"bulk cx 020"#, r###"print -r ${+aliases}"###);
        bulk_cx_fc_row_021 => (r#"bulk cx 021"#, r###"print -r ${+functions}"###);
        bulk_cx_fc_row_022 => (r#"bulk cx 022"#, r###"print -r $ZSH_NAME"###);
        bulk_cx_fc_row_023 => (r#"bulk cx 023"#, r###"print -r ${ZSH_VERSION%%.*}"###);
        bulk_cx_fc_row_024 => (r#"bulk cx 024"#, r###"whence -w print"###);
        bulk_cx_fc_row_025 => (r#"bulk cx 025"#, r###"command -v true"###);
        bulk_cx_fc_row_026 => (r#"bulk cx 026"#, r###"emulate -L zsh; print -r $?"###);
        bulk_cx_fc_row_027 => (r#"bulk cx 027"#, r###"alias za='echo z'; za; unalias za 2>/dev/null"###);
        bulk_cx_fc_row_028 => (r#"bulk cx 028"#, r###"read -r line <<< 'one two'; print -r $line"###);
        bulk_cx_fc_row_029 => (r#"bulk cx 029"#, r###"cat <<< 'herestring'"###);
        bulk_cx_fc_row_030 => (r#"bulk cx 030"#, r###"echo hello 2>/dev/null"###);
        bulk_cx_fc_row_031 => (r#"bulk cx 031"#, r###"printf '%s\n' a b c | head -1"###);
        bulk_cx_fc_row_032 => (r#"bulk cx 032"#, r###"true && echo yes"###);
        bulk_cx_fc_row_033 => (r#"bulk cx 033"#, r###"false || echo yes"###);
        bulk_cx_fc_row_034 => (r#"bulk cx 034"#, r###"(exit 3); print -r $?"###);
        bulk_cx_fc_row_035 => (r#"bulk cx 035"#, r###"print -r ${status}; (exit 4)"###);
        bulk_cx_fc_row_036 => (r#"bulk cx 036"#, r###"print -r $(( 1_000 + 1 ))"###);
        bulk_cx_fc_row_037 => (r#"bulk cx 037"#, r###"print -r $(( 5#101 ))"###);
        bulk_cx_fc_row_038 => (r#"bulk cx 038"#, r###"print -r $(( 0b1111 ))"###);
        bulk_cx_fc_row_039 => (r#"bulk cx 039"#, r###"print -r $(( 2 ** 3 ** 2 ))"###);
        bulk_cx_fc_row_040 => (r#"bulk cx 040"#, r###"float f=1.5; print -r $(( f * 2 ))"###);
        bulk_cx_fc_row_041 => (r#"bulk cx 041"#, r###"typeset -F2 f=3.14; print -r $f"###);
        bulk_cx_fc_row_042 => (r#"bulk cx 042"#, r###"[[ /etc/hosts -nt /tmp ]]; print -r $?"###);
        bulk_cx_fc_row_043 => (r#"bulk cx 043"#, r###"[[ /tmp -ot /etc/hosts ]]; print -r $?"###);
        bulk_cx_fc_row_044 => (r#"bulk cx 044"#, r###"[[ /etc/hosts -ef /etc/hosts ]]; print -r $?"###);
        bulk_cx_fc_row_045 => (r#"bulk cx 045"#, r####"setopt extendedglob; [[ abc = [a-z]## ]]; print -r $?"####);
        bulk_cx_fc_row_046 => (r#"bulk cx 046"#, r###"print -r ${(L)${(U)m}}; m=aBc"###);
        bulk_cx_fc_row_047 => (r#"bulk cx 047"#, r###"print -r ${#x}; x=hello"###);
        bulk_cx_fc_row_048 => (r#"bulk cx 048"#, r###"print -r ${#a}; a=(a b c)"###);
    }
}

mod corpus_dash_fc_bulk_cy {
    use super::*;

    parity_gap_tests! {
        bulk_cy_fc_row_001 => (r#"bulk cy 001"#, r###"typeset -A h; h=(k v); print -r ${(k)h}"###);
        bulk_cy_fc_row_002 => (r#"bulk cy 002"#, r###"typeset -A h; h=(a 1 b 2); print -r ${(kv)h}"###);
        bulk_cy_fc_row_003 => (r#"bulk cy 003"#, r###"print -r ${+options}"###);
        bulk_cy_fc_row_004 => (r#"bulk cy 004"#, r###"print -r ${+parameters}"###);
        bulk_cy_fc_row_005 => (r#"bulk cy 005"#, r###"print -r ${+aliases}"###);
        bulk_cy_fc_row_006 => (r#"bulk cy 006"#, r###"print -r ${+functions}"###);
        bulk_cy_fc_row_007 => (r#"bulk cy 007"#, r###"print -r $ZSH_NAME"###);
        bulk_cy_fc_row_008 => (r#"bulk cy 008"#, r###"print -r ${ZSH_VERSION%%.*}"###);
        bulk_cy_fc_row_009 => (r#"bulk cy 009"#, r###"whence -w print"###);
        bulk_cy_fc_row_010 => (r#"bulk cy 010"#, r###"command -v true"###);
        bulk_cy_fc_row_011 => (r#"bulk cy 011"#, r###"emulate -L zsh; print -r $?"###);
        bulk_cy_fc_row_012 => (r#"bulk cy 012"#, r###"alias za='echo z'; za; unalias za 2>/dev/null"###);
        bulk_cy_fc_row_013 => (r#"bulk cy 013"#, r###"read -r line <<< 'one two'; print -r $line"###);
        bulk_cy_fc_row_014 => (r#"bulk cy 014"#, r###"cat <<< 'herestring'"###);
        bulk_cy_fc_row_015 => (r#"bulk cy 015"#, r###"echo hello 2>/dev/null"###);
        bulk_cy_fc_row_016 => (r#"bulk cy 016"#, r###"printf '%s\n' a b c | head -1"###);
        bulk_cy_fc_row_017 => (r#"bulk cy 017"#, r###"true && echo yes"###);
        bulk_cy_fc_row_018 => (r#"bulk cy 018"#, r###"false || echo yes"###);
        bulk_cy_fc_row_019 => (r#"bulk cy 019"#, r###"(exit 3); print -r $?"###);
        bulk_cy_fc_row_020 => (r#"bulk cy 020"#, r###"print -r ${status}; (exit 4)"###);
        bulk_cy_fc_row_021 => (r#"bulk cy 021"#, r###"print -r $(( 1_000 + 1 ))"###);
        bulk_cy_fc_row_022 => (r#"bulk cy 022"#, r###"print -r $(( 5#101 ))"###);
        bulk_cy_fc_row_023 => (r#"bulk cy 023"#, r###"print -r $(( 0b1111 ))"###);
        bulk_cy_fc_row_024 => (r#"bulk cy 024"#, r###"print -r $(( 2 ** 3 ** 2 ))"###);
        bulk_cy_fc_row_025 => (r#"bulk cy 025"#, r###"float f=1.5; print -r $(( f * 2 ))"###);
        bulk_cy_fc_row_026 => (r#"bulk cy 026"#, r###"typeset -F2 f=3.14; print -r $f"###);
        bulk_cy_fc_row_027 => (r#"bulk cy 027"#, r###"[[ /etc/hosts -nt /tmp ]]; print -r $?"###);
        bulk_cy_fc_row_028 => (r#"bulk cy 028"#, r###"[[ /tmp -ot /etc/hosts ]]; print -r $?"###);
        bulk_cy_fc_row_029 => (r#"bulk cy 029"#, r###"[[ /etc/hosts -ef /etc/hosts ]]; print -r $?"###);
        bulk_cy_fc_row_030 => (r#"bulk cy 030"#, r####"setopt extendedglob; [[ abc = [a-z]## ]]; print -r $?"####);
        bulk_cy_fc_row_031 => (r#"bulk cy 031"#, r###"print -r ${(L)${(U)m}}; m=aBc"###);
        bulk_cy_fc_row_032 => (r#"bulk cy 032"#, r###"print -r ${#x}; x=hello"###);
        bulk_cy_fc_row_033 => (r#"bulk cy 033"#, r###"print -r ${#a}; a=(a b c)"###);
        bulk_cy_fc_row_034 => (r#"bulk cy 034"#, r###"print -r ${(c)#a}; a=(ab cd)"###);
        bulk_cy_fc_row_035 => (r#"bulk cy 035"#, r###"print -r ${(b)x}; x=hi"###);
        bulk_cy_fc_row_036 => (r#"bulk cy 036"#, r###"print -r ${(e):-2+2}"###);
        bulk_cy_fc_row_037 => (r#"bulk cy 037"#, r###"print -r ${(P)r}; r=HOME"###);
        bulk_cy_fc_row_038 => (r#"bulk cy 038"#, r###"print -r ${(on)n}; n=(10 2 1)"###);
        bulk_cy_fc_row_039 => (r#"bulk cy 039"#, r###"print -r ${(eu)n}; n=(a A b)"###);
        bulk_cy_fc_row_040 => (r#"bulk cy 040"#, r###"typeset -aU u=(a a b); print -r ${#u}"###);
        bulk_cy_fc_row_041 => (r#"bulk cy 041"#, r###"typeset -h hv=1; print -r ${+hv}"###);
        bulk_cy_fc_row_042 => (r#"bulk cy 042"#, r###"x=a1a2; p=a; print -r ${x//p/r}"###);
        bulk_cy_fc_row_043 => (r#"bulk cy 043"#, r###"for i in 1 2 3; do (( i == 2 )) && continue; print -r $i; done"###);
        bulk_cy_fc_row_044 => (r#"bulk cy 044"#, r###"while :; do break; print -r n; done; print -r after"###);
        bulk_cy_fc_row_045 => (r#"bulk cy 045"#, r###"case w in (a|b) echo ab ;; *) echo star ;; esac"###);
        bulk_cy_fc_row_046 => (r#"bulk cy 046"#, r###"if [[ -n '' ]]; then echo y; else echo n; fi"###);
        bulk_cy_fc_row_047 => (r#"bulk cy 047"#, r###"print -r ${argv[1]}; set -- p q"###);
        bulk_cy_fc_row_048 => (r#"bulk cy 048"#, r###"print -r $ARGC; set -- a b"###);
    }
}

mod corpus_dash_fc_bulk_cz {
    use super::*;

    parity_gap_tests! {
        bulk_cz_fc_row_001 => (r#"bulk cz 001"#, r###"false || echo yes"###);
        bulk_cz_fc_row_002 => (r#"bulk cz 002"#, r###"(exit 3); print -r $?"###);
        bulk_cz_fc_row_003 => (r#"bulk cz 003"#, r###"print -r ${status}; (exit 4)"###);
        bulk_cz_fc_row_004 => (r#"bulk cz 004"#, r###"print -r $(( 1_000 + 1 ))"###);
        bulk_cz_fc_row_005 => (r#"bulk cz 005"#, r###"print -r $(( 5#101 ))"###);
        bulk_cz_fc_row_006 => (r#"bulk cz 006"#, r###"print -r $(( 0b1111 ))"###);
        bulk_cz_fc_row_007 => (r#"bulk cz 007"#, r###"print -r $(( 2 ** 3 ** 2 ))"###);
        bulk_cz_fc_row_008 => (r#"bulk cz 008"#, r###"float f=1.5; print -r $(( f * 2 ))"###);
        bulk_cz_fc_row_009 => (r#"bulk cz 009"#, r###"typeset -F2 f=3.14; print -r $f"###);
        bulk_cz_fc_row_010 => (r#"bulk cz 010"#, r###"[[ /etc/hosts -nt /tmp ]]; print -r $?"###);
        bulk_cz_fc_row_011 => (r#"bulk cz 011"#, r###"[[ /tmp -ot /etc/hosts ]]; print -r $?"###);
        bulk_cz_fc_row_012 => (r#"bulk cz 012"#, r###"[[ /etc/hosts -ef /etc/hosts ]]; print -r $?"###);
        bulk_cz_fc_row_013 => (r#"bulk cz 013"#, r####"setopt extendedglob; [[ abc = [a-z]## ]]; print -r $?"####);
        bulk_cz_fc_row_014 => (r#"bulk cz 014"#, r###"print -r ${(L)${(U)m}}; m=aBc"###);
        bulk_cz_fc_row_015 => (r#"bulk cz 015"#, r###"print -r ${#x}; x=hello"###);
        bulk_cz_fc_row_016 => (r#"bulk cz 016"#, r###"print -r ${#a}; a=(a b c)"###);
        bulk_cz_fc_row_017 => (r#"bulk cz 017"#, r###"print -r ${(c)#a}; a=(ab cd)"###);
        bulk_cz_fc_row_018 => (r#"bulk cz 018"#, r###"print -r ${(b)x}; x=hi"###);
        bulk_cz_fc_row_019 => (r#"bulk cz 019"#, r###"print -r ${(e):-2+2}"###);
        bulk_cz_fc_row_020 => (r#"bulk cz 020"#, r###"print -r ${(P)r}; r=HOME"###);
        bulk_cz_fc_row_021 => (r#"bulk cz 021"#, r###"print -r ${(on)n}; n=(10 2 1)"###);
        bulk_cz_fc_row_022 => (r#"bulk cz 022"#, r###"print -r ${(eu)n}; n=(a A b)"###);
        bulk_cz_fc_row_023 => (r#"bulk cz 023"#, r###"typeset -aU u=(a a b); print -r ${#u}"###);
        bulk_cz_fc_row_024 => (r#"bulk cz 024"#, r###"typeset -h hv=1; print -r ${+hv}"###);
        bulk_cz_fc_row_025 => (r#"bulk cz 025"#, r###"x=a1a2; p=a; print -r ${x//p/r}"###);
        bulk_cz_fc_row_026 => (r#"bulk cz 026"#, r###"for i in 1 2 3; do (( i == 2 )) && continue; print -r $i; done"###);
        bulk_cz_fc_row_027 => (r#"bulk cz 027"#, r###"while :; do break; print -r n; done; print -r after"###);
        bulk_cz_fc_row_028 => (r#"bulk cz 028"#, r###"case w in (a|b) echo ab ;; *) echo star ;; esac"###);
        bulk_cz_fc_row_029 => (r#"bulk cz 029"#, r###"if [[ -n '' ]]; then echo y; else echo n; fi"###);
        bulk_cz_fc_row_030 => (r#"bulk cz 030"#, r###"print -r ${argv[1]}; set -- p q"###);
        bulk_cz_fc_row_031 => (r#"bulk cz 031"#, r###"print -r $ARGC; set -- a b"###);
        bulk_cz_fc_row_032 => (r#"bulk cz 032"#, r###"print -r ${dirstack[1]:-empty}"###);
        bulk_cz_fc_row_033 => (r#"bulk cz 033"#, r###"print -r ${+pipestatus}"###);
        bulk_cz_fc_row_034 => (r#"bulk cz 034"#, r###"print -r ${+history}"###);
        bulk_cz_fc_row_035 => (r#"bulk cz 035"#, r###"print -r ${+commands}"###);
        bulk_cz_fc_row_036 => (r#"bulk cz 036"#, r###"print -r ${+builtins}"###);
        bulk_cz_fc_row_037 => (r#"bulk cz 037"#, r###"print -r ${+widgets}"###);
        bulk_cz_fc_row_038 => (r#"bulk cz 038"#, r###"print -r ${+terminfo}"###);
        bulk_cz_fc_row_039 => (r#"bulk cz 039"#, r###"print -r ${+modules}"###);
        bulk_cz_fc_row_040 => (r#"bulk cz 040"#, r###"print -r ${+patchars}"###);
        bulk_cz_fc_row_041 => (r#"bulk cz 041"#, r###"print -r ${+reswords}"###);
        bulk_cz_fc_row_042 => (r#"bulk cz 042"#, r###"print -r ${+dis_aliases}"###);
        bulk_cz_fc_row_043 => (r#"bulk cz 043"#, r###"print -r ${+dis_functions}"###);
        bulk_cz_fc_row_044 => (r#"bulk cz 044"#, r###"print -r ${+parameters[(I)PATH]}"###);
        bulk_cz_fc_row_045 => (r#"bulk cz 045"#, r###"arr=(a b c d); print -r ${arr[2,3]}"###);
        bulk_cz_fc_row_046 => (r#"bulk cz 046"#, r###"arr=(1 2 3); print -r ${arr[1,-1]}"###);
        bulk_cz_fc_row_047 => (r#"bulk cz 047"#, r###"s=barfooxyz; print -r ${s[(i)foo]}"###);
        bulk_cz_fc_row_048 => (r#"bulk cz 048"#, r###"typeset -A h; h=(k v); print -r ${h[(R)v]}"###);
    }
}

mod corpus_dash_fc_bulk_da {
    use super::*;

    parity_gap_tests! {
        bulk_da_fc_row_001 => (r#"bulk da 001"#, r###"print -r ${(b)x}; x=hi"###);
        bulk_da_fc_row_002 => (r#"bulk da 002"#, r###"print -r ${(e):-2+2}"###);
        bulk_da_fc_row_003 => (r#"bulk da 003"#, r###"print -r ${(P)r}; r=HOME"###);
        bulk_da_fc_row_004 => (r#"bulk da 004"#, r###"print -r ${(on)n}; n=(10 2 1)"###);
        bulk_da_fc_row_005 => (r#"bulk da 005"#, r###"print -r ${(eu)n}; n=(a A b)"###);
        bulk_da_fc_row_006 => (r#"bulk da 006"#, r###"typeset -aU u=(a a b); print -r ${#u}"###);
        bulk_da_fc_row_007 => (r#"bulk da 007"#, r###"typeset -h hv=1; print -r ${+hv}"###);
        bulk_da_fc_row_008 => (r#"bulk da 008"#, r###"x=a1a2; p=a; print -r ${x//p/r}"###);
        bulk_da_fc_row_009 => (r#"bulk da 009"#, r###"for i in 1 2 3; do (( i == 2 )) && continue; print -r $i; done"###);
        bulk_da_fc_row_010 => (r#"bulk da 010"#, r###"while :; do break; print -r n; done; print -r after"###);
        bulk_da_fc_row_011 => (r#"bulk da 011"#, r###"case w in (a|b) echo ab ;; *) echo star ;; esac"###);
        bulk_da_fc_row_012 => (r#"bulk da 012"#, r###"if [[ -n '' ]]; then echo y; else echo n; fi"###);
        bulk_da_fc_row_013 => (r#"bulk da 013"#, r###"print -r ${argv[1]}; set -- p q"###);
        bulk_da_fc_row_014 => (r#"bulk da 014"#, r###"print -r $ARGC; set -- a b"###);
        bulk_da_fc_row_015 => (r#"bulk da 015"#, r###"print -r ${dirstack[1]:-empty}"###);
        bulk_da_fc_row_016 => (r#"bulk da 016"#, r###"print -r ${+pipestatus}"###);
        bulk_da_fc_row_017 => (r#"bulk da 017"#, r###"print -r ${+history}"###);
        bulk_da_fc_row_018 => (r#"bulk da 018"#, r###"print -r ${+commands}"###);
        bulk_da_fc_row_019 => (r#"bulk da 019"#, r###"print -r ${+builtins}"###);
        bulk_da_fc_row_020 => (r#"bulk da 020"#, r###"print -r ${+widgets}"###);
        bulk_da_fc_row_021 => (r#"bulk da 021"#, r###"print -r ${+terminfo}"###);
        bulk_da_fc_row_022 => (r#"bulk da 022"#, r###"print -r ${+modules}"###);
        bulk_da_fc_row_023 => (r#"bulk da 023"#, r###"print -r ${+patchars}"###);
        bulk_da_fc_row_024 => (r#"bulk da 024"#, r###"print -r ${+reswords}"###);
        bulk_da_fc_row_025 => (r#"bulk da 025"#, r###"print -r ${+dis_aliases}"###);
        bulk_da_fc_row_026 => (r#"bulk da 026"#, r###"print -r ${+dis_functions}"###);
        bulk_da_fc_row_027 => (r#"bulk da 027"#, r###"print -r ${+parameters[(I)PATH]}"###);
        bulk_da_fc_row_028 => (r#"bulk da 028"#, r###"arr=(a b c d); print -r ${arr[2,3]}"###);
        bulk_da_fc_row_029 => (r#"bulk da 029"#, r###"arr=(1 2 3); print -r ${arr[1,-1]}"###);
        bulk_da_fc_row_030 => (r#"bulk da 030"#, r###"s=barfooxyz; print -r ${s[(i)foo]}"###);
        bulk_da_fc_row_031 => (r#"bulk da 031"#, r###"typeset -A h; h=(k v); print -r ${h[(R)v]}"###);
        bulk_da_fc_row_032 => (r#"bulk da 032"#, r###"typeset -A h; h=(a 1 b 2); print -r ${h[(r)2]}"###);
        bulk_da_fc_row_033 => (r#"bulk da 033"#, r###"print -r $(( 9 & 6 ^ 3 ))"###);
        bulk_da_fc_row_034 => (r#"bulk da 034"#, r###"print -r $(( 128 >> 2 ))"###);
        bulk_da_fc_row_035 => (r#"bulk da 035"#, r###"print -r $(( ~(255) & 0xff ))"###);
        bulk_da_fc_row_036 => (r#"bulk da 036"#, r###"print -r $(( 3 <|> 5 ))"###);
        bulk_da_fc_row_037 => (r#"bulk da 037"#, r###"print -r $(( 3 <> 5 ))"###);
        bulk_da_fc_row_038 => (r#"bulk da 038"#, r###"(( 5#11 )); print -r $?"###);
        bulk_da_fc_row_039 => (r#"bulk da 039"#, r###"integer n=5; (( n ^= 3 )); print -r $n"###);
        bulk_da_fc_row_040 => (r#"bulk da 040"#, r###"integer n=5; (( n <<= 1 )); print -r $n"###);
        bulk_da_fc_row_041 => (r#"bulk da 041"#, r###"integer n=5; (( n >>= 1 )); print -r $n"###);
        bulk_da_fc_row_042 => (r#"bulk da 042"#, r###"integer n=5; (( n /= 2 )); print -r $n"###);
        bulk_da_fc_row_043 => (r#"bulk da 043"#, r###"integer n=5; (( n %= 3 )); print -r $n"###);
        bulk_da_fc_row_044 => (r#"bulk da 044"#, r###"typeset -i8 n=10; print -r $n"###);
        bulk_da_fc_row_045 => (r#"bulk da 045"#, r###"typeset -i16 n=255; print -r $n"###);
        bulk_da_fc_row_046 => (r#"bulk da 046"#, r###"typeset -E2 n=4000; print -r $n"###);
        bulk_da_fc_row_047 => (r#"bulk da 047"#, r###"typeset -R4 n=hi; print -r $n"###);
        bulk_da_fc_row_048 => (r#"bulk da 048"#, r###"typeset +L n=Ab; print -r $n"###);
    }
}

mod corpus_dash_fc_bulk_db {
    use super::*;

    parity_gap_tests! {
        bulk_db_fc_row_001 => (r#"bulk db 001"#, r###"print -r ${+history}"###);
        bulk_db_fc_row_002 => (r#"bulk db 002"#, r###"print -r ${+commands}"###);
        bulk_db_fc_row_003 => (r#"bulk db 003"#, r###"print -r ${+builtins}"###);
        bulk_db_fc_row_004 => (r#"bulk db 004"#, r###"print -r ${+widgets}"###);
        bulk_db_fc_row_005 => (r#"bulk db 005"#, r###"print -r ${+terminfo}"###);
        bulk_db_fc_row_006 => (r#"bulk db 006"#, r###"print -r ${+modules}"###);
        bulk_db_fc_row_007 => (r#"bulk db 007"#, r###"print -r ${+patchars}"###);
        bulk_db_fc_row_008 => (r#"bulk db 008"#, r###"print -r ${+reswords}"###);
        bulk_db_fc_row_009 => (r#"bulk db 009"#, r###"print -r ${+dis_aliases}"###);
        bulk_db_fc_row_010 => (r#"bulk db 010"#, r###"print -r ${+dis_functions}"###);
        bulk_db_fc_row_011 => (r#"bulk db 011"#, r###"print -r ${+parameters[(I)PATH]}"###);
        bulk_db_fc_row_012 => (r#"bulk db 012"#, r###"arr=(a b c d); print -r ${arr[2,3]}"###);
        bulk_db_fc_row_013 => (r#"bulk db 013"#, r###"arr=(1 2 3); print -r ${arr[1,-1]}"###);
        bulk_db_fc_row_014 => (r#"bulk db 014"#, r###"s=barfooxyz; print -r ${s[(i)foo]}"###);
        bulk_db_fc_row_015 => (r#"bulk db 015"#, r###"typeset -A h; h=(k v); print -r ${h[(R)v]}"###);
        bulk_db_fc_row_016 => (r#"bulk db 016"#, r###"typeset -A h; h=(a 1 b 2); print -r ${h[(r)2]}"###);
        bulk_db_fc_row_017 => (r#"bulk db 017"#, r###"print -r $(( 9 & 6 ^ 3 ))"###);
        bulk_db_fc_row_018 => (r#"bulk db 018"#, r###"print -r $(( 128 >> 2 ))"###);
        bulk_db_fc_row_019 => (r#"bulk db 019"#, r###"print -r $(( ~(255) & 0xff ))"###);
        bulk_db_fc_row_020 => (r#"bulk db 020"#, r###"print -r $(( 3 <|> 5 ))"###);
        bulk_db_fc_row_021 => (r#"bulk db 021"#, r###"print -r $(( 3 <> 5 ))"###);
        bulk_db_fc_row_022 => (r#"bulk db 022"#, r###"(( 5#11 )); print -r $?"###);
        bulk_db_fc_row_023 => (r#"bulk db 023"#, r###"integer n=5; (( n ^= 3 )); print -r $n"###);
        bulk_db_fc_row_024 => (r#"bulk db 024"#, r###"integer n=5; (( n <<= 1 )); print -r $n"###);
        bulk_db_fc_row_025 => (r#"bulk db 025"#, r###"integer n=5; (( n >>= 1 )); print -r $n"###);
        bulk_db_fc_row_026 => (r#"bulk db 026"#, r###"integer n=5; (( n /= 2 )); print -r $n"###);
        bulk_db_fc_row_027 => (r#"bulk db 027"#, r###"integer n=5; (( n %= 3 )); print -r $n"###);
        bulk_db_fc_row_028 => (r#"bulk db 028"#, r###"typeset -i8 n=10; print -r $n"###);
        bulk_db_fc_row_029 => (r#"bulk db 029"#, r###"typeset -i16 n=255; print -r $n"###);
        bulk_db_fc_row_030 => (r#"bulk db 030"#, r###"typeset -E2 n=4000; print -r $n"###);
        bulk_db_fc_row_031 => (r#"bulk db 031"#, r###"typeset -R4 n=hi; print -r $n"###);
        bulk_db_fc_row_032 => (r#"bulk db 032"#, r###"typeset +L n=Ab; print -r $n"###);
        bulk_db_fc_row_033 => (r#"bulk db 033"#, r###"typeset +U n=xy; print -r $n"###);
        bulk_db_fc_row_034 => (r#"bulk db 034"#, r###"typeset +i n=4; print -r $n"###);
        bulk_db_fc_row_035 => (r#"bulk db 035"#, r###"export EX=1; print -r $EX; unset EX"###);
        bulk_db_fc_row_036 => (r#"bulk db 036"#, r###"readonly ro=5; print -r $ro"###);
        bulk_db_fc_row_037 => (r#"bulk db 037"#, r###"print -r ${${v:-fb}}; unset v"###);
        bulk_db_fc_row_038 => (r#"bulk db 038"#, r###"print -r ${${v:+set}:-unset}; unset v"###);
        bulk_db_fc_row_039 => (r#"bulk db 039"#, r###"word=$'l1\nl2'; print -r ${(@f)word}"###);
        bulk_db_fc_row_040 => (r#"bulk db 040"#, r###"word=  hi  ; print -r ${(W)word}"###);
        bulk_db_fc_row_041 => (r#"bulk db 041"#, r###"print -r ${(z)word}; word=a b c"###);
        bulk_db_fc_row_042 => (r#"bulk db 042"#, r###"print -r ${(F)x}; x=$'p\nq'"###);
        bulk_db_fc_row_043 => (r#"bulk db 043"#, r###"print -r ${(A)x}; x=1 2"###);
        bulk_db_fc_row_044 => (r#"bulk db 044"#, r###"print -r ${(aa)x}; x=(1 2)"###);
        bulk_db_fc_row_045 => (r#"bulk db 045"#, r###"print -r ${(%)2}"###);
        bulk_db_fc_row_046 => (r#"bulk db 046"#, r###"o=8; print -r ${(0)o}"###);
        bulk_db_fc_row_047 => (r#"bulk db 047"#, r###"str=abc.def; print -r ${str:r}"###);
        bulk_db_fc_row_048 => (r#"bulk db 048"#, r###"str=abc.def; print -r ${str:e}"###);
    }
}

mod corpus_dash_fc_bulk_dc {
    use super::*;

    parity_gap_tests! {
        bulk_dc_fc_row_001 => (r#"bulk dc 001"#, r###"typeset -A h; h=(a 1 b 2); print -r ${h[(r)2]}"###);
        bulk_dc_fc_row_002 => (r#"bulk dc 002"#, r###"print -r $(( 9 & 6 ^ 3 ))"###);
        bulk_dc_fc_row_003 => (r#"bulk dc 003"#, r###"print -r $(( 128 >> 2 ))"###);
        bulk_dc_fc_row_004 => (r#"bulk dc 004"#, r###"print -r $(( ~(255) & 0xff ))"###);
        bulk_dc_fc_row_005 => (r#"bulk dc 005"#, r###"print -r $(( 3 <|> 5 ))"###);
        bulk_dc_fc_row_006 => (r#"bulk dc 006"#, r###"print -r $(( 3 <> 5 ))"###);
        bulk_dc_fc_row_007 => (r#"bulk dc 007"#, r###"(( 5#11 )); print -r $?"###);
        bulk_dc_fc_row_008 => (r#"bulk dc 008"#, r###"integer n=5; (( n ^= 3 )); print -r $n"###);
        bulk_dc_fc_row_009 => (r#"bulk dc 009"#, r###"integer n=5; (( n <<= 1 )); print -r $n"###);
        bulk_dc_fc_row_010 => (r#"bulk dc 010"#, r###"integer n=5; (( n >>= 1 )); print -r $n"###);
        bulk_dc_fc_row_011 => (r#"bulk dc 011"#, r###"integer n=5; (( n /= 2 )); print -r $n"###);
        bulk_dc_fc_row_012 => (r#"bulk dc 012"#, r###"integer n=5; (( n %= 3 )); print -r $n"###);
        bulk_dc_fc_row_013 => (r#"bulk dc 013"#, r###"typeset -i8 n=10; print -r $n"###);
        bulk_dc_fc_row_014 => (r#"bulk dc 014"#, r###"typeset -i16 n=255; print -r $n"###);
        bulk_dc_fc_row_015 => (r#"bulk dc 015"#, r###"typeset -E2 n=4000; print -r $n"###);
        bulk_dc_fc_row_016 => (r#"bulk dc 016"#, r###"typeset -R4 n=hi; print -r $n"###);
        bulk_dc_fc_row_017 => (r#"bulk dc 017"#, r###"typeset +L n=Ab; print -r $n"###);
        bulk_dc_fc_row_018 => (r#"bulk dc 018"#, r###"typeset +U n=xy; print -r $n"###);
        bulk_dc_fc_row_019 => (r#"bulk dc 019"#, r###"typeset +i n=4; print -r $n"###);
        bulk_dc_fc_row_020 => (r#"bulk dc 020"#, r###"export EX=1; print -r $EX; unset EX"###);
        bulk_dc_fc_row_021 => (r#"bulk dc 021"#, r###"readonly ro=5; print -r $ro"###);
        bulk_dc_fc_row_022 => (r#"bulk dc 022"#, r###"print -r ${${v:-fb}}; unset v"###);
        bulk_dc_fc_row_023 => (r#"bulk dc 023"#, r###"print -r ${${v:+set}:-unset}; unset v"###);
        bulk_dc_fc_row_024 => (r#"bulk dc 024"#, r###"word=$'l1\nl2'; print -r ${(@f)word}"###);
        bulk_dc_fc_row_025 => (r#"bulk dc 025"#, r###"word=  hi  ; print -r ${(W)word}"###);
        bulk_dc_fc_row_026 => (r#"bulk dc 026"#, r###"print -r ${(z)word}; word=a b c"###);
        bulk_dc_fc_row_027 => (r#"bulk dc 027"#, r###"print -r ${(F)x}; x=$'p\nq'"###);
        bulk_dc_fc_row_028 => (r#"bulk dc 028"#, r###"print -r ${(A)x}; x=1 2"###);
        bulk_dc_fc_row_029 => (r#"bulk dc 029"#, r###"print -r ${(aa)x}; x=(1 2)"###);
        bulk_dc_fc_row_030 => (r#"bulk dc 030"#, r###"print -r ${(%)2}"###);
        bulk_dc_fc_row_031 => (r#"bulk dc 031"#, r###"o=8; print -r ${(0)o}"###);
        bulk_dc_fc_row_032 => (r#"bulk dc 032"#, r###"str=abc.def; print -r ${str:r}"###);
        bulk_dc_fc_row_033 => (r#"bulk dc 033"#, r###"str=abc.def; print -r ${str:e}"###);
        bulk_dc_fc_row_034 => (r#"bulk dc 034"#, r###"[[ -h /dev/stdin ]]; print -r $?"###);
        bulk_dc_fc_row_035 => (r#"bulk dc 035"#, r###"[[ -p /dev/fd/0 ]]; print -r $?"###);
        bulk_dc_fc_row_036 => (r#"bulk dc 036"#, r###"[[ -O /etc/hosts ]]; print -r $?"###);
        bulk_dc_fc_row_037 => (r#"bulk dc 037"#, r###"[[ -G / ]]; print -r $?"###);
        bulk_dc_fc_row_038 => (r#"bulk dc 038"#, r###"[[ -a /etc/hosts ]]; print -r $?"###);
        bulk_dc_fc_row_039 => (r#"bulk dc 039"#, r###"[[ bee = *ee* ]]; print -r $?"###);
        bulk_dc_fc_row_040 => (r#"bulk dc 040"#, r###"[[ 1 -eq 1 ]]; print -r $?"###);
        bulk_dc_fc_row_041 => (r#"bulk dc 041"#, r###"[[ 1 -ne 2 ]]; print -r $?"###);
        bulk_dc_fc_row_042 => (r#"bulk dc 042"#, r###"[[ 3 -lt 5 ]]; print -r $?"###);
        bulk_dc_fc_row_043 => (r#"bulk dc 043"#, r###"[[ 5 -le 5 ]]; print -r $?"###);
        bulk_dc_fc_row_044 => (r#"bulk dc 044"#, r###"[[ 5 -gt 3 ]]; print -r $?"###);
        bulk_dc_fc_row_045 => (r#"bulk dc 045"#, r###"[[ 5 -ge 5 ]]; print -r $?"###);
        bulk_dc_fc_row_046 => (r#"bulk dc 046"#, r###"[[ -o nullglob ]]; print -r $?"###);
        bulk_dc_fc_row_047 => (r#"bulk dc 047"#, r###"unsetopt extendedglob 2>/dev/null; [[ -o extendedglob ]]; print -r $?"###);
        bulk_dc_fc_row_048 => (r#"bulk dc 048"#, r###"setopt extendedglob; [[ -o extendedglob ]]; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_dd {
    use super::*;

    parity_gap_tests! {
        bulk_dd_fc_row_001 => (r#"bulk dd 001"#, r###"typeset +U n=xy; print -r $n"###);
        bulk_dd_fc_row_002 => (r#"bulk dd 002"#, r###"typeset +i n=4; print -r $n"###);
        bulk_dd_fc_row_003 => (r#"bulk dd 003"#, r###"export EX=1; print -r $EX; unset EX"###);
        bulk_dd_fc_row_004 => (r#"bulk dd 004"#, r###"readonly ro=5; print -r $ro"###);
        bulk_dd_fc_row_005 => (r#"bulk dd 005"#, r###"print -r ${${v:-fb}}; unset v"###);
        bulk_dd_fc_row_006 => (r#"bulk dd 006"#, r###"print -r ${${v:+set}:-unset}; unset v"###);
        bulk_dd_fc_row_007 => (r#"bulk dd 007"#, r###"word=$'l1\nl2'; print -r ${(@f)word}"###);
        bulk_dd_fc_row_008 => (r#"bulk dd 008"#, r###"word=  hi  ; print -r ${(W)word}"###);
        bulk_dd_fc_row_009 => (r#"bulk dd 009"#, r###"print -r ${(z)word}; word=a b c"###);
        bulk_dd_fc_row_010 => (r#"bulk dd 010"#, r###"print -r ${(F)x}; x=$'p\nq'"###);
        bulk_dd_fc_row_011 => (r#"bulk dd 011"#, r###"print -r ${(A)x}; x=1 2"###);
        bulk_dd_fc_row_012 => (r#"bulk dd 012"#, r###"print -r ${(aa)x}; x=(1 2)"###);
        bulk_dd_fc_row_013 => (r#"bulk dd 013"#, r###"print -r ${(%)2}"###);
        bulk_dd_fc_row_014 => (r#"bulk dd 014"#, r###"o=8; print -r ${(0)o}"###);
        bulk_dd_fc_row_015 => (r#"bulk dd 015"#, r###"str=abc.def; print -r ${str:r}"###);
        bulk_dd_fc_row_016 => (r#"bulk dd 016"#, r###"str=abc.def; print -r ${str:e}"###);
        bulk_dd_fc_row_017 => (r#"bulk dd 017"#, r###"[[ -h /dev/stdin ]]; print -r $?"###);
        bulk_dd_fc_row_018 => (r#"bulk dd 018"#, r###"[[ -p /dev/fd/0 ]]; print -r $?"###);
        bulk_dd_fc_row_019 => (r#"bulk dd 019"#, r###"[[ -O /etc/hosts ]]; print -r $?"###);
        bulk_dd_fc_row_020 => (r#"bulk dd 020"#, r###"[[ -G / ]]; print -r $?"###);
        bulk_dd_fc_row_021 => (r#"bulk dd 021"#, r###"[[ -a /etc/hosts ]]; print -r $?"###);
        bulk_dd_fc_row_022 => (r#"bulk dd 022"#, r###"[[ bee = *ee* ]]; print -r $?"###);
        bulk_dd_fc_row_023 => (r#"bulk dd 023"#, r###"[[ 1 -eq 1 ]]; print -r $?"###);
        bulk_dd_fc_row_024 => (r#"bulk dd 024"#, r###"[[ 1 -ne 2 ]]; print -r $?"###);
        bulk_dd_fc_row_025 => (r#"bulk dd 025"#, r###"[[ 3 -lt 5 ]]; print -r $?"###);
        bulk_dd_fc_row_026 => (r#"bulk dd 026"#, r###"[[ 5 -le 5 ]]; print -r $?"###);
        bulk_dd_fc_row_027 => (r#"bulk dd 027"#, r###"[[ 5 -gt 3 ]]; print -r $?"###);
        bulk_dd_fc_row_028 => (r#"bulk dd 028"#, r###"[[ 5 -ge 5 ]]; print -r $?"###);
        bulk_dd_fc_row_029 => (r#"bulk dd 029"#, r###"[[ -o nullglob ]]; print -r $?"###);
        bulk_dd_fc_row_030 => (r#"bulk dd 030"#, r###"unsetopt extendedglob 2>/dev/null; [[ -o extendedglob ]]; print -r $?"###);
        bulk_dd_fc_row_031 => (r#"bulk dd 031"#, r###"setopt extendedglob; [[ -o extendedglob ]]; print -r $?"###);
        bulk_dd_fc_row_032 => (r#"bulk dd 032"#, r###"[[ -o no_extendedglob ]]; print -r $?"###);
        bulk_dd_fc_row_033 => (r#"bulk dd 033"#, r###"print -r $(( 1 , 2 , 3 ))"###);
        bulk_dd_fc_row_034 => (r#"bulk dd 034"#, r###"print -r $(( 3 < 5 ? 1 : 0 ))"###);
        bulk_dd_fc_row_035 => (r#"bulk dd 035"#, r###"print -r $(( 0xff & 0x0f ))"###);
        bulk_dd_fc_row_036 => (r#"bulk dd 036"#, r###"print -r $(( 1 << 4 ))"###);
        bulk_dd_fc_row_037 => (r#"bulk dd 037"#, r###"print -r $(( 16 >> 2 ))"###);
        bulk_dd_fc_row_038 => (r#"bulk dd 038"#, r###"print -r $(( -1 >> 1 ))"###);
        bulk_dd_fc_row_039 => (r#"bulk dd 039"#, r###"print -r $(( 8#17 ))"###);
        bulk_dd_fc_row_040 => (r#"bulk dd 040"#, r###"print -r $(( 16#ff ))"###);
        bulk_dd_fc_row_041 => (r#"bulk dd 041"#, r###"print -r $(( 2#1010 ))"###);
        bulk_dd_fc_row_042 => (r#"bulk dd 042"#, r###"print -r $(( 0b1010 ))"###);
        bulk_dd_fc_row_043 => (r#"bulk dd 043"#, r###"typeset -F1 c=1.05; print -r $(( c > 1 ))"###);
        bulk_dd_fc_row_044 => (r#"bulk dd 044"#, r###"print -r $(( 4 % 2 == 0 ))"###);
        bulk_dd_fc_row_045 => (r#"bulk dd 045"#, r###"print -r $(( 0 - 1 == -1 ))"###);
        bulk_dd_fc_row_046 => (r#"bulk dd 046"#, r###"print -r $(( 72 / 8 / 3 ))"###);
        bulk_dd_fc_row_047 => (r#"bulk dd 047"#, r###"print -r $(( 24 % 5 % 3 ))"###);
        bulk_dd_fc_row_048 => (r#"bulk dd 048"#, r###"print -r $(( 2 | 4 | 8 ))"###);
    }
}

mod corpus_dash_fc_bulk_de {
    use super::*;

    parity_gap_tests! {
        bulk_de_fc_row_001 => (r#"bulk de 001"#, r###"[[ -p /dev/fd/0 ]]; print -r $?"###);
        bulk_de_fc_row_002 => (r#"bulk de 002"#, r###"[[ -O /etc/hosts ]]; print -r $?"###);
        bulk_de_fc_row_003 => (r#"bulk de 003"#, r###"[[ -G / ]]; print -r $?"###);
        bulk_de_fc_row_004 => (r#"bulk de 004"#, r###"[[ -a /etc/hosts ]]; print -r $?"###);
        bulk_de_fc_row_005 => (r#"bulk de 005"#, r###"[[ bee = *ee* ]]; print -r $?"###);
        bulk_de_fc_row_006 => (r#"bulk de 006"#, r###"[[ 1 -eq 1 ]]; print -r $?"###);
        bulk_de_fc_row_007 => (r#"bulk de 007"#, r###"[[ 1 -ne 2 ]]; print -r $?"###);
        bulk_de_fc_row_008 => (r#"bulk de 008"#, r###"[[ 3 -lt 5 ]]; print -r $?"###);
        bulk_de_fc_row_009 => (r#"bulk de 009"#, r###"[[ 5 -le 5 ]]; print -r $?"###);
        bulk_de_fc_row_010 => (r#"bulk de 010"#, r###"[[ 5 -gt 3 ]]; print -r $?"###);
        bulk_de_fc_row_011 => (r#"bulk de 011"#, r###"[[ 5 -ge 5 ]]; print -r $?"###);
        bulk_de_fc_row_012 => (r#"bulk de 012"#, r###"[[ -o nullglob ]]; print -r $?"###);
        bulk_de_fc_row_013 => (r#"bulk de 013"#, r###"unsetopt extendedglob 2>/dev/null; [[ -o extendedglob ]]; print -r $?"###);
        bulk_de_fc_row_014 => (r#"bulk de 014"#, r###"setopt extendedglob; [[ -o extendedglob ]]; print -r $?"###);
        bulk_de_fc_row_015 => (r#"bulk de 015"#, r###"[[ -o no_extendedglob ]]; print -r $?"###);
        bulk_de_fc_row_016 => (r#"bulk de 016"#, r###"print -r $(( 1 , 2 , 3 ))"###);
        bulk_de_fc_row_017 => (r#"bulk de 017"#, r###"print -r $(( 3 < 5 ? 1 : 0 ))"###);
        bulk_de_fc_row_018 => (r#"bulk de 018"#, r###"print -r $(( 0xff & 0x0f ))"###);
        bulk_de_fc_row_019 => (r#"bulk de 019"#, r###"print -r $(( 1 << 4 ))"###);
        bulk_de_fc_row_020 => (r#"bulk de 020"#, r###"print -r $(( 16 >> 2 ))"###);
        bulk_de_fc_row_021 => (r#"bulk de 021"#, r###"print -r $(( -1 >> 1 ))"###);
        bulk_de_fc_row_022 => (r#"bulk de 022"#, r###"print -r $(( 8#17 ))"###);
        bulk_de_fc_row_023 => (r#"bulk de 023"#, r###"print -r $(( 16#ff ))"###);
        bulk_de_fc_row_024 => (r#"bulk de 024"#, r###"print -r $(( 2#1010 ))"###);
        bulk_de_fc_row_025 => (r#"bulk de 025"#, r###"print -r $(( 0b1010 ))"###);
        bulk_de_fc_row_026 => (r#"bulk de 026"#, r###"typeset -F1 c=1.05; print -r $(( c > 1 ))"###);
        bulk_de_fc_row_027 => (r#"bulk de 027"#, r###"print -r $(( 4 % 2 == 0 ))"###);
        bulk_de_fc_row_028 => (r#"bulk de 028"#, r###"print -r $(( 0 - 1 == -1 ))"###);
        bulk_de_fc_row_029 => (r#"bulk de 029"#, r###"print -r $(( 72 / 8 / 3 ))"###);
        bulk_de_fc_row_030 => (r#"bulk de 030"#, r###"print -r $(( 24 % 5 % 3 ))"###);
        bulk_de_fc_row_031 => (r#"bulk de 031"#, r###"print -r $(( 2 | 4 | 8 ))"###);
        bulk_de_fc_row_032 => (r#"bulk de 032"#, r###"print -r $(( 15 ^ 9 ))"###);
        bulk_de_fc_row_033 => (r#"bulk de 033"#, r###"print -r $(( 0 || 0 || 7 ))"###);
        bulk_de_fc_row_034 => (r#"bulk de 034"#, r###"print -r $(( 1 || -1 ))"###);
        bulk_de_fc_row_035 => (r#"bulk de 035"#, r###"print -r $(( (1>0) + (0>0) ))"###);
        bulk_de_fc_row_036 => (r#"bulk de 036"#, r###"print -r $(( 3 > 2 > 1 ))"###);
        bulk_de_fc_row_037 => (r#"bulk de 037"#, r###"print -r $(( (9>8)>>(1<0) ))"###);
        bulk_de_fc_row_038 => (r#"bulk de 038"#, r###"print -r $(( 5 ** 2 % 7 ))"###);
        bulk_de_fc_row_039 => (r#"bulk de 039"#, r###"print -r $(( 11 ** 2 % 50 ))"###);
        bulk_de_fc_row_040 => (r#"bulk de 040"#, r###"print -r $(( 100 / 20 / 5 ))"###);
        bulk_de_fc_row_041 => (r#"bulk de 041"#, r###"print -r $(( 2#101 & 2#010 ))"###);
        bulk_de_fc_row_042 => (r#"bulk de 042"#, r###"print -r $(( 0x80 >> 4 ))"###);
        bulk_de_fc_row_043 => (r#"bulk de 043"#, r###"print -r $(( 5 ** 0 ** 3 ))"###);
        bulk_de_fc_row_044 => (r#"bulk de 044"#, r###"print -r $(( -(-(-5)) ))"###);
        bulk_de_fc_row_045 => (r#"bulk de 045"#, r###"print -r $(( (1+2)*(3+4) ))"###);
        bulk_de_fc_row_046 => (r#"bulk de 046"#, r###"v1=v1; [[ v1 -ef v1 ]]; print -r $?"###);
        bulk_de_fc_row_047 => (r#"bulk de 047"#, r###"[[ "" != x ]]; print -r $?"###);
        bulk_de_fc_row_048 => (r#"bulk de 048"#, r###"[[ -n /dev/null ]]; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_df {
    use super::*;

    parity_gap_tests! {
        bulk_df_fc_row_001 => (r#"bulk df 001"#, r###"print -r $(( 0xff & 0x0f ))"###);
        bulk_df_fc_row_002 => (r#"bulk df 002"#, r###"print -r $(( 1 << 4 ))"###);
        bulk_df_fc_row_003 => (r#"bulk df 003"#, r###"print -r $(( 16 >> 2 ))"###);
        bulk_df_fc_row_004 => (r#"bulk df 004"#, r###"print -r $(( -1 >> 1 ))"###);
        bulk_df_fc_row_005 => (r#"bulk df 005"#, r###"print -r $(( 8#17 ))"###);
        bulk_df_fc_row_006 => (r#"bulk df 006"#, r###"print -r $(( 16#ff ))"###);
        bulk_df_fc_row_007 => (r#"bulk df 007"#, r###"print -r $(( 2#1010 ))"###);
        bulk_df_fc_row_008 => (r#"bulk df 008"#, r###"print -r $(( 0b1010 ))"###);
        bulk_df_fc_row_009 => (r#"bulk df 009"#, r###"typeset -F1 c=1.05; print -r $(( c > 1 ))"###);
        bulk_df_fc_row_010 => (r#"bulk df 010"#, r###"print -r $(( 4 % 2 == 0 ))"###);
        bulk_df_fc_row_011 => (r#"bulk df 011"#, r###"print -r $(( 0 - 1 == -1 ))"###);
        bulk_df_fc_row_012 => (r#"bulk df 012"#, r###"print -r $(( 72 / 8 / 3 ))"###);
        bulk_df_fc_row_013 => (r#"bulk df 013"#, r###"print -r $(( 24 % 5 % 3 ))"###);
        bulk_df_fc_row_014 => (r#"bulk df 014"#, r###"print -r $(( 2 | 4 | 8 ))"###);
        bulk_df_fc_row_015 => (r#"bulk df 015"#, r###"print -r $(( 15 ^ 9 ))"###);
        bulk_df_fc_row_016 => (r#"bulk df 016"#, r###"print -r $(( 0 || 0 || 7 ))"###);
        bulk_df_fc_row_017 => (r#"bulk df 017"#, r###"print -r $(( 1 || -1 ))"###);
        bulk_df_fc_row_018 => (r#"bulk df 018"#, r###"print -r $(( (1>0) + (0>0) ))"###);
        bulk_df_fc_row_019 => (r#"bulk df 019"#, r###"print -r $(( 3 > 2 > 1 ))"###);
        bulk_df_fc_row_020 => (r#"bulk df 020"#, r###"print -r $(( (9>8)>>(1<0) ))"###);
        bulk_df_fc_row_021 => (r#"bulk df 021"#, r###"print -r $(( 5 ** 2 % 7 ))"###);
        bulk_df_fc_row_022 => (r#"bulk df 022"#, r###"print -r $(( 11 ** 2 % 50 ))"###);
        bulk_df_fc_row_023 => (r#"bulk df 023"#, r###"print -r $(( 100 / 20 / 5 ))"###);
        bulk_df_fc_row_024 => (r#"bulk df 024"#, r###"print -r $(( 2#101 & 2#010 ))"###);
        bulk_df_fc_row_025 => (r#"bulk df 025"#, r###"print -r $(( 0x80 >> 4 ))"###);
        bulk_df_fc_row_026 => (r#"bulk df 026"#, r###"print -r $(( 5 ** 0 ** 3 ))"###);
        bulk_df_fc_row_027 => (r#"bulk df 027"#, r###"print -r $(( -(-(-5)) ))"###);
        bulk_df_fc_row_028 => (r#"bulk df 028"#, r###"print -r $(( (1+2)*(3+4) ))"###);
        bulk_df_fc_row_029 => (r#"bulk df 029"#, r###"v1=v1; [[ v1 -ef v1 ]]; print -r $?"###);
        bulk_df_fc_row_030 => (r#"bulk df 030"#, r###"[[ "" != x ]]; print -r $?"###);
        bulk_df_fc_row_031 => (r#"bulk df 031"#, r###"[[ -n /dev/null ]]; print -r $?"###);
        bulk_df_fc_row_032 => (r#"bulk df 032"#, r###"setopt extendedglob; [[ mix = [[:digit:]]# ]]; print -r $?"###);
        bulk_df_fc_row_033 => (r#"bulk df 033"#, r####"setopt extendedglob; [[ tag = (#m)[a-z]##_t ]]; print -r $?"####);
        bulk_df_fc_row_034 => (r#"bulk df 034"#, r###"setopt extendedglob; [[ foo = fo(#e) ]]; print -r $?"###);
        bulk_df_fc_row_035 => (r#"bulk df 035"#, r###"setopt extendedglob; [[ foo = (#s)fo ]]; print -r $?"###);
        bulk_df_fc_row_036 => (r#"bulk df 036"#, r###"[[ abc < abd ]]; print -r $?"###);
        bulk_df_fc_row_037 => (r#"bulk df 037"#, r###"[[ abc > abb ]]; print -r $?"###);
        bulk_df_fc_row_038 => (r#"bulk df 038"#, r###"[[ abc != def ]]; print -r $?"###);
        bulk_df_fc_row_039 => (r#"bulk df 039"#, r###"[[ abc == abc ]]; print -r $?"###);
        bulk_df_fc_row_040 => (r#"bulk df 040"#, r###"print -r ${(L)@}; set -- MIXED"###);
        bulk_df_fc_row_041 => (r#"bulk df 041"#, r###"slice=abcdef; print -r $slice[3,5]"###);
        bulk_df_fc_row_042 => (r#"bulk df 042"#, r###"typeset -aS ary=x y; print -r $ary[2]"###);
        bulk_df_fc_row_043 => (r#"bulk df 043"#, r###"pushd /tmp >/dev/null 2>&1; popd >/dev/null 2>&1; print -r $?"###);
        bulk_df_fc_row_044 => (r#"bulk df 044"#, r###"builtin cd -q / 2>/dev/null; print -r $?"###);
        bulk_df_fc_row_045 => (r#"bulk df 045"#, r###"cd /tmp 2>/dev/null; print -r ${PWD:t}"###);
        bulk_df_fc_row_046 => (r#"bulk df 046"#, r###"hash -r 2>/dev/null; print -r $?"###);
        bulk_df_fc_row_047 => (r#"bulk df 047"#, r###"autoload -Uz is-at-least 2>/dev/null; print -r $?"###);
        bulk_df_fc_row_048 => (r#"bulk df 048"#, r###"whence -v print 2>/dev/null; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_dg {
    use super::*;

    parity_gap_tests! {
        bulk_dg_fc_row_001 => (r#"bulk dg 001"#, r###"print -r $(( (1>0) + (0>0) ))"###);
        bulk_dg_fc_row_002 => (r#"bulk dg 002"#, r###"print -r $(( 3 > 2 > 1 ))"###);
        bulk_dg_fc_row_003 => (r#"bulk dg 003"#, r###"print -r $(( (9>8)>>(1<0) ))"###);
        bulk_dg_fc_row_004 => (r#"bulk dg 004"#, r###"print -r $(( 5 ** 2 % 7 ))"###);
        bulk_dg_fc_row_005 => (r#"bulk dg 005"#, r###"print -r $(( 11 ** 2 % 50 ))"###);
        bulk_dg_fc_row_006 => (r#"bulk dg 006"#, r###"print -r $(( 100 / 20 / 5 ))"###);
        bulk_dg_fc_row_007 => (r#"bulk dg 007"#, r###"print -r $(( 2#101 & 2#010 ))"###);
        bulk_dg_fc_row_008 => (r#"bulk dg 008"#, r###"print -r $(( 0x80 >> 4 ))"###);
        bulk_dg_fc_row_009 => (r#"bulk dg 009"#, r###"print -r $(( 5 ** 0 ** 3 ))"###);
        bulk_dg_fc_row_010 => (r#"bulk dg 010"#, r###"print -r $(( -(-(-5)) ))"###);
        bulk_dg_fc_row_011 => (r#"bulk dg 011"#, r###"print -r $(( (1+2)*(3+4) ))"###);
        bulk_dg_fc_row_012 => (r#"bulk dg 012"#, r###"v1=v1; [[ v1 -ef v1 ]]; print -r $?"###);
        bulk_dg_fc_row_013 => (r#"bulk dg 013"#, r###"[[ "" != x ]]; print -r $?"###);
        bulk_dg_fc_row_014 => (r#"bulk dg 014"#, r###"[[ -n /dev/null ]]; print -r $?"###);
        bulk_dg_fc_row_015 => (r#"bulk dg 015"#, r###"setopt extendedglob; [[ mix = [[:digit:]]# ]]; print -r $?"###);
        bulk_dg_fc_row_016 => (r#"bulk dg 016"#, r####"setopt extendedglob; [[ tag = (#m)[a-z]##_t ]]; print -r $?"####);
        bulk_dg_fc_row_017 => (r#"bulk dg 017"#, r###"setopt extendedglob; [[ foo = fo(#e) ]]; print -r $?"###);
        bulk_dg_fc_row_018 => (r#"bulk dg 018"#, r###"setopt extendedglob; [[ foo = (#s)fo ]]; print -r $?"###);
        bulk_dg_fc_row_019 => (r#"bulk dg 019"#, r###"[[ abc < abd ]]; print -r $?"###);
        bulk_dg_fc_row_020 => (r#"bulk dg 020"#, r###"[[ abc > abb ]]; print -r $?"###);
        bulk_dg_fc_row_021 => (r#"bulk dg 021"#, r###"[[ abc != def ]]; print -r $?"###);
        bulk_dg_fc_row_022 => (r#"bulk dg 022"#, r###"[[ abc == abc ]]; print -r $?"###);
        bulk_dg_fc_row_023 => (r#"bulk dg 023"#, r###"print -r ${(L)@}; set -- MIXED"###);
        bulk_dg_fc_row_024 => (r#"bulk dg 024"#, r###"slice=abcdef; print -r $slice[3,5]"###);
        bulk_dg_fc_row_025 => (r#"bulk dg 025"#, r###"typeset -aS ary=x y; print -r $ary[2]"###);
        bulk_dg_fc_row_026 => (r#"bulk dg 026"#, r###"pushd /tmp >/dev/null 2>&1; popd >/dev/null 2>&1; print -r $?"###);
        bulk_dg_fc_row_027 => (r#"bulk dg 027"#, r###"builtin cd -q / 2>/dev/null; print -r $?"###);
        bulk_dg_fc_row_028 => (r#"bulk dg 028"#, r###"cd /tmp 2>/dev/null; print -r ${PWD:t}"###);
        bulk_dg_fc_row_029 => (r#"bulk dg 029"#, r###"hash -r 2>/dev/null; print -r $?"###);
        bulk_dg_fc_row_030 => (r#"bulk dg 030"#, r###"autoload -Uz is-at-least 2>/dev/null; print -r $?"###);
        bulk_dg_fc_row_031 => (r#"bulk dg 031"#, r###"whence -v print 2>/dev/null; print -r $?"###);
        bulk_dg_fc_row_032 => (r#"bulk dg 032"#, r###"whence -p ls 2>/dev/null | head -1"###);
        bulk_dg_fc_row_033 => (r#"bulk dg 033"#, r###"typeset -f fn 2>/dev/null; fn(){ :; }; print -r $?"###);
        bulk_dg_fc_row_034 => (r#"bulk dg 034"#, r###"functions fn 2>/dev/null; fn(){ :; }; print -r $?"###);
        bulk_dg_fc_row_035 => (r#"bulk dg 035"#, r###"unfunction fn 2>/dev/null; fn(){ :; }; unfunction fn; print -r $?"###);
        bulk_dg_fc_row_036 => (r#"bulk dg 036"#, r###"print -r ${aliases[za]:-none}"###);
        bulk_dg_fc_row_037 => (r#"bulk dg 037"#, r###"print -r ${(t)parameters[PATH]}"###);
        bulk_dg_fc_row_038 => (r#"bulk dg 038"#, r###"print -r ${(k)parameters[(I)PATH]}"###);
        bulk_dg_fc_row_039 => (r#"bulk dg 039"#, r###"print -r ${+parameters[PATH]}"###);
        bulk_dg_fc_row_040 => (r#"bulk dg 040"#, r###"print -r ${+functions[fn]}; fn(){}"###);
        bulk_dg_fc_row_041 => (r#"bulk dg 041"#, r###"print -r ${+commands[print]}"###);
        bulk_dg_fc_row_042 => (r#"bulk dg 042"#, r###"print -r ${+zsh_eval_context}"###);
        bulk_dg_fc_row_043 => (r#"bulk dg 043"#, r###"print -r ${+functrace}"###);
        bulk_dg_fc_row_044 => (r#"bulk dg 044"#, r###"print -r ${+funcstack}"###);
        bulk_dg_fc_row_045 => (r#"bulk dg 045"#, r###"print -r ${+funcfiletrace}"###);
        bulk_dg_fc_row_046 => (r#"bulk dg 046"#, r###"print -r ${+jobstates}"###);
        bulk_dg_fc_row_047 => (r#"bulk dg 047"#, r###"print -r ${+jobtexts}"###);
        bulk_dg_fc_row_048 => (r#"bulk dg 048"#, r###"print -r ${+jobdirs}"###);
    }
}

mod corpus_dash_fc_bulk_dh {
    use super::*;

    parity_gap_tests! {
        bulk_dh_fc_row_001 => (r#"bulk dh 001"#, r###"setopt extendedglob; [[ foo = (#s)fo ]]; print -r $?"###);
        bulk_dh_fc_row_002 => (r#"bulk dh 002"#, r###"[[ abc < abd ]]; print -r $?"###);
        bulk_dh_fc_row_003 => (r#"bulk dh 003"#, r###"[[ abc > abb ]]; print -r $?"###);
        bulk_dh_fc_row_004 => (r#"bulk dh 004"#, r###"[[ abc != def ]]; print -r $?"###);
        bulk_dh_fc_row_005 => (r#"bulk dh 005"#, r###"[[ abc == abc ]]; print -r $?"###);
        bulk_dh_fc_row_006 => (r#"bulk dh 006"#, r###"print -r ${(L)@}; set -- MIXED"###);
        bulk_dh_fc_row_007 => (r#"bulk dh 007"#, r###"slice=abcdef; print -r $slice[3,5]"###);
        bulk_dh_fc_row_008 => (r#"bulk dh 008"#, r###"typeset -aS ary=x y; print -r $ary[2]"###);
        bulk_dh_fc_row_009 => (r#"bulk dh 009"#, r###"pushd /tmp >/dev/null 2>&1; popd >/dev/null 2>&1; print -r $?"###);
        bulk_dh_fc_row_010 => (r#"bulk dh 010"#, r###"builtin cd -q / 2>/dev/null; print -r $?"###);
        bulk_dh_fc_row_011 => (r#"bulk dh 011"#, r###"cd /tmp 2>/dev/null; print -r ${PWD:t}"###);
        bulk_dh_fc_row_012 => (r#"bulk dh 012"#, r###"hash -r 2>/dev/null; print -r $?"###);
        bulk_dh_fc_row_013 => (r#"bulk dh 013"#, r###"autoload -Uz is-at-least 2>/dev/null; print -r $?"###);
        bulk_dh_fc_row_014 => (r#"bulk dh 014"#, r###"whence -v print 2>/dev/null; print -r $?"###);
        bulk_dh_fc_row_015 => (r#"bulk dh 015"#, r###"whence -p ls 2>/dev/null | head -1"###);
        bulk_dh_fc_row_016 => (r#"bulk dh 016"#, r###"typeset -f fn 2>/dev/null; fn(){ :; }; print -r $?"###);
        bulk_dh_fc_row_017 => (r#"bulk dh 017"#, r###"functions fn 2>/dev/null; fn(){ :; }; print -r $?"###);
        bulk_dh_fc_row_018 => (r#"bulk dh 018"#, r###"unfunction fn 2>/dev/null; fn(){ :; }; unfunction fn; print -r $?"###);
        bulk_dh_fc_row_019 => (r#"bulk dh 019"#, r###"print -r ${aliases[za]:-none}"###);
        bulk_dh_fc_row_020 => (r#"bulk dh 020"#, r###"print -r ${(t)parameters[PATH]}"###);
        bulk_dh_fc_row_021 => (r#"bulk dh 021"#, r###"print -r ${(k)parameters[(I)PATH]}"###);
        bulk_dh_fc_row_022 => (r#"bulk dh 022"#, r###"print -r ${+parameters[PATH]}"###);
        bulk_dh_fc_row_023 => (r#"bulk dh 023"#, r###"print -r ${+functions[fn]}; fn(){}"###);
        bulk_dh_fc_row_024 => (r#"bulk dh 024"#, r###"print -r ${+commands[print]}"###);
        bulk_dh_fc_row_025 => (r#"bulk dh 025"#, r###"print -r ${+zsh_eval_context}"###);
        bulk_dh_fc_row_026 => (r#"bulk dh 026"#, r###"print -r ${+functrace}"###);
        bulk_dh_fc_row_027 => (r#"bulk dh 027"#, r###"print -r ${+funcstack}"###);
        bulk_dh_fc_row_028 => (r#"bulk dh 028"#, r###"print -r ${+funcfiletrace}"###);
        bulk_dh_fc_row_029 => (r#"bulk dh 029"#, r###"print -r ${+jobstates}"###);
        bulk_dh_fc_row_030 => (r#"bulk dh 030"#, r###"print -r ${+jobtexts}"###);
        bulk_dh_fc_row_031 => (r#"bulk dh 031"#, r###"print -r ${+jobdirs}"###);
        bulk_dh_fc_row_032 => (r#"bulk dh 032"#, r###"print -r ${+historywords}"###);
        bulk_dh_fc_row_033 => (r#"bulk dh 033"#, r###"print -r ${+usergroups}"###);
        bulk_dh_fc_row_034 => (r#"bulk dh 034"#, r###"print -r ${+dis_builtins}"###);
        bulk_dh_fc_row_035 => (r#"bulk dh 035"#, r###"print -r ${+dis_widgets}"###);
        bulk_dh_fc_row_036 => (r#"bulk dh 036"#, r###"print -r ${+dis_reswords}"###);
        bulk_dh_fc_row_037 => (r#"bulk dh 037"#, r###"print -r ${+dis_patchars}"###);
        bulk_dh_fc_row_038 => (r#"bulk dh 038"#, r###"print -r ${+dis_commands}"###);
        bulk_dh_fc_row_039 => (r#"bulk dh 039"#, r###"print -r ${+module_path}"###);
        bulk_dh_fc_row_040 => (r#"bulk dh 040"#, r###"print -r ${+functrace}"###);
        bulk_dh_fc_row_041 => (r#"bulk dh 041"#, r###"true | true | false; print -r ${pipestatus[3]}"###);
        bulk_dh_fc_row_042 => (r#"bulk dh 042"#, r###"{ true; false; }; print -r $?"###);
        bulk_dh_fc_row_043 => (r#"bulk dh 043"#, r###"fn(){ typeset -a la=(x y); print -r ${#la}; }; fn"###);
        bulk_dh_fc_row_044 => (r#"bulk dh 044"#, r###"print -r ${arr[@]:1:2}; arr=(a b c d)"###);
        bulk_dh_fc_row_045 => (r#"bulk dh 045"#, r###"print -r ${(pj:,:)a}; a=(x y)"###);
        bulk_dh_fc_row_046 => (r#"bulk dh 046"#, r###"print -r ${(Mk)h}; typeset -A h; h=(x 1 y 2)"###);
        bulk_dh_fc_row_047 => (r#"bulk dh 047"#, r###"print -r ${(oa)n}; n=(10 2 1)"###);
        bulk_dh_fc_row_048 => (r#"bulk dh 048"#, r###"print -r ${(On)n}; n=(10 2 1)"###);
    }
}

mod corpus_dash_fc_bulk_di {
    use super::*;

    parity_gap_tests! {
        bulk_di_fc_row_001 => (r#"bulk di 001"#, r###"unfunction fn 2>/dev/null; fn(){ :; }; unfunction fn; print -r $?"###);
        bulk_di_fc_row_002 => (r#"bulk di 002"#, r###"print -r ${aliases[za]:-none}"###);
        bulk_di_fc_row_003 => (r#"bulk di 003"#, r###"print -r ${(t)parameters[PATH]}"###);
        bulk_di_fc_row_004 => (r#"bulk di 004"#, r###"print -r ${(k)parameters[(I)PATH]}"###);
        bulk_di_fc_row_005 => (r#"bulk di 005"#, r###"print -r ${+parameters[PATH]}"###);
        bulk_di_fc_row_006 => (r#"bulk di 006"#, r###"print -r ${+functions[fn]}; fn(){}"###);
        bulk_di_fc_row_007 => (r#"bulk di 007"#, r###"print -r ${+commands[print]}"###);
        bulk_di_fc_row_008 => (r#"bulk di 008"#, r###"print -r ${+zsh_eval_context}"###);
        bulk_di_fc_row_009 => (r#"bulk di 009"#, r###"print -r ${+functrace}"###);
        bulk_di_fc_row_010 => (r#"bulk di 010"#, r###"print -r ${+funcstack}"###);
        bulk_di_fc_row_011 => (r#"bulk di 011"#, r###"print -r ${+funcfiletrace}"###);
        bulk_di_fc_row_012 => (r#"bulk di 012"#, r###"print -r ${+jobstates}"###);
        bulk_di_fc_row_013 => (r#"bulk di 013"#, r###"print -r ${+jobtexts}"###);
        bulk_di_fc_row_014 => (r#"bulk di 014"#, r###"print -r ${+jobdirs}"###);
        bulk_di_fc_row_015 => (r#"bulk di 015"#, r###"print -r ${+historywords}"###);
        bulk_di_fc_row_016 => (r#"bulk di 016"#, r###"print -r ${+usergroups}"###);
        bulk_di_fc_row_017 => (r#"bulk di 017"#, r###"print -r ${+dis_builtins}"###);
        bulk_di_fc_row_018 => (r#"bulk di 018"#, r###"print -r ${+dis_widgets}"###);
        bulk_di_fc_row_019 => (r#"bulk di 019"#, r###"print -r ${+dis_reswords}"###);
        bulk_di_fc_row_020 => (r#"bulk di 020"#, r###"print -r ${+dis_patchars}"###);
        bulk_di_fc_row_021 => (r#"bulk di 021"#, r###"print -r ${+dis_commands}"###);
        bulk_di_fc_row_022 => (r#"bulk di 022"#, r###"print -r ${+module_path}"###);
        bulk_di_fc_row_023 => (r#"bulk di 023"#, r###"print -r ${+functrace}"###);
        bulk_di_fc_row_024 => (r#"bulk di 024"#, r###"true | true | false; print -r ${pipestatus[3]}"###);
        bulk_di_fc_row_025 => (r#"bulk di 025"#, r###"{ true; false; }; print -r $?"###);
        bulk_di_fc_row_026 => (r#"bulk di 026"#, r###"fn(){ typeset -a la=(x y); print -r ${#la}; }; fn"###);
        bulk_di_fc_row_027 => (r#"bulk di 027"#, r###"print -r ${arr[@]:1:2}; arr=(a b c d)"###);
        bulk_di_fc_row_028 => (r#"bulk di 028"#, r###"print -r ${(pj:,:)a}; a=(x y)"###);
        bulk_di_fc_row_029 => (r#"bulk di 029"#, r###"print -r ${(Mk)h}; typeset -A h; h=(x 1 y 2)"###);
        bulk_di_fc_row_030 => (r#"bulk di 030"#, r###"print -r ${(oa)n}; n=(10 2 1)"###);
        bulk_di_fc_row_031 => (r#"bulk di 031"#, r###"print -r ${(On)n}; n=(10 2 1)"###);
        bulk_di_fc_row_032 => (r#"bulk di 032"#, r###"print -r ${(n)a}; a=(1 2 3)"###);
        bulk_di_fc_row_033 => (r#"bulk di 033"#, r###"print -r ${(N)a}; a=(1 2 3)"###);
        bulk_di_fc_row_034 => (r#"bulk di 034"#, r###"print -r ${(w)#w}; w=a b c"###);
        bulk_di_fc_row_035 => (r#"bulk di 035"#, r###"print -r ${(t)x}; x=hello"###);
        bulk_di_fc_row_036 => (r#"bulk di 036"#, r###"unset y; print -r ${+y}"###);
        bulk_di_fc_row_037 => (r#"bulk di 037"#, r###"x=hello; print -r ${+x}"###);
        bulk_di_fc_row_038 => (r#"bulk di 038"#, r###"print -r ${(q+)x}; x=hi"###);
        bulk_di_fc_row_039 => (r#"bulk di 039"#, r###"x=foo; print -r ${x:s/foo/bar/}"###);
        bulk_di_fc_row_040 => (r#"bulk di 040"#, r###"x=foofoo; print -r ${x//foo/bar}"###);
        bulk_di_fc_row_041 => (r#"bulk di 041"#, r###"x=abc; print -r ${x/#a/z}"###);
        bulk_di_fc_row_042 => (r#"bulk di 042"#, r###"x=abc; print -r ${x/%c/z}"###);
        bulk_di_fc_row_043 => (r#"bulk di 043"#, r###"print -r ${(j::)a}; a=(x y)"###);
        bulk_di_fc_row_044 => (r#"bulk di 044"#, r###"print -r ${(pj::)a}; a=(x y)"###);
        bulk_di_fc_row_045 => (r#"bulk di 045"#, r###"print -r ${(ps:\n:)x}; x=$'a\nb'"###);
        bulk_di_fc_row_046 => (r#"bulk di 046"#, r###"print -r ${(e)x}; x=$'2+2'"###);
        bulk_di_fc_row_047 => (r#"bulk di 047"#, r###"integer co=0; : $(( co=6 )); print -r $co"###);
        bulk_di_fc_row_048 => (r#"bulk di 048"#, r###"print -r $(( 1<<0 ))"###);
    }
}

mod corpus_dash_fc_bulk_dj {
    use super::*;

    parity_gap_tests! {
        bulk_dj_fc_row_001 => (r#"bulk dj 001"#, r###"print -r ${+dis_widgets}"###);
        bulk_dj_fc_row_002 => (r#"bulk dj 002"#, r###"print -r ${+dis_reswords}"###);
        bulk_dj_fc_row_003 => (r#"bulk dj 003"#, r###"print -r ${+dis_patchars}"###);
        bulk_dj_fc_row_004 => (r#"bulk dj 004"#, r###"print -r ${+dis_commands}"###);
        bulk_dj_fc_row_005 => (r#"bulk dj 005"#, r###"print -r ${+module_path}"###);
        bulk_dj_fc_row_006 => (r#"bulk dj 006"#, r###"print -r ${+functrace}"###);
        bulk_dj_fc_row_007 => (r#"bulk dj 007"#, r###"true | true | false; print -r ${pipestatus[3]}"###);
        bulk_dj_fc_row_008 => (r#"bulk dj 008"#, r###"{ true; false; }; print -r $?"###);
        bulk_dj_fc_row_009 => (r#"bulk dj 009"#, r###"fn(){ typeset -a la=(x y); print -r ${#la}; }; fn"###);
        bulk_dj_fc_row_010 => (r#"bulk dj 010"#, r###"print -r ${arr[@]:1:2}; arr=(a b c d)"###);
        bulk_dj_fc_row_011 => (r#"bulk dj 011"#, r###"print -r ${(pj:,:)a}; a=(x y)"###);
        bulk_dj_fc_row_012 => (r#"bulk dj 012"#, r###"print -r ${(Mk)h}; typeset -A h; h=(x 1 y 2)"###);
        bulk_dj_fc_row_013 => (r#"bulk dj 013"#, r###"print -r ${(oa)n}; n=(10 2 1)"###);
        bulk_dj_fc_row_014 => (r#"bulk dj 014"#, r###"print -r ${(On)n}; n=(10 2 1)"###);
        bulk_dj_fc_row_015 => (r#"bulk dj 015"#, r###"print -r ${(n)a}; a=(1 2 3)"###);
        bulk_dj_fc_row_016 => (r#"bulk dj 016"#, r###"print -r ${(N)a}; a=(1 2 3)"###);
        bulk_dj_fc_row_017 => (r#"bulk dj 017"#, r###"print -r ${(w)#w}; w=a b c"###);
        bulk_dj_fc_row_018 => (r#"bulk dj 018"#, r###"print -r ${(t)x}; x=hello"###);
        bulk_dj_fc_row_019 => (r#"bulk dj 019"#, r###"unset y; print -r ${+y}"###);
        bulk_dj_fc_row_020 => (r#"bulk dj 020"#, r###"x=hello; print -r ${+x}"###);
        bulk_dj_fc_row_021 => (r#"bulk dj 021"#, r###"print -r ${(q+)x}; x=hi"###);
        bulk_dj_fc_row_022 => (r#"bulk dj 022"#, r###"x=foo; print -r ${x:s/foo/bar/}"###);
        bulk_dj_fc_row_023 => (r#"bulk dj 023"#, r###"x=foofoo; print -r ${x//foo/bar}"###);
        bulk_dj_fc_row_024 => (r#"bulk dj 024"#, r###"x=abc; print -r ${x/#a/z}"###);
        bulk_dj_fc_row_025 => (r#"bulk dj 025"#, r###"x=abc; print -r ${x/%c/z}"###);
        bulk_dj_fc_row_026 => (r#"bulk dj 026"#, r###"print -r ${(j::)a}; a=(x y)"###);
        bulk_dj_fc_row_027 => (r#"bulk dj 027"#, r###"print -r ${(pj::)a}; a=(x y)"###);
        bulk_dj_fc_row_028 => (r#"bulk dj 028"#, r###"print -r ${(ps:\n:)x}; x=$'a\nb'"###);
        bulk_dj_fc_row_029 => (r#"bulk dj 029"#, r###"print -r ${(e)x}; x=$'2+2'"###);
        bulk_dj_fc_row_030 => (r#"bulk dj 030"#, r###"integer co=0; : $(( co=6 )); print -r $co"###);
        bulk_dj_fc_row_031 => (r#"bulk dj 031"#, r###"print -r $(( 1<<0 ))"###);
        bulk_dj_fc_row_032 => (r#"bulk dj 032"#, r###"print -r $(( 1<<10 ))"###);
        bulk_dj_fc_row_033 => (r#"bulk dj 033"#, r###"print -r $(( 0x7fffffff & 0 ))"###);
        bulk_dj_fc_row_034 => (r#"bulk dj 034"#, r###"print -r $(( 1000003 % 97 ))"###);
        bulk_dj_fc_row_035 => (r#"bulk dj 035"#, r###"print -r $(( 63 & 31 | 15 ))"###);
        bulk_dj_fc_row_036 => (r#"bulk dj 036"#, r###"print -r $(( 0x10001 % 256 ))"###);
        bulk_dj_fc_row_037 => (r#"bulk dj 037"#, r###"print -r $(( 2*2*2*2 ))"###);
        bulk_dj_fc_row_038 => (r#"bulk dj 038"#, r###"print -r $(( (1==1)+(0==1) ))"###);
        bulk_dj_fc_row_039 => (r#"bulk dj 039"#, r###"print -r $(( 1 && (0 || 1) ))"###);
        bulk_dj_fc_row_040 => (r#"bulk dj 040"#, r###"[[ zero = <-> ]]; print -r $?"###);
        bulk_dj_fc_row_041 => (r#"bulk dj 041"#, r###"[[ . = . ]]; print -r $?"###);
        bulk_dj_fc_row_042 => (r#"bulk dj 042"#, r###"[[ a -lt b ]]; print -r $?"###);
        bulk_dj_fc_row_043 => (r#"bulk dj 043"#, r####"[[ ABC = [A-Z]## ]]; print -r $?"####);
        bulk_dj_fc_row_044 => (r#"bulk dj 044"#, r###"[[ AAA =~ ^A+ ]]; print -r $?"###);
        bulk_dj_fc_row_045 => (r#"bulk dj 045"#, r###"[[ bot = *ot* ]]; print -r $?"###);
        bulk_dj_fc_row_046 => (r#"bulk dj 046"#, r###"[[ -e /dev/null ]]; print -r $?"###);
        bulk_dj_fc_row_047 => (r#"bulk dj 047"#, r###"[[ -s /dev/null ]]; print -r $?"###);
        bulk_dj_fc_row_048 => (r#"bulk dj 048"#, r###"[[ -u /etc/hosts ]]; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_dk {
    use super::*;

    parity_gap_tests! {
        bulk_dk_fc_row_001 => (r#"bulk dk 001"#, r###"print -r ${(w)#w}; w=a b c"###);
        bulk_dk_fc_row_002 => (r#"bulk dk 002"#, r###"print -r ${(t)x}; x=hello"###);
        bulk_dk_fc_row_003 => (r#"bulk dk 003"#, r###"unset y; print -r ${+y}"###);
        bulk_dk_fc_row_004 => (r#"bulk dk 004"#, r###"x=hello; print -r ${+x}"###);
        bulk_dk_fc_row_005 => (r#"bulk dk 005"#, r###"print -r ${(q+)x}; x=hi"###);
        bulk_dk_fc_row_006 => (r#"bulk dk 006"#, r###"x=foo; print -r ${x:s/foo/bar/}"###);
        bulk_dk_fc_row_007 => (r#"bulk dk 007"#, r###"x=foofoo; print -r ${x//foo/bar}"###);
        bulk_dk_fc_row_008 => (r#"bulk dk 008"#, r###"x=abc; print -r ${x/#a/z}"###);
        bulk_dk_fc_row_009 => (r#"bulk dk 009"#, r###"x=abc; print -r ${x/%c/z}"###);
        bulk_dk_fc_row_010 => (r#"bulk dk 010"#, r###"print -r ${(j::)a}; a=(x y)"###);
        bulk_dk_fc_row_011 => (r#"bulk dk 011"#, r###"print -r ${(pj::)a}; a=(x y)"###);
        bulk_dk_fc_row_012 => (r#"bulk dk 012"#, r###"print -r ${(ps:\n:)x}; x=$'a\nb'"###);
        bulk_dk_fc_row_013 => (r#"bulk dk 013"#, r###"print -r ${(e)x}; x=$'2+2'"###);
        bulk_dk_fc_row_014 => (r#"bulk dk 014"#, r###"integer co=0; : $(( co=6 )); print -r $co"###);
        bulk_dk_fc_row_015 => (r#"bulk dk 015"#, r###"print -r $(( 1<<0 ))"###);
        bulk_dk_fc_row_016 => (r#"bulk dk 016"#, r###"print -r $(( 1<<10 ))"###);
        bulk_dk_fc_row_017 => (r#"bulk dk 017"#, r###"print -r $(( 0x7fffffff & 0 ))"###);
        bulk_dk_fc_row_018 => (r#"bulk dk 018"#, r###"print -r $(( 1000003 % 97 ))"###);
        bulk_dk_fc_row_019 => (r#"bulk dk 019"#, r###"print -r $(( 63 & 31 | 15 ))"###);
        bulk_dk_fc_row_020 => (r#"bulk dk 020"#, r###"print -r $(( 0x10001 % 256 ))"###);
        bulk_dk_fc_row_021 => (r#"bulk dk 021"#, r###"print -r $(( 2*2*2*2 ))"###);
        bulk_dk_fc_row_022 => (r#"bulk dk 022"#, r###"print -r $(( (1==1)+(0==1) ))"###);
        bulk_dk_fc_row_023 => (r#"bulk dk 023"#, r###"print -r $(( 1 && (0 || 1) ))"###);
        bulk_dk_fc_row_024 => (r#"bulk dk 024"#, r###"[[ zero = <-> ]]; print -r $?"###);
        bulk_dk_fc_row_025 => (r#"bulk dk 025"#, r###"[[ . = . ]]; print -r $?"###);
        bulk_dk_fc_row_026 => (r#"bulk dk 026"#, r###"[[ a -lt b ]]; print -r $?"###);
        bulk_dk_fc_row_027 => (r#"bulk dk 027"#, r####"[[ ABC = [A-Z]## ]]; print -r $?"####);
        bulk_dk_fc_row_028 => (r#"bulk dk 028"#, r###"[[ AAA =~ ^A+ ]]; print -r $?"###);
        bulk_dk_fc_row_029 => (r#"bulk dk 029"#, r###"[[ bot = *ot* ]]; print -r $?"###);
        bulk_dk_fc_row_030 => (r#"bulk dk 030"#, r###"[[ -e /dev/null ]]; print -r $?"###);
        bulk_dk_fc_row_031 => (r#"bulk dk 031"#, r###"[[ -s /dev/null ]]; print -r $?"###);
        bulk_dk_fc_row_032 => (r#"bulk dk 032"#, r###"[[ -u /etc/hosts ]]; print -r $?"###);
        bulk_dk_fc_row_033 => (r#"bulk dk 033"#, r###"[[ -g / ]]; print -r $?"###);
        bulk_dk_fc_row_034 => (r#"bulk dk 034"#, r###"[[ -k /tmp ]]; print -r $?"###);
        bulk_dk_fc_row_035 => (r#"bulk dk 035"#, r###"[[ -b /dev/null ]]; print -r $?"###);
        bulk_dk_fc_row_036 => (r#"bulk dk 036"#, r###"[[ -c /dev/null ]]; print -r $?"###);
        bulk_dk_fc_row_037 => (r#"bulk dk 037"#, r###"print -r ${(l:5::0:)n}; n=42"###);
        bulk_dk_fc_row_038 => (r#"bulk dk 038"#, r###"print -r ${(r:5::0:)n}; n=42"###);
        bulk_dk_fc_row_039 => (r#"bulk dk 039"#, r###"print -r ${(c)str}; str=hello"###);
        bulk_dk_fc_row_040 => (r#"bulk dk 040"#, r###"print -r ${(u)arr}; arr=(a A b)"###);
        bulk_dk_fc_row_041 => (r#"bulk dk 041"#, r###"print -r ${(L)str}; str=HELLO"###);
        bulk_dk_fc_row_042 => (r#"bulk dk 042"#, r###"print -r ${(U)str}; str=hello"###);
        bulk_dk_fc_row_043 => (r#"bulk dk 043"#, r###"print -r ${(C)str}; str=hello world"###);
        bulk_dk_fc_row_044 => (r#"bulk dk 044"#, r###"print -r ${(Q)str}; str=$'a\nb'"###);
        bulk_dk_fc_row_045 => (r#"bulk dk 045"#, r###"print -r ${(qq)str}; str=hi"###);
        bulk_dk_fc_row_046 => (r#"bulk dk 046"#, r###"print -r ${(V)str}; str=hi"###);
        bulk_dk_fc_row_047 => (r#"bulk dk 047"#, r###"print -r ${(z)str}; str=$'a\0b'"###);
        bulk_dk_fc_row_048 => (r#"bulk dk 048"#, r###"print -r ${(j:-:)a}; a=(x y)"###);
    }
}

mod corpus_dash_fc_bulk_dl {
    use super::*;

    parity_gap_tests! {
        bulk_dl_fc_row_001 => (r#"bulk dl 001"#, r###"print -r $(( 1000003 % 97 ))"###);
        bulk_dl_fc_row_002 => (r#"bulk dl 002"#, r###"print -r $(( 63 & 31 | 15 ))"###);
        bulk_dl_fc_row_003 => (r#"bulk dl 003"#, r###"print -r $(( 0x10001 % 256 ))"###);
        bulk_dl_fc_row_004 => (r#"bulk dl 004"#, r###"print -r $(( 2*2*2*2 ))"###);
        bulk_dl_fc_row_005 => (r#"bulk dl 005"#, r###"print -r $(( (1==1)+(0==1) ))"###);
        bulk_dl_fc_row_006 => (r#"bulk dl 006"#, r###"print -r $(( 1 && (0 || 1) ))"###);
        bulk_dl_fc_row_007 => (r#"bulk dl 007"#, r###"[[ zero = <-> ]]; print -r $?"###);
        bulk_dl_fc_row_008 => (r#"bulk dl 008"#, r###"[[ . = . ]]; print -r $?"###);
        bulk_dl_fc_row_009 => (r#"bulk dl 009"#, r###"[[ a -lt b ]]; print -r $?"###);
        bulk_dl_fc_row_010 => (r#"bulk dl 010"#, r####"[[ ABC = [A-Z]## ]]; print -r $?"####);
        bulk_dl_fc_row_011 => (r#"bulk dl 011"#, r###"[[ AAA =~ ^A+ ]]; print -r $?"###);
        bulk_dl_fc_row_012 => (r#"bulk dl 012"#, r###"[[ bot = *ot* ]]; print -r $?"###);
        bulk_dl_fc_row_013 => (r#"bulk dl 013"#, r###"[[ -e /dev/null ]]; print -r $?"###);
        bulk_dl_fc_row_014 => (r#"bulk dl 014"#, r###"[[ -s /dev/null ]]; print -r $?"###);
        bulk_dl_fc_row_015 => (r#"bulk dl 015"#, r###"[[ -u /etc/hosts ]]; print -r $?"###);
        bulk_dl_fc_row_016 => (r#"bulk dl 016"#, r###"[[ -g / ]]; print -r $?"###);
        bulk_dl_fc_row_017 => (r#"bulk dl 017"#, r###"[[ -k /tmp ]]; print -r $?"###);
        bulk_dl_fc_row_018 => (r#"bulk dl 018"#, r###"[[ -b /dev/null ]]; print -r $?"###);
        bulk_dl_fc_row_019 => (r#"bulk dl 019"#, r###"[[ -c /dev/null ]]; print -r $?"###);
        bulk_dl_fc_row_020 => (r#"bulk dl 020"#, r###"print -r ${(l:5::0:)n}; n=42"###);
        bulk_dl_fc_row_021 => (r#"bulk dl 021"#, r###"print -r ${(r:5::0:)n}; n=42"###);
        bulk_dl_fc_row_022 => (r#"bulk dl 022"#, r###"print -r ${(c)str}; str=hello"###);
        bulk_dl_fc_row_023 => (r#"bulk dl 023"#, r###"print -r ${(u)arr}; arr=(a A b)"###);
        bulk_dl_fc_row_024 => (r#"bulk dl 024"#, r###"print -r ${(L)str}; str=HELLO"###);
        bulk_dl_fc_row_025 => (r#"bulk dl 025"#, r###"print -r ${(U)str}; str=hello"###);
        bulk_dl_fc_row_026 => (r#"bulk dl 026"#, r###"print -r ${(C)str}; str=hello world"###);
        bulk_dl_fc_row_027 => (r#"bulk dl 027"#, r###"print -r ${(Q)str}; str=$'a\nb'"###);
        bulk_dl_fc_row_028 => (r#"bulk dl 028"#, r###"print -r ${(qq)str}; str=hi"###);
        bulk_dl_fc_row_029 => (r#"bulk dl 029"#, r###"print -r ${(V)str}; str=hi"###);
        bulk_dl_fc_row_030 => (r#"bulk dl 030"#, r###"print -r ${(z)str}; str=$'a\0b'"###);
        bulk_dl_fc_row_031 => (r#"bulk dl 031"#, r###"print -r ${(j:-:)a}; a=(x y)"###);
        bulk_dl_fc_row_032 => (r#"bulk dl 032"#, r###"print -r ${(pj:-:)a}; a=(x y)"###);
        bulk_dl_fc_row_033 => (r#"bulk dl 033"#, r###"a=(1 2 3); print -r ${a[1,-1]}"###);
        bulk_dl_fc_row_034 => (r#"bulk dl 034"#, r###"a=(1 2 3); print -r ${a[1,2]}"###);
        bulk_dl_fc_row_035 => (r#"bulk dl 035"#, r###"a=(1 2 3); print -r ${a[-1]}"###);
        bulk_dl_fc_row_036 => (r#"bulk dl 036"#, r###"a=(1 2 3); print -r ${a[(i)2]}"###);
        bulk_dl_fc_row_037 => (r#"bulk dl 037"#, r###"a=(1 2 3); print -r ${a[(I)2]}"###);
        bulk_dl_fc_row_038 => (r#"bulk dl 038"#, r###"a=(1 2 3); print -r ${a[(R)9]}"###);
        bulk_dl_fc_row_039 => (r#"bulk dl 039"#, r###"a=(1 2 3); print -r ${a[(r)2]}"###);
        bulk_dl_fc_row_040 => (r#"bulk dl 040"#, r###"typeset -A m; m=(k v); print -r ${m[k]}"###);
        bulk_dl_fc_row_041 => (r#"bulk dl 041"#, r###"typeset -A m; m=(k v); print -r ${(k)m}"###);
        bulk_dl_fc_row_042 => (r#"bulk dl 042"#, r###"typeset -A m; m=(k v); print -r ${(v)m}"###);
        bulk_dl_fc_row_043 => (r#"bulk dl 043"#, r###"typeset -A m; m=(a 1 b 2); print -r ${(kv)m}"###);
        bulk_dl_fc_row_044 => (r#"bulk dl 044"#, r###"print -r ${(o)lst}; lst=(z a m)"###);
        bulk_dl_fc_row_045 => (r#"bulk dl 045"#, r###"print -r ${(O)lst}; lst=(z a m)"###);
        bulk_dl_fc_row_046 => (r#"bulk dl 046"#, r###"print -r ${(i)lst}; lst=(z a m)"###);
        bulk_dl_fc_row_047 => (r#"bulk dl 047"#, r###"a=(x y); print -r ${^a}"###);
        bulk_dl_fc_row_048 => (r#"bulk dl 048"#, r###"a=(1 2); b=(a b); print -r ${^a}${^b}"###);
    }
}

mod corpus_dash_fc_bulk_dm {
    use super::*;

    parity_gap_tests! {
        bulk_dm_fc_row_001 => (r#"bulk dm 001"#, r###"[[ -b /dev/null ]]; print -r $?"###);
        bulk_dm_fc_row_002 => (r#"bulk dm 002"#, r###"[[ -c /dev/null ]]; print -r $?"###);
        bulk_dm_fc_row_003 => (r#"bulk dm 003"#, r###"print -r ${(l:5::0:)n}; n=42"###);
        bulk_dm_fc_row_004 => (r#"bulk dm 004"#, r###"print -r ${(r:5::0:)n}; n=42"###);
        bulk_dm_fc_row_005 => (r#"bulk dm 005"#, r###"print -r ${(c)str}; str=hello"###);
        bulk_dm_fc_row_006 => (r#"bulk dm 006"#, r###"print -r ${(u)arr}; arr=(a A b)"###);
        bulk_dm_fc_row_007 => (r#"bulk dm 007"#, r###"print -r ${(L)str}; str=HELLO"###);
        bulk_dm_fc_row_008 => (r#"bulk dm 008"#, r###"print -r ${(U)str}; str=hello"###);
        bulk_dm_fc_row_009 => (r#"bulk dm 009"#, r###"print -r ${(C)str}; str=hello world"###);
        bulk_dm_fc_row_010 => (r#"bulk dm 010"#, r###"print -r ${(Q)str}; str=$'a\nb'"###);
        bulk_dm_fc_row_011 => (r#"bulk dm 011"#, r###"print -r ${(qq)str}; str=hi"###);
        bulk_dm_fc_row_012 => (r#"bulk dm 012"#, r###"print -r ${(V)str}; str=hi"###);
        bulk_dm_fc_row_013 => (r#"bulk dm 013"#, r###"print -r ${(z)str}; str=$'a\0b'"###);
        bulk_dm_fc_row_014 => (r#"bulk dm 014"#, r###"print -r ${(j:-:)a}; a=(x y)"###);
        bulk_dm_fc_row_015 => (r#"bulk dm 015"#, r###"print -r ${(pj:-:)a}; a=(x y)"###);
        bulk_dm_fc_row_016 => (r#"bulk dm 016"#, r###"a=(1 2 3); print -r ${a[1,-1]}"###);
        bulk_dm_fc_row_017 => (r#"bulk dm 017"#, r###"a=(1 2 3); print -r ${a[1,2]}"###);
        bulk_dm_fc_row_018 => (r#"bulk dm 018"#, r###"a=(1 2 3); print -r ${a[-1]}"###);
        bulk_dm_fc_row_019 => (r#"bulk dm 019"#, r###"a=(1 2 3); print -r ${a[(i)2]}"###);
        bulk_dm_fc_row_020 => (r#"bulk dm 020"#, r###"a=(1 2 3); print -r ${a[(I)2]}"###);
        bulk_dm_fc_row_021 => (r#"bulk dm 021"#, r###"a=(1 2 3); print -r ${a[(R)9]}"###);
        bulk_dm_fc_row_022 => (r#"bulk dm 022"#, r###"a=(1 2 3); print -r ${a[(r)2]}"###);
        bulk_dm_fc_row_023 => (r#"bulk dm 023"#, r###"typeset -A m; m=(k v); print -r ${m[k]}"###);
        bulk_dm_fc_row_024 => (r#"bulk dm 024"#, r###"typeset -A m; m=(k v); print -r ${(k)m}"###);
        bulk_dm_fc_row_025 => (r#"bulk dm 025"#, r###"typeset -A m; m=(k v); print -r ${(v)m}"###);
        bulk_dm_fc_row_026 => (r#"bulk dm 026"#, r###"typeset -A m; m=(a 1 b 2); print -r ${(kv)m}"###);
        bulk_dm_fc_row_027 => (r#"bulk dm 027"#, r###"print -r ${(o)lst}; lst=(z a m)"###);
        bulk_dm_fc_row_028 => (r#"bulk dm 028"#, r###"print -r ${(O)lst}; lst=(z a m)"###);
        bulk_dm_fc_row_029 => (r#"bulk dm 029"#, r###"print -r ${(i)lst}; lst=(z a m)"###);
        bulk_dm_fc_row_030 => (r#"bulk dm 030"#, r###"a=(x y); print -r ${^a}"###);
        bulk_dm_fc_row_031 => (r#"bulk dm 031"#, r###"a=(1 2); b=(a b); print -r ${^a}${^b}"###);
        bulk_dm_fc_row_032 => (r#"bulk dm 032"#, r###"setopt braceccl; print -r {a,b}"###);
        bulk_dm_fc_row_033 => (r#"bulk dm 033"#, r###"print -r {1..3}"###);
        bulk_dm_fc_row_034 => (r#"bulk dm 034"#, r###"print -r {01..03}"###);
        bulk_dm_fc_row_035 => (r#"bulk dm 035"#, r###"print -r {a..c}"###);
        bulk_dm_fc_row_036 => (r#"bulk dm 036"#, r###"print -r {1..4..2}"###);
        bulk_dm_fc_row_037 => (r#"bulk dm 037"#, r###"print -r ${~pattern}; pattern='*'; :"###);
        bulk_dm_fc_row_038 => (r#"bulk dm 038"#, r###"integer x=3; (( x++ )); print -r $x"###);
        bulk_dm_fc_row_039 => (r#"bulk dm 039"#, r###"integer x=3; (( ++x )); print -r $x"###);
        bulk_dm_fc_row_040 => (r#"bulk dm 040"#, r###"integer x=3; (( x-- )); print -r $x"###);
        bulk_dm_fc_row_041 => (r#"bulk dm 041"#, r###"integer x=3; print -r $(( x ** 2 ))"###);
        bulk_dm_fc_row_042 => (r#"bulk dm 042"#, r###"float f=1.5; print -r $(( f + 1 ))"###);
        bulk_dm_fc_row_043 => (r#"bulk dm 043"#, r###"print -r $(( 7 / 2 ))"###);
        bulk_dm_fc_row_044 => (r#"bulk dm 044"#, r###"print -r $(( 7.0 / 2 ))"###);
        bulk_dm_fc_row_045 => (r#"bulk dm 045"#, r###"(( 1 )); print -r $?"###);
        bulk_dm_fc_row_046 => (r#"bulk dm 046"#, r###"(( 0 )); print -r $?"###);
        bulk_dm_fc_row_047 => (r#"bulk dm 047"#, r###": $(( 0 )) || print -r z"###);
        bulk_dm_fc_row_048 => (r#"bulk dm 048"#, r###": $(( 1 )) && print -r y"###);
    }
}

mod corpus_dash_fc_bulk_dn {
    use super::*;

    parity_gap_tests! {
        bulk_dn_fc_row_001 => (r#"bulk dn 001"#, r###"print -r ${(j:-:)a}; a=(x y)"###);
        bulk_dn_fc_row_002 => (r#"bulk dn 002"#, r###"print -r ${(pj:-:)a}; a=(x y)"###);
        bulk_dn_fc_row_003 => (r#"bulk dn 003"#, r###"a=(1 2 3); print -r ${a[1,-1]}"###);
        bulk_dn_fc_row_004 => (r#"bulk dn 004"#, r###"a=(1 2 3); print -r ${a[1,2]}"###);
        bulk_dn_fc_row_005 => (r#"bulk dn 005"#, r###"a=(1 2 3); print -r ${a[-1]}"###);
        bulk_dn_fc_row_006 => (r#"bulk dn 006"#, r###"a=(1 2 3); print -r ${a[(i)2]}"###);
        bulk_dn_fc_row_007 => (r#"bulk dn 007"#, r###"a=(1 2 3); print -r ${a[(I)2]}"###);
        bulk_dn_fc_row_008 => (r#"bulk dn 008"#, r###"a=(1 2 3); print -r ${a[(R)9]}"###);
        bulk_dn_fc_row_009 => (r#"bulk dn 009"#, r###"a=(1 2 3); print -r ${a[(r)2]}"###);
        bulk_dn_fc_row_010 => (r#"bulk dn 010"#, r###"typeset -A m; m=(k v); print -r ${m[k]}"###);
        bulk_dn_fc_row_011 => (r#"bulk dn 011"#, r###"typeset -A m; m=(k v); print -r ${(k)m}"###);
        bulk_dn_fc_row_012 => (r#"bulk dn 012"#, r###"typeset -A m; m=(k v); print -r ${(v)m}"###);
        bulk_dn_fc_row_013 => (r#"bulk dn 013"#, r###"typeset -A m; m=(a 1 b 2); print -r ${(kv)m}"###);
        bulk_dn_fc_row_014 => (r#"bulk dn 014"#, r###"print -r ${(o)lst}; lst=(z a m)"###);
        bulk_dn_fc_row_015 => (r#"bulk dn 015"#, r###"print -r ${(O)lst}; lst=(z a m)"###);
        bulk_dn_fc_row_016 => (r#"bulk dn 016"#, r###"print -r ${(i)lst}; lst=(z a m)"###);
        bulk_dn_fc_row_017 => (r#"bulk dn 017"#, r###"a=(x y); print -r ${^a}"###);
        bulk_dn_fc_row_018 => (r#"bulk dn 018"#, r###"a=(1 2); b=(a b); print -r ${^a}${^b}"###);
        bulk_dn_fc_row_019 => (r#"bulk dn 019"#, r###"setopt braceccl; print -r {a,b}"###);
        bulk_dn_fc_row_020 => (r#"bulk dn 020"#, r###"print -r {1..3}"###);
        bulk_dn_fc_row_021 => (r#"bulk dn 021"#, r###"print -r {01..03}"###);
        bulk_dn_fc_row_022 => (r#"bulk dn 022"#, r###"print -r {a..c}"###);
        bulk_dn_fc_row_023 => (r#"bulk dn 023"#, r###"print -r {1..4..2}"###);
        bulk_dn_fc_row_024 => (r#"bulk dn 024"#, r###"print -r ${~pattern}; pattern='*'; :"###);
        bulk_dn_fc_row_025 => (r#"bulk dn 025"#, r###"integer x=3; (( x++ )); print -r $x"###);
        bulk_dn_fc_row_026 => (r#"bulk dn 026"#, r###"integer x=3; (( ++x )); print -r $x"###);
        bulk_dn_fc_row_027 => (r#"bulk dn 027"#, r###"integer x=3; (( x-- )); print -r $x"###);
        bulk_dn_fc_row_028 => (r#"bulk dn 028"#, r###"integer x=3; print -r $(( x ** 2 ))"###);
        bulk_dn_fc_row_029 => (r#"bulk dn 029"#, r###"float f=1.5; print -r $(( f + 1 ))"###);
        bulk_dn_fc_row_030 => (r#"bulk dn 030"#, r###"print -r $(( 7 / 2 ))"###);
        bulk_dn_fc_row_031 => (r#"bulk dn 031"#, r###"print -r $(( 7.0 / 2 ))"###);
        bulk_dn_fc_row_032 => (r#"bulk dn 032"#, r###"(( 1 )); print -r $?"###);
        bulk_dn_fc_row_033 => (r#"bulk dn 033"#, r###"(( 0 )); print -r $?"###);
        bulk_dn_fc_row_034 => (r#"bulk dn 034"#, r###": $(( 0 )) || print -r z"###);
        bulk_dn_fc_row_035 => (r#"bulk dn 035"#, r###": $(( 1 )) && print -r y"###);
        bulk_dn_fc_row_036 => (r#"bulk dn 036"#, r###"let x=2+2; print -r $x"###);
        bulk_dn_fc_row_037 => (r#"bulk dn 037"#, r###"(( x = 5 )); print -r $x"###);
        bulk_dn_fc_row_038 => (r#"bulk dn 038"#, r###"typeset -F f=2.5; print -r $f"###);
        bulk_dn_fc_row_039 => (r#"bulk dn 039"#, r###"typeset -E e=2.5; print -r $e"###);
        bulk_dn_fc_row_040 => (r#"bulk dn 040"#, r###"typeset -i n=07; print -r $n"###);
        bulk_dn_fc_row_041 => (r#"bulk dn 041"#, r###"typeset -l s=ABC; print -r $s"###);
        bulk_dn_fc_row_042 => (r#"bulk dn 042"#, r###"typeset -u s=abc; print -r $s"###);
        bulk_dn_fc_row_043 => (r#"bulk dn 043"#, r###"typeset -r x=1; x=2; print -r $x"###);
        bulk_dn_fc_row_044 => (r#"bulk dn 044"#, r###"typeset -h s; s=abc; print -r $s"###);
        bulk_dn_fc_row_045 => (r#"bulk dn 045"#, r###"typeset -H s; s=abc; print -r $s"###);
        bulk_dn_fc_row_046 => (r#"bulk dn 046"#, r###"typeset -b n=255; print -r $n"###);
        bulk_dn_fc_row_047 => (r#"bulk dn 047"#, r###"typeset -o n=7; print -r $n"###);
        bulk_dn_fc_row_048 => (r#"bulk dn 048"#, r###"typeset -aU u; u=(a a b); print -r ${(j:,:)u}"###);
    }
}

mod corpus_dash_fc_bulk_do {
    use super::*;

    parity_gap_tests! {
        bulk_do_fc_row_001 => (r#"bulk do 001"#, r###"print -r ${(i)lst}; lst=(z a m)"###);
        bulk_do_fc_row_002 => (r#"bulk do 002"#, r###"a=(x y); print -r ${^a}"###);
        bulk_do_fc_row_003 => (r#"bulk do 003"#, r###"a=(1 2); b=(a b); print -r ${^a}${^b}"###);
        bulk_do_fc_row_004 => (r#"bulk do 004"#, r###"setopt braceccl; print -r {a,b}"###);
        bulk_do_fc_row_005 => (r#"bulk do 005"#, r###"print -r {1..3}"###);
        bulk_do_fc_row_006 => (r#"bulk do 006"#, r###"print -r {01..03}"###);
        bulk_do_fc_row_007 => (r#"bulk do 007"#, r###"print -r {a..c}"###);
        bulk_do_fc_row_008 => (r#"bulk do 008"#, r###"print -r {1..4..2}"###);
        bulk_do_fc_row_009 => (r#"bulk do 009"#, r###"print -r ${~pattern}; pattern='*'; :"###);
        bulk_do_fc_row_010 => (r#"bulk do 010"#, r###"integer x=3; (( x++ )); print -r $x"###);
        bulk_do_fc_row_011 => (r#"bulk do 011"#, r###"integer x=3; (( ++x )); print -r $x"###);
        bulk_do_fc_row_012 => (r#"bulk do 012"#, r###"integer x=3; (( x-- )); print -r $x"###);
        bulk_do_fc_row_013 => (r#"bulk do 013"#, r###"integer x=3; print -r $(( x ** 2 ))"###);
        bulk_do_fc_row_014 => (r#"bulk do 014"#, r###"float f=1.5; print -r $(( f + 1 ))"###);
        bulk_do_fc_row_015 => (r#"bulk do 015"#, r###"print -r $(( 7 / 2 ))"###);
        bulk_do_fc_row_016 => (r#"bulk do 016"#, r###"print -r $(( 7.0 / 2 ))"###);
        bulk_do_fc_row_017 => (r#"bulk do 017"#, r###"(( 1 )); print -r $?"###);
        bulk_do_fc_row_018 => (r#"bulk do 018"#, r###"(( 0 )); print -r $?"###);
        bulk_do_fc_row_019 => (r#"bulk do 019"#, r###": $(( 0 )) || print -r z"###);
        bulk_do_fc_row_020 => (r#"bulk do 020"#, r###": $(( 1 )) && print -r y"###);
        bulk_do_fc_row_021 => (r#"bulk do 021"#, r###"let x=2+2; print -r $x"###);
        bulk_do_fc_row_022 => (r#"bulk do 022"#, r###"(( x = 5 )); print -r $x"###);
        bulk_do_fc_row_023 => (r#"bulk do 023"#, r###"typeset -F f=2.5; print -r $f"###);
        bulk_do_fc_row_024 => (r#"bulk do 024"#, r###"typeset -E e=2.5; print -r $e"###);
        bulk_do_fc_row_025 => (r#"bulk do 025"#, r###"typeset -i n=07; print -r $n"###);
        bulk_do_fc_row_026 => (r#"bulk do 026"#, r###"typeset -l s=ABC; print -r $s"###);
        bulk_do_fc_row_027 => (r#"bulk do 027"#, r###"typeset -u s=abc; print -r $s"###);
        bulk_do_fc_row_028 => (r#"bulk do 028"#, r###"typeset -r x=1; x=2; print -r $x"###);
        bulk_do_fc_row_029 => (r#"bulk do 029"#, r###"typeset -h s; s=abc; print -r $s"###);
        bulk_do_fc_row_030 => (r#"bulk do 030"#, r###"typeset -H s; s=abc; print -r $s"###);
        bulk_do_fc_row_031 => (r#"bulk do 031"#, r###"typeset -b n=255; print -r $n"###);
        bulk_do_fc_row_032 => (r#"bulk do 032"#, r###"typeset -o n=7; print -r $n"###);
        bulk_do_fc_row_033 => (r#"bulk do 033"#, r###"typeset -aU u; u=(a a b); print -r ${(j:,:)u}"###);
        bulk_do_fc_row_034 => (r#"bulk do 034"#, r###"local a; a=1; print -r $a"###);
        bulk_do_fc_row_035 => (r#"bulk do 035"#, r###"local -i n=5; print -r $(( n * 2 ))"###);
        bulk_do_fc_row_036 => (r#"bulk do 036"#, r###"local -a arr; arr=(x); print -r $arr[1]"###);
        bulk_do_fc_row_037 => (r#"bulk do 037"#, r###"fn(){ local x=1; print -r $x; }; fn"###);
        bulk_do_fc_row_038 => (r#"bulk do 038"#, r###"fn(){ typeset -a a; a=(1); print -r ${#a}; }; fn"###);
        bulk_do_fc_row_039 => (r#"bulk do 039"#, r###"autoload -Uz add-zsh-hook 2>/dev/null; print -r $?"###);
        bulk_do_fc_row_040 => (r#"bulk do 040"#, r###"emulate -L zsh; print -r $?"###);
        bulk_do_fc_row_041 => (r#"bulk do 041"#, r###"setopt localoptions; print -r $?"###);
        bulk_do_fc_row_042 => (r#"bulk do 042"#, r###"unsetopt localoptions 2>/dev/null; print -r $?"###);
        bulk_do_fc_row_043 => (r#"bulk do 043"#, r###"setopt pipefail; false | true; print -r $?"###);
        bulk_do_fc_row_044 => (r#"bulk do 044"#, r###"setopt no_pipefail; false | true; print -r $?"###);
        bulk_do_fc_row_045 => (r#"bulk do 045"#, r###"setopt nullglob; print -r ${#files}; files=(/no/such/*)"###);
        bulk_do_fc_row_046 => (r#"bulk do 046"#, r###"setopt nonomatch; print -r ${#files}; files=(/no/such/*)"###);
        bulk_do_fc_row_047 => (r#"bulk do 047"#, r###"setopt extendedglob; print -r $?"###);
        bulk_do_fc_row_048 => (r#"bulk do 048"#, r###"setopt shwordsplit; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_dp {
    use super::*;

    parity_gap_tests! {
        bulk_dp_fc_row_001 => (r#"bulk dp 001"#, r###"(( 1 )); print -r $?"###);
        bulk_dp_fc_row_002 => (r#"bulk dp 002"#, r###"(( 0 )); print -r $?"###);
        bulk_dp_fc_row_003 => (r#"bulk dp 003"#, r###": $(( 0 )) || print -r z"###);
        bulk_dp_fc_row_004 => (r#"bulk dp 004"#, r###": $(( 1 )) && print -r y"###);
        bulk_dp_fc_row_005 => (r#"bulk dp 005"#, r###"let x=2+2; print -r $x"###);
        bulk_dp_fc_row_006 => (r#"bulk dp 006"#, r###"(( x = 5 )); print -r $x"###);
        bulk_dp_fc_row_007 => (r#"bulk dp 007"#, r###"typeset -F f=2.5; print -r $f"###);
        bulk_dp_fc_row_008 => (r#"bulk dp 008"#, r###"typeset -E e=2.5; print -r $e"###);
        bulk_dp_fc_row_009 => (r#"bulk dp 009"#, r###"typeset -i n=07; print -r $n"###);
        bulk_dp_fc_row_010 => (r#"bulk dp 010"#, r###"typeset -l s=ABC; print -r $s"###);
        bulk_dp_fc_row_011 => (r#"bulk dp 011"#, r###"typeset -u s=abc; print -r $s"###);
        bulk_dp_fc_row_012 => (r#"bulk dp 012"#, r###"typeset -r x=1; x=2; print -r $x"###);
        bulk_dp_fc_row_013 => (r#"bulk dp 013"#, r###"typeset -h s; s=abc; print -r $s"###);
        bulk_dp_fc_row_014 => (r#"bulk dp 014"#, r###"typeset -H s; s=abc; print -r $s"###);
        bulk_dp_fc_row_015 => (r#"bulk dp 015"#, r###"typeset -b n=255; print -r $n"###);
        bulk_dp_fc_row_016 => (r#"bulk dp 016"#, r###"typeset -o n=7; print -r $n"###);
        bulk_dp_fc_row_017 => (r#"bulk dp 017"#, r###"typeset -aU u; u=(a a b); print -r ${(j:,:)u}"###);
        bulk_dp_fc_row_018 => (r#"bulk dp 018"#, r###"local a; a=1; print -r $a"###);
        bulk_dp_fc_row_019 => (r#"bulk dp 019"#, r###"local -i n=5; print -r $(( n * 2 ))"###);
        bulk_dp_fc_row_020 => (r#"bulk dp 020"#, r###"local -a arr; arr=(x); print -r $arr[1]"###);
        bulk_dp_fc_row_021 => (r#"bulk dp 021"#, r###"fn(){ local x=1; print -r $x; }; fn"###);
        bulk_dp_fc_row_022 => (r#"bulk dp 022"#, r###"fn(){ typeset -a a; a=(1); print -r ${#a}; }; fn"###);
        bulk_dp_fc_row_023 => (r#"bulk dp 023"#, r###"autoload -Uz add-zsh-hook 2>/dev/null; print -r $?"###);
        bulk_dp_fc_row_024 => (r#"bulk dp 024"#, r###"emulate -L zsh; print -r $?"###);
        bulk_dp_fc_row_025 => (r#"bulk dp 025"#, r###"setopt localoptions; print -r $?"###);
        bulk_dp_fc_row_026 => (r#"bulk dp 026"#, r###"unsetopt localoptions 2>/dev/null; print -r $?"###);
        bulk_dp_fc_row_027 => (r#"bulk dp 027"#, r###"setopt pipefail; false | true; print -r $?"###);
        bulk_dp_fc_row_028 => (r#"bulk dp 028"#, r###"setopt no_pipefail; false | true; print -r $?"###);
        bulk_dp_fc_row_029 => (r#"bulk dp 029"#, r###"setopt nullglob; print -r ${#files}; files=(/no/such/*)"###);
        bulk_dp_fc_row_030 => (r#"bulk dp 030"#, r###"setopt nonomatch; print -r ${#files}; files=(/no/such/*)"###);
        bulk_dp_fc_row_031 => (r#"bulk dp 031"#, r###"setopt extendedglob; print -r $?"###);
        bulk_dp_fc_row_032 => (r#"bulk dp 032"#, r###"setopt shwordsplit; print -r $?"###);
        bulk_dp_fc_row_033 => (r#"bulk dp 033"#, r###"setopt no_shwordsplit; print -r $?"###);
        bulk_dp_fc_row_034 => (r#"bulk dp 034"#, r###"setopt interactivecomments; print -r $?"###);
        bulk_dp_fc_row_035 => (r#"bulk dp 035"#, r###"setopt no_interactivecomments; print -r $?"###);
        bulk_dp_fc_row_036 => (r#"bulk dp 036"#, r###"setopt multios; print -r $?"###);
        bulk_dp_fc_row_037 => (r#"bulk dp 037"#, r###"setopt noclobber; print -r $?"###);
        bulk_dp_fc_row_038 => (r#"bulk dp 038"#, r###"setopt clobber; print -r $?"###);
        bulk_dp_fc_row_039 => (r#"bulk dp 039"#, r###"setopt histexpand; print -r $?"###);
        bulk_dp_fc_row_040 => (r#"bulk dp 040"#, r###"setopt no_histexpand; print -r $?"###);
        bulk_dp_fc_row_041 => (r#"bulk dp 041"#, r###"setopt banghist; print -r $?"###);
        bulk_dp_fc_row_042 => (r#"bulk dp 042"#, r###"setopt sharehistory; print -r $?"###);
        bulk_dp_fc_row_043 => (r#"bulk dp 043"#, r###"setopt incappendhistory; print -r $?"###);
        bulk_dp_fc_row_044 => (r#"bulk dp 044"#, r###"setopt extendedhistory; print -r $?"###);
        bulk_dp_fc_row_045 => (r#"bulk dp 045"#, r###"setopt histignoredups; print -r $?"###);
        bulk_dp_fc_row_046 => (r#"bulk dp 046"#, r###"setopt histignorespace; print -r $?"###);
        bulk_dp_fc_row_047 => (r#"bulk dp 047"#, r###"setopt histreduceblanks; print -r $?"###);
        bulk_dp_fc_row_048 => (r#"bulk dp 048"#, r###"setopt histverify; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_dq {
    use super::*;

    parity_gap_tests! {
        bulk_dq_fc_row_001 => (r#"bulk dq 001"#, r###"typeset -aU u; u=(a a b); print -r ${(j:,:)u}"###);
        bulk_dq_fc_row_002 => (r#"bulk dq 002"#, r###"local a; a=1; print -r $a"###);
        bulk_dq_fc_row_003 => (r#"bulk dq 003"#, r###"local -i n=5; print -r $(( n * 2 ))"###);
        bulk_dq_fc_row_004 => (r#"bulk dq 004"#, r###"local -a arr; arr=(x); print -r $arr[1]"###);
        bulk_dq_fc_row_005 => (r#"bulk dq 005"#, r###"fn(){ local x=1; print -r $x; }; fn"###);
        bulk_dq_fc_row_006 => (r#"bulk dq 006"#, r###"fn(){ typeset -a a; a=(1); print -r ${#a}; }; fn"###);
        bulk_dq_fc_row_007 => (r#"bulk dq 007"#, r###"autoload -Uz add-zsh-hook 2>/dev/null; print -r $?"###);
        bulk_dq_fc_row_008 => (r#"bulk dq 008"#, r###"emulate -L zsh; print -r $?"###);
        bulk_dq_fc_row_009 => (r#"bulk dq 009"#, r###"setopt localoptions; print -r $?"###);
        bulk_dq_fc_row_010 => (r#"bulk dq 010"#, r###"unsetopt localoptions 2>/dev/null; print -r $?"###);
        bulk_dq_fc_row_011 => (r#"bulk dq 011"#, r###"setopt pipefail; false | true; print -r $?"###);
        bulk_dq_fc_row_012 => (r#"bulk dq 012"#, r###"setopt no_pipefail; false | true; print -r $?"###);
        bulk_dq_fc_row_013 => (r#"bulk dq 013"#, r###"setopt nullglob; print -r ${#files}; files=(/no/such/*)"###);
        bulk_dq_fc_row_014 => (r#"bulk dq 014"#, r###"setopt nonomatch; print -r ${#files}; files=(/no/such/*)"###);
        bulk_dq_fc_row_015 => (r#"bulk dq 015"#, r###"setopt extendedglob; print -r $?"###);
        bulk_dq_fc_row_016 => (r#"bulk dq 016"#, r###"setopt shwordsplit; print -r $?"###);
        bulk_dq_fc_row_017 => (r#"bulk dq 017"#, r###"setopt no_shwordsplit; print -r $?"###);
        bulk_dq_fc_row_018 => (r#"bulk dq 018"#, r###"setopt interactivecomments; print -r $?"###);
        bulk_dq_fc_row_019 => (r#"bulk dq 019"#, r###"setopt no_interactivecomments; print -r $?"###);
        bulk_dq_fc_row_020 => (r#"bulk dq 020"#, r###"setopt multios; print -r $?"###);
        bulk_dq_fc_row_021 => (r#"bulk dq 021"#, r###"setopt noclobber; print -r $?"###);
        bulk_dq_fc_row_022 => (r#"bulk dq 022"#, r###"setopt clobber; print -r $?"###);
        bulk_dq_fc_row_023 => (r#"bulk dq 023"#, r###"setopt histexpand; print -r $?"###);
        bulk_dq_fc_row_024 => (r#"bulk dq 024"#, r###"setopt no_histexpand; print -r $?"###);
        bulk_dq_fc_row_025 => (r#"bulk dq 025"#, r###"setopt banghist; print -r $?"###);
        bulk_dq_fc_row_026 => (r#"bulk dq 026"#, r###"setopt sharehistory; print -r $?"###);
        bulk_dq_fc_row_027 => (r#"bulk dq 027"#, r###"setopt incappendhistory; print -r $?"###);
        bulk_dq_fc_row_028 => (r#"bulk dq 028"#, r###"setopt extendedhistory; print -r $?"###);
        bulk_dq_fc_row_029 => (r#"bulk dq 029"#, r###"setopt histignoredups; print -r $?"###);
        bulk_dq_fc_row_030 => (r#"bulk dq 030"#, r###"setopt histignorespace; print -r $?"###);
        bulk_dq_fc_row_031 => (r#"bulk dq 031"#, r###"setopt histreduceblanks; print -r $?"###);
        bulk_dq_fc_row_032 => (r#"bulk dq 032"#, r###"setopt histverify; print -r $?"###);
        bulk_dq_fc_row_033 => (r#"bulk dq 033"#, r###"setopt appendhistory; print -r $?"###);
        bulk_dq_fc_row_034 => (r#"bulk dq 034"#, r###"setopt no_beep; print -r $?"###);
        bulk_dq_fc_row_035 => (r#"bulk dq 035"#, r###"setopt no_listbeep; print -r $?"###);
        bulk_dq_fc_row_036 => (r#"bulk dq 036"#, r###"setopt auto_cd; print -r $?"###);
        bulk_dq_fc_row_037 => (r#"bulk dq 037"#, r###"setopt no_auto_cd; print -r $?"###);
        bulk_dq_fc_row_038 => (r#"bulk dq 038"#, r###"setopt correct; print -r $?"###);
        bulk_dq_fc_row_039 => (r#"bulk dq 039"#, r###"setopt nocorrect; print -r $?"###);
        bulk_dq_fc_row_040 => (r#"bulk dq 040"#, r###"setopt completealiases; print -r $?"###);
        bulk_dq_fc_row_041 => (r#"bulk dq 041"#, r###"setopt globdots; print -r $?"###);
        bulk_dq_fc_row_042 => (r#"bulk dq 042"#, r###"setopt noglobdots; print -r $?"###);
        bulk_dq_fc_row_043 => (r#"bulk dq 043"#, r###"setopt numericglobsort; print -r $?"###);
        bulk_dq_fc_row_044 => (r#"bulk dq 044"#, r###"setopt markdirs; print -r $?"###);
        bulk_dq_fc_row_045 => (r#"bulk dq 045"#, r###"setopt nomarkdirs; print -r $?"###);
        bulk_dq_fc_row_046 => (r#"bulk dq 046"#, r###"setopt chase_links; print -r $?"###);
        bulk_dq_fc_row_047 => (r#"bulk dq 047"#, r###"setopt no_chase_links; print -r $?"###);
        bulk_dq_fc_row_048 => (r#"bulk dq 048"#, r###"setopt pushdignoredups; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_dr {
    use super::*;

    parity_gap_tests! {
        bulk_dr_fc_row_001 => (r#"bulk dr 001"#, r###"setopt interactivecomments; print -r $?"###);
        bulk_dr_fc_row_002 => (r#"bulk dr 002"#, r###"setopt no_interactivecomments; print -r $?"###);
        bulk_dr_fc_row_003 => (r#"bulk dr 003"#, r###"setopt multios; print -r $?"###);
        bulk_dr_fc_row_004 => (r#"bulk dr 004"#, r###"setopt noclobber; print -r $?"###);
        bulk_dr_fc_row_005 => (r#"bulk dr 005"#, r###"setopt clobber; print -r $?"###);
        bulk_dr_fc_row_006 => (r#"bulk dr 006"#, r###"setopt histexpand; print -r $?"###);
        bulk_dr_fc_row_007 => (r#"bulk dr 007"#, r###"setopt no_histexpand; print -r $?"###);
        bulk_dr_fc_row_008 => (r#"bulk dr 008"#, r###"setopt banghist; print -r $?"###);
        bulk_dr_fc_row_009 => (r#"bulk dr 009"#, r###"setopt sharehistory; print -r $?"###);
        bulk_dr_fc_row_010 => (r#"bulk dr 010"#, r###"setopt incappendhistory; print -r $?"###);
        bulk_dr_fc_row_011 => (r#"bulk dr 011"#, r###"setopt extendedhistory; print -r $?"###);
        bulk_dr_fc_row_012 => (r#"bulk dr 012"#, r###"setopt histignoredups; print -r $?"###);
        bulk_dr_fc_row_013 => (r#"bulk dr 013"#, r###"setopt histignorespace; print -r $?"###);
        bulk_dr_fc_row_014 => (r#"bulk dr 014"#, r###"setopt histreduceblanks; print -r $?"###);
        bulk_dr_fc_row_015 => (r#"bulk dr 015"#, r###"setopt histverify; print -r $?"###);
        bulk_dr_fc_row_016 => (r#"bulk dr 016"#, r###"setopt appendhistory; print -r $?"###);
        bulk_dr_fc_row_017 => (r#"bulk dr 017"#, r###"setopt no_beep; print -r $?"###);
        bulk_dr_fc_row_018 => (r#"bulk dr 018"#, r###"setopt no_listbeep; print -r $?"###);
        bulk_dr_fc_row_019 => (r#"bulk dr 019"#, r###"setopt auto_cd; print -r $?"###);
        bulk_dr_fc_row_020 => (r#"bulk dr 020"#, r###"setopt no_auto_cd; print -r $?"###);
        bulk_dr_fc_row_021 => (r#"bulk dr 021"#, r###"setopt correct; print -r $?"###);
        bulk_dr_fc_row_022 => (r#"bulk dr 022"#, r###"setopt nocorrect; print -r $?"###);
        bulk_dr_fc_row_023 => (r#"bulk dr 023"#, r###"setopt completealiases; print -r $?"###);
        bulk_dr_fc_row_024 => (r#"bulk dr 024"#, r###"setopt globdots; print -r $?"###);
        bulk_dr_fc_row_025 => (r#"bulk dr 025"#, r###"setopt noglobdots; print -r $?"###);
        bulk_dr_fc_row_026 => (r#"bulk dr 026"#, r###"setopt numericglobsort; print -r $?"###);
        bulk_dr_fc_row_027 => (r#"bulk dr 027"#, r###"setopt markdirs; print -r $?"###);
        bulk_dr_fc_row_028 => (r#"bulk dr 028"#, r###"setopt nomarkdirs; print -r $?"###);
        bulk_dr_fc_row_029 => (r#"bulk dr 029"#, r###"setopt chase_links; print -r $?"###);
        bulk_dr_fc_row_030 => (r#"bulk dr 030"#, r###"setopt no_chase_links; print -r $?"###);
        bulk_dr_fc_row_031 => (r#"bulk dr 031"#, r###"setopt pushdignoredups; print -r $?"###);
        bulk_dr_fc_row_032 => (r#"bulk dr 032"#, r###"setopt pushdsilent; print -r $?"###);
        bulk_dr_fc_row_033 => (r#"bulk dr 033"#, r###"setopt pushdtohome; print -r $?"###);
        bulk_dr_fc_row_034 => (r#"bulk dr 034"#, r###"setopt autopushd; print -r $?"###);
        bulk_dr_fc_row_035 => (r#"bulk dr 035"#, r###"setopt pushdminus; print -r $?"###);
        bulk_dr_fc_row_036 => (r#"bulk dr 036"#, r###"setopt pushdignoredups; print -r $?"###);
        bulk_dr_fc_row_037 => (r#"bulk dr 037"#, r###"dirs -p 2>/dev/null | head -1; print -r $?"###);
        bulk_dr_fc_row_038 => (r#"bulk dr 038"#, r###"pushd /tmp 2>/dev/null; popd 2>/dev/null; print -r $?"###);
        bulk_dr_fc_row_039 => (r#"bulk dr 039"#, r###"cd -q / 2>/dev/null; print -r $?"###);
        bulk_dr_fc_row_040 => (r#"bulk dr 040"#, r###"print -r $PWD"###);
        bulk_dr_fc_row_041 => (r#"bulk dr 041"#, r###"print -r ${PWD:h}"###);
        bulk_dr_fc_row_042 => (r#"bulk dr 042"#, r###"print -r ${PWD:t}"###);
        bulk_dr_fc_row_043 => (r#"bulk dr 043"#, r###"print -r ${PWD:r}"###);
        bulk_dr_fc_row_044 => (r#"bulk dr 044"#, r###"print -r ${PWD:e}"###);
        bulk_dr_fc_row_045 => (r#"bulk dr 045"#, r###"print -r ${PWD:a}"###);
        bulk_dr_fc_row_046 => (r#"bulk dr 046"#, r###"print -r ${PWD:A}"###);
        bulk_dr_fc_row_047 => (r#"bulk dr 047"#, r###"read -r line <<< 'one'; print -r $line"###);
        bulk_dr_fc_row_048 => (r#"bulk dr 048"#, r###"read -r a b <<< 'x y'; print -r $a-$b"###);
    }
}

mod corpus_dash_fc_bulk_ds {
    use super::*;

    parity_gap_tests! {
        bulk_ds_fc_row_001 => (r#"bulk ds 001"#, r###"setopt no_listbeep; print -r $?"###);
        bulk_ds_fc_row_002 => (r#"bulk ds 002"#, r###"setopt auto_cd; print -r $?"###);
        bulk_ds_fc_row_003 => (r#"bulk ds 003"#, r###"setopt no_auto_cd; print -r $?"###);
        bulk_ds_fc_row_004 => (r#"bulk ds 004"#, r###"setopt correct; print -r $?"###);
        bulk_ds_fc_row_005 => (r#"bulk ds 005"#, r###"setopt nocorrect; print -r $?"###);
        bulk_ds_fc_row_006 => (r#"bulk ds 006"#, r###"setopt completealiases; print -r $?"###);
        bulk_ds_fc_row_007 => (r#"bulk ds 007"#, r###"setopt globdots; print -r $?"###);
        bulk_ds_fc_row_008 => (r#"bulk ds 008"#, r###"setopt noglobdots; print -r $?"###);
        bulk_ds_fc_row_009 => (r#"bulk ds 009"#, r###"setopt numericglobsort; print -r $?"###);
        bulk_ds_fc_row_010 => (r#"bulk ds 010"#, r###"setopt markdirs; print -r $?"###);
        bulk_ds_fc_row_011 => (r#"bulk ds 011"#, r###"setopt nomarkdirs; print -r $?"###);
        bulk_ds_fc_row_012 => (r#"bulk ds 012"#, r###"setopt chase_links; print -r $?"###);
        bulk_ds_fc_row_013 => (r#"bulk ds 013"#, r###"setopt no_chase_links; print -r $?"###);
        bulk_ds_fc_row_014 => (r#"bulk ds 014"#, r###"setopt pushdignoredups; print -r $?"###);
        bulk_ds_fc_row_015 => (r#"bulk ds 015"#, r###"setopt pushdsilent; print -r $?"###);
        bulk_ds_fc_row_016 => (r#"bulk ds 016"#, r###"setopt pushdtohome; print -r $?"###);
        bulk_ds_fc_row_017 => (r#"bulk ds 017"#, r###"setopt autopushd; print -r $?"###);
        bulk_ds_fc_row_018 => (r#"bulk ds 018"#, r###"setopt pushdminus; print -r $?"###);
        bulk_ds_fc_row_019 => (r#"bulk ds 019"#, r###"setopt pushdignoredups; print -r $?"###);
        bulk_ds_fc_row_020 => (r#"bulk ds 020"#, r###"dirs -p 2>/dev/null | head -1; print -r $?"###);
        bulk_ds_fc_row_021 => (r#"bulk ds 021"#, r###"pushd /tmp 2>/dev/null; popd 2>/dev/null; print -r $?"###);
        bulk_ds_fc_row_022 => (r#"bulk ds 022"#, r###"cd -q / 2>/dev/null; print -r $?"###);
        bulk_ds_fc_row_023 => (r#"bulk ds 023"#, r###"print -r $PWD"###);
        bulk_ds_fc_row_024 => (r#"bulk ds 024"#, r###"print -r ${PWD:h}"###);
        bulk_ds_fc_row_025 => (r#"bulk ds 025"#, r###"print -r ${PWD:t}"###);
        bulk_ds_fc_row_026 => (r#"bulk ds 026"#, r###"print -r ${PWD:r}"###);
        bulk_ds_fc_row_027 => (r#"bulk ds 027"#, r###"print -r ${PWD:e}"###);
        bulk_ds_fc_row_028 => (r#"bulk ds 028"#, r###"print -r ${PWD:a}"###);
        bulk_ds_fc_row_029 => (r#"bulk ds 029"#, r###"print -r ${PWD:A}"###);
        bulk_ds_fc_row_030 => (r#"bulk ds 030"#, r###"read -r line <<< 'one'; print -r $line"###);
        bulk_ds_fc_row_031 => (r#"bulk ds 031"#, r###"read -r a b <<< 'x y'; print -r $a-$b"###);
        bulk_ds_fc_row_032 => (r#"bulk ds 032"#, r###"print -r $'tab\there'"###);
        bulk_ds_fc_row_033 => (r#"bulk ds 033"#, r###"print -r $'line1\nline2'"###);
        bulk_ds_fc_row_034 => (r#"bulk ds 034"#, r###"printf '%q\n' 'a b'"###);
        bulk_ds_fc_row_035 => (r#"bulk ds 035"#, r###"printf '%s\n' ok"###);
        bulk_ds_fc_row_036 => (r#"bulk ds 036"#, r###"print -rn -- end"###);
        bulk_ds_fc_row_037 => (r#"bulk ds 037"#, r###"print -rl -- a b"###);
        bulk_ds_fc_row_038 => (r#"bulk ds 038"#, r###"print -fc '%s\n' hi"###);
        bulk_ds_fc_row_039 => (r#"bulk ds 039"#, r###"whence -w print 2>/dev/null; print -r $?"###);
        bulk_ds_fc_row_040 => (r#"bulk ds 040"#, r###"whence -c print 2>/dev/null; print -r $?"###);
        bulk_ds_fc_row_041 => (r#"bulk ds 041"#, r###"which print 2>/dev/null; print -r $?"###);
        bulk_ds_fc_row_042 => (r#"bulk ds 042"#, r###"command -v print 2>/dev/null; print -r $?"###);
        bulk_ds_fc_row_043 => (r#"bulk ds 043"#, r###"hash -r 2>/dev/null; print -r $?"###);
        bulk_ds_fc_row_044 => (r#"bulk ds 044"#, r###"rehash 2>/dev/null; print -r $?"###);
        bulk_ds_fc_row_045 => (r#"bulk ds 045"#, r###"unalias za 2>/dev/null; alias za=1; unalias za; print -r $?"###);
        bulk_ds_fc_row_046 => (r#"bulk ds 046"#, r###"alias -L za 2>/dev/null; alias za=z; print -r $?"###);
        bulk_ds_fc_row_047 => (r#"bulk ds 047"#, r###"export ZA=1; print -r $ZA"###);
        bulk_ds_fc_row_048 => (r#"bulk ds 048"#, r###"typeset +Z ZA; ZA=1; print -r $ZA"###);
    }
}

mod corpus_dash_fc_bulk_dt {
    use super::*;

    parity_gap_tests! {
        bulk_dt_fc_row_001 => (r#"bulk dt 001"#, r###"setopt pushdminus; print -r $?"###);
        bulk_dt_fc_row_002 => (r#"bulk dt 002"#, r###"setopt pushdignoredups; print -r $?"###);
        bulk_dt_fc_row_003 => (r#"bulk dt 003"#, r###"dirs -p 2>/dev/null | head -1; print -r $?"###);
        bulk_dt_fc_row_004 => (r#"bulk dt 004"#, r###"pushd /tmp 2>/dev/null; popd 2>/dev/null; print -r $?"###);
        bulk_dt_fc_row_005 => (r#"bulk dt 005"#, r###"cd -q / 2>/dev/null; print -r $?"###);
        bulk_dt_fc_row_006 => (r#"bulk dt 006"#, r###"print -r $PWD"###);
        bulk_dt_fc_row_007 => (r#"bulk dt 007"#, r###"print -r ${PWD:h}"###);
        bulk_dt_fc_row_008 => (r#"bulk dt 008"#, r###"print -r ${PWD:t}"###);
        bulk_dt_fc_row_009 => (r#"bulk dt 009"#, r###"print -r ${PWD:r}"###);
        bulk_dt_fc_row_010 => (r#"bulk dt 010"#, r###"print -r ${PWD:e}"###);
        bulk_dt_fc_row_011 => (r#"bulk dt 011"#, r###"print -r ${PWD:a}"###);
        bulk_dt_fc_row_012 => (r#"bulk dt 012"#, r###"print -r ${PWD:A}"###);
        bulk_dt_fc_row_013 => (r#"bulk dt 013"#, r###"read -r line <<< 'one'; print -r $line"###);
        bulk_dt_fc_row_014 => (r#"bulk dt 014"#, r###"read -r a b <<< 'x y'; print -r $a-$b"###);
        bulk_dt_fc_row_015 => (r#"bulk dt 015"#, r###"print -r $'tab\there'"###);
        bulk_dt_fc_row_016 => (r#"bulk dt 016"#, r###"print -r $'line1\nline2'"###);
        bulk_dt_fc_row_017 => (r#"bulk dt 017"#, r###"printf '%q\n' 'a b'"###);
        bulk_dt_fc_row_018 => (r#"bulk dt 018"#, r###"printf '%s\n' ok"###);
        bulk_dt_fc_row_019 => (r#"bulk dt 019"#, r###"print -rn -- end"###);
        bulk_dt_fc_row_020 => (r#"bulk dt 020"#, r###"print -rl -- a b"###);
        bulk_dt_fc_row_021 => (r#"bulk dt 021"#, r###"print -fc '%s\n' hi"###);
        bulk_dt_fc_row_022 => (r#"bulk dt 022"#, r###"whence -w print 2>/dev/null; print -r $?"###);
        bulk_dt_fc_row_023 => (r#"bulk dt 023"#, r###"whence -c print 2>/dev/null; print -r $?"###);
        bulk_dt_fc_row_024 => (r#"bulk dt 024"#, r###"which print 2>/dev/null; print -r $?"###);
        bulk_dt_fc_row_025 => (r#"bulk dt 025"#, r###"command -v print 2>/dev/null; print -r $?"###);
        bulk_dt_fc_row_026 => (r#"bulk dt 026"#, r###"hash -r 2>/dev/null; print -r $?"###);
        bulk_dt_fc_row_027 => (r#"bulk dt 027"#, r###"rehash 2>/dev/null; print -r $?"###);
        bulk_dt_fc_row_028 => (r#"bulk dt 028"#, r###"unalias za 2>/dev/null; alias za=1; unalias za; print -r $?"###);
        bulk_dt_fc_row_029 => (r#"bulk dt 029"#, r###"alias -L za 2>/dev/null; alias za=z; print -r $?"###);
        bulk_dt_fc_row_030 => (r#"bulk dt 030"#, r###"export ZA=1; print -r $ZA"###);
        bulk_dt_fc_row_031 => (r#"bulk dt 031"#, r###"typeset +Z ZA; ZA=1; print -r $ZA"###);
        bulk_dt_fc_row_032 => (r#"bulk dt 032"#, r###"typeset -x ZB=2; print -r $ZB"###);
        bulk_dt_fc_row_033 => (r#"bulk dt 033"#, r###"unset ZC; ZC=1; unset ZC; print -r ${+ZC}"###);
        bulk_dt_fc_row_034 => (r#"bulk dt 034"#, r###"typeset -tH hx; hx=ff; print -r $hx"###);
        bulk_dt_fc_row_035 => (r#"bulk dt 035"#, r###"shift; print -r $1; set -- a b c"###);
        bulk_dt_fc_row_036 => (r#"bulk dt 036"#, r###"(( $# )); print -r $#"###);
        bulk_dt_fc_row_037 => (r#"bulk dt 037"#, r###"print -r ${argv[1]}"###);
        bulk_dt_fc_row_038 => (r#"bulk dt 038"#, r###"print -r ${*[1]}"###);
        bulk_dt_fc_row_039 => (r#"bulk dt 039"#, r###"print -r $@[1]"###);
        bulk_dt_fc_row_040 => (r#"bulk dt 040"#, r###"print -r ${@:2}"###);
        bulk_dt_fc_row_041 => (r#"bulk dt 041"#, r###"select x in a b; do print -r $x; break; done <<< ''"###);
        bulk_dt_fc_row_042 => (r#"bulk dt 042"#, r###"zmodload zsh/zutil 2>/dev/null; print -r $?"###);
        bulk_dt_fc_row_043 => (r#"bulk dt 043"#, r###"zmodload -l 2>/dev/null | head -1; print -r $?"###);
        bulk_dt_fc_row_044 => (r#"bulk dt 044"#, r###"getconf PATH 2>/dev/null | head -c 1; print -r $?"###);
        bulk_dt_fc_row_045 => (r#"bulk dt 045"#, r###"getconf ARG_MAX 2>/dev/null; print -r $?"###);
        bulk_dt_fc_row_046 => (r#"bulk dt 046"#, r###"str=%n; print -r ${(%)str}"###);
        bulk_dt_fc_row_047 => (r#"bulk dt 047"#, r###"str=%N; print -r ${(%)str}"###);
        bulk_dt_fc_row_048 => (r#"bulk dt 048"#, r###"str=%~; print -r ${(%)str}"###);
    }
}

mod corpus_dash_fc_bulk_du {
    use super::*;

    parity_gap_tests! {
        bulk_du_fc_row_001 => (r#"bulk du 001"#, r###"print -r $'line1\nline2'"###);
        bulk_du_fc_row_002 => (r#"bulk du 002"#, r###"printf '%q\n' 'a b'"###);
        bulk_du_fc_row_003 => (r#"bulk du 003"#, r###"printf '%s\n' ok"###);
        bulk_du_fc_row_004 => (r#"bulk du 004"#, r###"print -rn -- end"###);
        bulk_du_fc_row_005 => (r#"bulk du 005"#, r###"print -rl -- a b"###);
        bulk_du_fc_row_006 => (r#"bulk du 006"#, r###"print -fc '%s\n' hi"###);
        bulk_du_fc_row_007 => (r#"bulk du 007"#, r###"whence -w print 2>/dev/null; print -r $?"###);
        bulk_du_fc_row_008 => (r#"bulk du 008"#, r###"whence -c print 2>/dev/null; print -r $?"###);
        bulk_du_fc_row_009 => (r#"bulk du 009"#, r###"which print 2>/dev/null; print -r $?"###);
        bulk_du_fc_row_010 => (r#"bulk du 010"#, r###"command -v print 2>/dev/null; print -r $?"###);
        bulk_du_fc_row_011 => (r#"bulk du 011"#, r###"hash -r 2>/dev/null; print -r $?"###);
        bulk_du_fc_row_012 => (r#"bulk du 012"#, r###"rehash 2>/dev/null; print -r $?"###);
        bulk_du_fc_row_013 => (r#"bulk du 013"#, r###"unalias za 2>/dev/null; alias za=1; unalias za; print -r $?"###);
        bulk_du_fc_row_014 => (r#"bulk du 014"#, r###"alias -L za 2>/dev/null; alias za=z; print -r $?"###);
        bulk_du_fc_row_015 => (r#"bulk du 015"#, r###"export ZA=1; print -r $ZA"###);
        bulk_du_fc_row_016 => (r#"bulk du 016"#, r###"typeset +Z ZA; ZA=1; print -r $ZA"###);
        bulk_du_fc_row_017 => (r#"bulk du 017"#, r###"typeset -x ZB=2; print -r $ZB"###);
        bulk_du_fc_row_018 => (r#"bulk du 018"#, r###"unset ZC; ZC=1; unset ZC; print -r ${+ZC}"###);
        bulk_du_fc_row_019 => (r#"bulk du 019"#, r###"typeset -tH hx; hx=ff; print -r $hx"###);
        bulk_du_fc_row_020 => (r#"bulk du 020"#, r###"shift; print -r $1; set -- a b c"###);
        bulk_du_fc_row_021 => (r#"bulk du 021"#, r###"(( $# )); print -r $#"###);
        bulk_du_fc_row_022 => (r#"bulk du 022"#, r###"print -r ${argv[1]}"###);
        bulk_du_fc_row_023 => (r#"bulk du 023"#, r###"print -r ${*[1]}"###);
        bulk_du_fc_row_024 => (r#"bulk du 024"#, r###"print -r $@[1]"###);
        bulk_du_fc_row_025 => (r#"bulk du 025"#, r###"print -r ${@:2}"###);
        bulk_du_fc_row_026 => (r#"bulk du 026"#, r###"select x in a b; do print -r $x; break; done <<< ''"###);
        bulk_du_fc_row_027 => (r#"bulk du 027"#, r###"zmodload zsh/zutil 2>/dev/null; print -r $?"###);
        bulk_du_fc_row_028 => (r#"bulk du 028"#, r###"zmodload -l 2>/dev/null | head -1; print -r $?"###);
        bulk_du_fc_row_029 => (r#"bulk du 029"#, r###"getconf PATH 2>/dev/null | head -c 1; print -r $?"###);
        bulk_du_fc_row_030 => (r#"bulk du 030"#, r###"getconf ARG_MAX 2>/dev/null; print -r $?"###);
        bulk_du_fc_row_031 => (r#"bulk du 031"#, r###"str=%n; print -r ${(%)str}"###);
        bulk_du_fc_row_032 => (r#"bulk du 032"#, r###"str=%N; print -r ${(%)str}"###);
        bulk_du_fc_row_033 => (r#"bulk du 033"#, r###"str=%~; print -r ${(%)str}"###);
        bulk_du_fc_row_034 => (r#"bulk du 034"#, r###"str=%d; print -r ${(%)str}"###);
        bulk_du_fc_row_035 => (r#"bulk du 035"#, r###"str=%m; print -r ${(%)str}"###);
        bulk_du_fc_row_036 => (r#"bulk du 036"#, r###"str=%#; print -r ${(%)str}"###);
        bulk_du_fc_row_037 => (r#"bulk du 037"#, r###"str=%?; print -r ${(%)str}"###);
        bulk_du_fc_row_038 => (r#"bulk du 038"#, r###"str=%_; print -r ${(%)str}"###);
        bulk_du_fc_row_039 => (r#"bulk du 039"#, r###"str=%h; print -r ${(%)str}"###);
        bulk_du_fc_row_040 => (r#"bulk du 040"#, r###"str=%!; print -r ${(%)str}"###);
        bulk_du_fc_row_041 => (r#"bulk du 041"#, r###"str=%i; print -r ${(%)str}"###);
        bulk_du_fc_row_042 => (r#"bulk du 042"#, r###"str=%I; print -r ${(%)str}"###);
        bulk_du_fc_row_043 => (r#"bulk du 043"#, r###"str=%j; print -r ${(%)str}"###);
        bulk_du_fc_row_044 => (r#"bulk du 044"#, r###"str=%C; print -r ${(%)str}"###);
        bulk_du_fc_row_045 => (r#"bulk du 045"#, r###"str=%c; print -r ${(%)str}"###);
        bulk_du_fc_row_046 => (r#"bulk du 046"#, r###"str=%D; print -r ${(%)str}"###);
        bulk_du_fc_row_047 => (r#"bulk du 047"#, r###"str=%W; print -r ${(%)str}"###);
        bulk_du_fc_row_048 => (r#"bulk du 048"#, r###"str=%*; print -r ${(%)str}"###);
    }
}

mod corpus_dash_fc_bulk_dv {
    use super::*;

    parity_gap_tests! {
        bulk_dv_fc_row_001 => (r#"bulk dv 001"#, r###"unset ZC; ZC=1; unset ZC; print -r ${+ZC}"###);
        bulk_dv_fc_row_002 => (r#"bulk dv 002"#, r###"typeset -tH hx; hx=ff; print -r $hx"###);
        bulk_dv_fc_row_003 => (r#"bulk dv 003"#, r###"shift; print -r $1; set -- a b c"###);
        bulk_dv_fc_row_004 => (r#"bulk dv 004"#, r###"(( $# )); print -r $#"###);
        bulk_dv_fc_row_005 => (r#"bulk dv 005"#, r###"print -r ${argv[1]}"###);
        bulk_dv_fc_row_006 => (r#"bulk dv 006"#, r###"print -r ${*[1]}"###);
        bulk_dv_fc_row_007 => (r#"bulk dv 007"#, r###"print -r $@[1]"###);
        bulk_dv_fc_row_008 => (r#"bulk dv 008"#, r###"print -r ${@:2}"###);
        bulk_dv_fc_row_009 => (r#"bulk dv 009"#, r###"select x in a b; do print -r $x; break; done <<< ''"###);
        bulk_dv_fc_row_010 => (r#"bulk dv 010"#, r###"zmodload zsh/zutil 2>/dev/null; print -r $?"###);
        bulk_dv_fc_row_011 => (r#"bulk dv 011"#, r###"zmodload -l 2>/dev/null | head -1; print -r $?"###);
        bulk_dv_fc_row_012 => (r#"bulk dv 012"#, r###"getconf PATH 2>/dev/null | head -c 1; print -r $?"###);
        bulk_dv_fc_row_013 => (r#"bulk dv 013"#, r###"getconf ARG_MAX 2>/dev/null; print -r $?"###);
        bulk_dv_fc_row_014 => (r#"bulk dv 014"#, r###"str=%n; print -r ${(%)str}"###);
        bulk_dv_fc_row_015 => (r#"bulk dv 015"#, r###"str=%N; print -r ${(%)str}"###);
        bulk_dv_fc_row_016 => (r#"bulk dv 016"#, r###"str=%~; print -r ${(%)str}"###);
        bulk_dv_fc_row_017 => (r#"bulk dv 017"#, r###"str=%d; print -r ${(%)str}"###);
        bulk_dv_fc_row_018 => (r#"bulk dv 018"#, r###"str=%m; print -r ${(%)str}"###);
        bulk_dv_fc_row_019 => (r#"bulk dv 019"#, r###"str=%#; print -r ${(%)str}"###);
        bulk_dv_fc_row_020 => (r#"bulk dv 020"#, r###"str=%?; print -r ${(%)str}"###);
        bulk_dv_fc_row_021 => (r#"bulk dv 021"#, r###"str=%_; print -r ${(%)str}"###);
        bulk_dv_fc_row_022 => (r#"bulk dv 022"#, r###"str=%h; print -r ${(%)str}"###);
        bulk_dv_fc_row_023 => (r#"bulk dv 023"#, r###"str=%!; print -r ${(%)str}"###);
        bulk_dv_fc_row_024 => (r#"bulk dv 024"#, r###"str=%i; print -r ${(%)str}"###);
        bulk_dv_fc_row_025 => (r#"bulk dv 025"#, r###"str=%I; print -r ${(%)str}"###);
        bulk_dv_fc_row_026 => (r#"bulk dv 026"#, r###"str=%j; print -r ${(%)str}"###);
        bulk_dv_fc_row_027 => (r#"bulk dv 027"#, r###"str=%C; print -r ${(%)str}"###);
        bulk_dv_fc_row_028 => (r#"bulk dv 028"#, r###"str=%c; print -r ${(%)str}"###);
        bulk_dv_fc_row_029 => (r#"bulk dv 029"#, r###"str=%D; print -r ${(%)str}"###);
        bulk_dv_fc_row_030 => (r#"bulk dv 030"#, r###"str=%W; print -r ${(%)str}"###);
        bulk_dv_fc_row_031 => (r#"bulk dv 031"#, r###"str=%*; print -r ${(%)str}"###);
        bulk_dv_fc_row_032 => (r#"bulk dv 032"#, r###"str=%v; print -r ${(%)str}"###);
        bulk_dv_fc_row_033 => (r#"bulk dv 033"#, r###"str=%L; print -r ${(%)str}"###);
        bulk_dv_fc_row_034 => (r#"bulk dv 034"#, r###"str=%l; print -r ${(%)str}"###);
        bulk_dv_fc_row_035 => (r#"bulk dv 035"#, r###"str=%y; print -r ${(%)str}"###);
        bulk_dv_fc_row_036 => (r#"bulk dv 036"#, r###"str=%/; print -r ${(%)str}"###);
        bulk_dv_fc_row_037 => (r#"bulk dv 037"#, r###"str=%<; print -r ${(%)str}"###);
        bulk_dv_fc_row_038 => (r#"bulk dv 038"#, r###"str=%>; print -r ${(%)str}"###);
        bulk_dv_fc_row_039 => (r#"bulk dv 039"#, r###"true; print -r $?"###);
        bulk_dv_fc_row_040 => (r#"bulk dv 040"#, r###"false; print -r $?"###);
        bulk_dv_fc_row_041 => (r#"bulk dv 041"#, r###"print -r hello"###);
        bulk_dv_fc_row_042 => (r#"bulk dv 042"#, r###"echo one two"###);
        bulk_dv_fc_row_043 => (r#"bulk dv 043"#, r###"test 1 -eq 1; print -r $?"###);
        bulk_dv_fc_row_044 => (r#"bulk dv 044"#, r###"[ 1 -eq 1 ]; print -r $?"###);
        bulk_dv_fc_row_045 => (r#"bulk dv 045"#, r###"command true; print -r $?"###);
        bulk_dv_fc_row_046 => (r#"bulk dv 046"#, r###"builtin true; print -r $?"###);
        bulk_dv_fc_row_047 => (r#"bulk dv 047"#, r###"if true; then echo t; fi"###);
        bulk_dv_fc_row_048 => (r#"bulk dv 048"#, r###"if false; then echo e; else echo f; fi"###);
    }
}

mod corpus_dash_fc_bulk_dw {
    use super::*;

    parity_gap_tests! {
        bulk_dw_fc_row_001 => (r#"bulk dw 001"#, r###"str=%~; print -r ${(%)str}"###);
        bulk_dw_fc_row_002 => (r#"bulk dw 002"#, r###"str=%d; print -r ${(%)str}"###);
        bulk_dw_fc_row_003 => (r#"bulk dw 003"#, r###"str=%m; print -r ${(%)str}"###);
        bulk_dw_fc_row_004 => (r#"bulk dw 004"#, r###"str=%#; print -r ${(%)str}"###);
        bulk_dw_fc_row_005 => (r#"bulk dw 005"#, r###"str=%?; print -r ${(%)str}"###);
        bulk_dw_fc_row_006 => (r#"bulk dw 006"#, r###"str=%_; print -r ${(%)str}"###);
        bulk_dw_fc_row_007 => (r#"bulk dw 007"#, r###"str=%h; print -r ${(%)str}"###);
        bulk_dw_fc_row_008 => (r#"bulk dw 008"#, r###"str=%!; print -r ${(%)str}"###);
        bulk_dw_fc_row_009 => (r#"bulk dw 009"#, r###"str=%i; print -r ${(%)str}"###);
        bulk_dw_fc_row_010 => (r#"bulk dw 010"#, r###"str=%I; print -r ${(%)str}"###);
        bulk_dw_fc_row_011 => (r#"bulk dw 011"#, r###"str=%j; print -r ${(%)str}"###);
        bulk_dw_fc_row_012 => (r#"bulk dw 012"#, r###"str=%C; print -r ${(%)str}"###);
        bulk_dw_fc_row_013 => (r#"bulk dw 013"#, r###"str=%c; print -r ${(%)str}"###);
        bulk_dw_fc_row_014 => (r#"bulk dw 014"#, r###"str=%D; print -r ${(%)str}"###);
        bulk_dw_fc_row_015 => (r#"bulk dw 015"#, r###"str=%W; print -r ${(%)str}"###);
        bulk_dw_fc_row_016 => (r#"bulk dw 016"#, r###"str=%*; print -r ${(%)str}"###);
        bulk_dw_fc_row_017 => (r#"bulk dw 017"#, r###"str=%v; print -r ${(%)str}"###);
        bulk_dw_fc_row_018 => (r#"bulk dw 018"#, r###"str=%L; print -r ${(%)str}"###);
        bulk_dw_fc_row_019 => (r#"bulk dw 019"#, r###"str=%l; print -r ${(%)str}"###);
        bulk_dw_fc_row_020 => (r#"bulk dw 020"#, r###"str=%y; print -r ${(%)str}"###);
        bulk_dw_fc_row_021 => (r#"bulk dw 021"#, r###"str=%/; print -r ${(%)str}"###);
        bulk_dw_fc_row_022 => (r#"bulk dw 022"#, r###"str=%<; print -r ${(%)str}"###);
        bulk_dw_fc_row_023 => (r#"bulk dw 023"#, r###"str=%>; print -r ${(%)str}"###);
        bulk_dw_fc_row_024 => (r#"bulk dw 024"#, r###"true; print -r $?"###);
        bulk_dw_fc_row_025 => (r#"bulk dw 025"#, r###"false; print -r $?"###);
        bulk_dw_fc_row_026 => (r#"bulk dw 026"#, r###"print -r hello"###);
        bulk_dw_fc_row_027 => (r#"bulk dw 027"#, r###"echo one two"###);
        bulk_dw_fc_row_028 => (r#"bulk dw 028"#, r###"test 1 -eq 1; print -r $?"###);
        bulk_dw_fc_row_029 => (r#"bulk dw 029"#, r###"[ 1 -eq 1 ]; print -r $?"###);
        bulk_dw_fc_row_030 => (r#"bulk dw 030"#, r###"command true; print -r $?"###);
        bulk_dw_fc_row_031 => (r#"bulk dw 031"#, r###"builtin true; print -r $?"###);
        bulk_dw_fc_row_032 => (r#"bulk dw 032"#, r###"if true; then echo t; fi"###);
        bulk_dw_fc_row_033 => (r#"bulk dw 033"#, r###"if false; then echo e; else echo f; fi"###);
        bulk_dw_fc_row_034 => (r#"bulk dw 034"#, r###"for i in a b; do print -r $i; done"###);
        bulk_dw_fc_row_035 => (r#"bulk dw 035"#, r###"i=0; while (( i < 2 )); do print -r $i; (( i++ )); done"###);
        bulk_dw_fc_row_036 => (r#"bulk dw 036"#, r###"repeat 2; do print -r r; done"###);
        bulk_dw_fc_row_037 => (r#"bulk dw 037"#, r###"case x in (x) echo ok ;; esac"###);
        bulk_dw_fc_row_038 => (r#"bulk dw 038"#, r###"[[ 1 -eq 1 ]] && echo and || echo or"###);
        bulk_dw_fc_row_039 => (r#"bulk dw 039"#, r###"[[ 1 -eq 2 ]] || echo orbranch"###);
        bulk_dw_fc_row_040 => (r#"bulk dw 040"#, r###"{ echo a; echo b; }"###);
        bulk_dw_fc_row_041 => (r#"bulk dw 041"#, r###"(echo sub)"###);
        bulk_dw_fc_row_042 => (r#"bulk dw 042"#, r###"(( 1 )) || echo no"###);
        bulk_dw_fc_row_043 => (r#"bulk dw 043"#, r###"(( 0 )) && echo no"###);
        bulk_dw_fc_row_044 => (r#"bulk dw 044"#, r###"print -r $(( 1 + 2 ))"###);
        bulk_dw_fc_row_045 => (r#"bulk dw 045"#, r###"print -r $(( 17 % 5 ))"###);
        bulk_dw_fc_row_046 => (r#"bulk dw 046"#, r###"print -r $(( 2 ** 8 ))"###);
        bulk_dw_fc_row_047 => (r#"bulk dw 047"#, r###"print -r $(( 1 && 0 || 2 ))"###);
        bulk_dw_fc_row_048 => (r#"bulk dw 048"#, r###"print -r $(( !0 ))"###);
    }
}

mod corpus_dash_fc_bulk_dx {
    use super::*;

    parity_gap_tests! {
        bulk_dx_fc_row_001 => (r#"bulk dx 001"#, r###"str=%j; print -r ${(%)str}"###);
        bulk_dx_fc_row_002 => (r#"bulk dx 002"#, r###"str=%C; print -r ${(%)str}"###);
        bulk_dx_fc_row_003 => (r#"bulk dx 003"#, r###"str=%c; print -r ${(%)str}"###);
        bulk_dx_fc_row_004 => (r#"bulk dx 004"#, r###"str=%D; print -r ${(%)str}"###);
        bulk_dx_fc_row_005 => (r#"bulk dx 005"#, r###"str=%W; print -r ${(%)str}"###);
        bulk_dx_fc_row_006 => (r#"bulk dx 006"#, r###"str=%*; print -r ${(%)str}"###);
        bulk_dx_fc_row_007 => (r#"bulk dx 007"#, r###"str=%v; print -r ${(%)str}"###);
        bulk_dx_fc_row_008 => (r#"bulk dx 008"#, r###"str=%L; print -r ${(%)str}"###);
        bulk_dx_fc_row_009 => (r#"bulk dx 009"#, r###"str=%l; print -r ${(%)str}"###);
        bulk_dx_fc_row_010 => (r#"bulk dx 010"#, r###"str=%y; print -r ${(%)str}"###);
        bulk_dx_fc_row_011 => (r#"bulk dx 011"#, r###"str=%/; print -r ${(%)str}"###);
        bulk_dx_fc_row_012 => (r#"bulk dx 012"#, r###"str=%<; print -r ${(%)str}"###);
        bulk_dx_fc_row_013 => (r#"bulk dx 013"#, r###"str=%>; print -r ${(%)str}"###);
        bulk_dx_fc_row_014 => (r#"bulk dx 014"#, r###"true; print -r $?"###);
        bulk_dx_fc_row_015 => (r#"bulk dx 015"#, r###"false; print -r $?"###);
        bulk_dx_fc_row_016 => (r#"bulk dx 016"#, r###"print -r hello"###);
        bulk_dx_fc_row_017 => (r#"bulk dx 017"#, r###"echo one two"###);
        bulk_dx_fc_row_018 => (r#"bulk dx 018"#, r###"test 1 -eq 1; print -r $?"###);
        bulk_dx_fc_row_019 => (r#"bulk dx 019"#, r###"[ 1 -eq 1 ]; print -r $?"###);
        bulk_dx_fc_row_020 => (r#"bulk dx 020"#, r###"command true; print -r $?"###);
        bulk_dx_fc_row_021 => (r#"bulk dx 021"#, r###"builtin true; print -r $?"###);
        bulk_dx_fc_row_022 => (r#"bulk dx 022"#, r###"if true; then echo t; fi"###);
        bulk_dx_fc_row_023 => (r#"bulk dx 023"#, r###"if false; then echo e; else echo f; fi"###);
        bulk_dx_fc_row_024 => (r#"bulk dx 024"#, r###"for i in a b; do print -r $i; done"###);
        bulk_dx_fc_row_025 => (r#"bulk dx 025"#, r###"i=0; while (( i < 2 )); do print -r $i; (( i++ )); done"###);
        bulk_dx_fc_row_026 => (r#"bulk dx 026"#, r###"repeat 2; do print -r r; done"###);
        bulk_dx_fc_row_027 => (r#"bulk dx 027"#, r###"case x in (x) echo ok ;; esac"###);
        bulk_dx_fc_row_028 => (r#"bulk dx 028"#, r###"[[ 1 -eq 1 ]] && echo and || echo or"###);
        bulk_dx_fc_row_029 => (r#"bulk dx 029"#, r###"[[ 1 -eq 2 ]] || echo orbranch"###);
        bulk_dx_fc_row_030 => (r#"bulk dx 030"#, r###"{ echo a; echo b; }"###);
        bulk_dx_fc_row_031 => (r#"bulk dx 031"#, r###"(echo sub)"###);
        bulk_dx_fc_row_032 => (r#"bulk dx 032"#, r###"(( 1 )) || echo no"###);
        bulk_dx_fc_row_033 => (r#"bulk dx 033"#, r###"(( 0 )) && echo no"###);
        bulk_dx_fc_row_034 => (r#"bulk dx 034"#, r###"print -r $(( 1 + 2 ))"###);
        bulk_dx_fc_row_035 => (r#"bulk dx 035"#, r###"print -r $(( 17 % 5 ))"###);
        bulk_dx_fc_row_036 => (r#"bulk dx 036"#, r###"print -r $(( 2 ** 8 ))"###);
        bulk_dx_fc_row_037 => (r#"bulk dx 037"#, r###"print -r $(( 1 && 0 || 2 ))"###);
        bulk_dx_fc_row_038 => (r#"bulk dx 038"#, r###"print -r $(( !0 ))"###);
        bulk_dx_fc_row_039 => (r#"bulk dx 039"#, r###"integer n=5; (( n += 2 )); print -r $n"###);
        bulk_dx_fc_row_040 => (r#"bulk dx 040"#, r###"integer n=5; (( n -= 1 )); print -r $n"###);
        bulk_dx_fc_row_041 => (r#"bulk dx 041"#, r###"integer n=5; (( n *= 2 )); print -r $n"###);
        bulk_dx_fc_row_042 => (r#"bulk dx 042"#, r###"integer n=5; (( n |= 3 )); print -r $n"###);
        bulk_dx_fc_row_043 => (r#"bulk dx 043"#, r###"integer n=5; (( n &= 3 )); print -r $n"###);
        bulk_dx_fc_row_044 => (r#"bulk dx 044"#, r###"print -r $(( true ))"###);
        bulk_dx_fc_row_045 => (r#"bulk dx 045"#, r###"print -r $(( false ))"###);
        bulk_dx_fc_row_046 => (r#"bulk dx 046"#, r###"[[ -e / ]]; print -r $?"###);
        bulk_dx_fc_row_047 => (r#"bulk dx 047"#, r###"[[ -d /tmp ]]; print -r $?"###);
        bulk_dx_fc_row_048 => (r#"bulk dx 048"#, r###"[[ -f /etc/hosts ]]; print -r $?"###);
    }
}

mod corpus_dash_fc_bulk_dy {
    use super::*;

    parity_gap_tests! {
        bulk_dy_fc_row_001 => (r#"bulk dy 001"#, r###"print -r hello"###);
        bulk_dy_fc_row_002 => (r#"bulk dy 002"#, r###"echo one two"###);
        bulk_dy_fc_row_003 => (r#"bulk dy 003"#, r###"test 1 -eq 1; print -r $?"###);
        bulk_dy_fc_row_004 => (r#"bulk dy 004"#, r###"[ 1 -eq 1 ]; print -r $?"###);
        bulk_dy_fc_row_005 => (r#"bulk dy 005"#, r###"command true; print -r $?"###);
        bulk_dy_fc_row_006 => (r#"bulk dy 006"#, r###"builtin true; print -r $?"###);
        bulk_dy_fc_row_007 => (r#"bulk dy 007"#, r###"if true; then echo t; fi"###);
        bulk_dy_fc_row_008 => (r#"bulk dy 008"#, r###"if false; then echo e; else echo f; fi"###);
        bulk_dy_fc_row_009 => (r#"bulk dy 009"#, r###"for i in a b; do print -r $i; done"###);
        bulk_dy_fc_row_010 => (r#"bulk dy 010"#, r###"i=0; while (( i < 2 )); do print -r $i; (( i++ )); done"###);
        bulk_dy_fc_row_011 => (r#"bulk dy 011"#, r###"repeat 2; do print -r r; done"###);
        bulk_dy_fc_row_012 => (r#"bulk dy 012"#, r###"case x in (x) echo ok ;; esac"###);
        bulk_dy_fc_row_013 => (r#"bulk dy 013"#, r###"[[ 1 -eq 1 ]] && echo and || echo or"###);
        bulk_dy_fc_row_014 => (r#"bulk dy 014"#, r###"[[ 1 -eq 2 ]] || echo orbranch"###);
        bulk_dy_fc_row_015 => (r#"bulk dy 015"#, r###"{ echo a; echo b; }"###);
        bulk_dy_fc_row_016 => (r#"bulk dy 016"#, r###"(echo sub)"###);
        bulk_dy_fc_row_017 => (r#"bulk dy 017"#, r###"(( 1 )) || echo no"###);
        bulk_dy_fc_row_018 => (r#"bulk dy 018"#, r###"(( 0 )) && echo no"###);
        bulk_dy_fc_row_019 => (r#"bulk dy 019"#, r###"print -r $(( 1 + 2 ))"###);
        bulk_dy_fc_row_020 => (r#"bulk dy 020"#, r###"print -r $(( 17 % 5 ))"###);
        bulk_dy_fc_row_021 => (r#"bulk dy 021"#, r###"print -r $(( 2 ** 8 ))"###);
        bulk_dy_fc_row_022 => (r#"bulk dy 022"#, r###"print -r $(( 1 && 0 || 2 ))"###);
        bulk_dy_fc_row_023 => (r#"bulk dy 023"#, r###"print -r $(( !0 ))"###);
        bulk_dy_fc_row_024 => (r#"bulk dy 024"#, r###"integer n=5; (( n += 2 )); print -r $n"###);
        bulk_dy_fc_row_025 => (r#"bulk dy 025"#, r###"integer n=5; (( n -= 1 )); print -r $n"###);
        bulk_dy_fc_row_026 => (r#"bulk dy 026"#, r###"integer n=5; (( n *= 2 )); print -r $n"###);
        bulk_dy_fc_row_027 => (r#"bulk dy 027"#, r###"integer n=5; (( n |= 3 )); print -r $n"###);
        bulk_dy_fc_row_028 => (r#"bulk dy 028"#, r###"integer n=5; (( n &= 3 )); print -r $n"###);
        bulk_dy_fc_row_029 => (r#"bulk dy 029"#, r###"print -r $(( true ))"###);
        bulk_dy_fc_row_030 => (r#"bulk dy 030"#, r###"print -r $(( false ))"###);
        bulk_dy_fc_row_031 => (r#"bulk dy 031"#, r###"[[ -e / ]]; print -r $?"###);
        bulk_dy_fc_row_032 => (r#"bulk dy 032"#, r###"[[ -d /tmp ]]; print -r $?"###);
        bulk_dy_fc_row_033 => (r#"bulk dy 033"#, r###"[[ -f /etc/hosts ]]; print -r $?"###);
        bulk_dy_fc_row_034 => (r#"bulk dy 034"#, r###"[[ -r /etc/hosts ]]; print -r $?"###);
        bulk_dy_fc_row_035 => (r#"bulk dy 035"#, r###"[[ -w /tmp ]]; print -r $?"###);
        bulk_dy_fc_row_036 => (r#"bulk dy 036"#, r###"[[ -x /bin/sh ]]; print -r $?"###);
        bulk_dy_fc_row_037 => (r#"bulk dy 037"#, r###"[[ 42 = <-> ]]; print -r $?"###);
        bulk_dy_fc_row_038 => (r#"bulk dy 038"#, r###"[[ abc = <-> ]]; print -r $?"###);
        bulk_dy_fc_row_039 => (r#"bulk dy 039"#, r####"[[ host = ##host ]]; print -r $?"####);
        bulk_dy_fc_row_040 => (r#"bulk dy 040"#, r###"[[ -v x ]]; print -r $?; x=1"###);
        bulk_dy_fc_row_041 => (r#"bulk dy 041"#, r###"unset y; [[ -v y ]]; print -r $?"###);
        bulk_dy_fc_row_042 => (r#"bulk dy 042"#, r###"setopt extendedglob; [[ abc = (#i)ABC ]]; print -r $?"###);
        bulk_dy_fc_row_043 => (r#"bulk dy 043"#, r###"setopt extendedglob; [[ foo = (#b)oo ]]; print -r $?"###);
        bulk_dy_fc_row_044 => (r#"bulk dy 044"#, r###"[[ abc = a* ]]; print -r $?"###);
        bulk_dy_fc_row_045 => (r#"bulk dy 045"#, r###"[[ abc =~ ^a ]]; print -r $?"###);
        bulk_dy_fc_row_046 => (r#"bulk dy 046"#, r###"[[ -z '' ]]; print -r $?"###);
        bulk_dy_fc_row_047 => (r#"bulk dy 047"#, r###"[[ -n abc ]]; print -r $?"###);
        bulk_dy_fc_row_048 => (r#"bulk dy 048"#, r###"typeset -i n=10; print -r $n"###);
    }
}

mod corpus_dash_fc_bulk_dz {
    use super::*;

    parity_gap_tests! {
        bulk_dz_fc_row_001 => (r#"bulk dz 001"#, r###"(( 0 )) && echo no"###);
        bulk_dz_fc_row_002 => (r#"bulk dz 002"#, r###"print -r $(( 1 + 2 ))"###);
        bulk_dz_fc_row_003 => (r#"bulk dz 003"#, r###"print -r $(( 17 % 5 ))"###);
        bulk_dz_fc_row_004 => (r#"bulk dz 004"#, r###"print -r $(( 2 ** 8 ))"###);
        bulk_dz_fc_row_005 => (r#"bulk dz 005"#, r###"print -r $(( 1 && 0 || 2 ))"###);
        bulk_dz_fc_row_006 => (r#"bulk dz 006"#, r###"print -r $(( !0 ))"###);
        bulk_dz_fc_row_007 => (r#"bulk dz 007"#, r###"integer n=5; (( n += 2 )); print -r $n"###);
        bulk_dz_fc_row_008 => (r#"bulk dz 008"#, r###"integer n=5; (( n -= 1 )); print -r $n"###);
        bulk_dz_fc_row_009 => (r#"bulk dz 009"#, r###"integer n=5; (( n *= 2 )); print -r $n"###);
        bulk_dz_fc_row_010 => (r#"bulk dz 010"#, r###"integer n=5; (( n |= 3 )); print -r $n"###);
        bulk_dz_fc_row_011 => (r#"bulk dz 011"#, r###"integer n=5; (( n &= 3 )); print -r $n"###);
        bulk_dz_fc_row_012 => (r#"bulk dz 012"#, r###"print -r $(( true ))"###);
        bulk_dz_fc_row_013 => (r#"bulk dz 013"#, r###"print -r $(( false ))"###);
        bulk_dz_fc_row_014 => (r#"bulk dz 014"#, r###"[[ -e / ]]; print -r $?"###);
        bulk_dz_fc_row_015 => (r#"bulk dz 015"#, r###"[[ -d /tmp ]]; print -r $?"###);
        bulk_dz_fc_row_016 => (r#"bulk dz 016"#, r###"[[ -f /etc/hosts ]]; print -r $?"###);
        bulk_dz_fc_row_017 => (r#"bulk dz 017"#, r###"[[ -r /etc/hosts ]]; print -r $?"###);
        bulk_dz_fc_row_018 => (r#"bulk dz 018"#, r###"[[ -w /tmp ]]; print -r $?"###);
        bulk_dz_fc_row_019 => (r#"bulk dz 019"#, r###"[[ -x /bin/sh ]]; print -r $?"###);
        bulk_dz_fc_row_020 => (r#"bulk dz 020"#, r###"[[ 42 = <-> ]]; print -r $?"###);
        bulk_dz_fc_row_021 => (r#"bulk dz 021"#, r###"[[ abc = <-> ]]; print -r $?"###);
        bulk_dz_fc_row_022 => (r#"bulk dz 022"#, r####"[[ host = ##host ]]; print -r $?"####);
        bulk_dz_fc_row_023 => (r#"bulk dz 023"#, r###"[[ -v x ]]; print -r $?; x=1"###);
        bulk_dz_fc_row_024 => (r#"bulk dz 024"#, r###"unset y; [[ -v y ]]; print -r $?"###);
        bulk_dz_fc_row_025 => (r#"bulk dz 025"#, r###"setopt extendedglob; [[ abc = (#i)ABC ]]; print -r $?"###);
        bulk_dz_fc_row_026 => (r#"bulk dz 026"#, r###"setopt extendedglob; [[ foo = (#b)oo ]]; print -r $?"###);
        bulk_dz_fc_row_027 => (r#"bulk dz 027"#, r###"[[ abc = a* ]]; print -r $?"###);
        bulk_dz_fc_row_028 => (r#"bulk dz 028"#, r###"[[ abc =~ ^a ]]; print -r $?"###);
        bulk_dz_fc_row_029 => (r#"bulk dz 029"#, r###"[[ -z '' ]]; print -r $?"###);
        bulk_dz_fc_row_030 => (r#"bulk dz 030"#, r###"[[ -n abc ]]; print -r $?"###);
        bulk_dz_fc_row_031 => (r#"bulk dz 031"#, r###"typeset -i n=10; print -r $n"###);
        bulk_dz_fc_row_032 => (r#"bulk dz 032"#, r###"typeset -l n=AbC; print -r $n"###);
        bulk_dz_fc_row_033 => (r#"bulk dz 033"#, r###"typeset -u n=xy; print -r $n"###);
        bulk_dz_fc_row_034 => (r#"bulk dz 034"#, r###"typeset -Z5 n=7; print -r $n"###);
        bulk_dz_fc_row_035 => (r#"bulk dz 035"#, r###"typeset -a a=(x y); print -r ${a[2]}"###);
        bulk_dz_fc_row_036 => (r#"bulk dz 036"#, r###"arr=(1 2); arr+=3; print -r ${arr[@]}"###);
        bulk_dz_fc_row_037 => (r#"bulk dz 037"#, r###"arr=(1); arr[1]+=2; print -r ${arr[1]}"###);
        bulk_dz_fc_row_038 => (r#"bulk dz 038"#, r###"unset v; print -r ${v:-def}"###);
        bulk_dz_fc_row_039 => (r#"bulk dz 039"#, r###"v=set; print -r ${v:+yes}"###);
        bulk_dz_fc_row_040 => (r#"bulk dz 040"#, r###"unset v; : ${v::=def}; print -r $v"###);
        bulk_dz_fc_row_041 => (r#"bulk dz 041"#, r###"print -r ${PWD:h}"###);
        bulk_dz_fc_row_042 => (r#"bulk dz 042"#, r###"print -r ${PWD:t}"###);
        bulk_dz_fc_row_043 => (r#"bulk dz 043"#, r###"true | true; print -r $?"###);
        bulk_dz_fc_row_044 => (r#"bulk dz 044"#, r###"true | false; print -r $?"###);
        bulk_dz_fc_row_045 => (r#"bulk dz 045"#, r###"print -r ${pipestatus[1]}; true | false"###);
        bulk_dz_fc_row_046 => (r#"bulk dz 046"#, r###"print -r ${#pipestatus}; true | true | true"###);
        bulk_dz_fc_row_047 => (r#"bulk dz 047"#, r###"set -- a b c; shift; print -r $1"###);
        bulk_dz_fc_row_048 => (r#"bulk dz 048"#, r###"set -- a b c; shift 2; print -r $#"###);
    }
}

mod corpus_dash_fc_bulk_ea {
    use super::*;

    parity_gap_tests! {
        bulk_ea_fc_row_001 => (r#"bulk ea 001"#, r###"[[ -w /tmp ]]; print -r $?"###);
        bulk_ea_fc_row_002 => (r#"bulk ea 002"#, r###"[[ -x /bin/sh ]]; print -r $?"###);
        bulk_ea_fc_row_003 => (r#"bulk ea 003"#, r###"[[ 42 = <-> ]]; print -r $?"###);
        bulk_ea_fc_row_004 => (r#"bulk ea 004"#, r###"[[ abc = <-> ]]; print -r $?"###);
        bulk_ea_fc_row_005 => (r#"bulk ea 005"#, r####"[[ host = ##host ]]; print -r $?"####);
        bulk_ea_fc_row_006 => (r#"bulk ea 006"#, r###"[[ -v x ]]; print -r $?; x=1"###);
        bulk_ea_fc_row_007 => (r#"bulk ea 007"#, r###"unset y; [[ -v y ]]; print -r $?"###);
        bulk_ea_fc_row_008 => (r#"bulk ea 008"#, r###"setopt extendedglob; [[ abc = (#i)ABC ]]; print -r $?"###);
        bulk_ea_fc_row_009 => (r#"bulk ea 009"#, r###"setopt extendedglob; [[ foo = (#b)oo ]]; print -r $?"###);
        bulk_ea_fc_row_010 => (r#"bulk ea 010"#, r###"[[ abc = a* ]]; print -r $?"###);
        bulk_ea_fc_row_011 => (r#"bulk ea 011"#, r###"[[ abc =~ ^a ]]; print -r $?"###);
        bulk_ea_fc_row_012 => (r#"bulk ea 012"#, r###"[[ -z '' ]]; print -r $?"###);
        bulk_ea_fc_row_013 => (r#"bulk ea 013"#, r###"[[ -n abc ]]; print -r $?"###);
        bulk_ea_fc_row_014 => (r#"bulk ea 014"#, r###"typeset -i n=10; print -r $n"###);
        bulk_ea_fc_row_015 => (r#"bulk ea 015"#, r###"typeset -l n=AbC; print -r $n"###);
        bulk_ea_fc_row_016 => (r#"bulk ea 016"#, r###"typeset -u n=xy; print -r $n"###);
        bulk_ea_fc_row_017 => (r#"bulk ea 017"#, r###"typeset -Z5 n=7; print -r $n"###);
        bulk_ea_fc_row_018 => (r#"bulk ea 018"#, r###"typeset -a a=(x y); print -r ${a[2]}"###);
        bulk_ea_fc_row_019 => (r#"bulk ea 019"#, r###"arr=(1 2); arr+=3; print -r ${arr[@]}"###);
        bulk_ea_fc_row_020 => (r#"bulk ea 020"#, r###"arr=(1); arr[1]+=2; print -r ${arr[1]}"###);
        bulk_ea_fc_row_021 => (r#"bulk ea 021"#, r###"unset v; print -r ${v:-def}"###);
        bulk_ea_fc_row_022 => (r#"bulk ea 022"#, r###"v=set; print -r ${v:+yes}"###);
        bulk_ea_fc_row_023 => (r#"bulk ea 023"#, r###"unset v; : ${v::=def}; print -r $v"###);
        bulk_ea_fc_row_024 => (r#"bulk ea 024"#, r###"print -r ${PWD:h}"###);
        bulk_ea_fc_row_025 => (r#"bulk ea 025"#, r###"print -r ${PWD:t}"###);
        bulk_ea_fc_row_026 => (r#"bulk ea 026"#, r###"true | true; print -r $?"###);
        bulk_ea_fc_row_027 => (r#"bulk ea 027"#, r###"true | false; print -r $?"###);
        bulk_ea_fc_row_028 => (r#"bulk ea 028"#, r###"print -r ${pipestatus[1]}; true | false"###);
        bulk_ea_fc_row_029 => (r#"bulk ea 029"#, r###"print -r ${#pipestatus}; true | true | true"###);
        bulk_ea_fc_row_030 => (r#"bulk ea 030"#, r###"set -- a b c; shift; print -r $1"###);
        bulk_ea_fc_row_031 => (r#"bulk ea 031"#, r###"set -- a b c; shift 2; print -r $#"###);
        bulk_ea_fc_row_032 => (r#"bulk ea 032"#, r###"fn(){ print -r $1; }; fn x"###);
        bulk_ea_fc_row_033 => (r#"bulk ea 033"#, r###"fn(){ local x=2; print -r $x; }; fn"###);
        bulk_ea_fc_row_034 => (r#"bulk ea 034"#, r###"fn(){ return 2; }; fn; print -r $?"###);
        bulk_ea_fc_row_035 => (r#"bulk ea 035"#, r###"print -r ${(q)x}; x=hi"###);
        bulk_ea_fc_row_036 => (r#"bulk ea 036"#, r###"print -r ${(qq)x}; x=hi"###);
        bulk_ea_fc_row_037 => (r#"bulk ea 037"#, r###"x=hi; print -r ${(q-)x}"###);
        bulk_ea_fc_row_038 => (r#"bulk ea 038"#, r###"x=hi; print -r ${(q+)x}"###);
        bulk_ea_fc_row_039 => (r#"bulk ea 039"#, r###"print -r ${(w)w}; w=a b c"###);
        bulk_ea_fc_row_040 => (r#"bulk ea 040"#, r###"print -r ${(u)a}; a=(a a b)"###);
        bulk_ea_fc_row_041 => (r#"bulk ea 041"#, r###"print -r ${(o)a}; a=(c b a)"###);
        bulk_ea_fc_row_042 => (r#"bulk ea 042"#, r###"print -r ${(j:,:)a}; a=(x y)"###);
        bulk_ea_fc_row_043 => (r#"bulk ea 043"#, r###"arr=(a b c); print -r ${arr[(I)b]}"###);
        bulk_ea_fc_row_044 => (r#"bulk ea 044"#, r###"arr=(a b c); print -r ${arr[(R)b]}"###);
        bulk_ea_fc_row_045 => (r#"bulk ea 045"#, r###"arr=(9 8 7); print -r ${arr[-2,-1]}"###);
        bulk_ea_fc_row_046 => (r#"bulk ea 046"#, r###"typeset -A h; h=(k v); print -r ${(k)h}"###);
        bulk_ea_fc_row_047 => (r#"bulk ea 047"#, r###"typeset -A h; h=(a 1 b 2); print -r ${(kv)h}"###);
        bulk_ea_fc_row_048 => (r#"bulk ea 048"#, r###"print -r ${+options}"###);
    }
}

mod corpus_dash_fc_bulk_eb {
    use super::*;

    parity_gap_tests! {
        bulk_eb_fc_row_001 => (r#"bulk eb 001"#, r###"typeset -a a=(x y); print -r ${a[2]}"###);
        bulk_eb_fc_row_002 => (r#"bulk eb 002"#, r###"arr=(1 2); arr+=3; print -r ${arr[@]}"###);
        bulk_eb_fc_row_003 => (r#"bulk eb 003"#, r###"arr=(1); arr[1]+=2; print -r ${arr[1]}"###);
        bulk_eb_fc_row_004 => (r#"bulk eb 004"#, r###"unset v; print -r ${v:-def}"###);
        bulk_eb_fc_row_005 => (r#"bulk eb 005"#, r###"v=set; print -r ${v:+yes}"###);
        bulk_eb_fc_row_006 => (r#"bulk eb 006"#, r###"unset v; : ${v::=def}; print -r $v"###);
        bulk_eb_fc_row_007 => (r#"bulk eb 007"#, r###"print -r ${PWD:h}"###);
        bulk_eb_fc_row_008 => (r#"bulk eb 008"#, r###"print -r ${PWD:t}"###);
        bulk_eb_fc_row_009 => (r#"bulk eb 009"#, r###"true | true; print -r $?"###);
        bulk_eb_fc_row_010 => (r#"bulk eb 010"#, r###"true | false; print -r $?"###);
        bulk_eb_fc_row_011 => (r#"bulk eb 011"#, r###"print -r ${pipestatus[1]}; true | false"###);
        bulk_eb_fc_row_012 => (r#"bulk eb 012"#, r###"print -r ${#pipestatus}; true | true | true"###);
        bulk_eb_fc_row_013 => (r#"bulk eb 013"#, r###"set -- a b c; shift; print -r $1"###);
        bulk_eb_fc_row_014 => (r#"bulk eb 014"#, r###"set -- a b c; shift 2; print -r $#"###);
        bulk_eb_fc_row_015 => (r#"bulk eb 015"#, r###"fn(){ print -r $1; }; fn x"###);
        bulk_eb_fc_row_016 => (r#"bulk eb 016"#, r###"fn(){ local x=2; print -r $x; }; fn"###);
        bulk_eb_fc_row_017 => (r#"bulk eb 017"#, r###"fn(){ return 2; }; fn; print -r $?"###);
        bulk_eb_fc_row_018 => (r#"bulk eb 018"#, r###"print -r ${(q)x}; x=hi"###);
        bulk_eb_fc_row_019 => (r#"bulk eb 019"#, r###"print -r ${(qq)x}; x=hi"###);
        bulk_eb_fc_row_020 => (r#"bulk eb 020"#, r###"x=hi; print -r ${(q-)x}"###);
        bulk_eb_fc_row_021 => (r#"bulk eb 021"#, r###"x=hi; print -r ${(q+)x}"###);
        bulk_eb_fc_row_022 => (r#"bulk eb 022"#, r###"print -r ${(w)w}; w=a b c"###);
        bulk_eb_fc_row_023 => (r#"bulk eb 023"#, r###"print -r ${(u)a}; a=(a a b)"###);
        bulk_eb_fc_row_024 => (r#"bulk eb 024"#, r###"print -r ${(o)a}; a=(c b a)"###);
        bulk_eb_fc_row_025 => (r#"bulk eb 025"#, r###"print -r ${(j:,:)a}; a=(x y)"###);
        bulk_eb_fc_row_026 => (r#"bulk eb 026"#, r###"arr=(a b c); print -r ${arr[(I)b]}"###);
        bulk_eb_fc_row_027 => (r#"bulk eb 027"#, r###"arr=(a b c); print -r ${arr[(R)b]}"###);
        bulk_eb_fc_row_028 => (r#"bulk eb 028"#, r###"arr=(9 8 7); print -r ${arr[-2,-1]}"###);
        bulk_eb_fc_row_029 => (r#"bulk eb 029"#, r###"typeset -A h; h=(k v); print -r ${(k)h}"###);
        bulk_eb_fc_row_030 => (r#"bulk eb 030"#, r###"typeset -A h; h=(a 1 b 2); print -r ${(kv)h}"###);
        bulk_eb_fc_row_031 => (r#"bulk eb 031"#, r###"print -r ${+options}"###);
        bulk_eb_fc_row_032 => (r#"bulk eb 032"#, r###"print -r ${+parameters}"###);
        bulk_eb_fc_row_033 => (r#"bulk eb 033"#, r###"print -r ${+aliases}"###);
        bulk_eb_fc_row_034 => (r#"bulk eb 034"#, r###"print -r ${+functions}"###);
        bulk_eb_fc_row_035 => (r#"bulk eb 035"#, r###"print -r $ZSH_NAME"###);
        bulk_eb_fc_row_036 => (r#"bulk eb 036"#, r###"print -r ${ZSH_VERSION%%.*}"###);
        bulk_eb_fc_row_037 => (r#"bulk eb 037"#, r###"whence -w print"###);
        bulk_eb_fc_row_038 => (r#"bulk eb 038"#, r###"command -v true"###);
        bulk_eb_fc_row_039 => (r#"bulk eb 039"#, r###"emulate -L zsh; print -r $?"###);
        bulk_eb_fc_row_040 => (r#"bulk eb 040"#, r###"alias za='echo z'; za; unalias za 2>/dev/null"###);
        bulk_eb_fc_row_041 => (r#"bulk eb 041"#, r###"read -r line <<< 'one two'; print -r $line"###);
        bulk_eb_fc_row_042 => (r#"bulk eb 042"#, r###"cat <<< 'herestring'"###);
        bulk_eb_fc_row_043 => (r#"bulk eb 043"#, r###"echo hello 2>/dev/null"###);
        bulk_eb_fc_row_044 => (r#"bulk eb 044"#, r###"printf '%s\n' a b c | head -1"###);
        bulk_eb_fc_row_045 => (r#"bulk eb 045"#, r###"true && echo yes"###);
        bulk_eb_fc_row_046 => (r#"bulk eb 046"#, r###"false || echo yes"###);
        bulk_eb_fc_row_047 => (r#"bulk eb 047"#, r###"(exit 3); print -r $?"###);
        bulk_eb_fc_row_048 => (r#"bulk eb 048"#, r###"print -r ${status}; (exit 4)"###);
    }
}

mod corpus_dash_fc_bulk_ec {
    use super::*;

    parity_gap_tests! {
        bulk_ec_fc_row_001 => (r#"bulk ec 001"#, r###"print -r ${(q)x}; x=hi"###);
        bulk_ec_fc_row_002 => (r#"bulk ec 002"#, r###"print -r ${(qq)x}; x=hi"###);
        bulk_ec_fc_row_003 => (r#"bulk ec 003"#, r###"x=hi; print -r ${(q-)x}"###);
        bulk_ec_fc_row_004 => (r#"bulk ec 004"#, r###"x=hi; print -r ${(q+)x}"###);
        bulk_ec_fc_row_005 => (r#"bulk ec 005"#, r###"print -r ${(w)w}; w=a b c"###);
        bulk_ec_fc_row_006 => (r#"bulk ec 006"#, r###"print -r ${(u)a}; a=(a a b)"###);
        bulk_ec_fc_row_007 => (r#"bulk ec 007"#, r###"print -r ${(o)a}; a=(c b a)"###);
        bulk_ec_fc_row_008 => (r#"bulk ec 008"#, r###"print -r ${(j:,:)a}; a=(x y)"###);
        bulk_ec_fc_row_009 => (r#"bulk ec 009"#, r###"arr=(a b c); print -r ${arr[(I)b]}"###);
        bulk_ec_fc_row_010 => (r#"bulk ec 010"#, r###"arr=(a b c); print -r ${arr[(R)b]}"###);
        bulk_ec_fc_row_011 => (r#"bulk ec 011"#, r###"arr=(9 8 7); print -r ${arr[-2,-1]}"###);
        bulk_ec_fc_row_012 => (r#"bulk ec 012"#, r###"typeset -A h; h=(k v); print -r ${(k)h}"###);
        bulk_ec_fc_row_013 => (r#"bulk ec 013"#, r###"typeset -A h; h=(a 1 b 2); print -r ${(kv)h}"###);
        bulk_ec_fc_row_014 => (r#"bulk ec 014"#, r###"print -r ${+options}"###);
        bulk_ec_fc_row_015 => (r#"bulk ec 015"#, r###"print -r ${+parameters}"###);
        bulk_ec_fc_row_016 => (r#"bulk ec 016"#, r###"print -r ${+aliases}"###);
        bulk_ec_fc_row_017 => (r#"bulk ec 017"#, r###"print -r ${+functions}"###);
        bulk_ec_fc_row_018 => (r#"bulk ec 018"#, r###"print -r $ZSH_NAME"###);
        bulk_ec_fc_row_019 => (r#"bulk ec 019"#, r###"print -r ${ZSH_VERSION%%.*}"###);
        bulk_ec_fc_row_020 => (r#"bulk ec 020"#, r###"whence -w print"###);
        bulk_ec_fc_row_021 => (r#"bulk ec 021"#, r###"command -v true"###);
        bulk_ec_fc_row_022 => (r#"bulk ec 022"#, r###"emulate -L zsh; print -r $?"###);
        bulk_ec_fc_row_023 => (r#"bulk ec 023"#, r###"alias za='echo z'; za; unalias za 2>/dev/null"###);
        bulk_ec_fc_row_024 => (r#"bulk ec 024"#, r###"read -r line <<< 'one two'; print -r $line"###);
        bulk_ec_fc_row_025 => (r#"bulk ec 025"#, r###"cat <<< 'herestring'"###);
        bulk_ec_fc_row_026 => (r#"bulk ec 026"#, r###"echo hello 2>/dev/null"###);
        bulk_ec_fc_row_027 => (r#"bulk ec 027"#, r###"printf '%s\n' a b c | head -1"###);
        bulk_ec_fc_row_028 => (r#"bulk ec 028"#, r###"true && echo yes"###);
        bulk_ec_fc_row_029 => (r#"bulk ec 029"#, r###"false || echo yes"###);
        bulk_ec_fc_row_030 => (r#"bulk ec 030"#, r###"(exit 3); print -r $?"###);
        bulk_ec_fc_row_031 => (r#"bulk ec 031"#, r###"print -r ${status}; (exit 4)"###);
        bulk_ec_fc_row_032 => (r#"bulk ec 032"#, r###"print -r $(( 1_000 + 1 ))"###);
        bulk_ec_fc_row_033 => (r#"bulk ec 033"#, r###"print -r $(( 5#101 ))"###);
        bulk_ec_fc_row_034 => (r#"bulk ec 034"#, r###"print -r $(( 0b1111 ))"###);
        bulk_ec_fc_row_035 => (r#"bulk ec 035"#, r###"print -r $(( 2 ** 3 ** 2 ))"###);
        bulk_ec_fc_row_036 => (r#"bulk ec 036"#, r###"float f=1.5; print -r $(( f * 2 ))"###);
        bulk_ec_fc_row_037 => (r#"bulk ec 037"#, r###"typeset -F2 f=3.14; print -r $f"###);
        bulk_ec_fc_row_038 => (r#"bulk ec 038"#, r###"[[ /etc/hosts -nt /tmp ]]; print -r $?"###);
        bulk_ec_fc_row_039 => (r#"bulk ec 039"#, r###"[[ /tmp -ot /etc/hosts ]]; print -r $?"###);
        bulk_ec_fc_row_040 => (r#"bulk ec 040"#, r###"[[ /etc/hosts -ef /etc/hosts ]]; print -r $?"###);
        bulk_ec_fc_row_041 => (r#"bulk ec 041"#, r####"setopt extendedglob; [[ abc = [a-z]## ]]; print -r $?"####);
        bulk_ec_fc_row_042 => (r#"bulk ec 042"#, r###"print -r ${(L)${(U)m}}; m=aBc"###);
        bulk_ec_fc_row_043 => (r#"bulk ec 043"#, r###"print -r ${#x}; x=hello"###);
        bulk_ec_fc_row_044 => (r#"bulk ec 044"#, r###"print -r ${#a}; a=(a b c)"###);
        bulk_ec_fc_row_045 => (r#"bulk ec 045"#, r###"print -r ${(c)#a}; a=(ab cd)"###);
        bulk_ec_fc_row_046 => (r#"bulk ec 046"#, r###"print -r ${(b)x}; x=hi"###);
        bulk_ec_fc_row_047 => (r#"bulk ec 047"#, r###"print -r ${(e):-2+2}"###);
        bulk_ec_fc_row_048 => (r#"bulk ec 048"#, r###"print -r ${(P)r}; r=HOME"###);
    }
}

mod corpus_dash_fc_bulk_ed {
    use super::*;

    parity_gap_tests! {
        bulk_ed_fc_row_001 => (r#"bulk ed 001"#, r###"print -r ${+aliases}"###);
        bulk_ed_fc_row_002 => (r#"bulk ed 002"#, r###"print -r ${+functions}"###);
        bulk_ed_fc_row_003 => (r#"bulk ed 003"#, r###"print -r $ZSH_NAME"###);
        bulk_ed_fc_row_004 => (r#"bulk ed 004"#, r###"print -r ${ZSH_VERSION%%.*}"###);
        bulk_ed_fc_row_005 => (r#"bulk ed 005"#, r###"whence -w print"###);
        bulk_ed_fc_row_006 => (r#"bulk ed 006"#, r###"command -v true"###);
        bulk_ed_fc_row_007 => (r#"bulk ed 007"#, r###"emulate -L zsh; print -r $?"###);
        bulk_ed_fc_row_008 => (r#"bulk ed 008"#, r###"alias za='echo z'; za; unalias za 2>/dev/null"###);
        bulk_ed_fc_row_009 => (r#"bulk ed 009"#, r###"read -r line <<< 'one two'; print -r $line"###);
        bulk_ed_fc_row_010 => (r#"bulk ed 010"#, r###"cat <<< 'herestring'"###);
        bulk_ed_fc_row_011 => (r#"bulk ed 011"#, r###"echo hello 2>/dev/null"###);
        bulk_ed_fc_row_012 => (r#"bulk ed 012"#, r###"printf '%s\n' a b c | head -1"###);
        bulk_ed_fc_row_013 => (r#"bulk ed 013"#, r###"true && echo yes"###);
        bulk_ed_fc_row_014 => (r#"bulk ed 014"#, r###"false || echo yes"###);
        bulk_ed_fc_row_015 => (r#"bulk ed 015"#, r###"(exit 3); print -r $?"###);
        bulk_ed_fc_row_016 => (r#"bulk ed 016"#, r###"print -r ${status}; (exit 4)"###);
        bulk_ed_fc_row_017 => (r#"bulk ed 017"#, r###"print -r $(( 1_000 + 1 ))"###);
        bulk_ed_fc_row_018 => (r#"bulk ed 018"#, r###"print -r $(( 5#101 ))"###);
        bulk_ed_fc_row_019 => (r#"bulk ed 019"#, r###"print -r $(( 0b1111 ))"###);
        bulk_ed_fc_row_020 => (r#"bulk ed 020"#, r###"print -r $(( 2 ** 3 ** 2 ))"###);
        bulk_ed_fc_row_021 => (r#"bulk ed 021"#, r###"float f=1.5; print -r $(( f * 2 ))"###);
        bulk_ed_fc_row_022 => (r#"bulk ed 022"#, r###"typeset -F2 f=3.14; print -r $f"###);
        bulk_ed_fc_row_023 => (r#"bulk ed 023"#, r###"[[ /etc/hosts -nt /tmp ]]; print -r $?"###);
        bulk_ed_fc_row_024 => (r#"bulk ed 024"#, r###"[[ /tmp -ot /etc/hosts ]]; print -r $?"###);
        bulk_ed_fc_row_025 => (r#"bulk ed 025"#, r###"[[ /etc/hosts -ef /etc/hosts ]]; print -r $?"###);
        bulk_ed_fc_row_026 => (r#"bulk ed 026"#, r####"setopt extendedglob; [[ abc = [a-z]## ]]; print -r $?"####);
        bulk_ed_fc_row_027 => (r#"bulk ed 027"#, r###"print -r ${(L)${(U)m}}; m=aBc"###);
        bulk_ed_fc_row_028 => (r#"bulk ed 028"#, r###"print -r ${#x}; x=hello"###);
        bulk_ed_fc_row_029 => (r#"bulk ed 029"#, r###"print -r ${#a}; a=(a b c)"###);
        bulk_ed_fc_row_030 => (r#"bulk ed 030"#, r###"print -r ${(c)#a}; a=(ab cd)"###);
        bulk_ed_fc_row_031 => (r#"bulk ed 031"#, r###"print -r ${(b)x}; x=hi"###);
        bulk_ed_fc_row_032 => (r#"bulk ed 032"#, r###"print -r ${(e):-2+2}"###);
        bulk_ed_fc_row_033 => (r#"bulk ed 033"#, r###"print -r ${(P)r}; r=HOME"###);
        bulk_ed_fc_row_034 => (r#"bulk ed 034"#, r###"print -r ${(on)n}; n=(10 2 1)"###);
        bulk_ed_fc_row_035 => (r#"bulk ed 035"#, r###"print -r ${(eu)n}; n=(a A b)"###);
        bulk_ed_fc_row_036 => (r#"bulk ed 036"#, r###"typeset -aU u=(a a b); print -r ${#u}"###);
        bulk_ed_fc_row_037 => (r#"bulk ed 037"#, r###"typeset -h hv=1; print -r ${+hv}"###);
        bulk_ed_fc_row_038 => (r#"bulk ed 038"#, r###"x=a1a2; p=a; print -r ${x//p/r}"###);
        bulk_ed_fc_row_039 => (r#"bulk ed 039"#, r###"for i in 1 2 3; do (( i == 2 )) && continue; print -r $i; done"###);
        bulk_ed_fc_row_040 => (r#"bulk ed 040"#, r###"while :; do break; print -r n; done; print -r after"###);
        bulk_ed_fc_row_041 => (r#"bulk ed 041"#, r###"case w in (a|b) echo ab ;; *) echo star ;; esac"###);
        bulk_ed_fc_row_042 => (r#"bulk ed 042"#, r###"if [[ -n '' ]]; then echo y; else echo n; fi"###);
        bulk_ed_fc_row_043 => (r#"bulk ed 043"#, r###"print -r ${argv[1]}; set -- p q"###);
        bulk_ed_fc_row_044 => (r#"bulk ed 044"#, r###"print -r $ARGC; set -- a b"###);
        bulk_ed_fc_row_045 => (r#"bulk ed 045"#, r###"print -r ${dirstack[1]:-empty}"###);
        bulk_ed_fc_row_046 => (r#"bulk ed 046"#, r###"print -r ${+pipestatus}"###);
        bulk_ed_fc_row_047 => (r#"bulk ed 047"#, r###"print -r ${+history}"###);
        bulk_ed_fc_row_048 => (r#"bulk ed 048"#, r###"print -r ${+commands}"###);
    }
}

mod corpus_dash_fc_bulk_ee {
    use super::*;

    parity_gap_tests! {
        bulk_ee_fc_row_001 => (r#"bulk ee 001"#, r###"print -r $(( 5#101 ))"###);
        bulk_ee_fc_row_002 => (r#"bulk ee 002"#, r###"print -r $(( 0b1111 ))"###);
        bulk_ee_fc_row_003 => (r#"bulk ee 003"#, r###"print -r $(( 2 ** 3 ** 2 ))"###);
        bulk_ee_fc_row_004 => (r#"bulk ee 004"#, r###"float f=1.5; print -r $(( f * 2 ))"###);
        bulk_ee_fc_row_005 => (r#"bulk ee 005"#, r###"typeset -F2 f=3.14; print -r $f"###);
        bulk_ee_fc_row_006 => (r#"bulk ee 006"#, r###"[[ /etc/hosts -nt /tmp ]]; print -r $?"###);
        bulk_ee_fc_row_007 => (r#"bulk ee 007"#, r###"[[ /tmp -ot /etc/hosts ]]; print -r $?"###);
        bulk_ee_fc_row_008 => (r#"bulk ee 008"#, r###"[[ /etc/hosts -ef /etc/hosts ]]; print -r $?"###);
        bulk_ee_fc_row_009 => (r#"bulk ee 009"#, r####"setopt extendedglob; [[ abc = [a-z]## ]]; print -r $?"####);
        bulk_ee_fc_row_010 => (r#"bulk ee 010"#, r###"print -r ${(L)${(U)m}}; m=aBc"###);
        bulk_ee_fc_row_011 => (r#"bulk ee 011"#, r###"print -r ${#x}; x=hello"###);
        bulk_ee_fc_row_012 => (r#"bulk ee 012"#, r###"print -r ${#a}; a=(a b c)"###);
        bulk_ee_fc_row_013 => (r#"bulk ee 013"#, r###"print -r ${(c)#a}; a=(ab cd)"###);
        bulk_ee_fc_row_014 => (r#"bulk ee 014"#, r###"print -r ${(b)x}; x=hi"###);
        bulk_ee_fc_row_015 => (r#"bulk ee 015"#, r###"print -r ${(e):-2+2}"###);
        bulk_ee_fc_row_016 => (r#"bulk ee 016"#, r###"print -r ${(P)r}; r=HOME"###);
        bulk_ee_fc_row_017 => (r#"bulk ee 017"#, r###"print -r ${(on)n}; n=(10 2 1)"###);
        bulk_ee_fc_row_018 => (r#"bulk ee 018"#, r###"print -r ${(eu)n}; n=(a A b)"###);
        bulk_ee_fc_row_019 => (r#"bulk ee 019"#, r###"typeset -aU u=(a a b); print -r ${#u}"###);
        bulk_ee_fc_row_020 => (r#"bulk ee 020"#, r###"typeset -h hv=1; print -r ${+hv}"###);
        bulk_ee_fc_row_021 => (r#"bulk ee 021"#, r###"x=a1a2; p=a; print -r ${x//p/r}"###);
        bulk_ee_fc_row_022 => (r#"bulk ee 022"#, r###"for i in 1 2 3; do (( i == 2 )) && continue; print -r $i; done"###);
        bulk_ee_fc_row_023 => (r#"bulk ee 023"#, r###"while :; do break; print -r n; done; print -r after"###);
        bulk_ee_fc_row_024 => (r#"bulk ee 024"#, r###"case w in (a|b) echo ab ;; *) echo star ;; esac"###);
        bulk_ee_fc_row_025 => (r#"bulk ee 025"#, r###"if [[ -n '' ]]; then echo y; else echo n; fi"###);
        bulk_ee_fc_row_026 => (r#"bulk ee 026"#, r###"print -r ${argv[1]}; set -- p q"###);
        bulk_ee_fc_row_027 => (r#"bulk ee 027"#, r###"print -r $ARGC; set -- a b"###);
        bulk_ee_fc_row_028 => (r#"bulk ee 028"#, r###"print -r ${dirstack[1]:-empty}"###);
        bulk_ee_fc_row_029 => (r#"bulk ee 029"#, r###"print -r ${+pipestatus}"###);
        bulk_ee_fc_row_030 => (r#"bulk ee 030"#, r###"print -r ${+history}"###);
        bulk_ee_fc_row_031 => (r#"bulk ee 031"#, r###"print -r ${+commands}"###);
        bulk_ee_fc_row_032 => (r#"bulk ee 032"#, r###"print -r ${+builtins}"###);
        bulk_ee_fc_row_033 => (r#"bulk ee 033"#, r###"print -r ${+widgets}"###);
        bulk_ee_fc_row_034 => (r#"bulk ee 034"#, r###"print -r ${+terminfo}"###);
        bulk_ee_fc_row_035 => (r#"bulk ee 035"#, r###"print -r ${+modules}"###);
        bulk_ee_fc_row_036 => (r#"bulk ee 036"#, r###"print -r ${+patchars}"###);
        bulk_ee_fc_row_037 => (r#"bulk ee 037"#, r###"print -r ${+reswords}"###);
        bulk_ee_fc_row_038 => (r#"bulk ee 038"#, r###"print -r ${+dis_aliases}"###);
        bulk_ee_fc_row_039 => (r#"bulk ee 039"#, r###"print -r ${+dis_functions}"###);
        bulk_ee_fc_row_040 => (r#"bulk ee 040"#, r###"print -r ${+parameters[(I)PATH]}"###);
        bulk_ee_fc_row_041 => (r#"bulk ee 041"#, r###"arr=(a b c d); print -r ${arr[2,3]}"###);
        bulk_ee_fc_row_042 => (r#"bulk ee 042"#, r###"arr=(1 2 3); print -r ${arr[1,-1]}"###);
        bulk_ee_fc_row_043 => (r#"bulk ee 043"#, r###"s=barfooxyz; print -r ${s[(i)foo]}"###);
        bulk_ee_fc_row_044 => (r#"bulk ee 044"#, r###"typeset -A h; h=(k v); print -r ${h[(R)v]}"###);
        bulk_ee_fc_row_045 => (r#"bulk ee 045"#, r###"typeset -A h; h=(a 1 b 2); print -r ${h[(r)2]}"###);
        bulk_ee_fc_row_046 => (r#"bulk ee 046"#, r###"print -r $(( 9 & 6 ^ 3 ))"###);
        bulk_ee_fc_row_047 => (r#"bulk ee 047"#, r###"print -r $(( 128 >> 2 ))"###);
        bulk_ee_fc_row_048 => (r#"bulk ee 048"#, r###"print -r $(( ~(255) & 0xff ))"###);
    }
}

mod corpus_dash_fc_bulk_ef {
    use super::*;

    parity_gap_tests! {
        bulk_ef_fc_row_001 => (r#"bulk ef 001"#, r###"print -r ${(eu)n}; n=(a A b)"###);
        bulk_ef_fc_row_002 => (r#"bulk ef 002"#, r###"typeset -aU u=(a a b); print -r ${#u}"###);
        bulk_ef_fc_row_003 => (r#"bulk ef 003"#, r###"typeset -h hv=1; print -r ${+hv}"###);
        bulk_ef_fc_row_004 => (r#"bulk ef 004"#, r###"x=a1a2; p=a; print -r ${x//p/r}"###);
        bulk_ef_fc_row_005 => (r#"bulk ef 005"#, r###"for i in 1 2 3; do (( i == 2 )) && continue; print -r $i; done"###);
        bulk_ef_fc_row_006 => (r#"bulk ef 006"#, r###"while :; do break; print -r n; done; print -r after"###);
        bulk_ef_fc_row_007 => (r#"bulk ef 007"#, r###"case w in (a|b) echo ab ;; *) echo star ;; esac"###);
        bulk_ef_fc_row_008 => (r#"bulk ef 008"#, r###"if [[ -n '' ]]; then echo y; else echo n; fi"###);
        bulk_ef_fc_row_009 => (r#"bulk ef 009"#, r###"print -r ${argv[1]}; set -- p q"###);
        bulk_ef_fc_row_010 => (r#"bulk ef 010"#, r###"print -r $ARGC; set -- a b"###);
        bulk_ef_fc_row_011 => (r#"bulk ef 011"#, r###"print -r ${dirstack[1]:-empty}"###);
        bulk_ef_fc_row_012 => (r#"bulk ef 012"#, r###"print -r ${+pipestatus}"###);
        bulk_ef_fc_row_013 => (r#"bulk ef 013"#, r###"print -r ${+history}"###);
        bulk_ef_fc_row_014 => (r#"bulk ef 014"#, r###"print -r ${+commands}"###);
        bulk_ef_fc_row_015 => (r#"bulk ef 015"#, r###"print -r ${+builtins}"###);
        bulk_ef_fc_row_016 => (r#"bulk ef 016"#, r###"print -r ${+widgets}"###);
        bulk_ef_fc_row_017 => (r#"bulk ef 017"#, r###"print -r ${+terminfo}"###);
        bulk_ef_fc_row_018 => (r#"bulk ef 018"#, r###"print -r ${+modules}"###);
        bulk_ef_fc_row_019 => (r#"bulk ef 019"#, r###"print -r ${+patchars}"###);
        bulk_ef_fc_row_020 => (r#"bulk ef 020"#, r###"print -r ${+reswords}"###);
        bulk_ef_fc_row_021 => (r#"bulk ef 021"#, r###"print -r ${+dis_aliases}"###);
        bulk_ef_fc_row_022 => (r#"bulk ef 022"#, r###"print -r ${+dis_functions}"###);
        bulk_ef_fc_row_023 => (r#"bulk ef 023"#, r###"print -r ${+parameters[(I)PATH]}"###);
        bulk_ef_fc_row_024 => (r#"bulk ef 024"#, r###"arr=(a b c d); print -r ${arr[2,3]}"###);
        bulk_ef_fc_row_025 => (r#"bulk ef 025"#, r###"arr=(1 2 3); print -r ${arr[1,-1]}"###);
        bulk_ef_fc_row_026 => (r#"bulk ef 026"#, r###"s=barfooxyz; print -r ${s[(i)foo]}"###);
        bulk_ef_fc_row_027 => (r#"bulk ef 027"#, r###"typeset -A h; h=(k v); print -r ${h[(R)v]}"###);
        bulk_ef_fc_row_028 => (r#"bulk ef 028"#, r###"typeset -A h; h=(a 1 b 2); print -r ${h[(r)2]}"###);
        bulk_ef_fc_row_029 => (r#"bulk ef 029"#, r###"print -r $(( 9 & 6 ^ 3 ))"###);
        bulk_ef_fc_row_030 => (r#"bulk ef 030"#, r###"print -r $(( 128 >> 2 ))"###);
        bulk_ef_fc_row_031 => (r#"bulk ef 031"#, r###"print -r $(( ~(255) & 0xff ))"###);
        bulk_ef_fc_row_032 => (r#"bulk ef 032"#, r###"print -r $(( 3 <|> 5 ))"###);
        bulk_ef_fc_row_033 => (r#"bulk ef 033"#, r###"print -r $(( 3 <> 5 ))"###);
        bulk_ef_fc_row_034 => (r#"bulk ef 034"#, r###"(( 5#11 )); print -r $?"###);
        bulk_ef_fc_row_035 => (r#"bulk ef 035"#, r###"integer n=5; (( n ^= 3 )); print -r $n"###);
        bulk_ef_fc_row_036 => (r#"bulk ef 036"#, r###"integer n=5; (( n <<= 1 )); print -r $n"###);
        bulk_ef_fc_row_037 => (r#"bulk ef 037"#, r###"integer n=5; (( n >>= 1 )); print -r $n"###);
        bulk_ef_fc_row_038 => (r#"bulk ef 038"#, r###"integer n=5; (( n /= 2 )); print -r $n"###);
        bulk_ef_fc_row_039 => (r#"bulk ef 039"#, r###"integer n=5; (( n %= 3 )); print -r $n"###);
        bulk_ef_fc_row_040 => (r#"bulk ef 040"#, r###"typeset -i8 n=10; print -r $n"###);
        bulk_ef_fc_row_041 => (r#"bulk ef 041"#, r###"typeset -i16 n=255; print -r $n"###);
        bulk_ef_fc_row_042 => (r#"bulk ef 042"#, r###"typeset -E2 n=4000; print -r $n"###);
        bulk_ef_fc_row_043 => (r#"bulk ef 043"#, r###"typeset -R4 n=hi; print -r $n"###);
        bulk_ef_fc_row_044 => (r#"bulk ef 044"#, r###"typeset +L n=Ab; print -r $n"###);
        bulk_ef_fc_row_045 => (r#"bulk ef 045"#, r###"typeset +U n=xy; print -r $n"###);
        bulk_ef_fc_row_046 => (r#"bulk ef 046"#, r###"typeset +i n=4; print -r $n"###);
        bulk_ef_fc_row_047 => (r#"bulk ef 047"#, r###"export EX=1; print -r $EX; unset EX"###);
        bulk_ef_fc_row_048 => (r#"bulk ef 048"#, r###"readonly ro=5; print -r $ro"###);
    }
}

mod corpus_dash_fc_bulk_eg {
    use super::*;

    parity_gap_tests! {
        bulk_eg_fc_row_001 => (r#"bulk eg 001"#, r###"print -r ${+terminfo}"###);
        bulk_eg_fc_row_002 => (r#"bulk eg 002"#, r###"print -r ${+modules}"###);
        bulk_eg_fc_row_003 => (r#"bulk eg 003"#, r###"print -r ${+patchars}"###);
        bulk_eg_fc_row_004 => (r#"bulk eg 004"#, r###"print -r ${+reswords}"###);
        bulk_eg_fc_row_005 => (r#"bulk eg 005"#, r###"print -r ${+dis_aliases}"###);
        bulk_eg_fc_row_006 => (r#"bulk eg 006"#, r###"print -r ${+dis_functions}"###);
        bulk_eg_fc_row_007 => (r#"bulk eg 007"#, r###"print -r ${+parameters[(I)PATH]}"###);
        bulk_eg_fc_row_008 => (r#"bulk eg 008"#, r###"arr=(a b c d); print -r ${arr[2,3]}"###);
        bulk_eg_fc_row_009 => (r#"bulk eg 009"#, r###"arr=(1 2 3); print -r ${arr[1,-1]}"###);
        bulk_eg_fc_row_010 => (r#"bulk eg 010"#, r###"s=barfooxyz; print -r ${s[(i)foo]}"###);
        bulk_eg_fc_row_011 => (r#"bulk eg 011"#, r###"typeset -A h; h=(k v); print -r ${h[(R)v]}"###);
        bulk_eg_fc_row_012 => (r#"bulk eg 012"#, r###"typeset -A h; h=(a 1 b 2); print -r ${h[(r)2]}"###);
        bulk_eg_fc_row_013 => (r#"bulk eg 013"#, r###"print -r $(( 9 & 6 ^ 3 ))"###);
        bulk_eg_fc_row_014 => (r#"bulk eg 014"#, r###"print -r $(( 128 >> 2 ))"###);
        bulk_eg_fc_row_015 => (r#"bulk eg 015"#, r###"print -r $(( ~(255) & 0xff ))"###);
        bulk_eg_fc_row_016 => (r#"bulk eg 016"#, r###"print -r $(( 3 <|> 5 ))"###);
        bulk_eg_fc_row_017 => (r#"bulk eg 017"#, r###"print -r $(( 3 <> 5 ))"###);
        bulk_eg_fc_row_018 => (r#"bulk eg 018"#, r###"(( 5#11 )); print -r $?"###);
        bulk_eg_fc_row_019 => (r#"bulk eg 019"#, r###"integer n=5; (( n ^= 3 )); print -r $n"###);
        bulk_eg_fc_row_020 => (r#"bulk eg 020"#, r###"integer n=5; (( n <<= 1 )); print -r $n"###);
        bulk_eg_fc_row_021 => (r#"bulk eg 021"#, r###"integer n=5; (( n >>= 1 )); print -r $n"###);
        bulk_eg_fc_row_022 => (r#"bulk eg 022"#, r###"integer n=5; (( n /= 2 )); print -r $n"###);
        bulk_eg_fc_row_023 => (r#"bulk eg 023"#, r###"integer n=5; (( n %= 3 )); print -r $n"###);
        bulk_eg_fc_row_024 => (r#"bulk eg 024"#, r###"typeset -i8 n=10; print -r $n"###);
        bulk_eg_fc_row_025 => (r#"bulk eg 025"#, r###"typeset -i16 n=255; print -r $n"###);
        bulk_eg_fc_row_026 => (r#"bulk eg 026"#, r###"typeset -E2 n=4000; print -r $n"###);
        bulk_eg_fc_row_027 => (r#"bulk eg 027"#, r###"typeset -R4 n=hi; print -r $n"###);
        bulk_eg_fc_row_028 => (r#"bulk eg 028"#, r###"typeset +L n=Ab; print -r $n"###);
        bulk_eg_fc_row_029 => (r#"bulk eg 029"#, r###"typeset +U n=xy; print -r $n"###);
        bulk_eg_fc_row_030 => (r#"bulk eg 030"#, r###"typeset +i n=4; print -r $n"###);
        bulk_eg_fc_row_031 => (r#"bulk eg 031"#, r###"export EX=1; print -r $EX; unset EX"###);
        bulk_eg_fc_row_032 => (r#"bulk eg 032"#, r###"readonly ro=5; print -r $ro"###);
        bulk_eg_fc_row_033 => (r#"bulk eg 033"#, r###"print -r ${${v:-fb}}; unset v"###);
        bulk_eg_fc_row_034 => (r#"bulk eg 034"#, r###"print -r ${${v:+set}:-unset}; unset v"###);
        bulk_eg_fc_row_035 => (r#"bulk eg 035"#, r###"word=$'l1\nl2'; print -r ${(@f)word}"###);
        bulk_eg_fc_row_036 => (r#"bulk eg 036"#, r###"word=  hi  ; print -r ${(W)word}"###);
        bulk_eg_fc_row_037 => (r#"bulk eg 037"#, r###"print -r ${(z)word}; word=a b c"###);
        bulk_eg_fc_row_038 => (r#"bulk eg 038"#, r###"print -r ${(F)x}; x=$'p\nq'"###);
        bulk_eg_fc_row_039 => (r#"bulk eg 039"#, r###"print -r ${(A)x}; x=1 2"###);
        bulk_eg_fc_row_040 => (r#"bulk eg 040"#, r###"print -r ${(aa)x}; x=(1 2)"###);
        bulk_eg_fc_row_041 => (r#"bulk eg 041"#, r###"print -r ${(%)2}"###);
        bulk_eg_fc_row_042 => (r#"bulk eg 042"#, r###"o=8; print -r ${(0)o}"###);
        bulk_eg_fc_row_043 => (r#"bulk eg 043"#, r###"str=abc.def; print -r ${str:r}"###);
        bulk_eg_fc_row_044 => (r#"bulk eg 044"#, r###"str=abc.def; print -r ${str:e}"###);
        bulk_eg_fc_row_045 => (r#"bulk eg 045"#, r###"[[ -h /dev/stdin ]]; print -r $?"###);
        bulk_eg_fc_row_046 => (r#"bulk eg 046"#, r###"[[ -p /dev/fd/0 ]]; print -r $?"###);
        bulk_eg_fc_row_047 => (r#"bulk eg 047"#, r###"[[ -O /etc/hosts ]]; print -r $?"###);
        bulk_eg_fc_row_048 => (r#"bulk eg 048"#, r###"[[ -G / ]]; print -r $?"###);
    }
}

