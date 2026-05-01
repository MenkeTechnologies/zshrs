//! Zsh token definitions - Direct port from zsh/Src/zsh.h
//!
//! This module defines all character tokens and lexical tokens
//! used by the zsh lexer and parser.

/// Character tokens - metafied characters with special meaning.
/// These appear in strings and represent tokenized versions of special characters.
/// Values match the C definitions in zsh.h exactly.
pub mod char_tokens {
    pub const META: char = '\u{83}';
    pub const POUND: char = '\u{84}'; // #
    pub const STRING: char = '\u{85}'; // $
    pub const HAT: char = '\u{86}'; // ^
    pub const STAR: char = '\u{87}'; // *
    pub const INPAR: char = '\u{88}'; // (
    pub const INPARMATH: char = '\u{89}'; // ((
    pub const OUTPAR: char = '\u{8a}'; // )
    pub const OUTPARMATH: char = '\u{8b}'; // ))
    pub const QSTRING: char = '\u{8c}'; // $ in double quotes
    pub const EQUALS: char = '\u{8d}'; // =
    pub const BAR: char = '\u{8e}'; // |
    pub const INBRACE: char = '\u{8f}'; // {
    pub const OUTBRACE: char = '\u{90}'; // }
    pub const INBRACK: char = '\u{91}'; // [
    pub const OUTBRACK: char = '\u{92}'; // ]
    pub const TICK: char = '\u{93}'; // `
    pub const INANG: char = '\u{94}'; // <
    pub const OUTANG: char = '\u{95}'; // >
    pub const OUTANGPROC: char = '\u{96}'; // > for process sub
    pub const QUEST: char = '\u{97}'; // ?
    pub const TILDE: char = '\u{98}'; // ~
    pub const QTICK: char = '\u{99}'; // ` in double quotes
    pub const COMMA: char = '\u{9a}'; // ,
    pub const DASH: char = '\u{9b}'; // - in patterns
    pub const BANG: char = '\u{9c}'; // ! in patterns

    pub const LAST_NORMAL_TOK: char = BANG;

    // Null arguments: placeholders for quotes
    pub const SNULL: char = '\u{9d}'; // single quote marker
    pub const DNULL: char = '\u{9e}'; // double quote marker
    pub const BNULL: char = '\u{9f}'; // backslash null

    pub const BNULLKEEP: char = '\u{a0}'; // backslash to keep as \
    pub const NULARG: char = '\u{a1}'; // null argument
    pub const MARKER: char = '\u{a2}'; // special marker

    /// Check if a character is a token
    #[inline]
    pub fn is_token(c: char) -> bool {
        let b = c as u32;
        b >= 0x84 && b <= 0xa2
    }

    /// Convert token back to its original character
    pub fn untokenize(c: char) -> Option<char> {
        match c {
            POUND => Some('#'),
            STRING | QSTRING => Some('$'),
            HAT => Some('^'),
            STAR => Some('*'),
            INPAR | INPARMATH => Some('('),
            OUTPAR | OUTPARMATH => Some(')'),
            EQUALS => Some('='),
            BAR => Some('|'),
            INBRACE => Some('{'),
            OUTBRACE => Some('}'),
            INBRACK => Some('['),
            OUTBRACK => Some(']'),
            TICK | QTICK => Some('`'),
            INANG => Some('<'),
            OUTANG | OUTANGPROC => Some('>'),
            QUEST => Some('?'),
            TILDE => Some('~'),
            COMMA => Some(','),
            DASH => Some('-'),
            BANG => Some('!'),
            SNULL | DNULL | BNULL | BNULLKEEP | NULARG | MARKER => None,
            _ => None,
        }
    }

    /// Token characters string - maps token values back to their literal chars
    /// Matches ztokens[] from lex.c: "#$^*(())$=|{}[]`<>>?~`,-!'\"\\\\"
    pub const ZTOKENS: &str = "#$^*(())$=|{}[]`<>>?~`,-!'\"\\\\";
}

