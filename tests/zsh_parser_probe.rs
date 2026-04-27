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

#[test]
fn probe_cond_shape() {
    use zsh::parser::{ZshParser, ZshCommand};
    let mut p = ZshParser::new("[[ a == a ]]");
    let prog = p.parse().unwrap();
    eprintln!("[[]] AST: {:#?}", prog);
    assert_eq!(prog.lists.len(), 1);
    let pipe = &prog.lists[0].sublist.pipe;
    eprintln!("first cmd: {:?}", pipe.cmd);
}

#[test]
fn probe_arith_shape() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("(( i < 3 ))");
    let prog = p.parse().unwrap();
    eprintln!("(()) AST: {:#?}", prog);
}

#[test]
fn probe_compile_arith() {
    use zsh::parser::ZshParser;
    use zsh::compile_zsh::ZshCompiler;
    let mut p = ZshParser::new("(( x = 2 + 3 )); echo $x");
    let prog = p.parse().unwrap();
    eprintln!("AST: {:#?}", prog);
    let comp = ZshCompiler::new();
    let chunk = comp.compile(&prog);
    eprintln!("ops: {}", chunk.ops.len());
    for (i, op) in chunk.ops.iter().enumerate() {
        eprintln!("  {:3}: {:?}", i, op);
    }
}

#[test]
fn probe_unary_test_op() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("[[ -d /tmp ]]");
    let prog = p.parse().unwrap();
    eprintln!("UNARY AST: {:#?}", prog);
}

#[test]
fn probe_case_pattern() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("case foo in *) echo def ;; esac");
    let prog = p.parse().unwrap();
    eprintln!("CASE AST: {:#?}", prog);
}

#[test]
fn probe_chain() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("false && echo no || echo yes");
    let prog = p.parse().unwrap();
    eprintln!("CHAIN AST: {:#?}", prog);
}

#[test]
fn probe_squoted() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("echo 'a $b c'");
    let prog = p.parse().unwrap();
    eprintln!("SQ AST: {:#?}", prog);
}

#[test]
fn probe_regex_anchor() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("[[ abc =~ ^a ]] && echo y");
    let prog = p.parse().unwrap();
    eprintln!("REGEX AST: {:#?}", prog);
}

#[test]
fn probe_dollar_single() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new(r#"echo $'a\tb'"#);
    let prog = p.parse().unwrap();
    eprintln!("DOLLAR-SINGLE AST: {:#?}", prog);
}

#[test]
fn probe_funcdef_compile() {
    use zsh::compile_zsh::ZshCompiler;
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("greet() { echo hi; }; greet");
    let prog = p.parse().unwrap();
    let comp = ZshCompiler::new();
    let chunk = comp.compile(&prog);
    eprintln!("ops: {}", chunk.ops.len());
    for (i, op) in chunk.ops.iter().enumerate() {
        eprintln!("  {:3}: {:?}", i, op);
    }
    eprintln!("constants: {}", chunk.constants.len());
    for (i, c) in chunk.constants.iter().enumerate() {
        let s = format!("{:?}", c);
        let truncated = if s.len() > 80 { &s[..80] } else { s.as_str() };
        eprintln!("  {:3}: {}", i, truncated);
    }
    eprintln!("names: {:?}", chunk.names);
    eprintln!("sub_chunks: {}", chunk.sub_chunks.len());
}

#[test]
fn probe_funcdef_ast() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("greet() { echo hi; }; greet");
    let prog = p.parse().unwrap();
    eprintln!("FUNC AST: {:#?}", prog);
}

#[test]
fn probe_assoc_two_in_dquote() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new(r#"echo "${foo[a]} ${foo[b]}""#);
    let prog = p.parse().unwrap();
    eprintln!("ASSOC-TWO AST: {:#?}", prog);
}

#[test]
fn probe_lex_pure_funcdef() {
    use zsh::lexer::ZshLexer;
    use zsh::tokens::LexTok;
    let mut lex = ZshLexer::new("f() { :; }; f");
    for _ in 0..15 {
        lex.zshlex();
        eprintln!("tok={:?} tokstr={:?} incmdpos={}", lex.tok, lex.tokstr, lex.incmdpos);
        if lex.tok == LexTok::Endinput {
            break;
        }
    }
}

#[test]
fn probe_lex_array_then_funcdef() {
    use zsh::lexer::ZshLexer;
    use zsh::tokens::LexTok;
    let mut lex = ZshLexer::new("g=(o1); f() { :; }; f");
    for _ in 0..15 {
        lex.zshlex();
        eprintln!("tok={:?} tokstr={:?}", lex.tok, lex.tokstr);
        if lex.tok == LexTok::Endinput {
            break;
        }
    }
}

