package com.menketechnologies.zshrs

import com.intellij.lexer.Lexer
import com.intellij.openapi.editor.colors.TextAttributesKey
import com.intellij.openapi.fileTypes.SyntaxHighlighter
import com.intellij.openapi.fileTypes.SyntaxHighlighterBase
import com.intellij.openapi.fileTypes.SyntaxHighlighterFactory
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.psi.TokenType
import com.intellij.psi.tree.IElementType

class ZshrsSyntaxHighlighter : SyntaxHighlighterBase() {
    override fun getHighlightingLexer(): Lexer = ZshrsLexer()

    override fun getTokenHighlights(type: IElementType): Array<TextAttributesKey> {
        val key: TextAttributesKey? = when (type) {
            ZshrsTokenTypes.COMMENT -> ZshrsColors.COMMENT
            ZshrsTokenTypes.SHEBANG -> ZshrsColors.SHEBANG
            ZshrsTokenTypes.STRING_DQ -> ZshrsColors.STRING_DQ
            ZshrsTokenTypes.STRING_SQ -> ZshrsColors.STRING_SQ
            ZshrsTokenTypes.STRING_DOLLAR -> ZshrsColors.STRING_DOLLAR
            ZshrsTokenTypes.BACKTICK -> ZshrsColors.BACKTICK
            ZshrsTokenTypes.HEREDOC -> ZshrsColors.HEREDOC
            ZshrsTokenTypes.STRING_ESCAPE -> ZshrsColors.STRING_ESCAPE
            ZshrsTokenTypes.NUMBER -> ZshrsColors.NUMBER

            ZshrsTokenTypes.KEYWORD -> ZshrsColors.KEYWORD
            ZshrsTokenTypes.CONTROL_KEYWORD -> ZshrsColors.CONTROL_KEYWORD
            ZshrsTokenTypes.DECL_KEYWORD -> ZshrsColors.DECL_KEYWORD
            ZshrsTokenTypes.FN_KEYWORD -> ZshrsColors.FN_KEYWORD
            ZshrsTokenTypes.LOOP_KEYWORD -> ZshrsColors.LOOP_KEYWORD
            ZshrsTokenTypes.MODIFIER_KEYWORD -> ZshrsColors.MODIFIER_KEYWORD
            ZshrsTokenTypes.IO_KEYWORD -> ZshrsColors.IO_KEYWORD

            ZshrsTokenTypes.BUILTIN -> ZshrsColors.BUILTIN
            ZshrsTokenTypes.EXTENSION_BUILTIN -> ZshrsColors.EXTENSION_BUILTIN
            ZshrsTokenTypes.COMPSYS_FUNCTION -> ZshrsColors.COMPSYS_FUNCTION
            ZshrsTokenTypes.COMMAND -> ZshrsColors.COMMAND
            ZshrsTokenTypes.FUNCTION_DECL -> ZshrsColors.FUNCTION_DECL
            ZshrsTokenTypes.IDENTIFIER -> ZshrsColors.IDENTIFIER
            ZshrsTokenTypes.OPTION_NAME -> ZshrsColors.OPTION_NAME

            ZshrsTokenTypes.SIGIL -> ZshrsColors.SIGIL
            ZshrsTokenTypes.VARIABLE -> ZshrsColors.VARIABLE
            ZshrsTokenTypes.SPECIAL_VAR -> ZshrsColors.SPECIAL_VAR
            ZshrsTokenTypes.PARAMETER_FLAG -> ZshrsColors.PARAMETER_FLAG
            ZshrsTokenTypes.ENV_VAR -> ZshrsColors.ENV_VAR
            ZshrsTokenTypes.PARAM_EXPANSION -> ZshrsColors.PARAM_EXPANSION

            ZshrsTokenTypes.OPERATOR -> ZshrsColors.OPERATOR
            ZshrsTokenTypes.ASSIGN_OP -> ZshrsColors.ASSIGN_OP
            ZshrsTokenTypes.PIPE -> ZshrsColors.PIPE
            ZshrsTokenTypes.REDIRECT -> ZshrsColors.REDIRECT
            ZshrsTokenTypes.LOGICAL_OP -> ZshrsColors.LOGICAL_OP
            ZshrsTokenTypes.BACKGROUND -> ZshrsColors.BACKGROUND
            ZshrsTokenTypes.GLOB -> ZshrsColors.GLOB

            ZshrsTokenTypes.PAREN -> ZshrsColors.PAREN
            ZshrsTokenTypes.LPAREN -> ZshrsColors.PAREN
            ZshrsTokenTypes.RPAREN -> ZshrsColors.PAREN
            ZshrsTokenTypes.BRACE -> ZshrsColors.BRACE
            ZshrsTokenTypes.LBRACE -> ZshrsColors.BRACE
            ZshrsTokenTypes.RBRACE -> ZshrsColors.BRACE
            ZshrsTokenTypes.BRACKET -> ZshrsColors.BRACKET
            ZshrsTokenTypes.LBRACKET -> ZshrsColors.BRACKET
            ZshrsTokenTypes.RBRACKET -> ZshrsColors.BRACKET
            ZshrsTokenTypes.COMMA -> ZshrsColors.COMMA
            ZshrsTokenTypes.SEMICOLON -> ZshrsColors.SEMICOLON
            ZshrsTokenTypes.DOUBLE_SEMI -> ZshrsColors.DOUBLE_SEMI

            TokenType.BAD_CHARACTER -> ZshrsColors.BAD_CHAR
            else -> null
        }
        return if (key == null) emptyArray() else arrayOf(key)
    }
}

class ZshrsSyntaxHighlighterFactory : SyntaxHighlighterFactory() {
    override fun getSyntaxHighlighter(project: Project?, virtualFile: VirtualFile?): SyntaxHighlighter =
        ZshrsSyntaxHighlighter()
}