/// Lexical tokens - returned by the lexer
/// These match enum lextok from zsh.h exactly
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LexTok {
    Nulltok = 0,
    Seper,          // 1 - ;  (separator, not necessarily literal semicolon)
    Newlin,         // 2 - \n
    Semi,           // 3 - ;
    Dsemi,          // 4 - ;;
    Amper,          // 5 - &
    Inpar,          // 6 - (
    Outpar,         // 7 - )
    Dbar,           // 8 - ||
    Damper,         // 9 - &&
    Outang,         // 10 - >
    Outangbang,     // 11 - >|
    Doutang,        // 12 - >>
    Doutangbang,    // 13 - >>|
    Inang,          // 14 - <
    Inoutang,       // 15 - <>
    Dinang,         // 16 - <<
    Dinangdash,     // 17 - <<-
    Inangamp,       // 18 - <&
    Outangamp,      // 19 - >&
    Ampoutang,      // 20 - &>
    Outangampbang,  // 21 - &>|
    Doutangamp,     // 22 - >>&
    Doutangampbang, // 23 - >>&|
    Trinang,        // 24 - <<<
    Bar,            // 25 - |
    Baramp,         // 26 - |&
    Inoutpar,       // 27 - ()
    Dinpar,         // 28 - ((
    Doutpar,        // 29 - ))
    Amperbang,      // 30 - &| or &!
    Semiamp,        // 31 - ;&
    Semibar,        // 32 - ;|

    // Non-punctuation tokens
    Doutbrack, // 33 - ]]
    String,    // 34 - word/string
    Envstring, // 35 - VAR=value
    Envarray,  // 36 - VAR=(...)
    Endinput,  // 37 - end of input
    Lexerr,    // 38 - lexer error

    // Reserved words
    Bang,      // 39 - !
    Dinbrack,  // 40 - [[
    Inbrace,   // 41 - {
    Outbrace,  // 42 - }
    Case,      // 43 - case
    Coproc,    // 44 - coproc
    Doloop,    // 45 - do
    Done,      // 46 - done
    Elif,      // 47 - elif
    Else,      // 48 - else
    Zend,      // 49 - end
    Esac,      // 50 - esac
    Fi,        // 51 - fi
    For,       // 52 - for
    Foreach,   // 53 - foreach
    Func,      // 54 - function
    If,        // 55 - if
    Nocorrect, // 56 - nocorrect
    Repeat,    // 57 - repeat
    Select,    // 58 - select
    Then,      // 59 - then
    Time,      // 60 - time
    Until,     // 61 - until
    While,     // 62 - while
    Typeset,   // 63 - typeset or similar
}

impl LexTok {
    /// Check if this token is a redirection operator
    pub fn is_redirop(self) -> bool {
        matches!(
            self,
            LexTok::Outang
                | LexTok::Outangbang
                | LexTok::Doutang
                | LexTok::Doutangbang
                | LexTok::Inang
                | LexTok::Inoutang
                | LexTok::Dinang
                | LexTok::Dinangdash
                | LexTok::Inangamp
                | LexTok::Outangamp
                | LexTok::Ampoutang
                | LexTok::Outangampbang
                | LexTok::Doutangamp
                | LexTok::Doutangampbang
                | LexTok::Trinang
        )
    }

    /// String representation of punctuation tokens
    pub fn as_str(self) -> Option<&'static str> {
        match self {
            LexTok::Nulltok => None,
            LexTok::Seper => Some(";"),
            LexTok::Newlin => Some("\\n"),
            LexTok::Semi => Some(";"),
            LexTok::Dsemi => Some(";;"),
            LexTok::Amper => Some("&"),
            LexTok::Inpar => Some("("),
            LexTok::Outpar => Some(")"),
            LexTok::Dbar => Some("||"),
            LexTok::Damper => Some("&&"),
            LexTok::Outang => Some(">"),
            LexTok::Outangbang => Some(">|"),
            LexTok::Doutang => Some(">>"),
            LexTok::Doutangbang => Some(">>|"),
            LexTok::Inang => Some("<"),
            LexTok::Inoutang => Some("<>"),
            LexTok::Dinang => Some("<<"),
            LexTok::Dinangdash => Some("<<-"),
            LexTok::Inangamp => Some("<&"),
            LexTok::Outangamp => Some(">&"),
            LexTok::Ampoutang => Some("&>"),
            LexTok::Outangampbang => Some("&>|"),
            LexTok::Doutangamp => Some(">>&"),
            LexTok::Doutangampbang => Some(">>&|"),
            LexTok::Trinang => Some("<<<"),
            LexTok::Bar => Some("|"),
            LexTok::Baramp => Some("|&"),
            LexTok::Inoutpar => Some("()"),
            LexTok::Dinpar => Some("(("),
            LexTok::Doutpar => Some("))"),
            LexTok::Amperbang => Some("&|"),
            LexTok::Semiamp => Some(";&"),
            LexTok::Semibar => Some(";|"),
            _ => None,
        }
    }
}

/// Redirection types - matches enum from zsh.h
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum RedirType {
    Write = 0,   // >
    Writenow,    // >|
    App,         // >>
    Appnow,      // >>|
    Errwrite,    // &>, >&
    Errwritenow, // >&|
    Errapp,      // >>&
    Errappnow,   // >>&|
    Readwrite,   // <>
    Read,        // <
    Heredoc,     // <<
    Heredocdash, // <<-
    Herestr,     // <<<
    Mergein,     // <&n
    Mergeout,    // >&n
    Close,       // >&-, <&-
    Inpipe,      // < <(...)
    Outpipe,     // > >(...)
}

