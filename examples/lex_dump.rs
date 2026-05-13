//! Quick lexer dumper: matches the C-side zshrs_dump module's output
//! format so divergences can be diff'd by hand.
fn main() {
    let path = std::env::args().nth(1).expect("usage: lex_dump FILE");
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {}", path, e));
    zsh::lex::lex_init(&src);
    use zsh::tokens::{ENDINPUT, LEXERR};
    loop {
        zsh::lex::ctxtlex();
        let tok = zsh::lex::tok();
        if tok == ENDINPUT { println!("ENDINPUT"); break; }
        if tok == LEXERR { println!("LEXERR"); break; }
        let raw = zsh::lex::tokstr().unwrap_or_default();
        let plain = zsh::lex::untokenize_preserve_quotes(&raw);
        let raw_bytes: Vec<u8> = raw.bytes().collect();
        println!(
            "tok={} cmdpos={} tokstr_plain={:?} raw_bytes={:?}",
            tok,
            zsh::lex::incmdpos(),
            plain,
            raw_bytes
        );
    }
}
