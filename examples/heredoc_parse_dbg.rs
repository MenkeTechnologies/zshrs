//! Debug: parse "cat <<EOF\nhi\nEOF" and dump LEX_HEREDOCS state.
use zsh::ported::lex;
use zsh::ported::parse;

fn main() {
    let input = "cat <<EOF\nhi\nEOF";
    parse::parse_init(input);
    let prog = parse::parse();
    let h = lex::LEX_HEREDOCS.with_borrow(|v| v.clone());
    eprintln!("LEX_HEREDOCS count={} state={:#?}", h.len(), h);
    eprintln!("Program: {:#?}", prog);
}
