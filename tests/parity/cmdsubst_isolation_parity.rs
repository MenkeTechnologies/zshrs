//! `$( … )` isolation matrix: state a command substitution sets must not
//! reach the parent shell.
//!
//! In C every command substitution is a fork — `getoutput`
//! (`Src/exec.c:4816` `zfork`) runs the body in a child that
//! `_realexit()`s, so whatever the body did to the shell's tables, its
//! environment, its working directory or its process attributes dies
//! with it. zshrs runs `$( … )` in process and has to hand each of those
//! back by hand; one case per thing the body can touch.
//!
//! Every case sets state inside `$( … )` (or a backtick / `( … )` / `<( … )`
//! spelling) and reads it back in the parent. The expected text is the
//! oracle's own answer (`zsh -f`, 5.9.2), pinned here so the test also
//! runs where no zsh is installed; where one is, the pin is re-checked
//! against it. Variable names are unique (`zq…`) and removed from the
//! inherited environment, so an exported variable of the same name in
//! the test runner cannot manufacture a divergence. Several cases read
//! the environment through `/usr/bin/env` rather than `typeset -p`: an
//! export that leaks is only visible to a CHILD of the parent.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

fn zsh_path() -> Option<&'static str> {
    ["/opt/homebrew/bin/zsh", "/usr/local/bin/zsh", "/bin/zsh", "/usr/bin/zsh"]
        .into_iter()
        .find(|p| Path::new(p).exists())
}

