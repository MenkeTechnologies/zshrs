package com.menketechnologies.zshrs

import com.intellij.psi.TokenType
import com.intellij.psi.tree.IElementType
import org.junit.Assert.*
import org.junit.Test

/**
 * Unit tests for [ZshrsLexer]. These run under `./gradlew test` and
 * exercise the hand-rolled tokenizer that feeds the syntax highlighter
 * before the LSP semantic-tokens response lands.
 */
class ZshrsLexerTest {

    private fun tokens(src: String): List<Pair<IElementType?, String>> {
        val lex = ZshrsLexer()
        lex.start(src, 0, src.length, 0)
        val out = mutableListOf<Pair<IElementType?, String>>()
        while (lex.tokenType != null) {
            val t = lex.tokenType
            val s = src.substring(lex.tokenStart, lex.tokenEnd)
            out += t to s
            lex.advance()
        }
        return out
    }

    private fun nonWs(src: String) = tokens(src).filter { it.first != TokenType.WHITE_SPACE }

    @Test fun `shebang on first line is its own token`() {
        val toks = nonWs("#!/usr/bin/env zshrs\necho hi\n")
        assertEquals(ZshrsTokenTypes.SHEBANG, toks[0].first)
        assertTrue(toks[0].second.startsWith("#!"))
    }

    @Test fun `hash-only comment is a regular line comment`() {
        val toks = nonWs("# just a note\n")
        assertEquals(ZshrsTokenTypes.COMMENT, toks[0].first)
    }

    @Test fun `control keywords classify correctly`() {
        val toks = nonWs("if then else elif fi for while do done case esac")
        for ((tt, _) in toks) assertEquals(ZshrsTokenTypes.CONTROL_KEYWORD, tt)
    }

    @Test fun `declaration keywords classify correctly`() {
        val src = "local typeset declare export readonly integer float"
        for ((tt, _) in nonWs(src)) {
            assertEquals(ZshrsTokenTypes.DECL_KEYWORD, tt)
        }
    }

    @Test fun `modifier keywords classify correctly`() {
        for ((tt, _) in nonWs("alias unalias setopt unsetopt zstyle zmodload autoload")) {
            assertEquals(ZshrsTokenTypes.MODIFIER_KEYWORD, tt)
        }
    }

    @Test fun `builtins are recognized over identifiers`() {
        val src = "cd pwd hash unhash jobs"
        for ((tt, _) in nonWs(src)) {
            assertEquals(ZshrsTokenTypes.BUILTIN, tt)
        }
    }

    @Test fun `dollar identifier is a variable`() {
        val toks = nonWs("echo \$foo")
        assertEquals(ZshrsTokenTypes.VARIABLE, toks[1].first)
        assertEquals("\$foo", toks[1].second)
    }

    @Test fun `dollar all-caps identifier classifies as env var`() {
        val toks = nonWs("echo \$HOME")
        assertEquals(ZshrsTokenTypes.ENV_VAR, toks[1].first)
    }

    @Test fun `special single-char variables are SPECIAL_VAR`() {
        // $?, $!, $$, $#, $*, $@, $-, $_, $0..$9
        val cases = listOf("\$?", "\$!", "\$\$", "\$#", "\$*", "\$@", "\$-", "\$_", "\$0", "\$9")
        for (c in cases) {
            val toks = nonWs(c)
            assertEquals("special-var lookup failed for $c", ZshrsTokenTypes.SPECIAL_VAR, toks[0].first)
        }
    }

    @Test fun `parameter expansion brace block is one token`() {
        val toks = nonWs("\${HOME:-/tmp}")
        assertEquals(ZshrsTokenTypes.PARAM_EXPANSION, toks[0].first)
        assertEquals("\${HOME:-/tmp}", toks[0].second)
    }

    @Test fun `double-quoted string`() {
        val toks = nonWs("echo \"hello world\"")
        assertEquals(ZshrsTokenTypes.STRING_DQ, toks[1].first)
        assertEquals("\"hello world\"", toks[1].second)
    }

    @Test fun `single-quoted string consumes literally`() {
        val toks = nonWs("'a \\\\n b'")
        assertEquals(ZshrsTokenTypes.STRING_SQ, toks[0].first)
    }

    @Test fun `ANSI-C dollar-quoted string`() {
        val toks = nonWs("\$'\\n hi'")
        assertEquals(ZshrsTokenTypes.STRING_DOLLAR, toks[0].first)
    }

    @Test fun `backtick command substitution`() {
        val toks = nonWs("`uname -a`")
        assertEquals(ZshrsTokenTypes.BACKTICK, toks[0].first)
    }

    @Test fun `pipe and logical operators`() {
        val toks = nonWs("a | b && c || d")
        val types = toks.map { it.first }
        assertTrue("expected PIPE present: $types", types.contains(ZshrsTokenTypes.PIPE))
        assertTrue("expected LOGICAL_OP present: $types", types.contains(ZshrsTokenTypes.LOGICAL_OP))
    }

    @Test fun `redirects tokenize`() {
        val toks = nonWs("a > out 2>&1 < in <<EOF\nbody\nEOF\n")
        val types = toks.map { it.first }
        assertTrue("missing REDIRECT: $types", types.contains(ZshrsTokenTypes.REDIRECT))
        assertTrue("missing HEREDOC: $types", types.contains(ZshrsTokenTypes.HEREDOC))
    }

