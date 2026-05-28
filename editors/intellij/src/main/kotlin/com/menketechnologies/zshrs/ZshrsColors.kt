package com.menketechnologies.zshrs

import com.intellij.openapi.editor.DefaultLanguageHighlighterColors as Defaults
import com.intellij.openapi.editor.HighlighterColors
import com.intellij.openapi.editor.colors.TextAttributesKey

/**
 * Stable, plugin-owned [TextAttributesKey]s for every zsh token category.
 * Each key inherits a sensible default but lives in its own `ZSHRS_*`
 * namespace so users can rebind any of them in
 * *Settings → Editor → Color Scheme → zshrs* without affecting the rest of
 * the IDE.
 */
object ZshrsColors {
    @JvmField val COMMENT = mk("ZSHRS_COMMENT", Defaults.LINE_COMMENT)
    /// `##` doc comments. Distinct slot from regular `#` comments —
    /// users typically pick a stronger color (e.g. brighter cyan) so
    /// the LSP-attached docstring lines stand out from inline code
    /// remarks. Defaults to `DOC_COMMENT` (IntelliJ's reserved
    /// JavaDoc-style slot) so themes that style javadocs get an
    /// out-of-the-box highlight.
    @JvmField val DOC_COMMENT = mk("ZSHRS_DOC_COMMENT", Defaults.DOC_COMMENT)
    @JvmField val SHEBANG = mk("ZSHRS_SHEBANG", Defaults.LINE_COMMENT)
    @JvmField val STRING_DQ = mk("ZSHRS_STRING_DQ", Defaults.STRING)
    @JvmField val STRING_SQ = mk("ZSHRS_STRING_SQ", Defaults.STRING)
    @JvmField val STRING_DOLLAR = mk("ZSHRS_STRING_DOLLAR", Defaults.STRING)
    @JvmField val BACKTICK = mk("ZSHRS_BACKTICK", Defaults.STRING)
    @JvmField val HEREDOC = mk("ZSHRS_HEREDOC", Defaults.STRING)
    @JvmField val STRING_ESCAPE = mk("ZSHRS_STRING_ESCAPE", Defaults.VALID_STRING_ESCAPE)
    /// Printf-style format spec inside a dq-string — `%d` / `%s` /
    /// `%10.2f` / `%-15s` / `%%`. Colored distinctly from the
    /// surrounding string literal so `printf "%d %s\n" 42 hi` shows
    /// the format placeholders standing out from the prose. Mirrors
    /// strykelang's STRING_FORMAT color slot.
    @JvmField val STRING_FORMAT = mk("ZSHRS_STRING_FORMAT", Defaults.VALID_STRING_ESCAPE)
    @JvmField val NUMBER = mk("ZSHRS_NUMBER", Defaults.NUMBER)

    @JvmField val KEYWORD = mk("ZSHRS_KEYWORD", Defaults.KEYWORD)
    @JvmField val CONTROL_KEYWORD = mk("ZSHRS_CONTROL_KEYWORD", Defaults.KEYWORD)
    @JvmField val DECL_KEYWORD = mk("ZSHRS_DECL_KEYWORD", Defaults.KEYWORD)
    @JvmField val FN_KEYWORD = mk("ZSHRS_FN_KEYWORD", Defaults.KEYWORD)
    @JvmField val LOOP_KEYWORD = mk("ZSHRS_LOOP_KEYWORD", Defaults.KEYWORD)
    @JvmField val MODIFIER_KEYWORD = mk("ZSHRS_MODIFIER_KEYWORD", Defaults.KEYWORD)
    @JvmField val IO_KEYWORD = mk("ZSHRS_IO_KEYWORD", Defaults.KEYWORD)

    @JvmField val BUILTIN = mk("ZSHRS_BUILTIN", Defaults.STATIC_METHOD)
    @JvmField val COMMAND = mk("ZSHRS_COMMAND", Defaults.FUNCTION_CALL)
    @JvmField val FUNCTION_DECL = mk("ZSHRS_FUNCTION_DECL", Defaults.FUNCTION_DECLARATION)
    @JvmField val IDENTIFIER = mk("ZSHRS_IDENTIFIER", Defaults.IDENTIFIER)
    @JvmField val OPTION_NAME = mk("ZSHRS_OPTION_NAME", Defaults.CONSTANT)
    // Distinct slots for zshrs-only builtins (coreutils drop-ins,
    // async/await/barrier, doctor, intercept, daemon `z*` …) and for
    // the compsys `_arguments` / `_files` / `_describe` family. Drive
    // visual separation between compat / extension / compsys so users
    // can spot at a glance which class a name belongs to. Fallbacks
    // mirror BUILTIN so the default scheme stays readable.
    @JvmField val EXTENSION_BUILTIN = mk("ZSHRS_EXTENSION_BUILTIN", Defaults.STATIC_METHOD)
    @JvmField val COMPSYS_FUNCTION = mk("ZSHRS_COMPSYS_FUNCTION", Defaults.STATIC_METHOD)

    @JvmField val SIGIL = mk("ZSHRS_SIGIL", Defaults.OPERATION_SIGN)
    @JvmField val VARIABLE = mk("ZSHRS_VARIABLE", Defaults.LOCAL_VARIABLE)
    @JvmField val SPECIAL_VAR = mk("ZSHRS_SPECIAL_VAR", Defaults.PREDEFINED_SYMBOL)
    @JvmField val PARAMETER_FLAG = mk("ZSHRS_PARAMETER_FLAG", Defaults.METADATA)
    @JvmField val ENV_VAR = mk("ZSHRS_ENV_VAR", Defaults.GLOBAL_VARIABLE)
    @JvmField val PARAM_EXPANSION = mk("ZSHRS_PARAM_EXPANSION", Defaults.BRACES)

    @JvmField val OPERATOR = mk("ZSHRS_OPERATOR", Defaults.OPERATION_SIGN)
    @JvmField val ASSIGN_OP = mk("ZSHRS_ASSIGN_OP", Defaults.OPERATION_SIGN)
    @JvmField val PIPE = mk("ZSHRS_PIPE", Defaults.LABEL)
    @JvmField val REDIRECT = mk("ZSHRS_REDIRECT", Defaults.LABEL)
    @JvmField val LOGICAL_OP = mk("ZSHRS_LOGICAL_OP", Defaults.OPERATION_SIGN)
    @JvmField val BACKGROUND = mk("ZSHRS_BACKGROUND", Defaults.OPERATION_SIGN)
    @JvmField val GLOB = mk("ZSHRS_GLOB", Defaults.MARKUP_ATTRIBUTE)

    @JvmField val PAREN = mk("ZSHRS_PAREN", Defaults.PARENTHESES)
    @JvmField val BRACE = mk("ZSHRS_BRACE", Defaults.BRACES)
    @JvmField val BRACKET = mk("ZSHRS_BRACKET", Defaults.BRACKETS)
    @JvmField val COMMA = mk("ZSHRS_COMMA", Defaults.COMMA)
    @JvmField val SEMICOLON = mk("ZSHRS_SEMICOLON", Defaults.SEMICOLON)
    @JvmField val DOUBLE_SEMI = mk("ZSHRS_DOUBLE_SEMI", Defaults.SEMICOLON)

    @JvmField val BAD_CHAR = mk("ZSHRS_BAD_CHAR", HighlighterColors.BAD_CHARACTER)

    private fun mk(name: String, fallback: TextAttributesKey): TextAttributesKey =
        TextAttributesKey.createTextAttributesKey(name, fallback)
}