/// Run `code` under `bin args… code`, with every `zq…` name the case uses
/// scrubbed from the environment. Stdout and exit status only: the two
/// shells' error-message prefixes differ by design. A shell still running
/// after 20s is killed and reported as `<timeout>`: a descriptor leaked
/// out of a substitution can keep its capture pipe open forever.
fn run(bin: &Path, args: &[&str], code: &str) -> (String, i32) {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .arg(code)
        .current_dir(std::env::temp_dir())
        .env_remove("ZSHRS_CACHE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for word in code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
        if word.starts_with("zq") || word.starts_with("ZQT") {
            cmd.env_remove(word);
        }
    }
    let mut child = cmd.spawn().expect("spawn shell");
    let mut out = child.stdout.take().expect("stdout pipe");
    let reader = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut s);
        s
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let status = loop {
        if let Some(st) = child.try_wait().expect("wait") {
            break Some(st);
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let stdout = String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned();
    match status {
        Some(st) => (stdout, st.code().unwrap_or(-1)),
        None => (format!("{stdout}<timeout>"), -1),
    }
}

fn check(code: &str, want_out: &str, want_status: i32) {
    if let Some(zsh) = zsh_path() {
        let got = run(Path::new(zsh), &["-f", "-c"], code);
        assert_eq!(
            (got.0.as_str(), got.1),
            (want_out, want_status),
            "the pinned answer no longer matches the oracle ({zsh}) for:\n{code}"
        );
    }
    let got = run(&zshrs_bin(), &["--zsh", "-f", "-c"], code);
    assert_eq!(
        (got.0.as_str(), got.1),
        (want_out, want_status),
        "state leaked out of the substitution (or was not inherited) in:\n{code}"
    );
}

#[test]
fn scalar_new() {
    check(
        r#"x=$(zqa=1); print -r -- "[$zqa] ${+zqa}""#,
        "[] 0\n",
        0,
    );
}

#[test]
fn scalar_assign() {
    check(
        r#"zqa=0; x=$(zqa=1); print $zqa"#,
        "0\n",
        0,
    );
}

#[test]
fn scalar_unset() {
    check(
        r#"zqa=0; x=$(unset zqa); print ${+zqa} $zqa"#,
        "1 0\n",
        0,
    );
}

#[test]
fn array_new() {
    check(
        r#"x=$(zqarr=(a b)); print ${#zqarr} ${+zqarr}"#,
        "0 0\n",
        0,
    );
}

#[test]
fn array_append() {
    check(
        r#"zqarr=(a b); x=$(zqarr+=(c)); print $zqarr"#,
        "a b\n",
        0,
    );
}

#[test]
fn array_elem_assign() {
    check(
        r#"zqarr=(a b); x=$(zqarr[1]=z); print $zqarr"#,
        "a b\n",
        0,
    );
}

#[test]
fn array_elem_delete() {
    check(
        r#"zqarr=(a b c); x=$(zqarr[2]=()); print $zqarr"#,
        "a b c\n",
        0,
    );
}

#[test]
fn array_unique() {
    check(
        r#"zqarr=(a a); x=$(typeset -U zqarr); zqarr+=(a); print $zqarr"#,
        "a a a\n",
        0,
    );
}

#[test]
fn assoc_elem_assign() {
    check(
        r#"typeset -A zqh; zqh=(a 1); x=$(zqh[b]=2); print ${(okv)zqh}"#,
        "1 a\n",
        0,
    );
}

#[test]
fn assoc_elem_unset() {
    check(
        r#"typeset -A zqh; zqh=(a 1 b 2); x=$(unset 'zqh[a]'); print ${(ok)zqh}"#,
        "a b\n",
        0,
    );
}

#[test]
fn assoc_new() {
    check(
        r#"x=$(typeset -A zqh2; zqh2=(a 1)); print ${+zqh2} ${(t)zqh2}"#,
        "0\n",
        0,
    );
}

#[test]
fn assoc_unset_whole() {
    check(
        r#"typeset -A zqh; zqh=(a 1); x=$(unset zqh); print ${(t)zqh} ${(kv)zqh}"#,
        "association a 1\n",
        0,
    );
}

#[test]
fn assoc_retype_scalar() {
    check(
        r#"typeset -A zqh; zqh=(a 1); x=$(unset zqh; zqh=s); print ${(t)zqh} ${(kv)zqh}"#,
        "association a 1\n",
        0,
    );
}

#[test]
fn export_new() {
    check(
        r#"x=$(export zqe=1); print ${+zqe}; /usr/bin/env | grep -c '^zqe='"#,
        "0\n0\n",
        1,
    );
}

#[test]
fn export_then_assign() {
    check(
        r#"x=$(export zqe2; zqe2=5); print ${+zqe2}; /usr/bin/env | grep -c '^zqe2='"#,
        "0\n0\n",
        1,
    );
}

#[test]
fn export_existing_plain() {
    check(
        r#"zqe3=a; x=$(export zqe3); /usr/bin/env | grep -c '^zqe3='; typeset -p zqe3"#,
        "0\ntypeset zqe3=a\n",
        0,
    );
}

#[test]
fn export_typeset_xi() {
    check(
        r#"x=$(typeset -x -i zqe4=3); print ${+zqe4}; /usr/bin/env | grep -c '^zqe4='"#,
        "0\n0\n",
        1,
    );
}

#[test]
fn export_exported_reassign() {
    check(
        r#"export zqe5=a; x=$(zqe5=b); print $zqe5; /usr/bin/env | grep '^zqe5='"#,
        "a\nzqe5=a\n",
        0,
    );
}

#[test]
fn export_exported_unset() {
    check(
        r#"export zqe6=a; x=$(unset zqe6); print $zqe6; /usr/bin/env | grep '^zqe6='"#,
        "a\nzqe6=a\n",
        0,
    );
}

#[test]
fn export_exported_unexport() {
    check(
        r#"export zqe7=a; x=$(typeset +x zqe7); typeset -p zqe7; /usr/bin/env | grep '^zqe7='"#,
        "export zqe7=a\nzqe7=a\n",
        0,
    );
}

#[test]
fn export_exported_reexport() {
    check(
        r#"export zqe8=a; x=$(export zqe8=b); /usr/bin/env | grep '^zqe8='"#,
        "zqe8=a\n",
        0,
    );
}

#[test]
fn export_seen_by_later_cmdsubst_env() {
    check(
        r#"x=$(export zqe10=1); print $(/usr/bin/env | grep -c '^zqe10=')"#,
        "0\n",
        0,
    );
}

#[test]
fn export_type_flag() {
    check(
        r#"x=$(export zqe11=1); print ${(t)zqe11}"#,
        "\n",
        0,
    );
}

#[test]
fn export_seen_inside() {
    check(
        r#"x=$(export zqe12=1; /usr/bin/env | grep -c '^zqe12='); print $x"#,
        "1\n",
        0,
    );
}

#[test]
fn export_parent_seen_inside() {
    check(
        r#"export zqe13=1; x=$(/usr/bin/env | grep -c '^zqe13='); print $x"#,
        "1\n",
        0,
    );
}

#[test]
fn export_path_env() {
    check(
        r#"x=$(PATH=/zqnonexistent); /usr/bin/env | grep -c '^PATH=/zqnonexistent$'"#,
        "0\n",
        1,
    );
}

#[test]
fn export_home_unset_env() {
    check(
        r#"x=$(unset HOME); /usr/bin/env | grep -c '^HOME='"#,
        "1\n",
        0,
    );
}

#[test]
fn export_pwd_env() {
    check(
        r#"cd /; x=$(cd /tmp); /usr/bin/env | grep '^PWD='"#,
        "PWD=/\n",
        0,
    );
}

#[test]
fn export_prefix_assign_env() {
    check(
        r#"x=$(zqe14=1 true); /usr/bin/env | grep -c '^zqe14='"#,
        "0\n",
        1,
    );
}

#[test]
fn export_in_function_local() {
    check(
        r#"f() { local -x zqe15=1; x=$(export zqe15=2); /usr/bin/env | grep '^zqe15=' }; f"#,
        "zqe15=1\n",
        0,
    );
}

#[test]
fn allexport() {
    check(
        r#"x=$(setopt allexport; zqe9=1); print ${+zqe9}; [[ -o allexport ]] && print on || print off; /usr/bin/env | grep -c '^zqe9='"#,
        "0\noff\n0\n",
        1,
    );
}

#[test]
fn readonly_new() {
    check(
        r#"x=$(readonly zqr=1); zqr=2; print $zqr"#,
        "2\n",
        0,
    );
}

#[test]
fn readonly_existing() {
    check(
        r#"zqr2=1; x=$(readonly zqr2); zqr2=3; print $zqr2"#,
        "3\n",
        0,
    );
}

#[test]
fn integer_new() {
    check(
        r#"x=$(integer zqi=3); zqi=1+1; print $zqi ${(t)zqi}"#,
        "1+1 scalar\n",
        0,
    );
}

#[test]
fn integer_existing() {
    check(
        r#"integer zqi2=1; x=$(zqi2=5); print $zqi2"#,
        "1\n",
        0,
    );
}

#[test]
fn float_new() {
    check(
        r#"x=$(float zqfl=1.5); print ${+zqfl}"#,
        "0\n",
        0,
    );
}

#[test]
fn typeset_g_top() {
    check(
        r#"x=$(typeset -g zqg2=1); print ${+zqg2}"#,
        "0\n",
        0,
    );
}

#[test]
fn typeset_g_from_function() {
    check(
        r#"f() { typeset -g zqg=1 }; x=$(f); print ${+zqg}"#,
        "0\n",
        0,
    );
}

#[test]
fn typeset_tied() {
    check(
        r#"x=$(typeset -T ZQT zqt; zqt=(a b)); print ${+ZQT} ${+zqt}"#,
        "0 0\n",
        0,
    );
}

#[test]
fn ifs_value() {
    check(
        r#"x=$(IFS=:); print -r -- "${(q)IFS}""#,
        "\\ $'\\t'$'\\n'$'\\0'\n",
        0,
    );
}

#[test]
fn ifs_split_result() {
    check(
        r#"print $(IFS=:; zqs=(a b c); print "$zqs[*]")"#,
        "a:b:c\n",
        0,
    );
}

#[test]
fn seconds() {
    check(
        r#"x=$(SECONDS=100); (( SECONDS < 50 )) && print ok"#,
        "ok\n",
        0,
    );
}

#[test]
fn cwd() {
    check(
        r#"cd /; x=$(cd /tmp); pwd; print $PWD"#,
        "/\n/\n",
        0,
    );
}

#[test]
fn cwd_dirstack() {
    check(
        r#"cd /; x=$(pushd /tmp >/dev/null); dirs"#,
        "/\n",
        0,
    );
}

#[test]
fn func_define() {
    check(
        r#"x=$(zqf() { :; }); print ${+functions[zqf]}"#,
        "0\n",
        0,
    );
}

#[test]
fn func_unfunction() {
    check(
        r#"zqf() { print a }; x=$(unfunction zqf); zqf"#,
        "a\n",
        0,
    );
}

#[test]
fn func_multi_unfunction() {
    check(
        r#"zqf1() { :; }; zqf2() { :; }; x=$(unfunction zqf1 zqf2); print ${+functions[zqf1]} ${+functions[zqf2]}"#,
        "1 1\n",
        0,
    );
}

#[test]
fn func_redefine_then_unfunction() {
    check(
        r#"zqf() { print a }; x=$(zqf() { print b }; unfunction zqf); zqf"#,
        "a\n",
        0,
    );
}

#[test]
fn func_redefine() {
    check(
        r#"zqf() { print a }; x=$(zqf() { print b }); zqf"#,
        "a\n",
        0,
    );
}

#[test]
fn func_keys_order_stable() {
    check(
        r#"zqf1() { :; }; zqf2() { :; }; zqf3() { :; }; a=${(k)functions}; x=$(unfunction zqf2; zqf4() { :; }); b=${(k)functions}; [[ $a == $b ]] && print same || print differ"#,
        "same\n",
        0,
    );
}

#[test]
fn autoload_new() {
    check(
        r#"x=$(autoload -Uz zqal); print ${+functions[zqal]}"#,
        "0\n",
        0,
    );
}

#[test]
fn autoload_whence() {
    check(
        r#"x=$(autoload -Uz zqal2); whence -w zqal2"#,
        "zqal2: none\n",
        1,
    );
}

#[test]
fn disable_builtin() {
    check(
        r#"x=$(disable echo); echo hi"#,
        "hi\n",
        0,
    );
}

#[test]
fn disable_alias() {
    check(
        r#"alias zqa3=print; x=$(disable -a zqa3); alias zqa3"#,
        "zqa3=print\n",
        0,
    );
}

#[test]
fn disable_function() {
    check(
        r#"zqf() { print a }; x=$(disable -f zqf); zqf"#,
        "a\n",
        0,
    );
}

#[test]
fn disable_reswd() {
    check(
        r#"x=$(disable -r repeat); eval 'repeat 2 print r'"#,
        "r\nr\n",
        0,
    );
}

#[test]
fn in_function_local() {
    check(
        r#"f() { local zql=1; x=$(zql=2); print $zql }; f"#,
        "1\n",
        0,
    );
}

#[test]
fn in_function_alias_and_var() {
    check(
        r#"f() { x=$(zqv=1; alias zqfa=print); print ${+zqv}; alias zqfa; print $? }; f"#,
        "0\n1\n",
        0,
    );
}

#[test]
fn alias_define() {
    check(
        r#"x=$(alias zqq=echo); alias zqq; print $?"#,
        "1\n",
        0,
    );
}

#[test]
fn alias_unalias() {
    check(
        r#"alias zqq=echo; x=$(unalias zqq); alias zqq"#,
        "zqq=echo\n",
        0,
    );
}

#[test]
fn alias_redefine() {
    check(
        r#"alias zqq=a; x=$(alias zqq=b); alias zqq"#,
        "zqq=a\n",
        0,
    );
}

#[test]
fn alias_expands_later() {
    check(
        r#"x=$(alias zqq=echo); eval 'zqq hi' 2>/dev/null; print $?"#,
        "127\n",
        0,
    );
}

#[test]
fn alias_unalias_pattern() {
    check(
        r#"alias zqq=1; x=$(unalias -m 'zq*'); alias zqq"#,
        "zqq=1\n",
        0,
    );
}

#[test]
fn alias_global_define() {
    check(
        r#"x=$(alias -g zqG=foo); alias -g zqG; print $?"#,
        "1\n",
        0,
    );
}

#[test]
fn alias_global_unalias() {
    check(
        r#"alias -g zqG=x; x=$(unalias zqG); alias -g zqG"#,
        "zqG=x\n",
        0,
    );
}

#[test]
fn alias_suffix_define() {
    check(
        r#"x=$(alias -s zqs=less); alias -s zqs; print $?"#,
        "1\n",
        0,
    );
}

#[test]
fn alias_suffix_unalias() {
    check(
        r#"alias -s zqs=less; x=$(unalias -s zqs); alias -s zqs"#,
        "zqs=less\n",
        0,
    );
}

#[test]
fn alias_via_eval() {
    check(
        r#"x=$(eval 'alias zqev=a'); alias zqev; print $?"#,
        "1\n",
        0,
    );
}

#[test]
fn alias_print_arg() {
    check(
        r#"print -- $(alias zqp=a); alias zqp; print $?"#,
        "\n1\n",
        0,
    );
}

#[test]
fn alias_quoted_subst() {
    check(
        r#"print -- "$(alias zqpq=a)"; alias zqpq; print $?"#,
        "\n1\n",
        0,
    );
}

#[test]
fn alias_two_substs_one_command() {
    check(
        r#"x=$(alias zqq2=a) y=$(alias zqq2); print "[$y]""#,
        "[]\n",
        0,
    );
}

#[test]
fn alias_aliases_param() {
    check(
        r#"x=$(alias zqap=a); print ${+aliases[zqap]}"#,
        "0\n",
        0,
    );
}

#[test]
fn alias_aliases_param_assign() {
    check(
        r#"x=$(aliases[zqapa]=b); alias zqapa; print $?"#,
        "1\n",
        0,
    );
}

#[test]
fn opt_set() {
    check(
        r#"x=$(setopt extendedglob); [[ -o extendedglob ]] && print on || print off"#,
        "off\n",
        0,
    );
}

#[test]
fn opt_unset() {
    check(
        r#"setopt extendedglob; x=$(unsetopt extendedglob); [[ -o extendedglob ]] && print on || print off"#,
        "on\n",
        0,
    );
}

#[test]
fn opt_errexit() {
    check(
        r#"x=$(set -e); [[ -o errexit ]] && print on || print off"#,
        "off\n",
        0,
    );
}

#[test]
fn emulate_sh() {
    check(
        r#"x=$(emulate sh); [[ -o shwordsplit ]] && print on || print off"#,
        "off\n",
        0,
    );
}

#[test]
fn emulate_ksh() {
    check(
        r#"x=$(emulate ksh); [[ -o ksharrays ]] && print on || print off; zqa=(a b); print $zqa[1]"#,
        "off\na\n",
        0,
    );
}

#[test]
fn trap_set() {
    check(
        r#"x=$(trap 'print t' USR1); trap"#,
        "",
        0,
    );
}

#[test]
fn trap_remove() {
    check(
        r#"trap 'print p' USR1; x=$(trap - USR1); trap"#,
        "trap -- 'print p' USR1\n",
        0,
    );
}

#[test]
fn trap_exit_fires_inside() {
    check(
        r#"x=$(trap 'print ex' EXIT); print "[$x]""#,
        "[ex]\n",
        0,
    );
}

#[test]
fn trap_function_form() {
    check(
        r#"x=$(TRAPUSR2() { :; }); print ${+functions[TRAPUSR2]}"#,
        "0\n",
        0,
    );
}

#[test]
fn nameddir_define() {
    check(
        r#"x=$(hash -d zqnd=/tmp); hash -d | grep -c zqnd"#,
        "0\n",
        1,
    );
}

#[test]
fn nameddir_unhash() {
    check(
        r#"hash -d zqnd=/tmp; x=$(unhash -d zqnd); hash -d | grep -c zqnd"#,
        "1\n",
        0,
    );
}

#[test]
fn nameddir_redefine() {
    check(
        r#"hash -d zqnd=/tmp; x=$(hash -d zqnd=/usr); hash -d | grep zqnd"#,
        "zqnd=/tmp\n",
        0,
    );
}

#[test]
fn nameddir_tilde_after() {
    check(
        r#"x=$(hash -d zqnd=/tmp); eval 'print ~zqnd' 2>/dev/null; print $?"#,
        "1\n",
        0,
    );
}

#[test]
fn nameddir_seen_inside() {
    check(
        r#"x=$(hash -d zqnd=/tmp; print ~zqnd); print $x"#,
        "/tmp\n",
        0,
    );
}

#[test]
fn nameddir_nameddirs_param() {
    check(
        r#"x=$(nameddirs[zqndp]=/tmp); print ${+nameddirs[zqndp]}"#,
        "0\n",
        0,
    );
}

#[test]
fn nameddir_from_param_tilde() {
    check(
        r#"x=$(zqnd2=/tmp; : ~zqnd2); hash -d | grep -c zqnd2"#,
        "0\n",
        1,
    );
}

#[test]
fn cmdhash_define() {
    check(
        r#"x=$(hash zqcmd=/bin/echo); hash | grep -c zqcmd"#,
        "0\n",
        1,
    );
}

#[test]
fn cmdhash_unhash() {
    check(
        r#"hash zqcmd=/bin/echo; x=$(unhash zqcmd); hash | grep -c zqcmd"#,
        "1\n",
        0,
    );
}

#[test]
fn cmdhash_rehash() {
    check(
        r#"hash zqc=/bin/echo; x=$(hash -r); hash | grep -c zqc"#,
        "1\n",
        0,
    );
}

#[test]
fn cmdhash_run_after() {
    check(
        r#"x=$(hash zqcmd=/bin/echo); zqcmd hi 2>/dev/null; print $?"#,
        "127\n",
        0,
    );
}

#[test]
fn cmdhash_commands_param() {
    check(
        r#"x=$(commands[zqcp]=/bin/echo); print ${+commands[zqcp]}"#,
        "0\n",
        0,
    );
}

#[test]
fn nested_var_and_alias() {
    check(
        r#"x=$(y=$(zqn=1; alias zqna=a); zqn2=1); print ${+zqn} ${+zqn2}; alias zqna; print $?"#,
        "0 0\n1\n",
        0,
    );
}

#[test]
fn nested_inner_print() {
    check(
        r#"x=$(print $(alias zqna=a; print in)); print $x; alias zqna; print $?"#,
        "in\n1\n",
        0,
    );
}

#[test]
fn nested_inner_sees_outer() {
    check(
        r#"x=$(alias zqo=a; y=$(alias zqo); print $y); print $x; alias zqo; print $?"#,
        "zqo=a\n1\n",
        0,
    );
}

#[test]
fn nested_export() {
    check(
        r#"x=$(y=$(export zqne=1); /usr/bin/env | grep -c '^zqne='); print $x; /usr/bin/env | grep -c '^zqne='"#,
        "0\n0\n",
        1,
    );
}

#[test]
fn subshell_inside_subst() {
    check(
        r#"x=$( (alias zqss=a) ); alias zqss; print $?"#,
        "1\n",
        0,
    );
}

#[test]
fn subst_inside_subshell() {
    check(
        r#"(x=$(alias zqsub=a); alias zqsub; print $?); alias zqsub; print $?"#,
        "1\n1\n",
        0,
    );
}

#[test]
fn backtick_var_alias() {
    check(
        r#"x=`zqb=1; alias zqba=a`; print ${+zqb}; alias zqba; print $?"#,
        "0\n1\n",
        0,
    );
}

#[test]
fn backtick_export() {
    check(
        r#"x=`export zqbe=1`; /usr/bin/env | grep -c '^zqbe='"#,
        "0\n",
        1,
    );
}

#[test]
fn backtick_hash() {
    check(
        r#"x=`hash -d zqbnd=/tmp; hash zqbc=/bin/echo`; hash -d | grep -c zqbnd; hash | grep -c zqbc"#,
        "0\n0\n",
        1,
    );
}

#[test]
fn positional_set() {
    check(
        r#"set -- a b; x=$(set -- c); print $*"#,
        "a b\n",
        0,
    );
}

#[test]
fn positional_shift() {
    check(
        r#"set -- a b; x=$(shift); print $#"#,
        "2\n",
        0,
    );
}

#[test]
fn positional_in_function() {
    check(
        r#"f() { x=$(set -- z); print $* }; f p q"#,
        "p q\n",
        0,
    );
}

#[test]
fn status_propagates() {
    check(
        r#"x=$(alias zqst=a; false); print $?; alias zqst; print $?"#,
        "1\n1\n",
        0,
    );
}

#[test]
fn umask() {
    check(
        r#"umask 022; x=$(umask 077); umask"#,
        "022\n",
        0,
    );
}

#[test]
fn ulimit_soft() {
    check(
        r#"x=$(ulimit -S -t 1000); ulimit -S -t"#,
        "unlimited\n",
        0,
    );
}

#[test]
fn fd_open_in_subst() {
    check(
        r#"x=$(exec 3>/dev/null); print hi >&3 2>/dev/null; print $?"#,
        "1\n",
        0,
    );
}

#[test]
fn fd_close_in_subst() {
    check(
        r#"exec 3>/dev/null; x=$(exec 3>&-); print hi >&3; print $?"#,
        "0\n",
        0,
    );
}

#[test]
fn procsubst_alias() {
    check(
        r#"cat <(alias zqps=a) >/dev/null; alias zqps; print $?"#,
        "1\n",
        0,
    );
}

#[test]
fn procsubst_export() {
    check(
        r#"cat <(export zqpse=1) >/dev/null; /usr/bin/env | grep -c '^zqpse='"#,
        "0\n",
        1,
    );
}

#[test]
fn pipeline_first_stage() {
    check(
        r#"alias zqpp=a | cat; alias zqpp; print $?"#,
        "1\n",
        0,
    );
}

#[test]
fn pipeline_last_stage() {
    check(
        r#"true | alias zqlast=a; alias zqlast; print $?"#,
        "zqlast=a\n0\n",
        0,
    );
}

#[test]
fn zstyle() {
    check(
        r#"x=$(zstyle ':zq' foo bar); zstyle -s ':zq' foo v; print $? $v"#,
        "1\n",
        0,
    );
}

#[test]
fn zmodload() {
    check(
        r#"x=$(zmodload zsh/mathfunc); zmodload | grep -c mathfunc"#,
        "0\n",
        1,
    );
}

#[test]
fn sched() {
    check(
        r#"x=$(sched +100 true); sched | wc -l | tr -d ' '"#,
        "0\n",
        0,
    );
}

#[test]
fn trap_not_inherited_listing() {
    check(
        r#"trap 'print p' USR1; print -r -- "[$(trap)]""#,
        "[]\n",
        0,
    );
}

#[test]
fn cwd_kernel() {
    check(
        r#"cd /usr; x=$(cd /bin); /bin/pwd"#,
        "/usr\n",
        0,
    );
}

#[test]
fn cwd_cd_pwd_idiom() {
    check(
        r#"cd /usr; d=$(cd /bin && pwd); print $d; /bin/pwd"#,
        "/bin\n/usr\n",
        0,
    );
}

#[test]
fn export_from_function() {
    check(
        r#"f() { export zqfe=1 }; x=$(f); /usr/bin/env | grep -c '^zqfe='"#,
        "0\n",
        1,
    );
}

#[test]
fn export_unset_restores_order() {
    check(
        r#"a=$(/usr/bin/env | grep -n '^HOME=' | cut -d: -f1); x=$(unset HOME); b=$(/usr/bin/env | grep -n '^HOME=' | cut -d: -f1); [[ $a == $b ]] && print same || print differ"#,
        "same\n",
        0,
    );
}

#[test]
fn export_value_in_place() {
    check(
        r#"export zqe16=a; a=$(/usr/bin/env | grep -n '^zqe16=' | cut -d: -f1); x=$(zqe16=bbbbbbbbbb); b=$(/usr/bin/env | grep -n '^zqe16=' | cut -d: -f1); [[ $a == $b ]] && print same || print differ; print $zqe16"#,
        "same\na\n",
        0,
    );
}

#[test]
fn paren_suffix_alias() {
    check(
        r#"(alias -s zqs=less); alias -s zqs; print $?"#,
        "1\n",
        0,
    );
}

#[test]
fn paren_nameddir() {
    check(
        r#"(hash -d zqnd=/tmp); hash -d | grep -c zqnd"#,
        "0\n",
        1,
    );
}

#[test]
fn paren_cmdhash() {
    check(
        r#"(hash zqcmd=/bin/echo); hash | grep -c zqcmd"#,
        "0\n",
        1,
    );
}

#[test]
fn paren_export() {
    check(
        r#"(export zqpe=1); /usr/bin/env | grep -c '^zqpe='"#,
        "0\n",
        1,
    );
}

#[test]
fn paren_sched() {
    check(
        r#"(sched +100 true); sched | wc -l | tr -d ' '"#,
        "0\n",
        0,
    );
}

#[test]
fn paren_dirstack() {
    check(
        r#"cd /; (pushd /tmp >/dev/null); dirs"#,
        "/\n",
        0,
    );
}

#[test]
fn paren_seconds() {
    check(
        r#"(SECONDS=100); (( SECONDS < 50 )) && print ok"#,
        "ok\n",
        0,
    );
}

#[test]
fn alias_keys_order_after_paren() {
    check(
        r#"for i in {1..60}; do alias zqk$i=x; done; a="${(k)aliases}"; (true); b="${(k)aliases}"; [[ $a == $b ]] && print same || print differ"#,
        "same\n",
        0,
    );
}

#[test]
fn alias_keys_order_after_subst() {
    check(
        r#"for i in {1..60}; do alias zqk$i=x; done; a="${(k)aliases}"; x=$(unalias zqk7 zqk9; alias zqk9=x zqk7=x); b="${(k)aliases}"; [[ $a == $b ]] && print same || print differ"#,
        "same\n",
        0,
    );
}

#[test]
fn fd_dup_stdout_no_hang() {
    check(
        r#"x=$(exec 3>&1; print in); print "done [$x]""#,
        "done [in]\n",
        0,
    );
}

#[test]
fn fd_redirect_stdout_inside() {
    check(
        r#"x=$(exec >/dev/null; print lost); print "[$x]"; print after"#,
        "[]\nafter\n",
        0,
    );
}

#[test]
fn paren_fd_open() {
    check(
        r#"(exec 3>/dev/null); print hi >&3 2>/dev/null; print $?"#,
        "1\n",
        0,
    );
}

#[test]
fn trap_parent_fires_after_subst() {
    check(
        r#"trap 'print got' USR1; x=$(kill -USR1 $$; print in); print "[$x]""#,
        "got\n[in]\n",
        0,
    );
}

#[test]
fn trap_err_survives() {
    check(
        r#"trap 'print e' ERR; print -r -- "[$(trap)]""#,
        "[trap -- 'print e' ERR]\n",
        0,
    );
}

#[test]
fn trap_function_form_inherited() {
    check(
        r#"TRAPUSR1() { :; }; x=$(print ${+functions[TRAPUSR1]}); print $x"#,
        "1\n",
        0,
    );
}

#[test]
fn trap_exit_parent_not_fired_by_subst() {
    check(
        r#"trap 'print pe' EXIT; x=$(print in); print "[$x]""#,
        "[in]\npe\n",
        0,
    );
}

// ── The environment a child of the PARENT sees, INTERACTIVELY ──
//
// Every case above runs `-c`, and a `-c` shell cannot show this one: the
// value of a PM_SPECIAL scalar like `TERM` only moves out of `pm->u.str`
// and into its GSU global once an interactive shell has assigned it, and
// only a prompt loop replays the parent's specials at the end of a
// subshell. Measured with the real config: the parameter kept reading
// `xterm-256color` while every child from the second command onward got
// an empty `TERM`, so `clear` said "TERM environment variable not set".
//
// Driven through `zsh/zpty` like the rest of the interactive suite, so
// both shells drive an interactive copy of themselves.

use crate::zpty_probe::{assert_same_verdict, DRAIN, OPEN};

/// Assign `TERM` (which routes through its GSU, leaving `u.str` empty),
/// run a subshell — whose exit replays the parent's specials — then ask
/// a CHILD what it sees. The marker is assembled at run time so the
/// echoed command line cannot satisfy the match.
const TERM_ENV: &str = r#"
zpty -w w 'TERM=$TERM; ( : )'
sleep 1
zpty -w w 'x=$(:); print "TE${:-}RM=[$(/usr/bin/printenv TERM)]"'
sleep 2
"#;

fn term_env_driver() -> String {
    format!(
        "{OPEN}{TERM_ENV}{DRAIN}
if [[ $all == *'TERM=[xterm-256color]'* ]]; then print \"TERMENV=yes\"; else print \"TERMENV=no\"; fi
"
    )
}

#[test]
fn a_child_of_the_parent_still_sees_term_after_a_subshell() {
    assert_same_verdict(
        &term_env_driver(),
        "TERMENV",
        "a child still got $TERM from the environment after a subshell",
    );
}