    @Test fun `space between heredoc-op and marker disqualifies the heredoc`() {
        // `<< EOF` (note space) is NOT a heredoc per POSIX — only the `<<` redirect
        val toks = nonWs("cat << EOF\n")
        // We should see the bare `<<` REDIRECT token, not a HEREDOC spanning lines
        assertTrue(toks.any { it.first == ZshrsTokenTypes.REDIRECT && it.second == "<<" })
        assertFalse(toks.any { it.first == ZshrsTokenTypes.HEREDOC })
    }

    @Test fun `here-string operator is a redirect`() {
        val toks = nonWs("read x <<< 'hi'")
        val types = toks.map { it.first }
        assertTrue("missing <<< redirect: $types", types.contains(ZshrsTokenTypes.REDIRECT))
    }

    @Test fun `case branch terminators`() {
        val toks = nonWs("case x in a) :;;& b) :;| c) :;; esac")
        val types = toks.map { it.first }
        assertTrue("expected DOUBLE_SEMI in case: $types", types.contains(ZshrsTokenTypes.DOUBLE_SEMI))
    }

    @Test fun `assignment forms`() {
        val toks = nonWs("a=1 b+=2")
        val types = toks.map { it.first }
        assertTrue("missing ASSIGN_OP: $types", types.contains(ZshrsTokenTypes.ASSIGN_OP))
    }

    @Test fun `numbers tokenize as NUMBER`() {
        val toks = nonWs("count=42")
        assertTrue("no NUMBER found: $toks", toks.any { it.first == ZshrsTokenTypes.NUMBER })
    }

    @Test fun `glob chars are GLOB`() {
        val toks = nonWs("ls *.zsh ?")
        val types = toks.map { it.first }
        assertTrue("missing GLOB: $types", types.contains(ZshrsTokenTypes.GLOB))
    }

    @Test fun `function keyword is FN_KEYWORD`() {
        val toks = nonWs("function f { :; }")
        assertEquals(ZshrsTokenTypes.FN_KEYWORD, toks[0].first)
    }

    @Test fun `backslash-newline is line continuation, not bad character`() {
        // Regression: `\` followed by newline used to fall through to
        // BAD_CHARACTER, painting every shell line continuation red.
        val src = "curl -sS -f \\\n    -H 'X: y'\n"
        val toks = tokens(src)
        assertFalse(
            "line-continuation `\\` should not be BAD_CHARACTER: $toks",
            toks.any { it.first == TokenType.BAD_CHARACTER },
        )
        // The `\<newline>` pair is whitespace (joins lines)
        assertTrue(
            "expected WHITE_SPACE for `\\<newline>`: $toks",
            toks.any { it.first == TokenType.WHITE_SPACE && it.second.contains('\n') && it.second.contains('\\') },
        )
    }

    @Test fun `backslash-escape outside string tokenizes as STRING_ESCAPE`() {
        // Bare `\$`, `\"`, `\(` etc. used to be BAD_CHARACTER.
        val toks = tokens("echo \\\$home")
        assertFalse(
            "bare `\\X` outside string should not be BAD_CHARACTER: $toks",
            toks.any { it.first == TokenType.BAD_CHARACTER },
        )
        assertTrue(
            "expected STRING_ESCAPE token for `\\\$`: $toks",
            toks.any { it.first == ZshrsTokenTypes.STRING_ESCAPE && it.second == "\\\$" },
        )
    }

    @Test fun `escaped brace inside parameter default does not break param expansion`() {
        // Pin: `${1:-{\}}` (zsh idiom for defaulting to literal `{}`) must
        // consume the whole expression as one PARAM_EXPANSION token.
        // Pre-fix the trailing `\` line-continuations on following lines
        // showed red because the lexer mis-balanced braces here.
        val toks = nonWs("\${1:-{\\}}")
        assertEquals(ZshrsTokenTypes.PARAM_EXPANSION, toks[0].first)
        assertEquals("\${1:-{\\}}", toks[0].second)
    }

    @Test fun `multi-line continuation block produces no bad characters`() {
        // The exact pattern from examples/daemon-shell.zsh that triggered
        // the false-positive squiggles: param-default with escaped brace,
        // then a curl invocation chained over four `\<newline>` continuations.
        val src = """
            _daemon_post() {
                local body="${'$'}{1:-{\}}"
                curl -sS -f \
                    -H 'Content-Type: application/json' \
                    --data-raw "${'$'}body" \
                    "${'$'}DAEMON_URL/op"
            }
        """.trimIndent() + "\n"
        val toks = tokens(src)
        assertFalse(
            "real daemon-shell snippet produced BAD_CHARACTER tokens: ${toks.filter { it.first == TokenType.BAD_CHARACTER }}",
            toks.any { it.first == TokenType.BAD_CHARACTER },
        )
    }

    @Test fun `heredoc body is captured to terminator`() {
        val src = "cat <<EOT\nhello\nworld\nEOT\nrest\n"
        val toks = nonWs(src)
        val heredoc = toks.first { it.first == ZshrsTokenTypes.HEREDOC }
        // Body must contain the marker line text
        assertTrue("heredoc didn't span body", heredoc.second.contains("hello"))
        assertTrue("heredoc didn't span body", heredoc.second.contains("world"))
        // After the heredoc, `rest` should still tokenize as an identifier (not consumed)
        val afterRest = toks.last { it.second == "rest" }
        assertEquals(ZshrsTokenTypes.IDENTIFIER, afterRest.first)
    }
}
