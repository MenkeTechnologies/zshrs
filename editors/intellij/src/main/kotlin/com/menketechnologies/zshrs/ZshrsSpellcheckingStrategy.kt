package com.menketechnologies.zshrs

import com.intellij.psi.PsiElement
import com.intellij.spellchecker.tokenizer.SpellcheckingStrategy
import com.intellij.spellchecker.tokenizer.Tokenizer

/**
 * Disable the platform's `TypoInspection` for shell tokens.
 *
 * The default platform behavior is to spell-check every string-literal /
 * comment-like token via the built-in `TextTokenizer`. In a shell script
 * that flags every non-English identifier, every path, every C-function /
 * file reference (`Src/glob.c bracecomplete`, `paramsubst`, `${(j:x:)arr}`),
 * and every banner divider as a typo. Users see the red squiggle wave on
 * `bracecomplete`, `paramsubst`, etc., none of which are typos.
 *
 * Strategy: return `EMPTY_TOKENIZER` for STRING_DQ / STRING_SQ /
 * STRING_DOLLAR / BACKTICK / HEREDOC / COMMENT / DOC_COMMENT / SHEBANG,
 * and any literal-text-bearing token where typo flagging is just noise.
 * The token still renders with its color from [ZshrsSyntaxHighlighter];
 * only the spell-check pass is suppressed.
 *
 * Variables, identifiers, and command names are NOT suppressed — those
 * are where a real typo *would* matter (`functino` vs `function`), and
 * the platform's word splitter already plays nicely with camel/snake.
 */
class ZshrsSpellcheckingStrategy : SpellcheckingStrategy() {
    override fun getTokenizer(element: PsiElement): Tokenizer<*> {
        val node = element.node ?: return super.getTokenizer(element)
        return when (node.elementType) {
            ZshrsTokenTypes.STRING_DQ,
            ZshrsTokenTypes.STRING_SQ,
            ZshrsTokenTypes.STRING_DOLLAR,
            ZshrsTokenTypes.BACKTICK,
            ZshrsTokenTypes.HEREDOC,
            ZshrsTokenTypes.STRING_ESCAPE,
            ZshrsTokenTypes.STRING_FORMAT,
            ZshrsTokenTypes.COMMENT,
            ZshrsTokenTypes.DOC_COMMENT,
            ZshrsTokenTypes.SHEBANG -> EMPTY_TOKENIZER
            else -> super.getTokenizer(element)
        }
    }
}
