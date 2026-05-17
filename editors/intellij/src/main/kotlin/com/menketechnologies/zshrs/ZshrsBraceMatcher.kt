package com.menketechnologies.zshrs

import com.intellij.lang.BracePair
import com.intellij.lang.PairedBraceMatcher
import com.intellij.psi.PsiFile
import com.intellij.psi.tree.IElementType

/**
 * Brace pairing for zsh. Powers auto-insertion of `)`/`}`/`]` when
 * typing `(`/`{`/`[`, AND structural brace highlighting when the cursor
 * sits next to a paired delimiter.
 */
class ZshrsBraceMatcher : PairedBraceMatcher {
    private val pairs = arrayOf(
        BracePair(ZshrsTokenTypes.LPAREN, ZshrsTokenTypes.RPAREN, false),
        BracePair(ZshrsTokenTypes.LBRACE, ZshrsTokenTypes.RBRACE, true),
        BracePair(ZshrsTokenTypes.LBRACKET, ZshrsTokenTypes.RBRACKET, false),
    )

    override fun getPairs(): Array<BracePair> = pairs

    override fun isPairedBracesAllowedBeforeType(
        lbraceType: IElementType,
        contextType: IElementType?,
    ): Boolean = true

    override fun getCodeConstructStart(file: PsiFile?, openingBraceOffset: Int): Int =
        openingBraceOffset
}
