//! Probe whether the ported ZshParser parses everyday constructs without
//! erroring. We don't validate AST shapes deeply yet — just that the parser
//! doesn't reject inputs that the corpus expects to handle.
//!
//! Failures here mean the port itself has gaps before we even start the
//! compile_zsh.rs migration.

use zsh::parser::{ZshParser, ZshCommand};

fn parse_ok(src: &str) {
    let mut parser = ZshParser::new(src);
    match parser.parse() {
        Ok(_program) => {}
        Err(errors) => panic!(
            "ZshParser rejected `{}`: {} errors. First: {:?}",
            src,
            errors.len(),
            errors.first()
        ),
    }
}

#[test] fn p_echo() { parse_ok("echo hi"); }
#[test] fn p_assign() { parse_ok("x=42"); }
#[test] fn p_assign_then_echo() { parse_ok("x=42; echo $x"); }
#[test] fn p_pipeline() { parse_ok("echo hi | cat"); }
#[test] fn p_and_or() { parse_ok("true && echo y || echo n"); }
#[test] fn p_if() { parse_ok("if true; then echo y; fi"); }
#[test] fn p_while() { parse_ok("while true; do echo x; done"); }
#[test] fn p_for() { parse_ok("for i in 1 2 3; do echo $i; done"); }
#[test] fn p_for_arith() { parse_ok("for ((i=0; i<3; i++)); do echo $i; done"); }
#[test] fn p_case() { parse_ok("case x in a) echo a ;; *) echo o ;; esac"); }
#[test] fn p_function() { parse_ok("greet() { echo hi $1; }"); }
#[test] fn p_function_kw() { parse_ok("function f { echo x; }"); }
#[test] fn p_select() { parse_ok("select x in a b; do echo $x; done"); }
#[test] fn p_redir_write() { parse_ok("echo hi > out.txt"); }
#[test] fn p_redir_append() { parse_ok("echo hi >> out.txt"); }
#[test] fn p_redir_read() { parse_ok("cat < in.txt"); }
#[test] fn p_redir_dup() { parse_ok("echo hi >&2"); }
#[test] fn p_heredoc() { parse_ok("cat <<EOF\nline\nEOF"); }
#[test] fn p_herestring() { parse_ok("tr a-z A-Z <<< hi"); }
#[test] fn p_proc_sub() { parse_ok("cat <(echo line)"); }
#[test] fn p_dquoted() { parse_ok(r#"echo "var=$x""#); }
#[test] fn p_squoted() { parse_ok("echo 'lit $x'"); }
#[test] fn p_dollar_paren() { parse_ok("x=$(echo nested)"); }
#[test] fn p_arith_subst() { parse_ok("echo $((2+3))"); }
#[test] fn p_arith_compound() { parse_ok("(( x = 2 + 3 ))"); }
#[test] fn p_double_bracket() { parse_ok("[[ a == a ]]"); }
#[test] fn p_double_bracket_regex() { parse_ok("[[ abc =~ ^a ]]"); }
#[test] fn p_array_literal() { parse_ok("arr=(a b c)"); }
#[test] fn p_array_index() { parse_ok("echo ${arr[1]}"); }
#[test] fn p_array_splice() { parse_ok("echo ${arr[@]}"); }
#[test] fn p_param_default() { parse_ok("echo ${x:-default}"); }
#[test] fn p_param_strip() { parse_ok("echo ${p#*/}"); }
#[test] fn p_zsh_flag() { parse_ok("echo ${(L)x}"); }
#[test] fn p_brace_range() { parse_ok("echo {1..5}"); }
#[test] fn p_brace_alt() { parse_ok("echo {a,b,c}"); }
#[test] fn p_subshell() { parse_ok("(x=inner; echo $x)"); }
#[test] fn p_brace_group() { parse_ok("{ x=set; echo $x; }"); }
#[test] fn p_trap() { parse_ok("trap 'echo bye' EXIT"); }
#[test] fn p_typeset_assoc() { parse_ok("typeset -A m"); }
#[test] fn p_assoc_set() { parse_ok("m[k]=v"); }
#[test] fn p_alias() { parse_ok("alias g='echo hi'"); }
#[test] fn p_eval() { parse_ok("eval 'echo from-eval'"); }
#[test] fn p_bg() { parse_ok("sleep 1 & echo done"); }
#[test] fn p_coproc() { parse_ok("coproc { echo hi; }"); }
#[test] fn p_negate() { parse_ok("! true"); }
#[test] fn p_repeat() { parse_ok("repeat 3 echo hi"); }
#[test] fn p_break_continue() {
    parse_ok("for i in 1 2 3; do break; done");
    parse_ok("while true; do continue; done");
}
#[test] fn p_return() { parse_ok("f() { return 5; }"); }
#[test] fn p_export() { parse_ok("export X=v"); }
#[test] fn p_local() { parse_ok("f() { local x=v; }"); }
#[test] fn p_complex_pipeline() {
    parse_ok("seq 100 | sort | uniq | wc -l");
}
#[test] fn p_multi_line_for() {
    parse_ok("for f in *.rs; do\n  echo $f\ndone");
}
#[test] fn p_nested_if() {
    parse_ok("if true; then\n  if true; then echo nested; fi\nfi");
}