impl RedirType {
    /// Check if this is a read-type redirection
    pub fn is_read(self) -> bool {
        matches!(
            self,
            RedirType::Read
                | RedirType::Readwrite
                | RedirType::Heredoc
                | RedirType::Heredocdash
                | RedirType::Herestr
                | RedirType::Mergein
                | RedirType::Inpipe
        )
    }

    /// Check if this is a file write redirection
    pub fn is_write_file(self) -> bool {
        matches!(
            self,
            RedirType::Write
                | RedirType::Writenow
                | RedirType::App
                | RedirType::Appnow
                | RedirType::Readwrite
        )
    }
}

/// Condition types for [[ ... ]] expressions
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CondType {
    Not = 0,
    And,
    Or,
    Streq,  // =
    Strdeq, // ==
    Strneq, // !=
    Strlt,  // <
    Strgtr, // >
    Nt,     // -nt (newer than)
    Ot,     // -ot (older than)
    Ef,     // -ef (same file)
    Eq,     // -eq
    Ne,     // -ne
    Lt,     // -lt
    Gt,     // -gt
    Le,     // -le
    Ge,     // -ge
    Regex,  // =~
    Mod,    // module test
    Modi,   // module test with infix
}

/// Characters that need quoting if meant literally
pub const SPECCHARS: &str = "#$^*()=|{}[]`<>?~;&\n\t \\'\"";

/// Characters that need quoting for pattern matching
pub const PATCHARS: &str = "#^*()|[]<>?~\\";

/// Check if character is a dash (literal or tokenized)
#[inline]
pub fn is_dash(c: char) -> bool {
    c == '-' || c == char_tokens::DASH
}

/// Lexer action codes for first character of token
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LexAct1 {
    Bkslash = 0,
    Comment = 1,
    Newlin = 2,
    Semi = 3,
    Amper = 5,
    Bar = 6,
    Inpar = 7,
    Outpar = 8,
    Inang = 13,
    Outang = 14,
    Other = 15,
}

/// Lexer action codes for subsequent characters in token
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LexAct2 {
    Break = 0,
    Outpar = 1,
    Bar = 2,
    String = 3,
    Inbrack = 4,
    Outbrack = 5,
    Tilde = 6,
    Inpar = 7,
    Inbrace = 8,
    Outbrace = 9,
    Outang = 10,
    Inang = 11,
    Equals = 12,
    Bkslash = 13,
    Quote = 14,
    Dquote = 15,
    Bquote = 16,
    Comma = 17,
    Dash = 18,
    Bang = 19,
    Other = 20,
    Meta = 21,
}

/// Reserved words table
pub static RESERVED_WORDS: &[(&str, LexTok)] = &[
    ("!", LexTok::Bang),
    ("[[", LexTok::Dinbrack),
    ("{", LexTok::Inbrace),
    ("}", LexTok::Outbrace),
    ("case", LexTok::Case),
    ("coproc", LexTok::Coproc),
    ("do", LexTok::Doloop),
    ("done", LexTok::Done),
    ("elif", LexTok::Elif),
    ("else", LexTok::Else),
    ("end", LexTok::Zend),
    ("esac", LexTok::Esac),
    ("fi", LexTok::Fi),
    ("for", LexTok::For),
    ("foreach", LexTok::Foreach),
    ("function", LexTok::Func),
    ("if", LexTok::If),
    ("nocorrect", LexTok::Nocorrect),
    ("repeat", LexTok::Repeat),
    ("select", LexTok::Select),
    ("then", LexTok::Then),
    ("time", LexTok::Time),
    ("until", LexTok::Until),
    ("while", LexTok::While),
];

/// Lookup a reserved word
pub fn lookup_reserved_word(s: &str) -> Option<LexTok> {
    RESERVED_WORDS
        .iter()
        .find(|(word, _)| *word == s)
        .map(|(_, tok)| *tok)
}

/// Typeset-like commands that affect parsing
pub static TYPESET_COMMANDS: &[&str] = &[
    "declare", "export", "float", "integer", "local", "readonly", "typeset",
];

/// Check if a command name is a typeset-like builtin
pub fn is_typeset_command(s: &str) -> bool {
    TYPESET_COMMANDS.contains(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_values() {
        assert_eq!(char_tokens::SNULL as u32, 0x9d);
        assert_eq!(char_tokens::DNULL as u32, 0x9e);
        assert_eq!(char_tokens::BNULL as u32, 0x9f);
    }

    #[test]
    fn test_reserved_words() {
        assert_eq!(lookup_reserved_word("if"), Some(LexTok::If));
        assert_eq!(lookup_reserved_word("then"), Some(LexTok::Then));
        assert_eq!(lookup_reserved_word("notakeyword"), None);
    }

    #[test]
    fn test_redirop() {
        assert!(LexTok::Outang.is_redirop());
        assert!(LexTok::Dinang.is_redirop());
        assert!(!LexTok::If.is_redirop());
        assert!(!LexTok::String.is_redirop());
    }
}
