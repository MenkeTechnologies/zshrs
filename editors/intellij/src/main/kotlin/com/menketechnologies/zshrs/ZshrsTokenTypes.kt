package com.menketechnologies.zshrs

import com.intellij.psi.tree.IElementType

class ZshrsTokenType(debugName: String) : IElementType(debugName, ZshrsLanguage)

/**
 * Fine-grained zsh token types. Each maps to its own [ZshrsColors] entry so
 * any category can be recolored independently. When adding a token here, also
 * add the matching case in [ZshrsSyntaxHighlighter.getTokenHighlights] and
 * the matching entry in [ZshrsColorSettingsPage.attrs].
 */
object ZshrsTokenTypes {
    // ── Trivia / literals ──────────────────────────────────────────────
    @JvmField val COMMENT = ZshrsTokenType("ZSHRS_COMMENT")
    @JvmField val SHEBANG = ZshrsTokenType("ZSHRS_SHEBANG")
    @JvmField val STRING_DQ = ZshrsTokenType("ZSHRS_STRING_DQ")
    @JvmField val STRING_SQ = ZshrsTokenType("ZSHRS_STRING_SQ")
    @JvmField val STRING_DOLLAR = ZshrsTokenType("ZSHRS_STRING_DOLLAR")   // $'...'
    @JvmField val BACKTICK = ZshrsTokenType("ZSHRS_BACKTICK")
    @JvmField val HEREDOC = ZshrsTokenType("ZSHRS_HEREDOC")
    @JvmField val STRING_ESCAPE = ZshrsTokenType("ZSHRS_STRING_ESCAPE")
    @JvmField val NUMBER = ZshrsTokenType("ZSHRS_NUMBER")

    // ── Keywords ─────────────────────────────────────────────────────────
    @JvmField val KEYWORD = ZshrsTokenType("ZSHRS_KEYWORD")
    @JvmField val CONTROL_KEYWORD = ZshrsTokenType("ZSHRS_CONTROL_KEYWORD")   // if/then/else/fi/for/while/do/done/case/esac
    @JvmField val DECL_KEYWORD = ZshrsTokenType("ZSHRS_DECL_KEYWORD")          // local/typeset/declare/export/readonly/integer/float/array
    @JvmField val FN_KEYWORD = ZshrsTokenType("ZSHRS_FN_KEYWORD")              // function
    @JvmField val LOOP_KEYWORD = ZshrsTokenType("ZSHRS_LOOP_KEYWORD")          // break/continue/return/exit
    @JvmField val MODIFIER_KEYWORD = ZshrsTokenType("ZSHRS_MODIFIER_KEYWORD")  // alias/unalias/setopt/unsetopt/zstyle/zmodload
    @JvmField val IO_KEYWORD = ZshrsTokenType("ZSHRS_IO_KEYWORD")              // source/. /eval/exec/trap

    // ── Builtins / commands ──────────────────────────────────────────────
    @JvmField val BUILTIN = ZshrsTokenType("ZSHRS_BUILTIN")
    @JvmField val COMMAND = ZshrsTokenType("ZSHRS_COMMAND")
    @JvmField val FUNCTION_DECL = ZshrsTokenType("ZSHRS_FUNCTION_DECL")
    @JvmField val IDENTIFIER = ZshrsTokenType("ZSHRS_IDENTIFIER")
    @JvmField val OPTION_NAME = ZshrsTokenType("ZSHRS_OPTION_NAME")            // setopt EXTENDED_GLOB → EXTENDED_GLOB

    // ── Variables ────────────────────────────────────────────────────────
    @JvmField val SIGIL = ZshrsTokenType("ZSHRS_SIGIL")                        // bare `$`
    @JvmField val VARIABLE = ZshrsTokenType("ZSHRS_VARIABLE")                  // $foo / ${foo}
    @JvmField val SPECIAL_VAR = ZshrsTokenType("ZSHRS_SPECIAL_VAR")            // $0 $? $! $$ $* $@ $# $- $_
    @JvmField val PARAMETER_FLAG = ZshrsTokenType("ZSHRS_PARAMETER_FLAG")      // (e), (P), (#), (u) inside ${...}
    @JvmField val ENV_VAR = ZshrsTokenType("ZSHRS_ENV_VAR")                    // ALL_CAPS variables
    @JvmField val PARAM_EXPANSION = ZshrsTokenType("ZSHRS_PARAM_EXPANSION")    // ${...} braces themselves

    // ── Operators ────────────────────────────────────────────────────────
    @JvmField val OPERATOR = ZshrsTokenType("ZSHRS_OPERATOR")
    @JvmField val ASSIGN_OP = ZshrsTokenType("ZSHRS_ASSIGN_OP")                // = += -= := ?=
    @JvmField val PIPE = ZshrsTokenType("ZSHRS_PIPE")                          // | |&
    @JvmField val REDIRECT = ZshrsTokenType("ZSHRS_REDIRECT")                  // > >> < << <<< 2>&1 etc.
    @JvmField val LOGICAL_OP = ZshrsTokenType("ZSHRS_LOGICAL_OP")              // && ||
    @JvmField val BACKGROUND = ZshrsTokenType("ZSHRS_BACKGROUND")              // &
    @JvmField val GLOB = ZshrsTokenType("ZSHRS_GLOB")                          // * ? [...] (#qX) etc.

    // ── Punctuation ──────────────────────────────────────────────────────
    // Split L/R variants so `lang.braceMatcher` can pair them; the umbrella
    // `PAREN`/`BRACE`/`BRACKET` names stay for compat (color slot fallback).
    @JvmField val PAREN = ZshrsTokenType("ZSHRS_PAREN")
    @JvmField val LPAREN = ZshrsTokenType("ZSHRS_LPAREN")
    @JvmField val RPAREN = ZshrsTokenType("ZSHRS_RPAREN")
    @JvmField val BRACE = ZshrsTokenType("ZSHRS_BRACE")
    @JvmField val LBRACE = ZshrsTokenType("ZSHRS_LBRACE")
    @JvmField val RBRACE = ZshrsTokenType("ZSHRS_RBRACE")
    @JvmField val BRACKET = ZshrsTokenType("ZSHRS_BRACKET")
    @JvmField val LBRACKET = ZshrsTokenType("ZSHRS_LBRACKET")
    @JvmField val RBRACKET = ZshrsTokenType("ZSHRS_RBRACKET")
    @JvmField val COMMA = ZshrsTokenType("ZSHRS_COMMA")
    @JvmField val SEMICOLON = ZshrsTokenType("ZSHRS_SEMICOLON")
    @JvmField val DOUBLE_SEMI = ZshrsTokenType("ZSHRS_DOUBLE_SEMI")            // ;; ;;& ;| (case branches)

    // ── Errors ───────────────────────────────────────────────────────────
    @JvmField val BAD = ZshrsTokenType("ZSHRS_BAD")
}
