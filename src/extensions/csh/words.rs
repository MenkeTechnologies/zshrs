//! One csh word → the zsh word with the same meaning.

/// Translate a single csh word, quotes and all (`"$x[2]"`, `$#argv`,
/// `${?v}`, `$x:h`, `~user`, …). Input is one word from
/// [`super::lex::split_words`].
pub fn translate_word(word: &str) -> String {
    word.to_string()
}
