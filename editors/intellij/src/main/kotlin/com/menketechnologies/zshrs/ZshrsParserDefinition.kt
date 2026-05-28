package com.menketechnologies.zshrs

import com.intellij.lang.ASTNode
import com.intellij.lang.ParserDefinition
import com.intellij.lang.PsiBuilder
import com.intellij.lang.PsiParser
import com.intellij.lexer.Lexer
import com.intellij.openapi.fileTypes.FileType
import com.intellij.openapi.project.Project
import com.intellij.psi.FileViewProvider
import com.intellij.psi.PsiElement
import com.intellij.psi.PsiFile
import com.intellij.psi.impl.source.tree.LeafPsiElement
import com.intellij.psi.tree.IFileElementType
import com.intellij.psi.tree.TokenSet

/**
 * Minimal parser definition for `.zsh` / `.zshrc` / etc. Provides
 * IntelliJ with a real `PsiFile` so the platform's keymap-driven
 * actions (Cmd-/ comment, brace-matcher cursor highlight, refactoring
 * keymaps, code-actions surfaced via the LSP) have a PSI to anchor to.
 *
 * Flat AST — every lexer token becomes a top-level leaf node. We don't
 * ship a real recursive-descent parser here because the zshrs LSP
 * already provides diagnostics, semantic tokens, refactorings, and
 * folding server-side; the PSI just needs to *exist*.
 */
class ZshrsParserDefinition : ParserDefinition {
    override fun createLexer(project: Project?): Lexer = ZshrsLexer()
    override fun createParser(project: Project?): PsiParser = ZshrsFlatParser()
    override fun getFileNodeType(): IFileElementType = FILE

    override fun getCommentTokens(): TokenSet =
        TokenSet.create(
            ZshrsTokenTypes.COMMENT,
            ZshrsTokenTypes.DOC_COMMENT,
            ZshrsTokenTypes.SHEBANG,
        )

    override fun getStringLiteralElements(): TokenSet = TokenSet.create(
        ZshrsTokenTypes.STRING_DQ,
        ZshrsTokenTypes.STRING_SQ,
        ZshrsTokenTypes.STRING_DOLLAR,
        ZshrsTokenTypes.BACKTICK,
        ZshrsTokenTypes.HEREDOC,
    )

    override fun createFile(viewProvider: FileViewProvider): PsiFile = ZshrsPsiFile(viewProvider)

    override fun createElement(node: ASTNode): PsiElement = LeafPsiElement(node.elementType, node.text)

    companion object {
        val FILE: IFileElementType = IFileElementType("ZSHRS_FILE", ZshrsLanguage)
    }
}

private class ZshrsFlatParser : PsiParser {
    override fun parse(root: com.intellij.psi.tree.IElementType, builder: PsiBuilder): ASTNode {
        val rootMarker = builder.mark()
        while (!builder.eof()) builder.advanceLexer()
        rootMarker.done(root)
        return builder.treeBuilt
    }
}

class ZshrsPsiFile(viewProvider: FileViewProvider) :
    com.intellij.extapi.psi.PsiFileBase(viewProvider, ZshrsLanguage) {
    override fun getFileType(): FileType = ZshrsFileType
    override fun toString(): String = "zshrs File"
}
