package com.menketechnologies.zshrs

import com.intellij.openapi.editor.colors.TextAttributesKey
import com.intellij.openapi.fileTypes.SyntaxHighlighter
import com.intellij.openapi.options.colors.AttributesDescriptor
import com.intellij.openapi.options.colors.ColorDescriptor
import com.intellij.openapi.options.colors.ColorSettingsPage
import javax.swing.Icon

class ZshrsColorSettingsPage : ColorSettingsPage {
    private val attrs = arrayOf(
        AttributesDescriptor("Comments//Line comment", ZshrsColors.COMMENT),
        AttributesDescriptor("Comments//Shebang (#!)", ZshrsColors.SHEBANG),
        AttributesDescriptor("Strings//Double-quoted (\"…\")", ZshrsColors.STRING_DQ),
        AttributesDescriptor("Strings//Single-quoted ('…')", ZshrsColors.STRING_SQ),
        AttributesDescriptor("Strings//ANSI-C quoting (\$'…')", ZshrsColors.STRING_DOLLAR),
        AttributesDescriptor("Strings//Command substitution backtick (`…`)", ZshrsColors.BACKTICK),
        AttributesDescriptor("Strings//Heredoc", ZshrsColors.HEREDOC),
        AttributesDescriptor("Strings//String escape", ZshrsColors.STRING_ESCAPE),
        AttributesDescriptor("Numbers//Integer / float", ZshrsColors.NUMBER),

        AttributesDescriptor("Keywords//Keyword (generic)", ZshrsColors.KEYWORD),
        AttributesDescriptor("Keywords//Control flow (if/then/for/while/case)", ZshrsColors.CONTROL_KEYWORD),
        AttributesDescriptor("Keywords//Declaration (local/typeset/export)", ZshrsColors.DECL_KEYWORD),
        AttributesDescriptor("Keywords//Function declaration (function)", ZshrsColors.FN_KEYWORD),
        AttributesDescriptor("Keywords//Loop control (break/continue/return)", ZshrsColors.LOOP_KEYWORD),
        AttributesDescriptor("Keywords//Modifier (setopt/zstyle/alias/autoload)", ZshrsColors.MODIFIER_KEYWORD),
        AttributesDescriptor("Keywords//I/O (source/eval/exec/echo/print)", ZshrsColors.IO_KEYWORD),

        AttributesDescriptor("Names//Builtin (cd/pwd/hash)", ZshrsColors.BUILTIN),
        AttributesDescriptor("Names//External command call", ZshrsColors.COMMAND),
        AttributesDescriptor("Names//Function declaration", ZshrsColors.FUNCTION_DECL),
        AttributesDescriptor("Names//Identifier", ZshrsColors.IDENTIFIER),
        AttributesDescriptor("Names//Option name (EXTENDED_GLOB)", ZshrsColors.OPTION_NAME),

        AttributesDescriptor("Variables//Sigil (bare \$)", ZshrsColors.SIGIL),
        AttributesDescriptor("Variables//Variable (\$name)", ZshrsColors.VARIABLE),
        AttributesDescriptor("Variables//Special (\$0 \$? \$! \$\$ \$# \$* \$@)", ZshrsColors.SPECIAL_VAR),
        AttributesDescriptor("Variables//Parameter flag ((e) (P) (#))", ZshrsColors.PARAMETER_FLAG),
        AttributesDescriptor("Variables//Env var (UPPER_CASE)", ZshrsColors.ENV_VAR),
        AttributesDescriptor("Variables//Parameter expansion (\${…})", ZshrsColors.PARAM_EXPANSION),

        AttributesDescriptor("Operators//Generic operator", ZshrsColors.OPERATOR),
        AttributesDescriptor("Operators//Assignment (= += -= ?= :=)", ZshrsColors.ASSIGN_OP),
        AttributesDescriptor("Operators//Pipe (| |&)", ZshrsColors.PIPE),
        AttributesDescriptor("Operators//Redirect (> >> < << <<<)", ZshrsColors.REDIRECT),
        AttributesDescriptor("Operators//Logical (&& ||)", ZshrsColors.LOGICAL_OP),
        AttributesDescriptor("Operators//Background (&)", ZshrsColors.BACKGROUND),
        AttributesDescriptor("Operators//Glob (* ? [ ])", ZshrsColors.GLOB),

        AttributesDescriptor("Punctuation//Parentheses ( )", ZshrsColors.PAREN),
        AttributesDescriptor("Punctuation//Braces { }", ZshrsColors.BRACE),
        AttributesDescriptor("Punctuation//Brackets [ ]", ZshrsColors.BRACKET),
        AttributesDescriptor("Punctuation//Comma", ZshrsColors.COMMA),
        AttributesDescriptor("Punctuation//Semicolon", ZshrsColors.SEMICOLON),
        AttributesDescriptor("Punctuation//Case-branch terminator (;; ;;& ;|)", ZshrsColors.DOUBLE_SEMI),

        AttributesDescriptor("Errors//Bad character", ZshrsColors.BAD_CHAR),
    )

    override fun getIcon(): Icon = ZshrsIcons.FILE
    override fun getHighlighter(): SyntaxHighlighter = ZshrsSyntaxHighlighter()
    override fun getDemoText(): String = DEMO
    override fun getAdditionalHighlightingTagToDescriptorMap(): MutableMap<String, TextAttributesKey>? = null
    override fun getAttributeDescriptors(): Array<AttributesDescriptor> = attrs
    override fun getColorDescriptors(): Array<ColorDescriptor> = ColorDescriptor.EMPTY_ARRAY
    override fun getDisplayName(): String = "zshrs"

    companion object {
        private val DEMO = """
            #!/usr/bin/env zshrs
            # zshrs demo — every token category for color tweaking.
            setopt EXTENDED_GLOB NULL_GLOB PIPE_FAIL
            autoload -Uz compinit && compinit -d ~/.cache/zshrs/zcompdump

            typeset -gA Z_PLUGIN_CACHE
            local -i count=0
            local TIMER=${"$"}EPOCHREALTIME

            function greet() {
                local name="${"$"}{1:-world}"
                print -r -- "hello, ${"$"}name (PID=${"$"}${"$"})"
                return 0
            }

            for f in ~/.zsh/*.zsh(N); do
                source "${"$"}f"
                (( count++ ))
            done

            if [[ -n "${"$"}HOME" && -d "${"$"}HOME/bin" ]]; then
                path=("${"$"}HOME/bin" ${"$"}path)
            elif (( count == 0 )); then
                echo "no plugins" >&2
            fi

            cat <<EOF | grep -E 'foo|bar' &>/tmp/out.log &
            multi-line
            heredoc body
            EOF

            case ${"$"}1 in
                start|run) greet "${"$"}@" ;;
                stop)      kill %1 ;;
                *)         print "usage: ${"$"}0 {start|stop}" ;;
            esac
        """.trimIndent()
    }
}