#[test]
fn probe_lex_printf() {
    use zsh::lexer::ZshLexer;
    use zsh::tokens::LexTok;
    let mut lex = ZshLexer::new(r#"printf "a\nb""#);
    for _ in 0..10 {
        lex.zshlex();
        eprintln!("tok={:?} tokstr={:?}", lex.tok, lex.tokstr);
        if lex.tok == LexTok::Endinput { break; }
    }
}

#[test]
fn probe_cmdsub_inner() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new(r#"echo $(printf "a\nb")"#);
    let prog = p.parse().unwrap();
    eprintln!("CMDSUB AST: {:#?}", prog);
}

#[test]
fn probe_array_then_funcdef() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("g=(o1); f() { :; }; f");
    let prog = p.parse().unwrap();
    eprintln!("ARR-FN AST: {:#?}", prog);
}

#[test]
fn probe_anon_fn() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("() { echo anon; }");
    let prog = p.parse().unwrap();
    eprintln!("ANON-FN AST: {:#?}", prog);
}

#[test]
fn probe_for_implicit_pos() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new(r#"f() { for x; do echo "[$x]"; done; }"#);
    let prog = p.parse().unwrap();
    eprintln!("FOR-IMPL AST: {:#?}", prog);
}

#[test]
fn probe_lex_regex_paren() {
    use zsh::lexer::ZshLexer;
    use zsh::tokens::LexTok;
    let src = r#"[[ "1.2" =~ ([0-9]+).([0-9]+) ]]"#;
    let mut lex = ZshLexer::new(src);
    for _ in 0..15 {
        lex.zshlex();
        eprintln!("tok={:?} tokstr={:?} incondpat={}", lex.tok, lex.tokstr, lex.incondpat);
        if lex.tok == LexTok::Endinput {
            break;
        }
    }
}

#[test]
fn probe_regex_with_paren() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new(r#"[[ "1.2" =~ ([0-9]+).([0-9]+) ]] && echo y"#);
    let prog = p.parse().unwrap();
    eprintln!("REGEX-PAREN AST: {:#?}", prog);
}

#[test]
fn probe_proc_sub_input() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("/bin/cat <(echo line)");
    let prog = p.parse().unwrap();
    eprintln!("PROC-SUB AST: {:#?}", prog);
}

#[test]
fn probe_bang_dollar() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("echo !$");
    let prog = p.parse().unwrap();
    eprintln!("BANG-DOLLAR AST: {:#?}", prog);
}

#[test]
fn probe_quote_escape_dollar() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new(r#"echo "\$lit""#);
    let prog = p.parse().unwrap();
    eprintln!("QUOTE-ESCAPE AST: {:#?}", prog);
}

#[test]
fn probe_test_bang() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("test ! -z foo");
    let prog = p.parse().unwrap();
    eprintln!("TEST-BANG AST: {:#?}", prog);
}

#[test]
fn probe_zshflag_count() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("a=(x y z); echo ${(#)a}");
    let prog = p.parse().unwrap();
    eprintln!("ZSHFLAG-COUNT AST: {:#?}", prog);
}

#[test]
fn probe_assoc_set_get() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("typeset -A m; m[k]=v; echo ${m[k]}");
    let prog = p.parse().unwrap();
    eprintln!("ASSOC AST: {:#?}", prog);
}

#[test]
fn probe_array_iter_for() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("a=(red green blue); for c in ${a[@]}; do echo $c; done");
    let prog = p.parse().unwrap();
    eprintln!("ARRAY-ITER AST: {:#?}", prog);
}

#[test]
fn probe_alias_assign() {
    use zsh::parser::ZshParser;
    let src = "alias g='echo greeted'";
    let mut p = ZshParser::new(src);
    let prog = p.parse().unwrap();
    eprintln!("ALIAS-ASSIGN AST: {:#?}", prog);
}

#[test]
fn probe_for_over_cmdsub() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("for w in $(echo a b c); do echo $w; done");
    let prog = p.parse().unwrap();
    eprintln!("FOR-OVER-CMDSUB AST: {:#?}", prog);
}

#[test]
fn probe_array_idx() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("arr=(a b c); echo $arr[2]");
    let prog = p.parse().unwrap();
    eprintln!("ARR-IDX AST: {:#?}", prog);
}

#[test]
fn probe_brace_alt() {
    use zsh::parser::ZshParser;
    let mut p = ZshParser::new("echo {a,b,c}");
    let prog = p.parse().unwrap();
    eprintln!("BRACE-ALT AST: {:#?}", prog);
}

#[test]
fn probe_heredoc_ast() {
    use zsh::parser::ZshParser;
    let src = "cat <<EOF\nline1\nline2\nEOF";
    let mut p = ZshParser::new(src);
    let prog = p.parse().unwrap();
    eprintln!("HEREDOC AST: {:#?}", prog);
}

#[test]
fn probe_dollar_at_for() {
    use zsh::parser::ZshParser;
    let src = r#"f() { for x in "$@"; do echo "[$x]"; done; }; f a "two w" c"#;
    let mut p = ZshParser::new(src);
    let prog = p.parse().unwrap();
    eprintln!("DOLLAR-AT AST: {:#?}", prog);
}
