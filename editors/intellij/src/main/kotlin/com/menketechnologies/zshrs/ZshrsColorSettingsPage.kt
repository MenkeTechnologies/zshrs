package com.menketechnologies.zshrs

import com.intellij.openapi.editor.colors.TextAttributesKey
import com.intellij.openapi.fileTypes.SyntaxHighlighter
import com.intellij.openapi.options.colors.AttributesDescriptor
import com.intellij.openapi.options.colors.ColorDescriptor
import com.intellij.openapi.options.colors.ColorSettingsPage
import javax.swing.Icon

class ZshrsColorSettingsPage : ColorSettingsPage {
    private val attrs = arrayOf(
        AttributesDescriptor("Comments//Line comment (#)", ZshrsColors.COMMENT),
        AttributesDescriptor("Comments//Doc comment (##)", ZshrsColors.DOC_COMMENT),
        AttributesDescriptor("Comments//Shebang (#!)", ZshrsColors.SHEBANG),
        AttributesDescriptor("Strings//Double-quoted (\"…\")", ZshrsColors.STRING_DQ),
        AttributesDescriptor("Strings//Single-quoted ('…')", ZshrsColors.STRING_SQ),
        AttributesDescriptor("Strings//ANSI-C quoting (\$'…')", ZshrsColors.STRING_DOLLAR),
        AttributesDescriptor("Strings//Command substitution backtick (`…`)", ZshrsColors.BACKTICK),
        AttributesDescriptor("Strings//Heredoc", ZshrsColors.HEREDOC),
        AttributesDescriptor("Strings//String escape", ZshrsColors.STRING_ESCAPE),
        AttributesDescriptor("Strings//Printf format specifier (%d %s %10.2f)", ZshrsColors.STRING_FORMAT),
        AttributesDescriptor("Numbers//Integer / float", ZshrsColors.NUMBER),

        AttributesDescriptor("Keywords//Keyword (generic)", ZshrsColors.KEYWORD),
        AttributesDescriptor("Keywords//Control flow (if/then/for/while/case)", ZshrsColors.CONTROL_KEYWORD),
        AttributesDescriptor("Keywords//Declaration (local/typeset/export)", ZshrsColors.DECL_KEYWORD),
        AttributesDescriptor("Keywords//Function declaration (function)", ZshrsColors.FN_KEYWORD),
        AttributesDescriptor("Keywords//Loop control (break/continue/return)", ZshrsColors.LOOP_KEYWORD),
        AttributesDescriptor("Keywords//Modifier (setopt/zstyle/alias/autoload)", ZshrsColors.MODIFIER_KEYWORD),
        AttributesDescriptor("Keywords//I/O (source/eval/exec/echo/print)", ZshrsColors.IO_KEYWORD),

        AttributesDescriptor("Names//Builtin — compat (cd/pwd/hash/echo)", ZshrsColors.BUILTIN),
        AttributesDescriptor("Names//Builtin — zshrs extension (date/cat/zd/async)", ZshrsColors.EXTENSION_BUILTIN),
        AttributesDescriptor("Names//Compsys function (_arguments/_files/_describe)", ZshrsColors.COMPSYS_FUNCTION),
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
        // BEGIN-CANONICAL: color-settings-demo
        // Regenerated from data/grammar/canonical.json by
        // scripts/gen_grammar_intellij.py. Every grammar/syntax category
        // appears at least once so each color slot has a live preview.
        private val DEMO = """
            #!/usr/bin/env zshrs
            ## demo.zsh — every token category for color tweaking.
            ## Doc comments (##) get their own color slot, distinct
            ## from regular # remarks below. Sourced from
            ## data/grammar/canonical.json — regenerate the demo
            ## via scripts/gen_grammar_intellij.py.
            # Regular code comment — uses the plain Line-comment slot.

            # ── options + module loading ──
            setopt EXTENDED_GLOB NULL_GLOB PIPE_FAIL NO_CLOBBER PROMPT_SUBST
            unsetopt BG_NICE
            zmodload zsh/datetime zsh/parameter zsh/zle
            autoload -Uz compinit && compinit -d ~/.cache/zshrs/zcompdump

            # ── decl keywords + special vars ──
            typeset -gA Z_PLUGIN_CACHE
            local -i count=0
            local TIMER=${"$"}EPOCHREALTIME
            integer pid=${"$"}${"$"} ppid=${"$"}PPID
            readonly RPROMPT_BACKUP=${"$"}RPROMPT

            ## Greet the user — attached as `greet`'s function doc.
            function greet() {
                local name="${"$"}{1:-world}"
                print -r -- "hello, ${"$"}name (PID=${"$"}${"$"})"
                return 0
            }

            # ── for / glob qualifiers / param expansion ──
            for f in ~/.zsh/*.zsh(N.r); do
                source "${"$"}f"
                (( count++ ))
                printf '%s\n' "${"$"}{f:t:r}"  # :t = tail, :r = root
            done

            # ── conditionals + arithmetic ──
            if [[ -n "${"$"}HOME" && -d "${"$"}HOME/bin" ]]; then
                path=("${"$"}HOME/bin" ${"$"}path)
            elif (( count == 0 || ${"$"}#argv == 0 )); then
                echo "no plugins" >&2
            fi

            # ── heredoc + pipe + redirect + background ──
            cat <<EOF | grep -E 'foo|bar' &>/tmp/out.log &
            multi-line
            heredoc body
            EOF

            # ── case with all 3 branch terminators ──
            case ${"$"}1 in
                start|run)  greet "${"$"}@" ;;
                stop)       kill %1 ;;&  # ;;& fall-through-and-test
                status)     jobs -l ;|   # ;| fall-through unconditional
                *)          print "usage: ${"$"}0 {start|stop}" ;;
            esac

            # ── parameter flags + special syntaxes ──
            print -- ${"$"}{(L)PATH}            # lowercase
            print -- ${"$"}{(j:,:)path}         # join array with ','
            print -- ${"$"}{(s:/:)PWD}          # split on '/'
            print -- ${"$"}{(P)var}             # indirect ref
            print -- ${"$"}{var//pat/repl}      # replace all
            print -- ${"$"}{var:#pat}           # remove matching

            # ── strings: ANSI-C, single, double, backtick, locale ──
            local ansi=$'tab\there\n'
            local literal='no ${"$"}expand here'
            local interp="expand ${"$"}var here"
            local cmdsub=`uname -m`
            local trans=${"$"}"locale-translated"

            # ── arithmetic command + (( )) + regex + procsub ──
            (( a = 1 + 2 * 3, b = a ** 2 ))
            [[ "${"$"}str" =~ ^[A-Z]+${"$"} ]] && print match
            diff <(sort a.txt) <(sort b.txt) >(tee out.log)

            # ── try/always ──
            {
                might_fail
            } always {
                cleanup_temp_files
            }

            # ── history expansion + word modifiers ──
            !!:gs/foo/bar/  # global substitute on previous cmd
            !$              # last word of previous cmd

            # ── zshrs-exclusive: parallel + AOP ──
            parallel for-each url in "https://example.com/"; do
                curl -s "${"$"}url"
            done
            intercept before 'rm' record-trash

        """.trimIndent()
        // END-CANONICAL: color-settings-demo
    }
}
